//! wait 工具：可被事件与用户打断的定时等待，替代 sleep（决策 D21）。
//!
//! 语义（docs/arch.md §7 D21）：睡满 `duration_ms` 正常返回；等待期间出现新事件——
//! 后台任务完成通知 / 子代理终态报告 / 运行中引导——提前返回并在结果里说明被什么唤醒
//! （事件正文仍走既有唯一注入点，本工具不携带正文）；用户可在界面「跳过等待」，
//! run 继续推进，模型立即去核实等待目的。
//!
//! 真实的等待在 Runner 的异步分支里（`craft-agent::runner::run_wait_tool`）：普通工具
//! 跑在 spawn_blocking 里，取消令牌与事件队列都无法中断它。本模块的 `execute` 只是
//! 注册表直呼时的薄壳说明（与 `agent.rs` 协议工具同构）。

use serde_json::json;

use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

/// wait 时长下限（毫秒）：短于 1 秒的等待没有消费者。
pub const WAIT_MIN_MS: i64 = 1_000;
/// wait 时长上限（毫秒）：与 shell 前台预算上限、agent_wait 上限对齐。
pub const WAIT_MAX_MS: i64 = 600_000;
/// wait 默认时长（毫秒）：够一次限流退避或服务就绪，不至于钉住会话太久。
pub const WAIT_DEFAULT_MS: i64 = 30_000;
/// purpose 参数长度上限：与 agent_spawn 的 description 同预算。
pub const PURPOSE_MAX_CHARS: usize = 120;

/// 钳制等待时长：缺省取默认值，越界钳到上下限（模型可自行纠正，不报错——与 agent_wait 同口径）。
pub fn clamp_wait_duration(ms: Option<i64>) -> i64 {
    match ms {
        Some(v) => v.clamp(WAIT_MIN_MS, WAIT_MAX_MS),
        None => WAIT_DEFAULT_MS,
    }
}

/// 截断 purpose 到预算内（字符级截断，避免劈开多字节字符）。
pub fn clamp_purpose(purpose: &str) -> String {
    purpose.trim().chars().take(PURPOSE_MAX_CHARS).collect()
}

pub struct Wait;

impl Wait {
    pub const NAME: &'static str = "wait";

    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: Self::NAME.into(),
            description: "等待一段固定时间（可被打断的 sleep 替代）。等待期间出现新事件——后台任务完成、\
                          子代理回报、用户追加输入——会提前唤醒，结果会说明被什么唤醒；用户也可能在界面上跳过等待，\
                          此时请立即继续核实你等待的状态。只用于有明确目的的短暂等待（限流退避、等服务就绪、重试间隔）；\
                          不要把本工具当轮询节奏器反复调用，等待后台任务不要用本工具（结束本轮等通知即可）。\
                          本工具不等子代理——那是 agent_wait。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "duration_ms": {
                        "type": "integer",
                        "minimum": WAIT_MIN_MS,
                        "maximum": WAIT_MAX_MS,
                        "description": format!("等待时长（毫秒），{}~{}，默认 {}", WAIT_MIN_MS, WAIT_MAX_MS, WAIT_DEFAULT_MS),
                    },
                    "purpose": {
                        "type": "string",
                        "description": "可选：一句话说明在等什么（如「等 dev server 就绪」）；用于界面显示与被跳过时的继续引导",
                    },
                },
            }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "等待".into(),
            summary_arg: Some("purpose".into()),
            summary_text: Some("定时等待".into()),
        }
    }
}

impl Tool for Wait {
    fn spec(&self) -> ToolSpec {
        Wait::spec()
    }

    fn execute(&self, _ctx: &ToolCtx) -> ToolResult {
        // 真实等待在 Runner 的异步分支（模块头说明了为什么不能在这里睡）。
        ToolResult::ok("wait 的真实等待由会话循环驱动（可被事件与用户打断）；注册表直呼不执行等待。")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::Value;

    use craft_store::db::Db;

    use super::*;

    fn make_ctx(name: &str, args: Value) -> ToolCtx {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cw-wait-tool-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open(&dir.join("t.db")).unwrap());
        ToolCtx {
            name: name.into(),
            args,
            target: String::new(),
            display: String::new(),
            workspace: String::new(),
            roots: vec![],
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: Arc::new(crate::registry::ToolEnv {
                backup_dir: String::new(),
                data_dir: String::new(),
                db,
                shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()),
                proxy_config: crate::registry::direct_proxy_config(),
            }),
        }
    }

    #[test]
    fn wait_spec_metadata() {
        let spec = Wait::spec();
        assert_eq!(spec.name, "wait");
        assert_eq!(spec.title, "等待");
        assert_eq!(spec.risk, PolicyRisk::R0);
        assert!(!spec.work, "Work 模式不可用");
        assert!(!spec.mutates);
        assert!(spec.path_arg.is_none() && spec.command_arg.is_none());
        // 边界文案：与 agent_wait / 后台任务划清，且不得出现「后台子代理」混称
        assert!(spec.description.contains("不要把本工具当轮询节奏器"));
        assert!(spec.description.contains("等待后台任务不要用本工具"));
        assert!(spec.description.contains("agent_wait"));
        assert!(!spec.description.contains("后台子代理"));
    }

    #[test]
    fn wait_duration_clamped_to_bounds() {
        assert_eq!(clamp_wait_duration(None), WAIT_DEFAULT_MS);
        assert_eq!(clamp_wait_duration(Some(0)), WAIT_MIN_MS);
        assert_eq!(clamp_wait_duration(Some(-5)), WAIT_MIN_MS);
        assert_eq!(clamp_wait_duration(Some(10_000_000)), WAIT_MAX_MS);
        assert_eq!(clamp_wait_duration(Some(5_000)), 5_000);
    }

    #[test]
    fn wait_purpose_trimmed_and_truncated() {
        assert_eq!(clamp_purpose("  等 dev server 就绪  "), "等 dev server 就绪");
        let long = "汉".repeat(PURPOSE_MAX_CHARS + 10);
        assert_eq!(clamp_purpose(&long).chars().count(), PURPOSE_MAX_CHARS);
    }

    #[test]
    fn wait_thin_shell_does_not_block() {
        let started = std::time::Instant::now();
        let result = Wait.execute(&make_ctx(Wait::NAME, json!({ "duration_ms": 60_000 })));
        assert!(!result.is_error);
        assert!(started.elapsed() < std::time::Duration::from_millis(500), "薄壳不得真的等待");
        assert!(result.output.contains("会话循环"), "应说明真实等待的位置：{}", result.output);
    }
}
