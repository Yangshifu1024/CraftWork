//! todo_write 内置工具（任务清单，纯展示输出）。

use serde_json::{json, Value};

use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

pub struct TodoWrite;

impl Tool for TodoWrite {
    fn spec(&self) -> ToolSpec {
        TodoWrite::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let items = ctx.args.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        if items.is_empty() {
            return ToolResult::err("items 不能为空");
        }
        let lines: Vec<String> = items
            .iter()
            .take(50)
            .map(|raw| {
                let text = raw.get("text").and_then(Value::as_str).unwrap_or_default();
                let text: String = text.chars().take(200).collect();
                let status = raw.get("status").and_then(Value::as_str).unwrap_or("pending");
                let mark = match status {
                    "active" => "◐",
                    "done" => "●",
                    _ => "○",
                };
                format!("{mark} {text}")
            })
            .collect();
        ToolResult::ok_with_meta(lines.join("\n"), json!({ "items": items.len() }))
    }
}

impl TodoWrite {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "todo_write".into(),
        description: "更新任务清单（任务级进度展示给用户）。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "description": "任务列表",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "active", "done"] },
                        },
                        "required": ["text", "status"],
                    },
                },
            },
            "required": ["items"],
        }),
        risk: PolicyRisk::R0,
        path_arg: None,
        command_arg: None,
        work: true,
        title: String::new(),
        summary_arg: None,
        summary_text: Some("更新任务清单".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolEnv;

    fn make_ctx(args: Value) -> ToolCtx {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cw-todo-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = std::sync::Arc::new(craft_store::db::Db::open(&dir.join("t.db")).unwrap());
        ToolCtx {
            name: "todo_write".into(),
            args,
            target: String::new(),
            display: String::new(),
            workspace: String::new(),
            roots: vec![],
            target_root: String::new(),
            err: None,
            env: std::sync::Arc::new(ToolEnv { backup_dir: String::new(), db }),
        }
    }

    #[test]
    fn 任务清单_状态符号与上限() {
        let ctx = make_ctx(json!({"items": [
            {"text": "任务一", "status": "done"},
            {"text": "任务二", "status": "active"},
            {"text": "任务三", "status": "pending"},
            {"text": "任务四", "status": "奇怪的"},
        ]}));
        let result = TodoWrite.execute(&ctx);
        assert!(!result.is_error);
        assert!(result.output.contains("● 任务一"));
        assert!(result.output.contains("◐ 任务二"));
        assert!(result.output.contains("○ 任务三"));
        assert!(result.output.contains("○ 任务四"), "未知状态回退 pending");
    }

    #[test]
    fn 任务清单_空列表拒绝() {
        let ctx = make_ctx(json!({"items": []}));
        assert!(TodoWrite.execute(&ctx).is_error);
    }
}
