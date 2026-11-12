//! 单服务器连接与调用：把 rmcp 的异步客户端封装成「连接 / 列工具 / 调工具」三个动作。
//!
//! 只在本 crate 的 runtime 线程内被调用（见 worker 模块），因此可以自由使用 `async`
//! 与 `&mut self`，不需要考虑跨线程同步。

use std::collections::HashMap;
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, ContentBlock, Implementation, JsonObject,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::transport::streamable_http_client::{StreamableHttpClientTransport, StreamableHttpClientTransportConfig};
use serde_json::{Map, Value};

use crate::config::{ServerConfig, Transport};
use crate::media::MediaCollector;
use crate::worker::{CallOutcome, ServerSnapshot, ToolDescriptor};

/// 一台上线服务器的连接态。
///
/// 关键点：`RunningService` 的服务类型参数是「handler 类型」。用 `()` 作 handler
/// 最简单（`ClientHandler for ()` 提供默认能力），但它不携带 clientInfo——
/// 为了让服务器看到 `CraftWork` 而不是匿名客户端，这里用 `ClientConfig` 作 handler：
/// rmcp 为 `ClientConfig` 实现了 `ClientHandler`，其 `get_info()` 原样返回自身。
struct Connection {
    service: RunningService<RoleClient, ClientConfig>,
    /// 建立连接时拿到的服务器自述与工具表（重连时整体替换）
    snapshot: ServerSnapshot,
}

/// runtime 线程内的服务器表；生命周期与线程一致。
#[derive(Default)]
pub struct RuntimeState {
    connections: HashMap<String, Connection>,
}

impl RuntimeState {
    /// 建立连接并拉取工具列表。失败只影响这一个服务器。
    pub async fn connect(&mut self, config: ServerConfig) -> Result<ServerSnapshot, String> {
        // 同名重建：先断旧连接，避免同一个 id 上叠两条子进程
        self.disconnect(&config.id).await;

        let client_config = ClientConfig::new(
            // 不宣告 sampling / roots / elicitation：CraftWork 不实现这些反向调用，
            // 宣告了却不响应会让服务器卡在等待里。
            ClientCapabilities::default(),
            Implementation::new("CraftWork", crate::client_version()),
        );

        let timeout = Duration::from_millis(config.timeout_ms.max(1) as u64);
        // handler 用 ClientConfig 本身：rmcp 为它实现了 ClientHandler，
        // 服务器因此能看到 clientInfo = CraftWork <version>。
        let handler = client_config.clone();
        let service = match config.transport {
            Transport::Stdio => {
                let transport = build_stdio_transport(&config)?;
                tokio::time::timeout(timeout, handler.serve(transport))
                    .await
                    .map_err(|_| format!("连接超时（{} 毫秒）", config.timeout_ms))?
                    .map_err(|e| format!("连接失败：{}", describe_error(&e)))?
            }
            Transport::Http | Transport::Sse => {
                let transport = build_http_transport(&config)?;
                tokio::time::timeout(timeout, handler.serve(transport))
                    .await
                    .map_err(|_| format!("连接超时（{} 毫秒）", config.timeout_ms))?
                    .map_err(|e| format!("连接失败：{}", describe_error(&e)))?
            }
        };

        let peer_info = service.peer().peer_info();
        let listed = tokio::time::timeout(timeout, service.peer().list_tools(None))
            .await
            .map_err(|_| format!("获取工具列表超时（{} 毫秒）", config.timeout_ms))?
            .map_err(|e| format!("获取工具列表失败：{}", describe_error(&e)))?;

        let tools: Vec<ToolDescriptor> = listed.tools.iter().map(descriptor_from_tool).collect();
        let snapshot = ServerSnapshot {
            server_name: peer_info
                .as_ref()
                .and_then(|info| info.server_info.as_ref())
                .map(|info| info.name.clone())
                .unwrap_or_default(),
            server_version: peer_info
                .as_ref()
                .and_then(|info| info.server_info.as_ref())
                .map(|info| info.version.clone())
                .unwrap_or_default(),
            protocol_version: peer_info
                .as_ref()
                .map(|info| info.protocol_version.to_string())
                .unwrap_or_default(),
            instructions: peer_info
                .as_ref()
                .and_then(|info| info.instructions.clone())
                .unwrap_or_default(),
            tools,
        };
        self.connections.insert(config.id.clone(), Connection { service, snapshot: snapshot.clone() });
        Ok(snapshot)
    }

