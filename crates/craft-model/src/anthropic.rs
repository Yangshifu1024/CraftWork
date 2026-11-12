//! Anthropic 协议适配器。
//! 关键点：三个 cache_control 断点（系统稳定块 / 系统动态块 / 历史最后一块，配合
//! 系统提示词缓存层命中增量前缀）、连续同角色合并（协议要求 user/assistant 交替）、
//! tool 结果以 user 身份回传、思考档位按 thinking 预算映射。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::errors::{missing_endpoint_error, stream_error, ModelError};
use crate::sse::{line_to_event, open_sse, LineReader, SseEvent, TailBuffer, ToolCallBuilder};
use crate::types::{
    attachment_prompt_text, header_has_session_placeholder, resolve_header_value, Chunk, Client, Request, Usage, RAW_TAIL_CAPACITY,
};
use crate::openai::make_stream_end;

/// 端点拼接：base 已含 /v1 时跳过 path 的第一段，避免重复 /v1。
pub fn join_api_path(base: &str, path: &str) -> String {
    let clean_base = base.trim_end_matches('/');
    let segments = path.trim_start_matches('/');
    let first_seg = segments.split('/').next().unwrap_or_default();
    if !first_seg.is_empty() && clean_base.ends_with(&format!("/{first_seg}")) {
        return format!("{clean_base}/{}", &segments[first_seg.len() + 1..]);
    }
    format!("{clean_base}/{segments}")
}

/// 消息序列转换：system 汇入顶层；连续同角色合并（满足 user/assistant 交替要求）。
pub fn build_anthropic_messages(req: &Request) -> (Vec<Value>, Vec<Value>) {
    let mut system: Vec<Value> = Vec::new();
    for block in &req.system {
        let mut entry = json!({ "type": "text", "text": block.content });
        if block.cache_control {
            entry["cache_control"] = json!({ "type": "ephemeral" });
        }
        system.push(entry);
    }

    let mut messages: Vec<Value> = Vec::new();
    let push = |messages: &mut Vec<Value>, role: &str, block: Value| {
        if let Some(last) = messages
            .last_mut()
            .filter(|last| last["role"] == role)
        {
            last["content"].as_array_mut().unwrap().push(block);
            return;
        }
        messages.push(json!({ "role": role, "content": [block] }));
    };

    for msg in &req.messages {
        match msg.role.as_str() {
            "system" => {
                system.push(json!({ "type": "text", "text": msg.content }));
            }
            "user" => {
                let mut text_parts: Vec<String> = vec![msg.content.clone()];
                for att in &msg.attachments {
                    if !att.skipped.is_empty() {
                        continue;
                    }
                    if !att.data.is_empty() {
                        push(
                            &mut messages,
                            "user",
                            json!({ "type": "image", "source": { "type": "base64", "media_type": att.mime, "data": att.data } }),
                        );
                    }
                    if !att.text.is_empty() {
                        text_parts.push(attachment_prompt_text(att));
                    }
                }
                let text = text_parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
                if !text.is_empty() {
                    push(&mut messages, "user", json!({ "type": "text", "text": text }));
                }
            }
            "assistant" => {
                if !msg.content.is_empty() {
                    push(&mut messages, "assistant", json!({ "type": "text", "text": msg.content }));
                }
                for call in &msg.tool_calls {
                    push(
                        &mut messages,
                        "assistant",
                        json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": call.args }),
                    );
                }
            }
            "tool" => {
                // 工具结果 + 图片（MCP 截图等）：tool_result 的 content 改为数组，
                // 图片块在前、文本块在后。顺序是硬约束——Anthropic 兼容网关只解析
                // tool_result 开头的连续 image 块，文本领先后图片会被静默丢弃
                // （实机踩坑记录：兼容网关只认 image-first）。
                // 模型不支持视觉（supports_vision=false）时跳过图片，只发文本，
                // 避免请求被 API 以不支持的 content 类型拒绝。
                let valid_images: Vec<&crate::types::Attachment> = msg
                    .attachments
                    .iter()
                    .filter(|att| att.skipped.is_empty() && !att.data.is_empty())
                    .collect();
                let images: Vec<&crate::types::Attachment> =
                    if req.supports_vision { valid_images.clone() } else { Vec::new() };
                let skipped_images = valid_images.len() - images.len();
                if images.is_empty() {
                    let mut text = msg.content.clone();
                    if skipped_images > 0 {
                        text.push_str(&format!(
                            "\n\n[本次工具结果包含 {skipped_images} 张图片，当前模型不支持图片输入，已省略]"
                        ));
                    }
                    push(
                        &mut messages,
                        "user",
                        json!({ "type": "tool_result", "tool_use_id": msg.tool_call_id, "content": text }),
                    );
                } else {
                    let mut blocks: Vec<Value> = images
                        .iter()
                        .map(|att| {
                            json!({ "type": "image", "source": { "type": "base64", "media_type": att.mime, "data": att.data } })
                        })
                        .collect();
                    blocks.push(json!({ "type": "text", "text": msg.content }));
                    push(
                        &mut messages,
                        "user",
                        json!({ "type": "tool_result", "tool_use_id": msg.tool_call_id, "content": blocks }),
                    );
                }
            }
            _ => {}
        }
    }

    // 第三个缓存断点：历史最后一条消息的最后一个内容块（Agent 每轮尾部追加，下一轮从断点命中）
    if let Some(last_block) = messages
        .last_mut()
        .and_then(|last| last["content"].as_array_mut())
        .and_then(|arr| arr.last_mut())
    {
        last_block["cache_control"] = json!({ "type": "ephemeral" });
    }

    (system, messages)
}

