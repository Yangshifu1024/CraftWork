//! todo_write 内置工具（任务清单，纯展示输出）。
//!
//! 只把清单渲染为带状态符号的文本，不落库、不持久化——任务进度由事件流呈现给用户，
//! 模型通过再次调用本工具更新清单。

use serde_json::{json, Value};

use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

/// 更新任务清单（R0，Work 模式可用）。
pub struct TodoWrite;

/// 状态别名归一化：模型不一定遵守 schema 枚举（常见输出 completed / in_progress），
/// 统一映射回规范值，未知值回退 pending。工具输出与事件广播共用，保证两侧一致。
pub fn normalize_status(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "done" | "completed" | "complete" => "done",
        "active" | "in_progress" | "in-progress" => "active",
        _ => "pending",
    }
}

/// 归一化 items 数组中的 status 字段（原样保留其余字段），供 todo 事件广播前调用。
pub fn normalize_items(items: &Value) -> Value {
    match items {
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|raw| {
                    let mut item = raw.clone();
                    let status = raw.get("status").and_then(Value::as_str).map(normalize_status);
                    item["status"] = Value::String(status.unwrap_or("pending").into());
                    item
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// 计划模式下待办清单的第一项必须表达「退出计划模式」的语义。
///
/// 为什么要有这条：计划模式的核心是「先定方案、经批准再动手」，而模型常把计划写长、
/// 把「退出计划模式」这件事忘在最后（甚至不写进待办），于是调研做完就一直停在计划态。
/// 把「退出计划模式」钉在首项，等于强制模型一开始就把「用 exit_plan 收尾」当作第一件事。
/// 判定按关键词而非严格相等：模型自然语言写法很多（退出计划 / 提交计划审批 / exit plan），
/// 强行逐字匹配只会逼它凑字符串。
pub fn first_item_is_exit_plan(first: &Value) -> bool {
    let text = first.get("text").and_then(Value::as_str).unwrap_or_default();
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("退出计划") || lowered.contains("离开计划") {
        return true;
    }
    // 英文：只认独立的 exit + plan 词对，不做子串匹配——
    // "exitplanning later" 这类词会被子串匹配误判成「要退出计划」。
    // 先把空白 / 下划线 / 连字符统一成分隔符，再按非字母数字切词。
    let normalized: String = lowered
        .chars()
        .map(|c| if matches!(c, '_' | '-') { ' ' } else { c })
        .collect();
    normalized
        .split(|c: char| !c.is_alphanumeric())
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0] == "exit" && w[1] == "plan")
}

impl Tool for TodoWrite {
    fn spec(&self) -> ToolSpec {
        TodoWrite::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let items = ctx.args.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        if items.is_empty() {
            return ToolResult::err("items 不能为空");
        }
        // 计划态专属校验：第一项必须是「退出计划模式」。模型据此不会把这件事漏到实施阶段之后。
        if ctx.plan_active && !first_item_is_exit_plan(&items[0]) {
            return ToolResult::err_code(
                "计划模式下的待办清单，第一项必须是「退出计划模式」（例如「退出计划模式（提交计划待批准）」）。\
                 请把它放到 items 的第一条后重新提交；其余任务从第二项开始。",
                "invalid_args",
            );
        }
        let lines: Vec<String> = items
            .iter()
            .take(50) // 条数上限，防超长清单刷屏
            .map(|raw| {
                let text = raw.get("text").and_then(Value::as_str).unwrap_or_default();
                let text: String = text.chars().take(200).collect(); // 单条文本截断（按字符，不切坏多字节）
                let status = raw.get("status").and_then(Value::as_str).map(normalize_status).unwrap_or("pending");
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
        description: "更新任务清单（任务级进度展示给用户）。计划模式下第一项必须是「退出计划模式」，其余任务从第二项开始。".into(),
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
                            "status": { "type": "string", "enum": ["pending", "active", "done"],
                                "description": "只能取 pending / active / done；不要用 completed 或 in_progress 等别名" },
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
        mutates: false,
        work: true,
        title: "更新任务清单".into(),
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
            plan_active: false,
            thread_id: String::new(),
            env: std::sync::Arc::new(ToolEnv { backup_dir: String::new(), data_dir: String::new(), db, shell_tasks: std::sync::Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() }),
        }
    }

    #[test]
    fn todo_list_status_symbols_and_limit() {
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
    fn todo_list_empty_items_rejected() {
        let ctx = make_ctx(json!({"items": []}));
        assert!(TodoWrite.execute(&ctx).is_error);
    }

    /// 计划模式下：首项必须是「退出计划模式」，否则拒绝并提示正确写法。
    #[test]
    fn plan_mode_requires_exit_plan_as_first_item() {
        let mut ctx = make_ctx(json!({"items": [
            {"text": "调研现有实现", "status": "active"},
            {"text": "退出计划模式", "status": "pending"},
        ]}));
        ctx.plan_active = true;
        let result = TodoWrite.execute(&ctx);
        assert!(result.is_error, "首项不是退出计划模式时应拒绝");
        assert_eq!(result.code, "invalid_args");
        assert!(result.output.contains("第一项"), "错误信息应指明第一项约束");

        // 首项合规（含状态无关）即通过
        let mut ok = make_ctx(json!({"items": [
            {"text": "退出计划模式（提交计划待批准）", "status": "pending"},
            {"text": "调研现有实现", "status": "active"},
        ]}));
        ok.plan_active = true;
        let result = TodoWrite.execute(&ok);
        assert!(!result.is_error, "首项合规时应通过：{}", result.output);
    }

    /// 非计划模式不做首项校验：简单任务不该被迫加一条无意义的待办。
    #[test]
    fn first_item_check_only_applies_in_plan_mode() {
        let ctx = make_ctx(json!({"items": [
            {"text": "改一行代码", "status": "active"},
        ]}));
        assert!(!ctx.plan_active);
        assert!(!TodoWrite.execute(&ctx).is_error, "非计划态不应强制首项");
    }

    /// 首项判定对中英文与常见写法宽容，避免逼模型凑字符串。
    #[test]
    fn exit_plan_first_item_matching_is_tolerant() {
        for text in [
            "退出计划模式",
            "退出计划模式（提交计划待批准）",
            "退出计划：提交方案",
            "离开计划模式",
            "Exit Plan",
            "exit_plan 提交计划",
            "  EXIT-PLAN  ",
        ] {
            assert!(first_item_is_exit_plan(&json!({"text": text})), "应识别为退出计划模式：{text:?}");
        }
        for text in ["调研", "写实现计划", "exitplanning later", ""] {
            assert!(!first_item_is_exit_plan(&json!({"text": text})), "不应误判：{text:?}");
        }
        // 缺 text 字段不 panic
        assert!(!first_item_is_exit_plan(&json!({"status": "pending"})));
    }

    #[test]
    fn todo_list_normalizes_status_aliases() {
        let ctx = make_ctx(json!({"items": [
            {"text": "任务一", "status": "completed"},
            {"text": "任务二", "status": "in_progress"},
            {"text": "任务三", "status": "DONE"},
            {"text": "任务四", "status": "奇怪的"},
            {"text": "任务五"},
        ]}));
        let result = TodoWrite.execute(&ctx);
        assert!(!result.is_error);
        assert!(result.output.contains("● 任务一"), "completed 归一化为 done");
        assert!(result.output.contains("◐ 任务二"), "in_progress 归一化为 active");
        assert!(result.output.contains("● 任务三"), "大小写不敏感");
        assert!(result.output.contains("○ 任务四"), "未知值回退 pending");
        assert!(result.output.contains("○ 任务五"), "缺失 status 回退 pending");

        let items = normalize_items(ctx.args.get("items").unwrap());
        assert_eq!(items[0]["status"], "done");
        assert_eq!(items[1]["status"], "active");
        assert_eq!(items[2]["status"], "done");
        assert_eq!(items[3]["status"], "pending");
        assert_eq!(items[4]["status"], "pending");
        assert_eq!(items[0]["text"], "任务一", "其余字段原样保留");

        // 非数组输入原样返回，不 panic
        assert_eq!(normalize_items(&Value::Null), Value::Null);
    }
}
