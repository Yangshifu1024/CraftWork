//! MCP 工具桥：把 MCP 服务器的工具描述符投影成 CraftWork 的 [`Tool`]。
//!
//! 命名规则（与主流 MCP 客户端一致，用户从别处抄配置过来不必重新理解）：
//! `mcp__<归一服务器名>__<归一工具名>`。两段都做 `[^A-Za-z0-9_-] → _` 归一，
//! 已有 `mcp__` 前缀的放行逻辑见 `Registry::specs_for_mode`（Work 模式可用）。
//!
//! 风险映射（安全属性，改动前先读 D16）：
//! - 基础：stdio 服务器 → R2（本机执行外部程序）；远程服务器 → R3（触网，受限工作区据此直接拒绝）
//! - `readOnlyHint = true` → 降一级（stdio R1 / 远程 R2）
//! - `destructiveHint = true` → 升到 R4
//!
//! `mutates = !readOnlyHint`：计划模式的硬门卡按它判定，而不是按风险等级
//! （理由同 fs_write：单文件写入风险不高，但计划态最该拦的就是它）。
//! 门卡只在分级档启用；自由档按「放行一切操作」的档位语义不拦（D8 2026-11-10 修订）。

use std::sync::Arc;

use craft_policy::PolicyRisk;
use serde_json::Value;

use crate::registry::{Tool, ToolCtx, ToolMedia, ToolResult, ToolSpec};

/// 模型可见工具名的总长上限。多数 provider 对函数名有长度限制（OpenAI 64、
/// Anthropic 64），这里留 4 字符余量；超限时前缀保留、后缀换成内容哈希，
/// 保证「同一个服务器工具集里不撞车」且跨进程稳定。
const MAX_TOOL_NAME_LEN: usize = 60;

/// MCP 工具的默认超时（毫秒），与生态通行取值一致。
pub const DEFAULT_TOOL_TIMEOUT_MS: i64 = 30_000;
/// 工具级超时上限（毫秒）：30 分钟（`_meta.timeoutMs` 超过部分被夹断）。
pub const MAX_TOOL_TIMEOUT_MS: i64 = 30 * 60 * 1000;

/// 安全上限：单服务器最多注册的工具数。超出部分不注册（在设置页说明），
/// 防止一个大服务器把上下文与工具面撑爆。
pub const MAX_TOOLS_PER_SERVER: usize = 60;

/// 一个 MCP 工具。
///
/// 持有 `Arc<McpPool>` 而不是连接本身：连接生命周期由池管理（断线重连、
/// 应用退出清理），工具只是「怎么调」的适配器。
pub struct McpTool {
    /// 服务器配置 ID（`mcp_xxx`）
    server_id: String,
    /// 服务器配置名（用户可读，也是工具名第一段的来源）
    server_name: String,
    /// 是否远程传输（决定基础风险等级）
    remote: bool,
    /// 服务器级超时（毫秒）
    server_timeout_ms: i64,
    /// 描述符（工具名、描述、schema、注解、工具级超时）
    descriptor: craft_mcp::ToolDescriptor,
    /// 模型可见工具名（构造时算好，避免每次 spec() 重算）
    model_name: String,
    pool: Arc<craft_mcp::McpPool>,
}

impl McpTool {
    /// 构造一个 MCP 工具。
    ///
    /// 返回 `Err` 的情形：归一后的模型名=== 与其他工具冲突由调用方检查
    /// （Registry 的同名注册会静默覆盖，工具桥必须在注册前自己挡）；
    /// 这里只负责生成本身的名称与校验长度。
    pub fn new(
        server_id: impl Into<String>,
        server_name: impl Into<String>,
        remote: bool,
        server_timeout_ms: i64,
        descriptor: craft_mcp::ToolDescriptor,
        pool: Arc<craft_mcp::McpPool>,
    ) -> Self {
        let server_id = server_id.into();
        let server_name = server_name.into();
        let model_name = model_tool_name(&server_name, &descriptor.tool_name);
        Self { server_id, server_name, remote, server_timeout_ms, descriptor, model_name, pool }
    }