/// 拼装 Anthropic 协议请求体：budget 型思考档位映射 thinking 预算并抬高 max_tokens；
/// 工具 schema 为空对象时回退最小合法 schema；零 max_tokens 回退 4096。
pub fn build_anthropic_body(model_id: &str, req: &Request) -> Value {
    let (system, messages) = build_anthropic_messages(req);
    let mut body = json!({
        "model": model_id,
        "max_tokens": if req.max_tokens > 0 { req.max_tokens } else { 4096 },
        "messages": messages,
        "stream": true,
    });
    // 思考档位：budget 型映射 thinking 预算（固定映射值，档位归一 off/low/high）。
    // Anthropic 要求 max_tokens > budget_tokens，启用思考时抬高 max_tokens；off/未选择不发，
    // 请求体与既有行为一致。
    if req.reasoning_kind == "budget" && !req.reasoning_level.is_empty() && req.reasoning_level != "off" {
        let budget: i64 = if req.reasoning_level == "high" { 31999 } else { 4096 };
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        let current = body["max_tokens"].as_i64().unwrap_or(4096);
        if current <= budget {
            body["max_tokens"] = json!(budget + 8192);
        }
    }
    if !system.is_empty() {
        body["system"] = Value::Array(system);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": if t.schema.is_object() && !t.schema.as_object().unwrap().is_empty() {
                            t.schema.clone()
                        } else {
                            json!({ "type": "object", "properties": {} })
                        },
                    })
                })
                .collect(),
        );
    }
    body
}

/// Anthropic 协议客户端（密钥经 x-api-key 头发送，协议版本固定 2023-06-01）。
#[derive(Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    provider_name: String,
    /// 提供商级自定义请求头（用户在提供商配置里声明，随每次请求附加）。
    extra_headers: Vec<crate::types::HeaderPair>,
}

impl AnthropicClient {
    /// 创建客户端；endpoint 为 base 地址（含 /v1 前缀也可，拼接时自动去重）。
    /// proxy 为出站代理配置（调用方从 settings 即时解析传入，None = system 模式兜底），
    /// 经 craft_core::network 应用到 HTTP 客户端（D14）。
    pub fn new(
        endpoint: &str,
        api_key: &str,
        provider_name: &str,
        proxy: Option<&craft_core::network::ProxyConfig>,
    ) -> Self {
        Self {
            http: crate::net::build_http_client(proxy),
            endpoint: endpoint.to_string(),
            api_key: api_key.to_string(),
            provider_name: provider_name.to_string(),
            extra_headers: Vec::new(),
        }
    }

    /// 附加提供商级自定义请求头（链式）。
    pub fn with_headers(mut self, headers: Vec<crate::types::HeaderPair>) -> Self {
        self.extra_headers = headers;
        self
    }
}

