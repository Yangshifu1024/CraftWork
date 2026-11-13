//! 子代理（Subagent）工具声明：agent_spawn / agent_wait / agent_list / agent_close。
//!
//! 决策 D15 与行为规格见 D15。四个工具是单一扁平面，按本仓库既有惯例落地：
//!
//! 职责边界（重要，与 `plan.rs` 同构）：
//! - 本模块的工具 `execute` 是**薄壳**：只做参数校验与确认文本，不持有任何运行状态。
//!   子会话创建、子 run 启动、并发计数、终态回报与唤醒，全部在 `craft-agent` 的
//!   Runner（`run_tool` 的 agent_* 特判分支 + `subagent` 模块）完成。
//! - 四个工具的风险声明都是「派发/查询编排」这一动作本身的风险，不代表子代理实际做的事：
//!   子代理内部的写文件、执行命令照旧逐个走策略引擎与审批档位（不允许借子代理提权）。
//!   `agent_spawn` 因此声明 R0（派发无副作用），`agent_close` 声明 R1（会中止在途工作）。
//!
//! 模型可见的四个工具是一个扁平集合，而不是把多种执行形态塞进单个入口：
//! 扁平命名让「派发 / 等待 / 查看 / 停止」的意图在工具名上直接可读，也避免单个 schema
//! 因形态分支膨胀（多形态 schema 是能力协商的代价，本仓库一期的形态是固定的）。

use serde_json::{json, Value};

use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

/// 派发任务的摘要长度上限（字符）：卡片与状态列表都只展示这一行。
pub const DESCRIPTION_MAX_CHARS: usize = 120;
/// 任务正文长度上限（字符）：超出直接拒绝，避免把超长上下文塞进子会话首轮。
pub const PROMPT_MAX_CHARS: usize = 32_000;
/// 子代理短名合法字符数上限（与 schema 的 `^[a-z][a-z0-9_]{0,31}$` 对齐）。
pub const AGENT_NAME_MAX_CHARS: usize = 32;
/// agent_wait 超时下限（毫秒）：太短的等待等于忙轮询。
pub const WAIT_MIN_MS: i64 = 5_000;
/// agent_wait 超时默认值（毫秒）：默认 60 秒，够一次中等规模的调研收尾。
pub const WAIT_DEFAULT_MS: i64 = 60_000;
/// agent_wait 超时上限（毫秒）：模型可以长等，但不能无限占住主循环。
pub const WAIT_MAX_MS: i64 = 600_000;

/// 派发子代理（编排入口，R0；Work 模式不提供——Work 只有文档能力，没有可并行的探查工作）。
pub struct AgentSpawn;

/// 等待子代理进入终态（R0）。
pub struct AgentWait;

/// 列出本会话派生的子代理（R0）。
pub struct AgentList;

/// 停止在跑的子代理（R1：会中止在途工作，但不动文件/命令）。
pub struct AgentClose;

/// 四个工具名的统一常量集合：Runner 的工具面过滤（子代理不能再派发子代理）与
/// 「哪些调用是子代理协议调用」的判断都读这一份，避免字符串散落。
pub const SUBAGENT_TOOLS: &[&str] = &[AgentSpawn::NAME, AgentWait::NAME, AgentList::NAME, AgentClose::NAME];

/// 子代理可用工具面里被排除的本机协议工具：计划协议（子代理不参与计划审阅）。
pub const SUBAGENT_EXCLUDED_PROTOCOL_TOOLS: &[&str] = &["enter_plan", "exit_plan"];

impl AgentSpawn {
    pub const NAME: &'static str = "agent_spawn";
}

impl AgentWait {
    pub const NAME: &'static str = "agent_wait";
}

impl AgentList {
    pub const NAME: &'static str = "agent_list";
}

impl AgentClose {
    pub const NAME: &'static str = "agent_close";
}

/// 短名归一：转小写、非法字符换下划线、超长截断；空结果回落空串（由调用方决定兜底名）。
/// 归一后仍以字母开头才保留，否则返回空串——schema 的正则以字母开头，工具侧同样执行。
pub fn normalize_agent_name(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    let mut out = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
        if out.chars().count() >= AGENT_NAME_MAX_CHARS {
            break;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        trimmed
    } else {
        String::new()
    }
}