    /// 调用一个工具，返回已归一的文本 + 媒体。
    pub async fn call(
        &mut self,
        server_id: &str,
        tool_name: &str,
        arguments: Value,
        timeout_ms: i64,
    ) -> Result<CallOutcome, String> {
        let Some(connection) = self.connections.get(server_id) else {
            return Err("服务器未连接".into());
        };
        if connection.service.is_closed() {
            return Err("服务器连接已断开".into());
        }
        let mut params = CallToolRequestParams::new(tool_name.to_string());
        // MCP 要求参数是对象；非对象入参按空对象处理（工具自己的 schema 会报错）
        params.arguments = arguments.as_object().map(|map| JsonObject::from(map.clone()));
        let timeout = Duration::from_millis(timeout_ms.max(1) as u64);
        let result = tokio::time::timeout(timeout, connection.service.peer().call_tool(params))
            .await
            .map_err(|_| format!("调用超时（{timeout_ms} 毫秒）"))?
            .map_err(|e| format!("调用失败：{}", describe_error(&e)))?;
        Ok(normalize_call_result(&result))
    }

    pub fn is_alive(&self, server_id: &str) -> bool {
        self.connections.get(server_id).is_some_and(|c| !c.service.is_closed())
    }

    /// 已连接服务器的工具表（未连接返回 None）。
    pub fn snapshot(&self, server_id: &str) -> Option<&ServerSnapshot> {
        self.connections.get(server_id).map(|c| &c.snapshot)
    }

    pub async fn disconnect(&mut self, server_id: &str) {
        if let Some(mut connection) = self.connections.remove(server_id) {
            // close() 会触发 transport.close()（rmcp 的 graceful_shutdown）：关 stdin 等
            // EOF（最多 3 秒）→ SIGTERM 再等（最多 3 秒）→ SIGKILL，走完整个序列才保证
            // 子进程真的退出。总超时必须盖过整个序列：一旦截断，rmcp 只能把 kill 任务
            // `tokio::spawn` 回 runtime 队列兜底，而退出路径上 runtime 线程随 Shutdown
            // 立即结束，兜底任务永远不执行——不响应 stdin EOF 的长驻服务器就成了孤儿
            // 进程（集成测试 shutdown_kills_stubborn_child_process 防回归）。
            // 15 秒 = 3+3 秒内部等待 + 调度余量；正常服务器在 EOF 或 SIGTERM 就退出，
            // 实际耗时约 3 秒。多等是为了确定性：退出路径上宁可慢几秒也不留孤儿。
            let _ = connection.service.close_with_timeout(Duration::from_secs(15)).await;
        }
    }

    pub async fn close_all(&mut self) {
        let ids: Vec<String> = self.connections.keys().cloned().collect();
        for id in ids {
            self.disconnect(&id).await;
        }
    }
}