/// SSE 事件处理上下文（跨事件状态）。
struct AnthropicStreamState {
    usage: Usage,
    usage_seen: bool,
    /// 缓存写 / 读分量的内部记账：usage.cache_tokens 是合并值，message_delta
    /// 可能只带其中一个字段（非 0 覆盖），重算合并值需要两个分量都在手。
    cache_write: i64,
    cache_read: i64,
    tool_calls: ToolCallBuilder,
    /// index → 已见 content_block_start 的 tool_use（参数稍后增量给出）
    pending: std::collections::HashMap<i64, (String, String)>,
    /// message_delta 携带的结束原因（"end_turn" / "max_tokens" / "tool_use"…），原样带出
    stop_reason: Option<String>,
    /// 是否已收到 message_stop（显式结束帧；EOF 兜底判定用）
    seen_stop: bool,
}

impl AnthropicStreamState {
    fn new() -> Self {
        Self {
            usage: Usage::empty(),
            usage_seen: false,
            cache_write: 0,
            cache_read: 0,
            tool_calls: ToolCallBuilder::default(),
            pending: std::collections::HashMap::new(),
            stop_reason: None,
            seen_stop: false,
        }
    }

    /// 按 Anthropic 语义重算归一化用量：input 不含缓存，完整提示词 = input + 缓存写 + 缓存读。
    fn recompute_usage(&mut self) {
        self.usage.cache_tokens = self.cache_write + self.cache_read;
        self.usage.cache_write_tokens = self.cache_write;
        self.usage.prompt_tokens = self.usage.input_tokens + self.cache_write + self.cache_read;
    }

    /// 从 usage 对象合并缓存字段（0 / 缺失不覆盖已知值），随后重算归一化口径。
    fn merge_cache_fields(&mut self, u: &Value) {
        if let Some(write) = u.get("cache_creation_input_tokens").and_then(Value::as_i64).filter(|v| *v != 0) {
            self.cache_write = write;
        }
        if let Some(read) = u.get("cache_read_input_tokens").and_then(Value::as_i64).filter(|v| *v != 0) {
            self.cache_read = read;
        }
        self.recompute_usage();
    }