/// 等待超时归一：缺省取默认值，越界钳制到 [MIN, MAX]。
pub fn clamp_wait_timeout(requested: Option<i64>) -> i64 {
    match requested {
        Some(ms) => ms.clamp(WAIT_MIN_MS, WAIT_MAX_MS),
        None => WAIT_DEFAULT_MS,
    }
}

impl Tool for AgentSpawn {
    fn spec(&self) -> ToolSpec {
        AgentSpawn::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let prompt = ctx.str_arg("prompt");
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            return ToolResult::err_code("prompt 不能为空：请把要交给子代理的完整任务写进 prompt", "invalid_args");
        }
        if prompt.chars().count() > PROMPT_MAX_CHARS {
            return ToolResult::err_code(
                format!("任务过长：{} 字符，上限 {PROMPT_MAX_CHARS} 字符，请精简后重新提交", prompt.chars().count()),
                "invalid_args",
            );
        }
        let description = ctx.str_arg("description");
        if description.trim().is_empty() {
            return ToolResult::err_code("description 不能为空：请用 3~8 个词概括这次派发，供界面与状态列表展示", "invalid_args");
        }
        if description.chars().count() > DESCRIPTION_MAX_CHARS {
            return ToolResult::err_code(
                format!("description 过长：{} 字符，上限 {DESCRIPTION_MAX_CHARS} 字符", description.chars().count()),
                "invalid_args",
            );
        }
        let agent_type = ctx.str_arg("agent_type");
        if !agent_type.is_empty() && !matches!(agent_type.as_str(), "general" | "explore") {
            return ToolResult::err_code(
                format!("未知的子代理类型：{agent_type}（可选 general / explore）"),
                "invalid_args",
            );
        }
        // 名字非法不拒绝：归一失败时由 Runner 按类型自动生成（避免一处小瑕疵打断派发）
        let requested_name = ctx.str_arg("name");
        let name = normalize_agent_name(&requested_name);
        // 校验通过即返回：真正的派发在 Runner 特判分支（创建子会话、启动子 run、登记注册表）。
        // runInBackground 回显与 Runner 的取值同口径：省略即后台（D31）。
        ToolResult::ok_with_meta(
            "子代理派发请求已接受。",
            json!({
                "agentType": if agent_type.is_empty() { "general".to_string() } else { agent_type },
                "requestedName": name,
                "runInBackground": ctx.args.get("run_in_background").and_then(Value::as_bool).unwrap_or(true),
            }),
        )
    }
}

impl Tool for AgentWait {
    fn spec(&self) -> ToolSpec {
        AgentWait::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let requested = ctx.args.get("timeout_ms").and_then(Value::as_i64);
        let timeout = clamp_wait_timeout(requested);
        ToolResult::ok_with_meta(
            "已开始等待子代理。",
            json!({ "timeoutMs": timeout, "requestedTimeoutMs": requested, "clamped": requested.is_some_and(|ms| ms != timeout) }),
        )
    }
}

impl Tool for AgentList {
    fn spec(&self) -> ToolSpec {
        AgentList::spec()
    }

    fn execute(&self, _ctx: &ToolCtx) -> ToolResult {
        ToolResult::ok("已列出本会话派生的子代理。")
    }
}

impl Tool for AgentClose {
    fn spec(&self) -> ToolSpec {
        AgentClose::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let name = ctx.str_arg("name");
        if name.trim().is_empty() {
            return ToolResult::err_code("name 不能为空：请用 agent_list 查看子代理名称", "invalid_args");
        }
        ToolResult::ok(format!("已请求停止子代理 {name}。"))
    }
}

/// 派发策略文案（模型可见）：先判定关键路径、只外派可并行的旁路任务、
/// 子任务必须自足、写入范围不重叠。这一段是模型能否用好子代理的主要杠杆，改动需同步
/// D15、D31 与 prompt.rs 的行为契约条目。
const SPAWN_STRATEGY: &str = "\
派发前的判断（不要跳过）：
- 先形成自己的高层计划，明确哪一步是「当前关键路径上必须由你立刻做的事」，哪一些是可以并行推进的旁路任务。关键路径任务自己做，别派出去再干等。
- 只外派边界清晰、能独立完成、且与你的下一步不互相阻塞的工作；任务越耦合、越紧急、越需要来回确认，越应该自己做。
- 子任务必须自足：把目标、范围、已知约束、验收方式写进 prompt。子代理看不到你的对话历史，只有你在 prompt 里写下的内容。
- 写入范围必须不重叠：需要改代码时，每个子代理的改动文件集合要互不相交，否则会互相覆盖。
- 不要重复工作：已经派出去的任务不要自己再做一遍；同一件事不要重复派发。