/// stdio 传输：独立进程组（异步子进程随之退出）、清洗后的环境 + 用户显式配置的 env。
///
/// 环境策略（安全边界，见 D16）：基础环境来自
/// `craft_core::platform::sanitized_env`（白名单 + 剔除 TOKEN/SECRET 等），
/// 再叠加用户在该服务器配置里**显式填写**的变量——用户主动填写即视为授权，
/// 但绝不注入模型 provider 的密钥。
fn build_stdio_transport(config: &ServerConfig) -> Result<TokioChildProcess, String> {
    use process_wrap::tokio::CommandWrap;
    // 整组终止没有单一的跨平台抽象：process-wrap 10 里 ProcessGroup 仅 Unix
    //（setpgid），Windows 的等价物是 JobObject（作业对象整树终止），两者被平台
    // cfg 分别门控——在 Windows 上 import ProcessGroup 是 E0432（CI 实测）。
    #[cfg(unix)]
    use process_wrap::tokio::ProcessGroup;
    #[cfg(windows)]
    use process_wrap::tokio::{JobObject, KillOnDrop};

    let mut command = tokio::process::Command::new(&config.command);
    if !config.args.is_empty() {
        command.args(&config.args);
    }
    if !config.cwd.is_empty() {
        command.current_dir(&config.cwd);
    }
    command.env_clear();
    for (key, value) in crate::stdio::stdio_env(config) {
        command.env(key, value);
    }
    // 不分配控制台窗口：与命令子进程同一口径（见 craft-core::platform::CREATE_NO_WINDOW）。
    // 否则每次启动 stdio MCP 服务器，屏幕上都会闪一个黑窗。
    // tokio 的 Command 自带 windows 版 creation_flags，不需要 std 的 CommandExt（引了反而 unused）。
    #[cfg(windows)]
    command.creation_flags(craft_core::platform::CREATE_NO_WINDOW);
    // 独立进程组/作业对象：服务器自己 fork 出的子孙进程在超时/退出时能被整组杀掉
    // （与 shell_exec 的安全语义一致，见 craft-core::platform::kill_process_group）。
    let mut wrap = CommandWrap::from(command);
    #[cfg(unix)]
    wrap.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    {
        // JobObject 把子进程整树收进作业；KillOnDrop 让意外 drop 也终止整树，
        // 与 Unix 侧 rmcp ChildWithCleanup::drop 的兜底杀组语义对齐。
        wrap.wrap(JobObject);
        wrap.wrap(KillOnDrop);
    }
    let (transport, stderr) = TokioChildProcess::builder(wrap)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 MCP 服务器进程失败：{e}"))?;
    if let Some(stderr) = stderr {
        // stderr 交给后台线程读掉：不读会把管道灌满导致服务器阻塞在写日志上。
        crate::stdio::spawn_stderr_drain(config.id.clone(), stderr);
    }
    Ok(transport)
}

/// 远程传输：streamable HTTP（rmcp 客户端会自动兼容旧式 SSE 服务）。
///
/// 使用仓库统一的 reqwest 客户端构造（经 `craft_core::network` 应用代理，D14），
/// 而不是让 rmcp 自己建 client——否则用户的代理设置对 MCP 不生效。
fn build_http_transport(
    config: &ServerConfig,
) -> Result<StreamableHttpClientTransport<reqwest::Client>, String> {
    let client = crate::http::build_client().map_err(|e| format!("构造 HTTP 客户端失败：{e}"))?;
    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(config.url.clone());
    if !config.headers.is_empty() {
        transport_config = transport_config.custom_headers(crate::http::header_map(&config.headers)?);
    }
    Ok(StreamableHttpClientTransport::with_client(client, transport_config))
}

fn descriptor_from_tool(tool: &rmcp::model::Tool) -> ToolDescriptor {
    let annotations = tool.annotations.as_ref();
    ToolDescriptor {
        tool_name: tool.name.to_string(),
        title: tool.title.clone(),
        description: tool.description.as_ref().map(|d| d.to_string()).unwrap_or_default(),
        input_schema: Value::Object((*tool.input_schema).clone()),
        read_only_hint: annotations.and_then(|a| a.read_only_hint).unwrap_or(false),
        destructive_hint: annotations.and_then(|a| a.destructive_hint).unwrap_or(false),
        idempotent_hint: annotations.and_then(|a| a.idempotent_hint).unwrap_or(false),
        meta_timeout_ms: tool
            .meta
            .as_ref()
            .and_then(|m| m.get("timeoutMs"))
            .and_then(Value::as_i64)
            .filter(|v| *v > 0),
    }
}