    /// 解码一个事件：Some(Ok(Some(chunk))) 产出片段；Some(Ok(None)) 无产出；Some(Err) 流内错误；None 非法忽略。
    fn decode(&mut self, provider: &str, payload: &str) -> Option<Result<Option<Chunk>, ModelError>> {
        let Ok(event) = serde_json::from_str::<Value>(payload) else {
            return None;
        };
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or_default();
        match event_type {
            "error" => {
                let err = event.get("error");
                let message = err.and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or_default();
                let kind = err.and_then(|e| e.get("type")).and_then(Value::as_str).unwrap_or_default();
                Some(Err(stream_error(provider, message, kind)))
            }
            "message_start" => {
                // is_object 防御：部分网关发 "usage": null（pointer 命中 Null 却无字段）
                if let Some(u) = event.pointer("/message/usage").filter(|u| u.is_object()) {
                    self.usage_seen = true;
                    self.usage.input_tokens = u.get("input_tokens").and_then(Value::as_i64).unwrap_or(0);
                    self.merge_cache_fields(u);
                    // 提前产出部分用量（input/缓存已确定、output 为 0）：取消/流错误路径据此部分记账，
                    // 正常路径由 message_delta / 收尾 chunk 覆盖为全量
                    return Some(Ok(Some(Chunk { usage: Some(self.usage.clone()), ..Default::default() })));
                }
                Some(Ok(None))
            }
            "content_block_start" => {
                let block = event.get("content_block");
                let block_type = block.and_then(|b| b.get("type")).and_then(Value::as_str).unwrap_or_default();
                if block_type == "tool_use" {
                    let id = block.and_then(|b| b.get("id")).and_then(Value::as_str).unwrap_or_default().to_string();
                    let name = block.and_then(|b| b.get("name")).and_then(Value::as_str).unwrap_or_default().to_string();
                    if !id.is_empty() && !name.is_empty() {
                        let index = event.get("index").and_then(Value::as_i64).unwrap_or(0);
                        self.pending.insert(index, (id.clone(), name.clone()));
                        // content_block_start 自带 input（部分模型直接给全量参数）：
                        // 非空时序列化后作为参数首段并入聚合，后续增量接着拼
                        let input = block.and_then(|b| b.get("input")).cloned().unwrap_or(json!({}));
                        let raw = serde_json::to_string(&input).unwrap_or_default();
                        if raw != "{}" {
                            self.tool_calls.add(index, &id, &name, &raw);
                        }
                    }
                }
                Some(Ok(None))
            }
            "content_block_delta" => {
                let delta = event.get("delta");
                let delta_type = delta.and_then(|d| d.get("type")).and_then(Value::as_str).unwrap_or_default();
                match delta_type {
                    "text_delta" => {
                        let text = delta.and_then(|d| d.get("text")).and_then(Value::as_str).unwrap_or_default();
                        if text.is_empty() {
                            Some(Ok(None))
                        } else {
                            Some(Ok(Some(Chunk { text: Some(text.to_string()), ..Default::default() })))
                        }
                    }
                    "thinking_delta" => {
                        let thinking = delta.and_then(|d| d.get("thinking")).and_then(Value::as_str).unwrap_or_default();
                        if thinking.is_empty() {
                            Some(Ok(None))
                        } else {
                            Some(Ok(Some(Chunk { reasoning: Some(thinking.to_string()), ..Default::default() })))
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !partial.is_empty() {
                            let index = event.get("index").and_then(Value::as_i64).unwrap_or(0);
                            let info = self.pending.get(&index).cloned().unwrap_or_default();
                            self.tool_calls.add(index, &info.0, &info.1, partial);
                        }
                        Some(Ok(None))
                    }
                    _ => Some(Ok(None)),
                }
            }
            "message_delta" => {
                if let Some(reason) = event
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    self.stop_reason = Some(reason.to_string());
                }
                if let Some(u) = event.get("usage").filter(|u| u.is_object()) {
                    self.usage_seen = true;
                    if let Some(output) = u.get("output_tokens").and_then(Value::as_i64) {
                        self.usage.output_tokens = output;
                    }
                    // message_delta 通常只带 output_tokens；input_tokens 为 0（缺失）时不覆盖
                    if let Some(input) = u.get("input_tokens").and_then(Value::as_i64).filter(|v| *v != 0) {
                        self.usage.input_tokens = input;
                    }
                    // 部分网关把缓存字段并入 message_delta（非 0 才合并，防 0 值覆盖已知值）
                    self.merge_cache_fields(u);
                }
                Some(Ok(None))
            }
            "message_stop" => {
                self.seen_stop = true;
                Some(Ok(None))
            }
            _ => Some(Ok(None)),
        }
    }
}

