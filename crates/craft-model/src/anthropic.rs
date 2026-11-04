//! Anthropic 适配器。
//! 关键点：三个 cache_control 断点（系统稳定块 / 系统动态块 / 历史最后一块）、
//! 连续同角色合并、tool 结果以 user 身份回传。
//! 对应 src/main/model/anthropic.ts。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::errors::{missing_endpoint_error, stream_error, ModelError};
use crate::sse::{line_to_event, open_sse, LineReader, SseEvent, ToolCallBuilder};
use crate::types::{attachment_prompt_text, Chunk, Client, Request, Usage};

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

pub fn build_anthropic_body(model_id: &str, req: &Request) -> Value {
    let (system, messages) = build_anthropic_messages(req);
    let mut body = json!({
        "model": model_id,
        "max_tokens": if req.max_tokens > 0 { req.max_tokens } else { 4096 },
        "messages": messages,
        "stream": true,
    });
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

#[derive(Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    provider_name: String,
}

impl AnthropicClient {
    pub fn new(endpoint: &str, api_key: &str, provider_name: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            api_key: api_key.to_string(),
            provider_name: provider_name.to_string(),
        }
    }
}

/// SSE 事件处理上下文（跨事件状态）。
struct AnthropicStreamState {
    usage: Usage,
    usage_seen: bool,
    tool_calls: ToolCallBuilder,
    /// index → 已见 content_block_start 的 tool_use（参数稍后增量给出）
    pending: std::collections::HashMap<i64, (String, String)>,
}

impl AnthropicStreamState {
    fn new() -> Self {
        Self {
            usage: Usage::empty(),
            usage_seen: false,
            tool_calls: ToolCallBuilder::default(),
            pending: std::collections::HashMap::new(),
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
                if let Some(u) = event.get("usage") {
                    self.usage_seen = true;
                    if let Some(output) = u.get("output_tokens").and_then(Value::as_i64) {
                        self.usage.output_tokens = output;
                    }
                    if let Some(input) = u.get("input_tokens").and_then(Value::as_i64).filter(|v| *v != 0) {
                        self.usage.input_tokens = input;
                    }
                }
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
        })
    }

    fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
        let body = build_anthropic_body(&req.model, req);
        let endpoint = self.endpoint.clone();
        let api_key = self.api_key.clone();
        let provider = self.provider_name.clone();
        let http = self.http.clone();
        Box::pin(async_stream::stream! {
            if endpoint.is_empty() {
                yield Err(missing_endpoint_error(&provider));
                return;
            }
            let url = join_api_path(&endpoint, "/v1/messages");
            let request = http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&body);
            let resp = match open_sse(request, &provider).await {
                Ok(resp) => resp,
                Err(err) => {
                    yield Err(err);
                    return;
                }
            };

            let mut state = AnthropicStreamState::new();
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
                        Some(SseEvent::Data(payload)) => match state.decode(&provider, &payload) {
                            Some(Ok(Some(chunk))) => yield Ok(chunk),
                            Some(Err(err)) => {
                                yield Err(err);
                                return;
                            }
                            _ => {}
                        },
                        None => {}
                    }
                }
            }

            for call in state.tool_calls.flush() {
                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
            }
            yield Ok(Chunk { usage: Some(if state.usage_seen { state.usage } else { Usage::empty() }), ..Default::default() });
            yield Ok(Chunk { done: true, ..Default::default() });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, SystemBlock, ToolCall, ToolSpec};

    #[test]
    fn 端点拼接_v1去重() {
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
        }
    }

    #[test]
    fn 消息转换_连续同角色合并与缓存断点() {
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
    fn 请求体构造() {
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
    fn 事件解码_完整流() {
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
            .decode("p", r#"{"type":"message_delta","usage":{"output_tokens":42}}"#)
            .unwrap()
            .unwrap();
        assert_eq!(state.usage.output_tokens, 42);
        // 流内错误
        assert!(state.decode("p", r#"{"type":"error","error":{"message":"overloaded","type":"x"}}"#).unwrap().is_err());
        let calls = state.tool_calls.flush();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "t1");
        assert_eq!(calls[0].args, serde_json::json!({"a": 1}));
    }
}