/// 把 MCP 的 `CallToolResult` 归一为文本 + 媒体。
///
/// 规则（媒体改为落盘引用而非内联）：
/// - text 块按序拼接（空块跳过）
/// - image 块进媒体收集器，不合规的降级为占位说明
/// - audio / resource / resource_link 降级为文本占位（CraftWork 无对应媒体通道）
/// - `structuredContent` 有内容时追加 `Structured content:` 段
/// - `isError` 时给文本加前缀
pub fn normalize_call_result(result: &rmcp::model::CallToolResult) -> CallOutcome {
    let mut texts: Vec<String> = Vec::new();
    let mut collector = MediaCollector::new();
    let mut image_index = 0usize;
    for block in &result.content {
        match block {
            ContentBlock::Text(text) => {
                if !text.text.is_empty() {
                    texts.push(text.text.clone());
                }
            }
            ContentBlock::Image(image) => {
                collector.push_image(image_index, &image.data, &image.mime_type);
                image_index += 1;
            }
            ContentBlock::Audio(audio) => {
                texts.push(format!("[MCP 音频内容已省略：{}]", audio.mime_type));
            }
            ContentBlock::Resource(resource) => {
                texts.push(format!("MCP 资源内容：{}", stringify_resource(&resource.resource)));
            }
            ContentBlock::ResourceLink(link) => {
                texts.push(format!("MCP 资源链接：{} {}", link.uri, link.name));
            }
            _ => {}
        }
    }
    // 占位说明跟在文本之后：用户与模型都能看到「这里本该有图但没给」
    texts.extend(collector.notes.iter().cloned());

    if let Some(structured) = &result.structured_content
        && has_informative_value(structured) {
            texts.push(format!("Structured content:\n{}", stringify(structured)));
        }

    let mut text = texts.join("\n\n");
    let is_error = result.is_error.unwrap_or(false);
    if is_error {
        text = if text.is_empty() {
            "MCP 工具返回错误".to_string()
        } else {
            format!("MCP 工具返回错误：\n{text}")
        };
    }
    if text.is_empty() {
        text = "（MCP 工具没有返回内容）".to_string();
    }
    CallOutcome { text, media: collector.media, is_error }
}

/// 空值（null / 空对象 / 空数组 / 空串）不追加结构化段，避免制造无信息的噪声块。
fn has_informative_value(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(arr) => !arr.is_empty(),
        Value::Object(map) => !map.is_empty(),
        _ => true,
    }
}