impl Client for AnthropicClient {
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(AnthropicClient {
            http: self.http.clone(),
            endpoint: self.endpoint.clone(),
            api_key: self.api_key.clone(),
            provider_name: self.provider_name.clone(),
            extra_headers: self.extra_headers.clone(),
        })
    }

    fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
        let body = build_anthropic_body(&req.model, req);
        let endpoint = self.endpoint.clone();
        let api_key = self.api_key.clone();
        let provider = self.provider_name.clone();
        let http = self.http.clone();
        let extra_headers = self.extra_headers.clone();
        Box::pin(async_stream::stream! {
            if endpoint.is_empty() {
                yield Err(missing_endpoint_error(&provider));
                return;
            }
            let url = join_api_path(&endpoint, "/v1/messages");
            let mut request = http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01");
            // 提供商自定义头最后附加：与协议固定头同名时可覆盖（保存侧已禁止冲突名，防御性兜底）；
            // {sessionId} 占位符按请求的会话 ID 替换，未携带会话 ID 的头跳过（不发坏路由值）
            for h in &extra_headers {
                if header_has_session_placeholder(&h.value) && req.session_id.is_empty() {
                    continue;
                }
                request = request
                    .header(h.name.as_str(), resolve_header_value(&h.value, &req.session_id).as_str());
            }
            let request = request.json(&body);
            let resp = match open_sse(request, &provider).await {
                Ok(resp) => resp,
                Err(err) => {
                    yield Err(err);
                    return;
                }
            };

            let mut state = AnthropicStreamState::new();
            let mut tail = TailBuffer::new(RAW_TAIL_CAPACITY);
            let mut bytes_stream = resp.bytes_stream();
            let mut reader = LineReader::new();

            loop {
                let chunk = bytes_stream.next().await;
                let bytes = match chunk {
                    Some(Ok(bytes)) => bytes,
                    Some(Err(err)) => {
                        yield Err(crate::errors::transport_error(&provider, &err));
                        return;
                    }
                    None => {
                        if let Some(payload) = reader
                            .finish()
                            .as_deref()
                            .and_then(line_to_event)
                            .and_then(|event| match event {
                                SseEvent::Data(payload) => Some(payload),
                                SseEvent::Done => None,
                            })
                        {
                            tail.push(&payload);
                            match state.decode(&provider, &payload) {
                                Some(Ok(Some(chunk))) => yield Ok(chunk),
                                Some(Err(err)) => {
                                    yield Err(err);
                                    return;
                                }
                                _ => {}
                            }
                        }
                        break;
                    }
                };
                for line in reader.push_bytes(&bytes) {
                    match line_to_event(&line) {
                        Some(SseEvent::Done) => {}
                        Some(SseEvent::Data(payload)) => {
                            tail.push(&payload);
                            match state.decode(&provider, &payload) {
                                Some(Ok(Some(chunk))) => yield Ok(chunk),
                                Some(Err(err)) => {
                                    yield Err(err);
                                    return;
                                }
                                _ => {}
                            }
                        }
                        None => {}
                    }
                }
            }

            // 收尾：残余工具调用 → 用量（未见 usage 事件时不产出：全 0 值会污染「最新一行用量」，
            // 调用方按「网关未给用量」处理）→ done 标记（携带结束元数据）
            for call in state.tool_calls.flush() {
                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
            }
            if state.usage_seen {
                yield Ok(Chunk { usage: Some(state.usage), ..Default::default() });
            }
            yield Ok(Chunk {
                done: true,
                end: Some(make_stream_end(
                    state.stop_reason,
                    state.seen_stop,
                    std::mem::take(&mut reader.stats),
                    tail.as_str().to_string(),
                )),
                ..Default::default()
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, SystemBlock, ToolCall, ToolSpec};

    #[test]
    fn endpoint_path_join_v1_deduplication() {
        assert_eq!(join_api_path("https://api.anthropic.com", "/v1/messages"), "https://api.anthropic.com/v1/messages");
        assert_eq!(join_api_path("https://api.anthropic.com/v1", "/v1/messages"), "https://api.anthropic.com/v1/messages");
        assert_eq!(join_api_path("https://gw.example.com/api/", "/v1/messages"), "https://gw.example.com/api/v1/messages");
        assert_eq!(join_api_path("https://gw.example.com", "v1/messages"), "https://gw.example.com/v1/messages");
    }

    fn sample_request() -> Request {
        Request {
            model: "claude-x".into(),
            mode: "code".into(),
            system: vec![
                SystemBlock { content: "稳定块".into(), cache_control: true },
                SystemBlock { content: "动态块".into(), cache_control: false },
            ],
            messages: vec![
                Message::new("user", "你好"),
                {
                    let mut m = Message::new("assistant", "回答");
                    m.tool_calls = vec![ToolCall { id: "call_1".into(), name: "read".into(), args: serde_json::json!({"p": 1}) }];
                    m
                },
                {
                    let mut m = Message::new("tool", "结果");
                    m.tool_call_id = "call_1".into();
                    m
                },
                Message::new("user", "继续"),
            ],
            tools: vec![ToolSpec { name: "read".into(), description: "读".into(), schema: serde_json::json!({"type": "object"}) }],
            max_tokens: 2048,
            reasoning_kind: String::new(),
            reasoning_level: String::new(),
            session_id: "th_sample".into(),
            // 默认无视觉能力（不发图片）；图片投影专项测试里显式开启
            supports_vision: false,
        }
    }

    #[test]
    fn thinking_budget_injection_and_max_tokens_lift() {
        // budget 型 low/high 注入 thinking 预算，max_tokens 抬到 budget 之上
        let mut req = sample_request();
        req.reasoning_kind = "budget".into();
        req.reasoning_level = "low".into();
        let body = build_anthropic_body("claude-x", &req);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 4096);
        assert_eq!(body["max_tokens"], 12288); // 4096 + 8192

        req.reasoning_level = "high".into();
        let body = build_anthropic_body("claude-x", &req);
        assert_eq!(body["thinking"]["budget_tokens"], 31999);
        assert_eq!(body["max_tokens"], 40191);

        // max_tokens 本就大于预算时不改动
        req.max_tokens = 64000;
        req.reasoning_level = "high".into();
        let body = build_anthropic_body("claude-x", &req);
        assert_eq!(body["max_tokens"], 64000);

        // off / 未选择：不发 thinking，行为与既有一致
        for level in ["off", ""] {
            req.reasoning_level = level.into();
            let body = build_anthropic_body("claude-x", &req);
            assert!(body.get("thinking").is_none());
        }
        // effort 型模型不经 budget 路径
        req.reasoning_kind = "effort".into();
        req.reasoning_level = "high".into();
        let body = build_anthropic_body("claude-x", &req);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn message_conversion_consecutive_same_role_merge_and_cache_breakpoints() {
        let (system, messages) = build_anthropic_messages(&sample_request());
        // system：稳定块（带 cache_control）+ 动态块
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert!(system[1].get("cache_control").is_none());

        // user(你好) → assistant(回答 + tool_use 合并为一条) → user(tool_result + 继续 合并为一条)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        let assistant_blocks = messages[1]["content"].as_array().unwrap();
        assert_eq!(assistant_blocks.len(), 2);
        assert_eq!(assistant_blocks[0]["type"], "text");
        assert_eq!(assistant_blocks[1]["type"], "tool_use");

        let last = messages.last().unwrap();
        let last_blocks = last["content"].as_array().unwrap();
        assert_eq!(last_blocks.len(), 2);
        assert_eq!(last_blocks[0]["type"], "tool_result");
        assert_eq!(last_blocks[0]["tool_use_id"], "call_1");
        assert_eq!(last_blocks[1]["type"], "text");
        // 第三个缓存断点：最后一条消息的最后一个内容块
        assert_eq!(last_blocks[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn request_body_construction() {
        let body = build_anthropic_body("claude-x", &sample_request());
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["stream"], true);
        assert_eq!(body["tools"][0]["input_schema"], serde_json::json!({"type": "object"}));
        // 空 schema 时回退最小对象
        let mut empty = sample_request();
        empty.tools = vec![ToolSpec { name: "t".into(), description: String::new(), schema: serde_json::json!({}) }];
        let body2 = build_anthropic_body("claude-x", &empty);
        assert_eq!(body2["tools"][0]["input_schema"], serde_json::json!({"type": "object", "properties": {}}));
        // 零 maxTokens 回退 4096
        empty.max_tokens = 0;
        assert_eq!(build_anthropic_body("claude-x", &empty)["max_tokens"], 4096);
    }

    #[test]
    fn repaired_sandwiched_sequence_projects_tool_result_before_feedback_user() {
        // `crates/craft-model/src/integrity.rs` 模块头：夹层坏序列经整理后投影。Anthropic 虽以 user
        // 承载 tool_result 而更宽松，但顺序错乱同样让模型读不懂上下文——tool_result 块
        // 必须先于用户反馈文本进入请求。
        let repaired = crate::integrity::repair_tool_call_integrity(vec![
            Message::new("user", "做个计划"),
            {
                let mut m = Message::new("assistant", "");
                m.tool_calls = vec![crate::types::ToolCall { id: "call_1".into(), name: "exit_plan".into(), args: serde_json::json!({}) }];
                m
            },
            Message::new("user", "第一步拆细一点"),
            {
                let mut m = Message::new("tool", "用户没有批准这份计划");
                m.tool_call_id = "call_1".into();
                m
            },
        ]);
        assert_eq!(repaired.fixes.len(), 1, "夹层用户消息应被移动");
        let req = Request { messages: repaired.messages, ..sample_request() };
        let (_, messages) = build_anthropic_messages(&req);
        // user(做个计划) → assistant(tool_use) → user(tool_result + 反馈文本)：连续 user
        // 消息按既有规则合并为一条，tool_result 块必须排在反馈 text 块之前——
        // Anthropic 协议下这就是「工具结果先于用户反馈」的形状。
        let mut tool_result_pos: Option<(usize, usize)> = None;
        let mut feedback_pos: Option<(usize, usize)> = None;
        for (mi, m) in messages.iter().enumerate() {
            if let Some(blocks) = m["content"].as_array() {
                for (bi, b) in blocks.iter().enumerate() {
                    if b["type"] == "tool_result" {
                        tool_result_pos = Some((mi, bi));
                    }
                    if b["type"] == "text" && b["text"] == "第一步拆细一点" {
                        feedback_pos = Some((mi, bi));
                    }
                }
            }
        }
        let (tm, tb) = tool_result_pos.expect("应存在 tool_result 块");
        let (fm, fb) = feedback_pos.expect("反馈文本应保留在请求里");
        assert!(
            tm < fm || (tm == fm && tb < fb),
            "tool_result 块必须先于用户反馈：{messages:?}"
        );
    }

    #[test]
    fn event_decoding_full_stream() {
        let mut state = AnthropicStreamState::new();
        state
            .decode(
                "p",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":100,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}"#,
            )
            .unwrap()
            .unwrap();
        assert_eq!((state.usage.input_tokens, state.usage.cache_tokens, state.usage.cache_write_tokens), (100, 15, 10));

        state
            .decode("p", r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"read","input":{}}}"#)
            .unwrap()
            .unwrap();
        let chunk = state
            .decode("p", r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好"}}"#)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(chunk.text.as_deref(), Some("你好"));
        let chunk = state
            .decode("p", r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"想"}}"#)
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(chunk.reasoning.as_deref(), Some("想"));
        state
            .decode("p", r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"a\":1}"}}"#)
            .unwrap()
            .unwrap();
        state
            .decode("p", r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":42}}"#)
            .unwrap()
            .unwrap();
        assert_eq!(state.usage.output_tokens, 42);
        // message_delta 通常只带 output_tokens：prompt_tokens 保持 message_start 的归一化值（100+10+5）
        assert_eq!(state.usage.prompt_tokens, 115);
        // stop_reason 原样带出（max_tokens 即输出上限截断的标志）
        assert_eq!(state.stop_reason.as_deref(), Some("max_tokens"));
        assert!(!state.seen_stop);
        state.decode("p", r#"{"type":"message_stop"}"#).unwrap().unwrap();
        assert!(state.seen_stop);
        // 流内错误
        assert!(state.decode("p", r#"{"type":"error","error":{"message":"overloaded","type":"x"}}"#).unwrap().is_err());
        let calls = state.tool_calls.flush();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "t1");
        assert_eq!(calls[0].args, serde_json::json!({"a": 1}));
    }

    #[test]
    fn message_start_emits_partial_usage_chunk() {
        let mut state = AnthropicStreamState::new();
        let chunk = state
            .decode(
                "p",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":100,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}"#,
            )
            .unwrap()
            .unwrap()
            .expect("message_start 应产出部分用量 chunk");
        let usage = chunk.usage.expect("部分用量 chunk 应携带 usage");
        assert_eq!((usage.input_tokens, usage.output_tokens), (100, 0));
        // 归一化口径：input 不含缓存，完整提示词 = input + 缓存写 + 缓存读
        assert_eq!(usage.prompt_tokens, 115);
        assert_eq!((usage.cache_tokens, usage.cache_write_tokens), (15, 10));
    }

    #[test]
    fn message_delta_cache_fields_merged() {
        // 部分网关把缓存字段并入 message_delta：非 0 合并、归一化口径重算
        let mut state = AnthropicStreamState::new();
        state
            .decode("p", r#"{"type":"message_start","message":{"usage":{"input_tokens":50}}}"#)
            .unwrap()
            .unwrap();
        assert_eq!(state.usage.prompt_tokens, 50);
        state
            .decode(
                "p",
                r#"{"type":"message_delta","delta":{},"usage":{"output_tokens":20,"cache_creation_input_tokens":8,"cache_read_input_tokens":4}}"#,
            )
            .unwrap()
            .unwrap();
        assert_eq!((state.usage.cache_tokens, state.usage.cache_write_tokens), (12, 8));
        assert_eq!(state.usage.prompt_tokens, 62);
        // 0 值不覆盖已知缓存分量
        state
            .decode("p", r#"{"type":"message_delta","delta":{},"usage":{"cache_read_input_tokens":0}}"#)
            .unwrap()
            .unwrap();
        assert_eq!((state.usage.cache_tokens, state.usage.prompt_tokens), (12, 62));
    }

    #[test]
    fn usage_null_payload_not_marked_seen() {
        let mut state = AnthropicStreamState::new();
        // 部分网关发 "usage": null：不得置 usage_seen（否则收尾产出全 0 用量行）
        state
            .decode("p", r#"{"type":"message_start","message":{"usage":null}}"#)
            .unwrap()
            .unwrap();
        assert!(!state.usage_seen);
        assert_eq!(state.usage.prompt_tokens, 0);
    }

    /// 图片投影的核心约束：tool_result 的 content 数组必须 image-first（图片在前、
    /// 文本在后）。Anthropic 兼容网关只解析开头的连续 image 块，文本领先会让图片
    /// 被静默丢弃——这是实机踩过的坑，单测钉住顺序防止回归。
    #[test]
    fn tool_result_with_images_is_image_first() {
        let mut req = sample_request();
        req.supports_vision = true;
        // 把样例里的 tool 消息（索引 2）换成本测试的带图版本
        let mut tool_msg = Message::new("tool", "截图说明文本");
        tool_msg.tool_call_id = "call_1".into();
        tool_msg.attachments = vec![crate::types::Attachment {
            name: "mcp-image-1.png".into(),
            mime: "image/png".into(),
            text: String::new(),
            data: std::sync::Arc::from("aGVsbG8="),
            size: 5,
            size_px: None,
            skipped: String::new(),
        }];
        req.messages[2] = tool_msg;

        let (system, messages) = build_anthropic_messages(&req);
        assert!(!system.is_empty(), "sample_request 带系统块，这里只关心消息侧投影");
        // tool_result 是 user 消息 content 数组里的一个块，它自己的 content 又是数组：
        // 第 0 项 image、第 1 项 text（image-first）
        let tool_result = messages
            .iter()
            .find_map(|m| {
                m["content"].as_array().and_then(|arr| {
                    arr.iter().find(|b| b["type"] == "tool_result" && b["content"].is_array())
                })
            })
            .expect("应存在带图片数组的 tool_result 块");
        let content = tool_result["content"].as_array().expect("content 应是数组");
        assert_eq!(content[0]["type"], "image", "图片块必须在最前：{content:?}");
        assert_eq!(content[0]["source"]["type"], "base64");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[0]["source"]["data"], "aGVsbG8=");
        assert_eq!(content[1]["type"], "text", "文本块在图片之后");
        assert_eq!(content[1]["text"], "截图说明文本");
    }

    /// 不支持视觉的模型：不发图片，只留占位说明（请求不会被 API 拒绝）。
    #[test]
    fn tool_result_images_are_dropped_without_vision() {
        let mut req = sample_request();
        req.supports_vision = false;
        let mut tool_msg = Message::new("tool", "截图说明文本");
        tool_msg.tool_call_id = "call_1".into();
        tool_msg.attachments = vec![crate::types::Attachment {
            name: "mcp-image-1.png".into(),
            mime: "image/png".into(),
            text: String::new(),
            data: std::sync::Arc::from("aGVsbG8="),
            size: 5,
            size_px: None,
            skipped: String::new(),
        }];
        req.messages[2] = tool_msg;

        let (_, messages) = build_anthropic_messages(&req);
        let tool_result = messages
            .iter()
            .find_map(|m| {
                m["content"].as_array().and_then(|arr| {
                    arr.iter().find(|b| b["type"] == "tool_result" && b["content"].is_string())
                })
            })
            .expect("应存在纯文本 tool_result 块");
        // content 是字符串且带占位说明，没有任何 image 块
        let text = tool_result["content"].as_str().expect("无视觉时应回退纯文本");
        assert!(text.contains("不支持图片输入"), "{text}");
        assert!(!text.contains("aGVsbG8="), "base64 不得泄漏进文本");
    }
}
