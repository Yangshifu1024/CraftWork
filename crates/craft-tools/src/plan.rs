//! 计划模式协议工具：enter_plan / exit_plan。
//!
//! 把「进入计划态」「提交计划等待审批」从纯提示词约束升级为显式协议——模型完成
//! 调研后必须调用 exit_plan，把完整计划正文作为结构化入参（plan 字段）提交，
//! 而不是只写在回复文本里。
//!
//! 职责边界（重要）：
//! - 本模块的工具 execute 是薄壳：只做参数校验与确认文本，**不持有任何计划态**。
//!   计划状态（plan_active）属于 Runner 的 run 级上下文，「进入/退出计划态」的状态
//!   切换、exit_plan 的无条件审批边界、批准后的落盘，全部在 craft-agent 的 Runner
//!   特判分支完成（工具层不持状态，与 todo_write 的事件广播同构）。
//! - exit_plan 的 plan 参数是审批边界的数据载体：1~20000 字符、trim 后非空；
//!   校验失败返回错误结果，但**不改写入参原文**——批准落盘时必须使用模型提交的
//!   原始字符串（曾有实现在 schema 校验阶段 trim 丢了原文，前置条件：批准落盘用的是原文）。
//! - exit_plan 在 Runner 中无条件走审批（包括自由档），不受审批档位影响；
//!   spec 的风险等级（R1）只供审批卡片展示与未知路径兜底。

use serde_json::{json, Value};

use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

/// 计划正文长度上限（字符数，按 Unicode 标量计）。
pub const PLAN_MAX_CHARS: usize = 20_000;

/// 进入计划模式：模型对复杂任务主动进入计划态（R0，免审批——进入计划态本身
/// 无副作用）。Work 模式不提供（计划态只对 Code 有意义）。
pub struct EnterPlan;

/// 提交计划等待用户审批：计划正文的审批边界工具（R1；Runner 对它无条件弹卡，
/// 包括自由档——计划审批始终需要用户确认）。Work 模式不提供。
pub struct ExitPlan;

/// 计划模式协议工具名集合：Runner 的 plan 闸门用它放行协议工具本身
/// （计划态下模型必须还能调用 enter_plan 幂等进入、exit_plan 提交计划）。
pub const PLAN_MODE_TOOLS: &[&str] = &[EnterPlan::NAME, ExitPlan::NAME];

impl EnterPlan {
    pub const NAME: &'static str = "enter_plan";
}

impl ExitPlan {
    pub const NAME: &'static str = "exit_plan";
}

impl Tool for EnterPlan {
    fn spec(&self) -> ToolSpec {
        EnterPlan::spec()
    }

    fn execute(&self, _ctx: &ToolCtx) -> ToolResult {
        // 无参数、无副作用：真正的状态切换由 Runner 特判完成（置 plan_active、
        // 广播 plan.entered、注入计划态提示词）。重复调用幂等（已在计划态时
        // Runner 原样返回确认文本，不重复广播）。
        ToolResult::ok(
            "已进入计划模式。请以调研与分析为主出一份完整计划，完成调研后调用 exit_plan 提交。",
        )
    }
}

impl Tool for ExitPlan {
    fn spec(&self) -> ToolSpec {
        ExitPlan::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        // 只校验不改写：计划正文原样留在 args 里，由 Runner 取用（审批展示、批准落盘）。
        let Some(plan) = ctx.args.get("plan").and_then(Value::as_str) else {
            return ToolResult::err_code("exit_plan 缺少 plan 参数，请把完整计划作为字符串传入", "invalid_args");
        };
        let trimmed = plan.trim();
        if trimmed.is_empty() {
            return ToolResult::err_code("计划内容不能为空，请把完整计划作为 plan 参数传入", "invalid_args");
        }
        if plan.chars().count() > PLAN_MAX_CHARS {
            return ToolResult::err_code(
                format!("计划过长：{} 字符，上限 {PLAN_MAX_CHARS} 字符，请精简后重新提交", plan.chars().count()),
                "invalid_args",
            );
        }
        // 校验通过即返回：审批弹卡、批准/拒绝处理都在 Runner 特判分支。
        ToolResult::ok_with_meta(
            "计划已提交，等待用户审阅。",
            json!({ "planChars": plan.chars().count() }),
        )
    }
}

impl EnterPlan {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: Self::NAME.into(),
            description: "对非平凡的实现任务，先进入计划模式：调研并设计实施方案，经用户批准后再动手。新功能、多种可行方案、影响既有行为、架构取舍、多文件改动、需求不清时优先使用；单行修复、需求明确的简单改动、纯调研任务不要使用。".into(),
            schema: json!({
                "type": "object",
                "properties": {},
            }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "进入计划模式".into(),
            summary_arg: None,
            summary_text: Some("进入计划模式，先调研出计划".into()),
        }
    }
}