fn stringify(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// 内嵌资源转文本：文本资源直接给正文，二进制资源给「blob + mime」的说明，
/// 不把 base64 灌进模型上下文（CraftWork 没有资源解码通道）。
fn stringify_resource(resource: &rmcp::model::ResourceContents) -> String {
    match resource {
        rmcp::model::ResourceContents::TextResourceContents { uri, text, mime_type, .. } => {
            let mime = mime_type.clone().unwrap_or_default();
            format!("{uri}（{mime}）\n{text}")
        }
        rmcp::model::ResourceContents::BlobResourceContents { uri, blob, mime_type, .. } => {
            let mime = mime_type.clone().unwrap_or_default();
            format!(
                "{uri}（{mime}，{} 字节的二进制内容已省略）",
                crate::media::base64_decoded_len(blob)
            )
        }
        // 枚举标了 non_exhaustive：未来新增内容类型时降级为提示，不中断调用
        _ => "（未知的 MCP 资源类型，内容已省略）".to_string(),
    }
}

/// 错误归一：保留原始消息（含 JSON-RPC 错误码），便于用户排查。
fn describe_error(error: &impl std::fmt::Display) -> String {
    let message = error.to_string();
    // rmcp 的消息可能很长且含调试细节；截断到合理长度避免撑爆审批卡与日志
    let trimmed: String = message.chars().take(500).collect();
    if trimmed.chars().count() < message.chars().count() { format!("{trimmed}…") } else { trimmed }
}

/// 结构化内容的便捷构造（供测试与工具层复用）：保证是对象。
pub fn ensure_object_schema(schema: Value) -> Value {
    let mut map = match schema {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("type".into(), Value::String("object".into()));
    match map.get("properties") {
        Some(Value::Object(_)) => {}
        _ => {
            map.insert("properties".into(), Value::Object(Map::new()));
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, TextContent};

    /// 构造测试用的结果：`CallToolResult` 是 non_exhaustive，只能经构造函数
    /// 或默认值做字段赋值。
    fn result_with(content: Vec<ContentBlock>) -> CallToolResult {
        let mut result = CallToolResult::default();
        result.content = content;
        result
    }

    #[test]
    fn text_blocks_are_joined_in_order() {
        let result = result_with(vec![
            ContentBlock::Text(TextContent::new("第一段")),
            ContentBlock::Text(TextContent::new("")),
            ContentBlock::Text(TextContent::new("第二段")),
        ]);
        let outcome = normalize_call_result(&result);
        assert_eq!(outcome.text, "第一段\n\n第二段");
        assert!(!outcome.is_error);
        assert!(outcome.media.is_empty());
    }

    #[test]
    fn images_are_collected_as_media() {
        let result = result_with(vec![
            ContentBlock::Text(TextContent::new("这是截图")),
            ContentBlock::image(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==",
                "image/png",
            ),
        ]);
        let outcome = normalize_call_result(&result);
        assert_eq!(outcome.media.len(), 1);
        assert_eq!(outcome.media[0].mime, "image/png");
        assert_eq!(outcome.text, "这是截图");
    }

    #[test]
    fn unsupported_image_becomes_text_placeholder() {
        let result = result_with(vec![ContentBlock::image("AAAA", "image/tiff")]);
        let outcome = normalize_call_result(&result);
        assert!(outcome.media.is_empty());
        assert!(outcome.text.contains("不支持的格式 image/tiff"));
    }

    #[test]
    fn audio_and_resource_become_text_placeholders() {
        let result = result_with(vec![ContentBlock::audio("AAAA", "audio/wav")]);
        let outcome = normalize_call_result(&result);
        assert!(outcome.text.contains("MCP 音频内容已省略：audio/wav"));
    }

    #[test]
    fn structured_content_is_appended_only_when_informative() {
        let mut with_value = result_with(vec![ContentBlock::Text(TextContent::new("ok"))]);
        with_value.structured_content = Some(serde_json::json!({"count": 3}));
        let outcome = normalize_call_result(&with_value);
        assert!(outcome.text.contains("Structured content:"));
        assert!(outcome.text.contains("\"count\": 3"));

        let mut empty = result_with(vec![ContentBlock::Text(TextContent::new("ok"))]);
        empty.structured_content = Some(serde_json::json!({}));
        let outcome = normalize_call_result(&empty);
        assert!(!outcome.text.contains("Structured content:"));
    }

    #[test]
    fn error_result_gets_prefix() {
        let mut result = result_with(vec![ContentBlock::Text(TextContent::new("文件不存在"))]);
        result.is_error = Some(true);
        let outcome = normalize_call_result(&result);
        assert!(outcome.is_error);
        assert!(outcome.text.starts_with("MCP 工具返回错误："));
    }

    #[test]
    fn empty_result_has_placeholder_text() {
        let result = CallToolResult::default();
        let outcome = normalize_call_result(&result);
        assert_eq!(outcome.text, "（MCP 工具没有返回内容）");
    }

    #[test]
    fn object_schema_is_forced() {
        let schema = ensure_object_schema(serde_json::json!({ "anyOf": [] }));
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());

        let array_schema = ensure_object_schema(serde_json::json!([]));
        assert_eq!(array_schema["type"], "object");
    }
}
