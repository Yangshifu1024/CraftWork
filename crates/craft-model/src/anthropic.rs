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
                push(
                    &mut messages,
                    "user",
                    json!({ "type": "tool_result", "tool_use_id": msg.tool_call_id, "content": msg.content }),
                );
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
    // 思考档位：budget 型映射 thinking 预算（跟随 ZCode 的固定映射值，档位归一 off/low/high）。
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
    pub fn new(endpoint: &str, api_key: &str, provider_name: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
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
            tool_calls: ToolCallBuilder::default(),
            pending: std::collections::HashMap::new(),
            stop_reason: None,
            seen_stop: false,
        }
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
                if let Some(u) = event.pointer("/message/usage") {
                    self.usage_seen = true;
                    self.usage.input_tokens = u.get("input_tokens").and_then(Value::as_i64).unwrap_or(0);
                    self.usage.cache_tokens = u.get("cache_creation_input_tokens").and_then(Value::as_i64).unwrap_or(0)
                        + u.get("cache_read_input_tokens").and_then(Value::as_i64).unwrap_or(0);
                    self.usage.cache_write_tokens = u.get("cache_creation_input_tokens").and_then(Value::as_i64).unwrap_or(0);
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
                if let Some(u) = event.get("usage") {
                    self.usage_seen = true;
                    if let Some(output) = u.get("output_tokens").and_then(Value::as_i64) {
                        self.usage.output_tokens = output;
                    }
                    // message_delta 通常只带 output_tokens；input_tokens 为 0（缺失）时不覆盖
                    if let Some(input) = u.get("input_tokens").and_then(Value::as_i64).filter(|v| *v != 0) {
                        self.usage.input_tokens = input;
                    }
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

            // 收尾：残余工具调用 → 用量（未见 usage 事件时给空值）→ done 标记（携带结束元数据）
            for call in state.tool_calls.flush() {
                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
            }
            yield Ok(Chunk { usage: Some(if state.usage_seen { state.usage } else { Usage::empty() }), ..Default::default() });
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
}