派发之后：
- 缺省即后台并行：同一批工具调用是串行执行的，但每次派发缺省就是后台运行——在同一条回复里连续发出的多个派发会依次受理、各自并行推进，这是唯一的并行路径。独立的子任务请在一条回复里连续派出，派完立即去做不重叠的工作。
- 只在你必须立刻拿到结论才能继续下一步时，才显式传 run_in_background=false 走前台（会阻塞到子代理结束）；需要收敛时用 agent_wait（只等待子代理，不等待后台任务），优先长等待而不是反复短等。
- 子代理的结论不会自动展示给用户，必须由你阅读、核实并转述；不要把它的原文整段贴给用户当作自己的结论。
- 子代理同样受用户审批档位约束：它遇到需要审批的操作时，审批卡会出现在当前对话里等用户决定。";

impl AgentSpawn {
    /// 默认 spec：`agent_type` 只列内置两型（无自定义定义时的形态）。
    pub fn spec() -> ToolSpec {
        Self::spec_with_types(&[], "")
    }

    /// 按可用类型生成 spec（决策 D15 扩展）：
    /// - `types` 是已启用的自定义子代理名字（追加到 enum 与 description）；
    /// - `type_descriptions` 是有自定义定义时的补充说明段（每行一个名字与简介）。
    ///
    /// Runner 在每次组装工具面时调用它，因此新增/停用自定义定义下一轮对话即生效。
    pub fn spec_with_types(custom_names: &[String], custom_descriptions: &str) -> ToolSpec {
        let mut enum_values: Vec<serde_json::Value> = vec![json!("general"), json!("explore")];
        enum_values.extend(custom_names.iter().map(|n| json!(n)));
        let custom_hint = if custom_names.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n除内置两型外，用户还定义了自定义子代理（各自有独立的工具面、模型与轮数上限）。\
                 它们与内置两型同等可用，每行冒号后是适用场景，请据此判断该不该交给它：\n{}",
                custom_descriptions.trim()
            )
        };
        ToolSpec {
            name: Self::NAME.into(),
            description: format!(
                "把一个边界清晰的子任务派发给子代理执行，它可以有自己的模型与工具集，执行完把结论交回给你。\
                 agent_type=general 拥有与主代理相同的工具（可改文件、跑命令），agent_type=explore 只读（检索与阅读代码，不能写）。\
                 缺省后台执行：立即返回启动回执，完成后自动向你回报结论；显式 run_in_background=false 才前台等待结果。{custom_hint}\n\n{SPAWN_STRATEGY}"
            ),
            schema: json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "3~8 个词的短摘要，用于界面卡片与子代理状态列表（不要写成一整句话）",
                    },
                    "prompt": {
                        "type": "string",
                        "description": "交给子代理的完整任务：目标、范围、约束、验收方式。子代理看不到本对话历史，必须写全",
                    },
                    "agent_type": {
                        "type": "string",
                        "enum": enum_values,
                        "description": "general（默认）拥有完整工具；explore 只读，适合检索与阅读代码；其余为用户自定义的子代理名",
                    },
                    "name": {
                        "type": "string",
                        "description": "可选短名（小写字母开头，可含数字与下划线，最长 32 字符），用于 agent_wait / agent_close 引用；省略时自动生成",
                    },
                    "model": {
                        "type": "string",
                        "description": "可选模型引用 providerID/modelID，覆盖该子代理档案里的默认模型；省略时用档案默认，档案也没配则继承当前会话模型",
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "省略或 true=后台执行（缺省），立即返回并在完成后自动回报结论；显式 false=前台阻塞等待结果，仅在你必须立刻拿到结论才能继续时用",
                    },
                },
                "required": ["description", "prompt"],
            }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "派发子代理".into(),
            summary_arg: Some("description".into()),
            summary_text: None,
        }
    }
}