    /// 模型可见工具名（注册与注销都按它）。
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// 服务器侧原始工具名（调用时回传）。
    pub fn tool_name(&self) -> &str {
        &self.descriptor.tool_name
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    /// 实际生效的超时：工具级 `_meta.timeoutMs` 覆盖服务器级，上限 30 分钟。
    fn effective_timeout_ms(&self) -> i64 {
        let candidate = self.descriptor.meta_timeout_ms.unwrap_or(self.server_timeout_ms);
        candidate.clamp(1_000, MAX_TOOL_TIMEOUT_MS)
    }

    /// 风险等级：见模块文档的映射规则。
    fn risk_level(&self) -> PolicyRisk {
        let base = if self.remote { PolicyRisk::R3 } else { PolicyRisk::R2 };
        if self.descriptor.destructive_hint {
            return PolicyRisk::R4;
        }
        if self.descriptor.read_only_hint {
            // 只读降一级：stdio R2→R1，远程 R3→R2
            return match base {
                PolicyRisk::R2 => PolicyRisk::R1,
                PolicyRisk::R3 => PolicyRisk::R2,
                other => other,
            };
        }
        base
    }
}

impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        let description = if self.descriptor.description.trim().is_empty() {
            format!("由 MCP 服务器「{}」提供的工具「{}」", self.server_name, self.descriptor.tool_name)
        } else {
            self.descriptor.description.clone()
        };
        ToolSpec {
            name: self.model_name.clone(),
            description,
            // 强制 object schema：部分服务器的 inputSchema 缺失或不是对象，
            // 直接转发会被 provider 拒绝（与主流客户端的 inputSchema 归一处理一致）
            schema: craft_mcp::client::ensure_object_schema(self.descriptor.input_schema.clone()),
            risk: self.risk_level(),
            // MCP 工具没有路径/命令参数：路径校验与命令分类都不适用
            path_arg: None,
            command_arg: None,
            mutates: !self.descriptor.read_only_hint,
            // Work 模式可用（沿用 Registry 对 mcp__ 前缀的既有放行逻辑）
            work: true,
            // 人类可读标题：对话流工具行、审批卡与桌面通知都直接展示它。
            // 用服务器/工具的原始名而不是归一名（后者带下划线不可读）。
            title: format!("MCP 工具 {} / {}", self.server_name, self.descriptor.tool_name),
            summary_arg: None,
            // 审计 detail 与界面摘要只用「服务器名/工具名」：env、headers 的值与图片 base64
            // 一律不进日志、事件与审计——凭据明文只存在 settings 表里，不外溢。
            summary_text: Some(format!("{}/{}", self.server_name, self.descriptor.tool_name)),
        }
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let timeout = self.effective_timeout_ms();
        let arguments = if ctx.args.is_object() { ctx.args.clone() } else { Value::Object(Default::default()) };
        match self.pool.call_tool(&self.server_id, &self.descriptor.tool_name, arguments, timeout) {
            Ok(outcome) => {
                let media: Vec<ToolMedia> = outcome
                    .media
                    .into_iter()
                    .map(|item| ToolMedia { name: item.name, mime: item.mime, base64: item.base64, size: item.size })
                    .collect();
                let mut result = if outcome.is_error {
                    ToolResult::err(outcome.text)
                } else {
                    ToolResult::ok(outcome.text)
                };
                result.media = media;
                result
            }
            Err(message) => ToolResult::err_code(message, "mcp_error"),
        }
    }

    /// 审批理由：说明这是经外部服务器执行的调用，而不是笼统的「执行命令」。
    fn assess_reason(&self) -> Option<String> {
        Some(format!(
            "调用外部 MCP 服务器「{}」的工具「{}」",
            self.server_name, self.descriptor.tool_name
        ))
    }

    /// 授权目标：`<服务器>/<工具>`。
    ///
    /// 必须非空：`Grants` 按 `tool|target` 建键，空 target 会让 session / workspace
    /// 级授权永远缓存不上（每次调用都重新弹卡）。
    fn grant_target(&self, _ctx: &ToolCtx) -> Option<String> {
        Some(format!("{}/{}", self.server_name, self.descriptor.tool_name))
    }
}