impl ExitPlan {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: Self::NAME.into(),
            description: "处于计划模式且完成调研后，用本工具提交完整计划等待用户审批：计划全文放在 plan 参数中（用户会在审批卡里审阅该内容），不要只写在回复里。尚未明确的需求先向用户提问，不要用本工具试探。计划被拒绝时按用户反馈修订后重新提交；获批后退出计划模式直接开始实施。".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "plan": {
                        "type": "string",
                        "minLength": 1,
                        "maxLength": PLAN_MAX_CHARS,
                        "description": "完整的分步实施计划：目标、将要改动或新增的文件、每一步的做法、验证方式与风险",
                    },
                },
                "required": ["plan"],
            }),
            risk: PolicyRisk::R1,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "计划审阅".into(),
            summary_arg: None,
            summary_text: Some("提交计划，等待用户确认".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolEnv;

    fn make_ctx(tool: &str, args: Value) -> ToolCtx {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cw-plan-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = std::sync::Arc::new(craft_store::db::Db::open(&dir.join("t.db")).unwrap());
        ToolCtx {
            name: tool.into(),
            args,
            target: String::new(),
            display: String::new(),
            workspace: String::new(),
            roots: vec![],
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: std::sync::Arc::new(ToolEnv { data_dir: String::new(), db, shell_tasks: std::sync::Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() }),
        }
    }

    #[test]
    fn plan_specs_declare_readonly_protocol_metadata() {
        // enter_plan：R0（免审批进入）、无参、两种目录模式过滤口径一致（work=false）
        let enter = EnterPlan::spec();
        assert_eq!(enter.name, "enter_plan");
        assert_eq!(enter.risk, PolicyRisk::R0);
        assert!(!enter.work);
        assert!(enter.path_arg.is_none() && enter.command_arg.is_none());

        // exit_plan：R1、带 plan 参数、不可在 Work 模式使用
        let exit = ExitPlan::spec();
        assert_eq!(exit.name, "exit_plan");
        assert_eq!(exit.risk, PolicyRisk::R1);
        assert!(!exit.work);
        // 协议工具名集合与 spec 声明一致（plan 闸门的放行依据）
        assert_eq!(PLAN_MODE_TOOLS, &["enter_plan", "exit_plan"]);
    }

    #[test]
    fn enter_plan_execute_returns_guidance_without_side_effects() {
        let result = EnterPlan.execute(&make_ctx(EnterPlan::NAME, json!({})));
        assert!(!result.is_error);
        assert!(result.output.contains("exit_plan"), "确认文本应引导模型后续调用 exit_plan");
    }

    #[test]
    fn exit_plan_accepts_valid_plan_and_keeps_args_untouched() {
        // 原文含首尾空白：校验按 trim 判断，但不改写入参（批准落盘要用原文）
        let raw = "\n  # 计划\n\n1. 改 A 文件\n2. 跑测试  \n";
        let ctx = make_ctx(ExitPlan::NAME, json!({ "plan": raw }));
        let result = ExitPlan.execute(&ctx);
        assert!(!result.is_error);
        assert_eq!(ctx.args["plan"], raw, "execute 不得改写 plan 参数原文");
    }

    #[test]
    fn exit_plan_rejects_missing_or_blank_plan() {
        // 缺参
        let missing = ExitPlan.execute(&make_ctx(ExitPlan::NAME, json!({})));
        assert!(missing.is_error);
        assert_eq!(missing.code, "invalid_args");
        // 非字符串
        let wrong_type = ExitPlan.execute(&make_ctx(ExitPlan::NAME, json!({ "plan": 42 })));
        assert!(wrong_type.is_error);
        // 空串与纯空白
        for raw in ["", "   \n\t "] {
            let result = ExitPlan.execute(&make_ctx(ExitPlan::NAME, json!({ "plan": raw })));
            assert!(result.is_error, "空白计划应被拒绝：{raw:?}");
            assert_eq!(result.code, "invalid_args");
        }
    }

    #[test]
    fn exit_plan_rejects_oversized_plan_by_char_count() {
        // 上限按字符数计（20_000），多字节字符不误伤
        let boundary = "汉".repeat(PLAN_MAX_CHARS);
        let ok = ExitPlan.execute(&make_ctx(ExitPlan::NAME, json!({ "plan": boundary })));
        assert!(!ok.is_error, "恰好 {PLAN_MAX_CHARS} 字符应通过");

        let oversized = "汉".repeat(PLAN_MAX_CHARS + 1);
        let result = ExitPlan.execute(&make_ctx(ExitPlan::NAME, json!({ "plan": oversized })));
        assert!(result.is_error);
        assert_eq!(result.code, "invalid_args");
        assert!(
            result.output.contains(&format!("{PLAN_MAX_CHARS}")),
            "错误信息应包含字符数上限",
        );
    }

    #[test]
    fn registry_assess_and_execute_plan_tools_end_to_end() {        let dir = std::env::temp_dir().join(format!("cw-plan-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let registry = crate::registry::Registry::new("", std::sync::Arc::new(
            craft_store::db::Db::open(&dir.join("t.db")).unwrap(),
        ));
        registry.register(Box::new(EnterPlan));
        registry.register(Box::new(ExitPlan));

        // assess：风险等级与存在性正确，无评估期错误
        let enter_assessment = registry.assess(&[], EnterPlan::NAME, &json!({}));
        assert!(enter_assessment.exists);
        assert_eq!(enter_assessment.risk, PolicyRisk::R0);
        assert!(enter_assessment.prepared.as_ref().unwrap().err.is_none());

        let exit_assessment = registry.assess(&[], ExitPlan::NAME, &json!({ "plan": "# 计划" }));
        assert!(exit_assessment.exists);
        assert_eq!(exit_assessment.risk, PolicyRisk::R1);

        // execute 接缝：统一出口正常返回
        let ctx = exit_assessment.prepared.unwrap();
        let executed = registry.execute(&ctx);
        assert!(!executed.is_error);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