impl AgentWait {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: Self::NAME.into(),
            description: "等待子代理进入终态（完成 / 失败 / 被停止），返回其状态与结论。\
                          默认等 60 秒，超时则返回当前状态让主循环继续。只在下一步确实依赖子代理结果时使用——\
                          能做的事先做，不要反复短等。本工具只等待 agent_spawn 派出的子代理，不等待后台任务\
                          （后台任务完成会自动通知，不需要等待）。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要等待的子代理名称列表；省略则等待本会话全部在跑的子代理",
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": WAIT_MIN_MS,
                        "maximum": WAIT_MAX_MS,
                        "description": format!("等待上限（毫秒），{WAIT_MIN_MS}~{WAIT_MAX_MS}，默认 {WAIT_DEFAULT_MS}"),
                    },
                },
            }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "等待子代理".into(),
            summary_arg: None,
            summary_text: Some("等待子代理返回结果".into()),
        }
    }
}

impl AgentList {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: Self::NAME.into(),
            description: "列出本会话派生的子代理：名称、类型、状态、耗时、工具调用次数与结论摘要。\
                          用于确认现状或找回之前派发过的子代理。"
                .into(),
            schema: json!({ "type": "object", "properties": {} }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "子代理列表".into(),
            summary_arg: None,
            summary_text: Some("查看子代理状态".into()),
        }
    }
}

