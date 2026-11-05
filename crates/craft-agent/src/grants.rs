//! 授权缓存（对应策略层的「本会话 / 本工作区允许」语义）。
//! 会话授权只在内存（进程退出失效）；工作区授权写设置（重启有效）。
//! 键：精确 `tool|target`；前缀 `tool|prefix:<prefix>`（仅 shell_exec 使用）。

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::json;

use craft_store::db::Db;
use craft_store::records::{get_setting, set_setting};

const WORKSPACE_GRANTS_KEY: &str = "policy.workspace_grants";

/// 授权缓存：会话级授权只在内存（进程退出失效）；工作区级授权写设置表（重启有效）。
pub struct Grants {
    db: Arc<Db>,
    /// 会话级授权键集合：`tool|target` 或 `shell_exec|prefix:<prefix>`
    memory: std::sync::Mutex<HashSet<String>>,
}

impl Grants {
    pub fn new(db: Arc<Db>) -> Self {
        Self {
            db,
            memory: std::sync::Mutex::new(HashSet::new()),
        }
    }

    /// 读取工作区级授权表（settings 中的 JSON map）；缺失或损坏按空表处理。
    fn workspace_grants(&self) -> std::collections::HashMap<String, bool> {
        self.db
            .with(|conn| get_setting(conn, WORKSPACE_GRANTS_KEY))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value::<std::collections::HashMap<String, bool>>(v).ok())
            .unwrap_or_default()
    }

    /// 是否命中既有授权：先查会话（内存）再查工作区（设置表）。
    /// 精确键 `tool|target` 直接命中（含 run_tool 传入的 `prefix:<cmd>` 形式）；
    /// shell_exec 另支持 target 为原始命令时推导前缀、按 `prefix:<cmd>` 键命中。
    pub fn allowed(&self, tool: &str, target: &str) -> bool {
        if !target.is_empty() && self.memory.lock().unwrap().contains(&format!("{tool}|{target}")) {
            return true;
        }
        if tool == "shell_exec" && !target.is_empty() {
            let prefix = command_prefix(target);
            if !prefix.is_empty() && self.memory.lock().unwrap().contains(&format!("shell_exec|prefix:{prefix}")) {
                return true;
            }
        }
        let workspace = self.workspace_grants();
        if !target.is_empty() && workspace.get(&format!("{tool}|{target}")) == Some(&true) {
            return true;
        }
        if tool == "shell_exec" && !target.is_empty() {
            let prefix = command_prefix(target);
            if !prefix.is_empty() && workspace.get(&format!("shell_exec|prefix:{prefix}")) == Some(&true) {
                return true;
            }
        }
        false
    }

    /// 写入一条授权：session 只进内存；workspace 额外持久化到设置表（once 不应到达此处）。
    pub fn grant(&self, tool: &str, target: &str, scope: &str) {
        let key = format!("{tool}|{target}");
        self.memory.lock().unwrap().insert(key.clone());
        if scope == "workspace" {
            let mut workspace = self.workspace_grants();
            workspace.insert(key, true);
            let value = serde_json::to_value(&workspace).unwrap_or_else(|_| json!({}));
            let _ = self.db.with(|conn| set_setting(conn, WORKSPACE_GRANTS_KEY, &value));
        }
    }

    /// 当前会话级授权键快照（仅内存部分；调试与状态展示用）。
    pub fn snapshot(&self) -> Vec<String> {
        self.memory.lock().unwrap().iter().cloned().collect()
    }
}

/// 命令授权前缀：空/含分隔符或替换语义 → 无前缀；
/// 首段是 shell 解释器 → 无前缀；只有一段或第二段是选项 → 前缀=第一段；否则前两段。
pub fn command_prefix(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    const REJECT_CHARS: &[&str] = &[";", "|", "&", ">", "<", "`", "\n", "\r", "$(", "${"];
    if REJECT_CHARS.iter().any(|c| trimmed.contains(c)) {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    const INTERPRETERS: &[&str] = &["env", "sudo", "bash", "sh", "zsh", "fish", "powershell", "cmd", "eval", "exec"];
    let first = parts.first().copied().unwrap_or_default();
    if parts.len() == 1 || INTERPRETERS.contains(&first.to_lowercase().as_str()) {
        return String::new();
    }
    let second = parts.get(1).copied().unwrap_or_default();
    if second.starts_with('-') {
        return first.to_string();
    }
    format!("{first} {second}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db() -> Arc<Db> {
        let dir = std::env::temp_dir().join(format!(
            "cw-grants-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Arc::new(Db::open(&dir.join("t.db")).unwrap())
    }

    #[test]
    fn command_prefix_rules() {
        // 第二段是选项：前缀只取第一段
        assert_eq!(command_prefix("ls -la src"), "ls");
        assert_eq!(command_prefix("git status"), "git status");
        assert_eq!(command_prefix("git"), "");
        assert_eq!(command_prefix("bash -c 'x'"), "");
        assert_eq!(command_prefix("sudo rm x"), "");
        assert_eq!(command_prefix("echo a && rm b"), "");
        assert_eq!(command_prefix("npm install"), "npm install");
        assert_eq!(command_prefix("  "), "");
    }

    #[test]
    fn session_and_workspace_grants_prefix_hit() {
        let db = tmp_db();
        let grants = Grants::new(Arc::clone(&db));
        assert!(!grants.allowed("fs_write", "/tmp/a.txt"));
        grants.grant("fs_write", "/tmp/a.txt", "session");
        assert!(grants.allowed("fs_write", "/tmp/a.txt"));
        assert!(!grants.allowed("fs_write", "/tmp/b.txt"));
        // 前缀授权：target 以 prefix: 开头存入
        grants.grant("shell_exec", "prefix:npm test", "session");
        assert!(grants.allowed("shell_exec", "prefix:npm test"));
        // workspace 授权落设置（重开实例仍有效）
        grants.grant("fs_edit", "/tmp/c.txt", "workspace");
        let grants2 = Grants::new(Arc::clone(&db));
        assert!(grants2.allowed("fs_edit", "/tmp/c.txt"));
    }
}