/// 生成模型可见工具名：`mcp__<server>__<tool>`。
///
/// - 两段都经 `sanitize_name_part` 归一
/// - 总长超过 [`MAX_TOOL_NAME_LEN`] 时，保留前 44 字符并追加 8 位内容哈希
/// - 归一后为空的两段回落到 `unknown`（`sanitize_name_part` 已保证）
pub fn model_tool_name(server_name: &str, tool_name: &str) -> String {
    let server = craft_mcp::config::sanitize_name_part(server_name);
    let tool = craft_mcp::config::sanitize_name_part(tool_name);
    let full = format!("mcp__{server}__{tool}");
    if full.chars().count() <= MAX_TOOL_NAME_LEN {
        return full;
    }
    // 超长：截前缀 + 内容哈希。哈希取完整名的前 8 位，对同一 (server, tool)
    // 组合稳定，跨进程、跨会话一致（授权缓存与历史重建都依赖这个稳定性）。
    // 前缀预算 = 上限 − 2（分隔符）− 8（哈希）。
    let digest = craft_core::platform::sha256_hex(full.as_bytes());
    let suffix: String = digest.chars().take(8).collect();
    let head_len = MAX_TOOL_NAME_LEN - 2 - suffix.chars().count();
    let head: String = full.chars().take(head_len).collect();
    // 截断点可能落在下划线后，去掉尾随下划线避免出现 <hash> 前的三连下划线
    format!("{}_{suffix}", head.trim_end_matches('_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_mcp::pool::McpPool;

    fn descriptor(tool_name: &str) -> craft_mcp::ToolDescriptor {
        craft_mcp::ToolDescriptor {
            tool_name: tool_name.to_string(),
            title: None,
            description: String::new(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            read_only_hint: false,
            destructive_hint: false,
            idempotent_hint: false,
            meta_timeout_ms: None,
        }
    }

    fn tool(server: &str, remote: bool, descriptor: craft_mcp::ToolDescriptor) -> McpTool {
        McpTool::new("mcp_demo", server, remote, DEFAULT_TOOL_TIMEOUT_MS, descriptor, Arc::new(McpPool::new()))
    }

    #[test]
    fn tool_name_uses_double_underscore_separator() {
        assert_eq!(model_tool_name("filesystem", "read_file"), "mcp__filesystem__read_file");
        // 空格与非法字符归一为下划线
        assert_eq!(model_tool_name("my server", "read file"), "mcp__my_server__read_file");
    }

    #[test]
    fn long_tool_names_are_truncated_with_stable_hash() {
        let long_tool = "a".repeat(120);
        let name = model_tool_name("server", &long_tool);
        assert!(
            name.chars().count() <= MAX_TOOL_NAME_LEN,
            "超长名应被截断到 {MAX_TOOL_NAME_LEN}：{}（{} 字符）",
            name,
            name.chars().count()
        );
        assert!(name.starts_with("mcp__server__"));
        // 同一输入两次生成必须一致（授权缓存与历史重建依赖稳定性）
        assert_eq!(name, model_tool_name("server", &long_tool));
        // 不同工具名不能撞车
        let other = model_tool_name("server", &"b".repeat(120));
        assert_ne!(name, other);
    }

    /// stdio 默认 R2、远程默认 R3——远程调用必须让受限工作区能拦下来。
    #[test]
    fn base_risk_depends_on_transport() {
        let local = tool("demo", false, descriptor("run"));
        assert_eq!(local.spec().risk, PolicyRisk::R2);
        let remote = tool("demo", true, descriptor("run"));
        assert_eq!(remote.spec().risk, PolicyRisk::R3, "远程工具应触发受限工作区的断网拒绝");
    }

    #[test]
    fn read_only_hint_lowers_risk_by_one_level() {
        let mut d = descriptor("read");
        d.read_only_hint = true;
        assert_eq!(tool("demo", false, d.clone()).spec().risk, PolicyRisk::R1);

        let mut d = descriptor("read");
        d.read_only_hint = true;
        assert_eq!(tool("demo", true, d).spec().risk, PolicyRisk::R2);
    }

    #[test]
    fn destructive_hint_raises_to_r4() {
        let mut d = descriptor("wipe");
        d.destructive_hint = true;
        assert_eq!(tool("demo", false, d).spec().risk, PolicyRisk::R4);
    }

    /// mutates 是计划模式硬门卡（分级档）的唯一依据：只读工具必须放行，写工具必须拦住。
    /// 自由档不启用该门卡，故这里只钉标注本身，不涉及档位。
    #[test]
    fn mutates_follows_read_only_hint() {
        let mut read_only = descriptor("read");
        read_only.read_only_hint = true;
        let spec = tool("demo", false, read_only).spec();
        assert!(!spec.mutates, "readOnlyHint=true 不应标记为写工具");

        let spec = tool("demo", false, descriptor("write")).spec();
        assert!(spec.mutates, "未声明只读的工具必须按写工具处理");
    }

    #[test]
    fn tool_is_available_in_work_mode() {
        assert!(tool("demo", false, descriptor("read")).spec().work);
    }

    #[test]
    fn title_and_summary_are_human_readable() {
        let spec = tool("我的服务器", false, descriptor("get_weather")).spec();
        assert!(spec.title.contains("我的服务器"));
        assert!(spec.title.contains("get_weather"));
        assert_eq!(spec.summary_text.as_deref(), Some("我的服务器/get_weather"));
        // 标题不能等于模型名（否则对话流会显示 `mcp__x__y` 这种内部标识符）
        assert_ne!(spec.title, spec.name);
    }

    #[test]
    fn empty_description_falls_back_to_generated_text() {
        let spec = tool("demo", false, descriptor("do_it")).spec();
        assert!(spec.description.contains("MCP 服务器"));
        assert!(spec.description.contains("do_it"));
    }

    /// schema 必须是 object：非 object 的 inputSchema 直接转发会被 provider 拒绝。
    #[test]
    fn input_schema_is_forced_to_object() {
        let mut d = descriptor("weird");
        d.input_schema = serde_json::json!({"anyOf": [{"type": "string"}]});
        let spec = tool("demo", false, d).spec();
        assert_eq!(spec.schema["type"], "object");
        assert!(spec.schema["properties"].is_object());
    }

    #[test]
    fn grant_target_is_stable_and_non_empty() {
        let tool = tool("demo", false, descriptor("read"));
        let pool = Arc::new(McpPool::new());
        let registry = craft_store::Db::open(&std::env::temp_dir().join("cw-mcp-grant.db")).expect("打开测试库");
        let registry = crate::registry::Registry::new("", Arc::new(registry));
        registry.register(Box::new(McpTool::new(
            "mcp_demo",
            "demo",
            false,
            DEFAULT_TOOL_TIMEOUT_MS,
            descriptor("read"),
            Arc::clone(&pool),
        )));
        let assessment = registry.assess(&[], tool.model_name(), &serde_json::json!({}));
        assert_eq!(assessment.target, "demo/read", "授权目标必须是 <server>/<tool>");
        assert!(assessment.reason.contains("外部 MCP 服务器"), "理由应说明是外部调用：{}", assessment.reason);
        // 未注册时走「未知工具」兜底
        assert!(!assessment.prepared.is_none());
        pool.shutdown();
    }

    #[test]
    fn tool_level_timeout_overrides_server_level() {
        let mut d = descriptor("slow");
        d.meta_timeout_ms = Some(900_000);
        let tool = tool("demo", false, d);
        assert_eq!(tool.effective_timeout_ms(), 900_000);
    }

    #[test]
    fn timeout_is_clamped_to_max() {
        let mut d = descriptor("forever");
        d.meta_timeout_ms = Some(MAX_TOOL_TIMEOUT_MS * 3);
        assert_eq!(tool("demo", false, d).effective_timeout_ms(), MAX_TOOL_TIMEOUT_MS);
    }
}