impl AgentClose {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: Self::NAME.into(),
            description: "停止一个仍在运行的子代理（会同时中止它内部正在进行的操作）。\
                          已完成的子代理无需停止；停止后它的结论不再回注。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "要停止的子代理名称（agent_list 可查）",
                    },
                },
                "required": ["name"],
            }),
            risk: PolicyRisk::R1,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "停止子代理".into(),
            summary_arg: Some("name".into()),
            summary_text: None,
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
            "cw-agent-tool-{}-{}",
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
    fn tool_names_and_risk_declaration() {
        assert_eq!(SUBAGENT_TOOLS, &["agent_spawn", "agent_wait", "agent_list", "agent_close"]);
        // 派发/查询是编排动作（R0），停止会中止在途工作（R1）；都不是 Work 模式能力
        let spawn = AgentSpawn::spec();
        assert_eq!(spawn.name, "agent_spawn");
        assert_eq!(spawn.risk, PolicyRisk::R0);
        assert!(!spawn.work);
        assert!(spawn.path_arg.is_none() && spawn.command_arg.is_none());
        // 派发策略文案必须进 description（模型可见的唯一派发指引）
        assert!(spawn.description.contains("关键路径"));
        assert!(spawn.description.contains("写入范围必须不重叠"));
        assert!(spawn.description.contains("审批卡"));
        // 并行事实（D31）：缺省即后台并行，前台是显式选择；同一批工具调用本身串行
        assert!(spawn.description.contains("缺省即后台并行"), "应说明默认后台：{}", spawn.description);
        assert!(spawn.description.contains("唯一的并行路径"), "应说明唯一并行路径");
        assert!(spawn.description.contains("run_in_background=false"), "应说明前台模式是显式选择");
        assert!(spawn.description.contains("串行执行"), "应说明同一批调用是串行的");

        let close = AgentClose::spec();
        assert_eq!(close.risk, PolicyRisk::R1);
        for spec in [AgentWait::spec(), AgentList::spec()] {
            assert_eq!(spec.risk, PolicyRisk::R0);
            assert!(!spec.work);
        }
        // agent_wait 只等待子代理，不等待后台任务（后台任务完成会自动通知）
        let wait = AgentWait::spec();
        assert!(wait.description.contains("不等待后台任务"), "应写明等待对象边界：{}", wait.description);
        assert!(
            spawn.description.contains("不等待后台任务"),
            "派发策略应写明 agent_wait 与后台任务的边界：{}",
            spawn.description
        );
        for spec in [wait, spawn, AgentList::spec(), AgentClose::spec()] {
            assert!(!spec.description.contains("后台子代理"), "不得出现「后台子代理」混称");
        }
    }

    #[test]
    fn spawn_rejects_missing_prompt_and_description() {
        let missing_prompt = AgentSpawn.execute(&make_ctx(AgentSpawn::NAME, json!({ "description": "调研" })));
        assert!(missing_prompt.is_error);
        assert_eq!(missing_prompt.code, "invalid_args");
        let blank_prompt = AgentSpawn.execute(&make_ctx(AgentSpawn::NAME, json!({ "description": "调研", "prompt": "  \n " })));
        assert!(blank_prompt.is_error);
        let missing_desc = AgentSpawn.execute(&make_ctx(AgentSpawn::NAME, json!({ "prompt": "做点事" })));
        assert!(missing_desc.is_error);
        assert!(missing_desc.output.contains("description"));
    }

    #[test]
    fn spawn_rejects_oversized_prompt_and_unknown_agent_type() {
        let too_long = "汉".repeat(PROMPT_MAX_CHARS + 1);
        let result = AgentSpawn.execute(&make_ctx(AgentSpawn::NAME, json!({ "description": "调研", "prompt": too_long })));
        assert!(result.is_error);
        assert!(result.output.contains(&PROMPT_MAX_CHARS.to_string()));

        let unknown = AgentSpawn.execute(&make_ctx(
            AgentSpawn::NAME,
            json!({ "description": "调研", "prompt": "看看代码", "agent_type": "reviewer" }),
        ));
        assert!(unknown.is_error);
        assert!(unknown.output.contains("general"));
    }

    #[test]
    fn spawn_accepts_valid_call_and_reports_defaults() {
        let result = AgentSpawn.execute(&make_ctx(
            AgentSpawn::NAME,
            json!({ "description": "梳理入口", "prompt": "读 src 目录，列出入口文件", "name": "Explorer-1" }),
        ));
        assert!(!result.is_error);
        let meta = result.meta.unwrap();
        assert_eq!(meta["agentType"], "general", "省略 agent_type 默认 general");
        assert_eq!(meta["requestedName"], "explorer_1", "短名归一为小写");
        assert_eq!(meta["runInBackground"], true, "省略 run_in_background 默认后台（D31）");
    }

    #[test]
    fn normalize_agent_name_rules() {
        assert_eq!(normalize_agent_name("Worker"), "worker");
        assert_eq!(normalize_agent_name("  my-agent 2  "), "my_agent_2");
        assert_eq!(normalize_agent_name("a__b"), "a__b", "已存在的连续下划线原样保留（不做压缩）");
        // 数字开头 / 空串 / 纯符号 → 空串（由 Runner 兜底生成）
        assert_eq!(normalize_agent_name("1worker"), "");
        assert_eq!(normalize_agent_name("   "), "");
        assert_eq!(normalize_agent_name("---"), "");
        // 超长截断
        let long = normalize_agent_name(&"a".repeat(80));
        assert_eq!(long.chars().count(), AGENT_NAME_MAX_CHARS);
    }

    #[test]
    fn wait_timeout_clamped_to_bounds() {
        assert_eq!(clamp_wait_timeout(None), WAIT_DEFAULT_MS);
        assert_eq!(clamp_wait_timeout(Some(10)), WAIT_MIN_MS);
        assert_eq!(clamp_wait_timeout(Some(999_999)), WAIT_MAX_MS);
        assert_eq!(clamp_wait_timeout(Some(30_000)), 30_000);

        let clamped = AgentWait.execute(&make_ctx(AgentWait::NAME, json!({ "timeout_ms": 100 })));
        assert!(!clamped.is_error);
        let meta = clamped.meta.unwrap();
        assert_eq!(meta["timeoutMs"], WAIT_MIN_MS);
        assert_eq!(meta["clamped"], true);
    }

    #[test]
    fn close_requires_name() {
        assert!(AgentClose.execute(&make_ctx(AgentClose::NAME, json!({}))).is_error);
        assert!(!AgentClose.execute(&make_ctx(AgentClose::NAME, json!({ "name": "worker" }))).is_error);
    }

    #[test]
    fn excluded_protocol_tools_never_overlap_subagent_tools() {
        for name in SUBAGENT_EXCLUDED_PROTOCOL_TOOLS {
            assert!(!SUBAGENT_TOOLS.contains(name), "协议工具 {name} 不应出现在子代理协议工具集合里");
        }
    }
}
