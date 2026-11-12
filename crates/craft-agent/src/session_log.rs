//! 会话日志：JSONL 追加器（open-append-close，无长驻句柄）+ 等级门控。
//!
//! 目的：模型流与运行循环的观测证据——排查「回复截断」「界面停滞」时，
//! 与事件表落库数据对照还原后端时间线（流怎么结束的、finish_reason 是什么、
//! 网关给的 token 数与落库字节是否对得上、任务是否无声死亡）。
//!
//! 写入模式权衡：单次 open+write+close 约 10~100μs，info 级每 run 约 30~100 行、
//! debug 级约每 2 秒 1 行聚合行（不逐 chunk 记），占用主流程时间可忽略；
//! 换取崩溃安全（无用户态缓冲）、文件永不被锁（可随时删除/外发）、
//! 无句柄生命周期管理（run panic / 取消 / 进程被杀路径无需清理）。
//! 不做 fsync：断电/崩溃最多丢尾部数行，可接受（审计事实走 SQLite 事件表）。
//!
//! 文件划分：`<thread-id>.log` 会话内 run 时间线；`app.log` 共享应用事件
//! （启动恢复、无会话归属的广播失败）；`model.log` 共享提供商层事件（重试等）。
//! 安全：不写密钥与请求头；内容快照限长；写入失败降级 stderr，不影响主流程。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

/// 等级取值（与设置 logs.level 字符串一一对应）。
pub const LEVEL_OFF: u8 = 0;
pub const LEVEL_ERROR: u8 = 1;
pub const LEVEL_INFO: u8 = 2;
pub const LEVEL_DEBUG: u8 = 3;

/// 字符串 → 等级（未知值回落 info，与设置读取侧兜底一致）。
pub fn parse_level(s: &str) -> u8 {
    match s {
        "off" => LEVEL_OFF,
        "error" => LEVEL_ERROR,
        "debug" => LEVEL_DEBUG,
        _ => LEVEL_INFO,
    }
}

/// 等级 → 字符串（设置界面与日志行自描述用）。
pub fn level_name(level: u8) -> &'static str {
    match level {
        LEVEL_OFF => "off",
        LEVEL_ERROR => "error",
        LEVEL_INFO => "info",
        LEVEL_DEBUG => "debug",
        _ => "info",
    }
}

/// 会话日志器：目录 + 等级缓存（设置保存侧更新，写入侧只读）。
#[derive(Clone)]
pub struct SessionLog {
    dir: PathBuf,
    level: Arc<AtomicU8>,
}

impl SessionLog {
    /// 以数据目录构造（日志固定落在 `<data_dir>/logs/`）。
    pub fn new(data_dir: &std::path::Path, level: Arc<AtomicU8>) -> Self {
        Self {
            dir: data_dir.join("logs"),
            level,
        }
    }

    /// 当前等级（设置界面展示与测试用）。
    pub fn level(&self) -> u8 {
        self.level.load(Ordering::Relaxed)
    }

    /// 写一行 JSONL：`{ts, level, event, ...fields}`。
    /// 等级高于当前设置则跳过；文件按名追加（会话日志 / 共享日志同一入口）。
    pub fn write(&self, file: &str, level: u8, event: &str, fields: Value) {
        if level > self.level.load(Ordering::Relaxed) {
            return;
        }
        let mut line = json!({
            "ts": now_millis(),
            "level": level_name(level),
            "event": event,
        });
        if let (Some(obj), Some(extra)) = (line.as_object_mut(), fields.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        if let Err(err) = std::fs::create_dir_all(&self.dir).and_then(|()| {
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(file))?;
            writeln!(f, "{line}")
        }) {
            eprintln!("craftwork: 会话日志写入失败 {}: {err}", self.dir.join(file).display());
        }
    }

    /// 共享应用日志（app.log）：启动恢复、无会话归属的广播失败等。
    pub fn write_app(&self, level: u8, event: &str, fields: Value) {
        self.write("app.log", level, event, fields);
    }

    /// 共享提供商层日志（model.log）：重试等经工厂回调进入的事件。
    pub fn write_model(&self, level: u8, event: &str, message: &str) {
        self.write("model.log", level, event, json!({ "message": message }));
    }

    /// 会话日志（<thread-id>.log）：run 循环时间线。
    pub fn write_thread(&self, thread_id: &str, level: u8, event: &str, fields: Value) {
        self.write(&format!("{thread_id}.log"), level, event, fields);
    }
}

/// 尾部快照：取末尾 `limit_chars` 个字符（字符边界安全），供日志记录长文本尾部。
pub fn tail_snapshot(s: &str, limit_chars: usize) -> String {
    if s.chars().count() <= limit_chars {
        return s.to_string();
    }
    let skip = s.chars().count() - limit_chars;
    s.chars().skip(skip).collect()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parse_and_name_round_trip() {
        for (s, level) in [("off", LEVEL_OFF), ("error", LEVEL_ERROR), ("info", LEVEL_INFO), ("debug", LEVEL_DEBUG)] {
            assert_eq!(parse_level(s), level);
            assert_eq!(level_name(level), s);
        }
        // 未知值回落 info
        assert_eq!(parse_level("whatever"), LEVEL_INFO);
    }

    #[test]
    fn write_respects_level_gate_and_appends_jsonl() {
        let dir = std::env::temp_dir().join(format!("cw-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = SessionLog::new(&dir, Arc::new(AtomicU8::new(LEVEL_INFO)));

        log.write_thread("th_x", LEVEL_DEBUG, "debug_only", json!({ "n": 1 }));
        log.write_thread("th_x", LEVEL_INFO, "info_line", json!({ "n": 2 }));
        log.write_thread("th_x", LEVEL_ERROR, "error_line", json!({ "n": 3 }));

        let content = std::fs::read_to_string(dir.join("logs").join("th_x.log")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "debug 行应被等级门控跳过");
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["event"], "info_line");
        assert_eq!(first["level"], "info");
        assert_eq!(first["n"], 2);
        assert!(first["ts"].is_i64());
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["event"], "error_line");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_change_takes_effect_immediately() {
        let dir = std::env::temp_dir().join(format!("cw-log-level-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let level = Arc::new(AtomicU8::new(LEVEL_OFF));
        let log = SessionLog::new(&dir, Arc::clone(&level));

        log.write_thread("th_y", LEVEL_ERROR, "off_silent", json!({}));
        let path = dir.join("logs").join("th_y.log");
        assert!(!path.exists(), "off 时不落文件");

        level.store(LEVEL_ERROR, Ordering::Relaxed);
        log.write_thread("th_y", LEVEL_ERROR, "now_visible", json!({}));
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_snapshot_takes_last_chars_and_handles_short_input() {
        assert_eq!(tail_snapshot("abcdef", 3), "def");
        assert_eq!(tail_snapshot("短", 8), "短");
        let long = "一二三五四六七八九十";
        assert_eq!(tail_snapshot(long, 2), "九十");
        assert_eq!(tail_snapshot("", 4), "");
    }
}
