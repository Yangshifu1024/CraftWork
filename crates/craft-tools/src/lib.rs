//! 工具框架（Registry）与内置工具：fs_* / apply_patch / shell_exec / git_* /
//! todo_write / doc_* / agent_*（子代理编排）/ wait（可被打断的定时等待，D21）。
//!
//! 架构位置：本 crate 是 Agent 的工具层，不依赖 Tauri；壳层（src-tauri）负责把
//! 内置工具注册进 [`registry::Registry`]。工具实现 [`registry::Tool`] trait
//! （spec + execute，同步执行，调用方负责放入阻塞线程）；路径解析、输出截断与
//! 写前备份（ctx.env.backup）由 Registry 统一处理，单个工具不自行重复这些横切逻辑。

pub mod agent;
pub mod doc;
pub mod fs;
pub mod mcp;
pub mod plan;
pub mod registry;
pub mod shell;
pub mod shell_tasks;
pub mod skills;
#[cfg(test)]
pub(crate) mod test_util;
pub mod todo;
pub mod wait;
pub mod web;

pub use registry::{human_size, is_ignored_dir, more_hint, truncate_output, Assessment, Registry, Tool, ToolCtx, ToolEnv, ToolMedia, ToolResult, ToolSpec, MAX_OUTPUT_BYTES, MAX_READ_BYTES, MAX_READ_LINES};

/// 全部内置工具（注册顺序即清单顺序）：壳层与测试共用这一份清单，
/// 避免「新增工具却忘了注册」与「注册清单另处维护」两处漂移。
///
/// 协议类工具（enter_plan / exit_plan、agent_*）同样在内：它们的 execute 是薄壳，
/// 状态切换在 Runner 特判分支完成，但仍需注册——参数校验、风险等级与标题都来自 spec。
///
/// 这里**故意没有** `skill_install`：技能安装 / 升级 / 回滚 / 卸载是用户在设置页直接做的
/// 动作（落 `audit_log`），Agent 不能自己装技能（决策 D12，`skill_install` 留二期）。
pub fn builtin_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(fs::FsList),
        Box::new(fs::FsRead),
        Box::new(fs::FsGlob),
        Box::new(fs::FsGrep),
        Box::new(fs::FsWrite),
        Box::new(fs::FsEdit),
        Box::new(fs::ApplyPatch),
        Box::new(shell::ShellExec),
        Box::new(shell::BashOutput),
        Box::new(shell::KillShell),
        Box::new(shell::GitStatus),
        Box::new(shell::GitDiff),
        Box::new(shell::GitLog),
        Box::new(shell::GitCommit),
        Box::new(todo::TodoWrite),
        Box::new(plan::EnterPlan),
        Box::new(plan::ExitPlan),
        Box::new(doc::DocRead),
        Box::new(doc::DocEdit),
        Box::new(doc::XlsxSetCell),
        Box::new(skills::SkillInvoke),
        Box::new(agent::AgentSpawn),
        Box::new(agent::AgentWait),
        Box::new(agent::AgentList),
        Box::new(agent::AgentClose),
        Box::new(wait::Wait),
        Box::new(web::WebFetch),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个内置工具都必须有人类可读标题：对话流工具行、审批卡与桌面通知都直接展示它，
    /// 空串会回退成 `fs_read` 这类内部名（用户不可读），等于漏填。
    #[test]
    fn every_builtin_tool_has_human_readable_title() {
        for tool in builtin_tools() {
            let spec = tool.spec();
            assert!(!spec.title.trim().is_empty(), "工具 {} 缺少 title", spec.name);
            assert_ne!(spec.title, spec.name, "工具 {} 的 title 不能等于工具名", spec.name);
            assert!(!spec.title.contains('_'), "工具 {} 的 title 应是中文短语而不是标识符：{}", spec.name, spec.title);
        }
    }

    /// 内置工具清单与注册表对拍：builtin_tools 里不能有重名（HashMap 注册会静默覆盖）。
    #[test]
    fn builtin_tool_names_are_unique() {
        let mut names: Vec<String> = builtin_tools().iter().map(|t| t.spec().name).collect();
        let total = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), total, "内置工具存在重名：{names:?}");
    }

    /// execute 必须在 tools 锁外执行工具：工具体内重新取注册表锁（specs / spec_for /
    /// 另一次 execute）不能死锁。曾踩坑：旧实现持 tools_guard 进入 tool.execute，
    /// shell_exec 同步等待子进程分钟级，期间并发 run 的所有工具评估与执行全部冻结。
    /// 探针工具在 execute 里另起线程取锁并限时回传：若仍持锁执行，回传超时即失败。
    #[test]
    fn execute_runs_tool_outside_registry_lock() {
        use std::sync::mpsc;
        use std::time::Duration;

        struct LockProbe {
            registry: std::sync::Arc<Registry>,
        }
        impl Tool for LockProbe {
            fn spec(&self) -> ToolSpec {
                ToolSpec {
                    name: "probe_lock".into(),
                    description: "锁纪律探针".into(),
                    schema: serde_json::json!({ "type": "object" }),
                    risk: craft_policy::PolicyRisk::R0,
                    path_arg: None,
                    command_arg: None,
                    mutates: false,
                    work: false,
                    title: "锁探针".into(),
                    summary_arg: None,
                    summary_text: None,
                }
            }
            fn execute(&self, _ctx: &ToolCtx) -> ToolResult {
                let registry = std::sync::Arc::clone(&self.registry);
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = registry.specs(); // 与 execute 竞争同一把 tools 锁
                    tx.send(()).expect("回传线程存活");
                });
                match rx.recv_timeout(Duration::from_secs(5)) {
                    Ok(()) => ToolResult::ok("工具在锁外执行"),
                    Err(_) => ToolResult::err_code("tools 锁被 execute 占用（持锁执行工具）", "deadlock"),
                }
            }
        }

        let dir = std::env::temp_dir().join(format!("cw-tools-lock-{}-probe", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        let db = std::sync::Arc::new(craft_store::Db::open(&dir.join("test.db")).expect("打开测试库"));
        let registry = std::sync::Arc::new(Registry::new("", "", db));
        registry.register(Box::new(LockProbe { registry: std::sync::Arc::clone(&registry) }));

        // 走公开接缝拿预构建 ctx（env 为私有字段，测试不手工拼 ToolCtx）
        let assessment = registry.assess(&[], "probe_lock", &serde_json::json!({}));
        let ctx = assessment.prepared.expect("已注册工具的 assess 必然产出 ctx");
        let result = registry.execute(&ctx);
        assert!(!result.is_error, "工具体内取锁应立即成功而不是死锁：{:?}", result.output);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
