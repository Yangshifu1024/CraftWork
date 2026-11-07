//! OpenAI / OpenAI-compatible 协议适配器（两种协议共用同一请求体与解码路径）。
//! 请求体拼接规则固定不增减空白——前缀缓存依赖字节稳定。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::errors::{missing_endpoint_error, stream_error, ModelError};
use crate::sse::{open_sse, line_to_event, LineReader, SseEvent, TailBuffer, ToolCallBuilder};
use crate::types::{
    attachment_prompt_text, header_has_session_placeholder, resolve_header_value, Chunk, Client, DoneVia, Message, Request, StreamEnd,
    SystemBlock, ToolSpec, Usage, RAW_TAIL_CAPACITY,
};

/// 端点 → chat/completions 完整地址（容忍端点尾部斜杠）。
fn join_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    format!("{trimmed}/chat/completions")
}

/// 系统分块拼接为一条 system 消息（空行连接，字节稳定）。
fn join_system(blocks: &[SystemBlock]) -> String {
    blocks
        .iter()
        .map(|b| b.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 用户消息内容的两种协议形态：纯文本，或含图片时的多 parts（文本 + image_url）。
#[derive(Debug, Clone, PartialEq)]
enum ContentPart {
    Text(String),
    Parts(Vec<Value>),
}

/// 用户消息 → 协议内容：无图片时附件文本经防注入包装并入纯文本；
/// 有图片时输出多 parts（文本块在前，图片按 base64 data URL 追加），跳过项忽略。
fn user_content(msg: &Message) -> ContentPart {
    let has_images = msg.attachments.iter().any(|a| !a.data.is_empty() && a.skipped.is_empty());
    if !has_images {
        if msg.attachments.is_empty() {
            return ContentPart::Text(msg.content.clone());
        }
        let texts: Vec<String> = msg
            .attachments
            .iter()
            .filter(|a| !a.text.is_empty() && a.skipped.is_empty())
            .map(attachment_prompt_text)
            .collect();
        return ContentPart::Text(if texts.is_empty() {
            msg.content.clone()
        } else {
            format!("{}\n\n{}", msg.content, texts.join("\n\n"))
        });
    }
    let mut parts: Vec<Value> = Vec::new();
    let mut text = msg.content.clone();
    for att in &msg.attachments {
        if !att.skipped.is_empty() {
            continue;
        }
        if !att.text.is_empty() {
            text.push_str(&format!("\n\n{}", attachment_prompt_text(att)));
        }
    }
    if !text.is_empty() {
        parts.push(json!({ "type": "text", "text": text }));
    }
    for att in &msg.attachments {
        if !att.skipped.is_empty() || att.data.is_empty() {
            continue;
        }
        parts.push(json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", att.mime, att.data) } }));
    }
    ContentPart::Parts(parts)
}

/// 拼装 OpenAI 协议请求体：system 合并为一条消息；max_tokens 为 0 / 无工具时省略对应字段
/// （省略优于传零，兼容不认这些字段的网关）；思考档位注入规则见行内注释。
pub fn build_openai_body(model_id: &str, req: &Request) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    if !req.system.is_empty() {
        messages.push(json!({ "role": "system", "content": join_system(&req.system) }));
    }
    for msg in &req.messages {
        match msg.role.as_str() {
            "user" => {
                let content = match user_content(msg) {
                    ContentPart::Text(text) => Value::String(text),
                    ContentPart::Parts(parts) => Value::Array(parts),
                };
                messages.push(json!({ "role": "user", "content": content }));
            }
            "assistant" => {
                let mut entry = json!({ "role": "assistant", "content": msg.content });
                if !msg.tool_calls.is_empty() {
                    entry["tool_calls"] = Value::Array(
                        msg.tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": call.id,
                                    "type": "function",
                                    "function": { "name": call.name, "arguments": serde_json::to_string(&call.args).unwrap_or_else(|_| "{}".into()) },
                                })
                            })
                            .collect(),
                    );
                }
                messages.push(entry);
            }
            "tool" => {
                messages.push(json!({ "role": "tool", "tool_call_id": msg.tool_call_id, "content": msg.content }));
            }
            _ => {}
        }
    }
    let mut body = json!({
        "model": model_id,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if req.max_tokens > 0 {
        body["max_tokens"] = json!(req.max_tokens);
    }
    // 思考档位：effort 型直传枚举（gpt/o 系与认 reasoning_effort 的兼容网关）；
    // budget 型不经此路径（openai 协议不认预算字段）；未选择不发，请求体与既有行为一致。
    if req.reasoning_kind == "effort" && !req.reasoning_level.is_empty() && req.reasoning_level != "off" {
        body["reasoning_effort"] = json!(req.reasoning_level);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t: &ToolSpec| {
                    json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.schema } })
                })
                .collect(),
        );
    }
    body
}

/// OpenAI 协议客户端（openai 与 openai-compatible 提供商共用）。
#[derive(Clone)]
pub struct OpenAICompatClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    provider_name: String,
    /// 提供商级自定义请求头（用户在提供商配置里声明，随每次请求附加）。
    extra_headers: Vec<crate::types::HeaderPair>,
}

impl OpenAICompatClient {
    /// 创建客户端；api_key 为空时不发 Authorization 头（无密钥提供商）。
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

impl Client for OpenAICompatClient {
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(OpenAICompatClient {
            http: self.http.clone(),
            endpoint: self.endpoint.clone(),
            api_key: self.api_key.clone(),
            provider_name: self.provider_name.clone(),
            extra_headers: self.extra_headers.clone(),
        })
    }

    fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
        let body = build_openai_body(&req.model, req);
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
            let mut request = http
                .post(join_url(&endpoint))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream");
            if !api_key.is_empty() {
                request = request.header("Authorization", format!("Bearer {api_key}"));
            }
            // 提供商自定义头最后附加：与认证头同名时可覆盖（保存侧已禁止冲突名，防御性兜底）；
            // {sessionId} 占位符按请求的会话 ID 替换，未携带会话 ID 的头跳过（不发坏路由值）
            for h in &extra_headers {
                if header_has_session_placeholder(&h.value) && req.session_id.is_empty() {
                    continue;
                }
                request = request
                    .header(h.name.as_str(), resolve_header_value(&h.value, &req.session_id).as_str());
            }
            request = request.json(&body);
            let resp = match open_sse(request, &provider).await {
                Ok(resp) => resp,
                Err(err) => {
                    yield Err(err);
                    return;
                }
            };

            let mut text_seen = false;
            let mut usage_seen = false;
            let mut usage = Usage::empty();
            let mut tool_calls = ToolCallBuilder::default();
            let mut finish_reason: Option<String> = None;
            let mut seen_done_line = false;
            let mut tail = TailBuffer::new(RAW_TAIL_CAPACITY);
            let mut bytes_stream = resp.bytes_stream();
            let mut reader = LineReader::new();

            loop {
                // 字节块 → 行（LineReader 核心可同步单测）；取消语义 = 调用方 drop 流
                let chunk = bytes_stream.next().await;
                let bytes = match chunk {
                    Some(Ok(bytes)) => bytes,
                    Some(Err(err)) => {
                        yield Err(crate::errors::transport_error(&provider, &err));
                        return;
                    }
                    None => {
                        // 流结束
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
                            match decode_openai_event(
                                &provider,
                                &payload,
                                &mut usage,
                                &mut usage_seen,
                                &mut tool_calls,
                                &mut text_seen,
                                &mut finish_reason,
                            ) {
                                Some(Ok(Some(chunk))) => yield Ok(chunk),
                                Some(Ok(None)) => {}
                                Some(Err(err)) => {
                                    yield Err(err);
                                    return;
                                }
                                None => {}
                            }
                        }
                        break;
                    }
                };
                for line in reader.push_bytes(&bytes) {
                    match line_to_event(&line) {
                        Some(SseEvent::Done) => {
                            seen_done_line = true;
                            for call in tool_calls.flush() {
                                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
                            }
                            tool_calls = ToolCallBuilder::default();
                        }
                        Some(SseEvent::Data(payload)) => {
                            tail.push(&payload);
                            match decode_openai_event(
                                &provider,
                                &payload,
                                &mut usage,
                                &mut usage_seen,
                                &mut tool_calls,
                                &mut text_seen,
                                &mut finish_reason,
                            ) {
                                Some(Ok(Some(chunk))) => yield Ok(chunk),
                                Some(Ok(None)) => {}
                                Some(Err(err)) => {
                                    yield Err(err);
                                    return;
                                }
                                None => {}
                            }
                        }
                        None => {}
                    }
                }
            }

            // 收尾：残余工具调用 → 用量（未见 usage 事件时不产出：全 0 值会污染「最新一行用量」，
            // 调用方按「网关未给用量」处理）→ done 标记（携带结束元数据）
            for call in tool_calls.flush() {
                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
            }
            if usage_seen {
                yield Ok(Chunk { usage: Some(usage), ..Default::default() });
            }
            yield Ok(Chunk {
                done: true,
                end: Some(make_stream_end(finish_reason, seen_done_line, std::mem::take(&mut reader.stats), tail.as_str().to_string())),
                ..Default::default()
            });
        })
    }
}

/// 组装流结束元数据（纯函数，收尾时调用；可单测）。
pub fn make_stream_end(finish_reason: Option<String>, seen_done_line: bool, sse: crate::types::SseStats, raw_tail: String) -> StreamEnd {
    StreamEnd {
        finish_reason,
        done_via: if seen_done_line { DoneVia::DoneLine } else { DoneVia::Eof },
        sse,
        raw_tail,
    }
}

/// 解码一个 OpenAI SSE 事件：返回 Some(Ok(Some(chunk))) 产出片段；
/// Some(Ok(None)) 无产出；Some(Err) 流内错误；None 非法载荷忽略。
/// finish_reason 记入外部状态（带 finish_reason 的 delta 通常无正文），收尾随 done 块带出。
#[allow(clippy::too_many_arguments)]
fn decode_openai_event(
    provider: &str,
    payload: &str,
    usage: &mut Usage,
    usage_seen: &mut bool,
    tool_calls: &mut ToolCallBuilder,
    text_seen: &mut bool,
    finish_reason: &mut Option<String>,
) -> Option<Result<Option<Chunk>, ModelError>> {
    let Ok(event) = serde_json::from_str::<Value>(payload) else {
        return None; // 非 JSON 载荷（心跳 / 注释行）直接忽略
    };
    if let Some(err) = event.get("error") {
        let message = err.get("message").and_then(Value::as_str).unwrap_or_default();
        let kind = err
            .get("type")
            .or_else(|| err.get("code"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Some(Err(stream_error(provider, message, kind)));
    }
    if let Some(reason) = event
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        *finish_reason = Some(reason.to_string());
    }
    let delta = event
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"));

    let mut chunk = Chunk::default();
    if let Some(delta) = delta {
        let reasoning = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        if let Some(reasoning) = reasoning {
            *text_seen = true;
            chunk.reasoning = Some(reasoning.to_string());
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            *text_seen = true;
            chunk.text = Some(text.to_string());
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_i64).unwrap_or(0);
                let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
                let function = call.get("function");
                let name = function.and_then(|f| f.get("name")).and_then(Value::as_str).unwrap_or_default();
                let arguments = function.and_then(|f| f.get("arguments")).and_then(Value::as_str).unwrap_or_default();
                tool_calls.add(index, id, name, arguments);
            }
        }
    }
    // is_object 防御：部分网关在普通 chunk 里发 "usage": null，命中后字段全被清零
    if let Some(event_usage) = event.get("usage").filter(|u| u.is_object()) {
        *usage_seen = true;
        usage.input_tokens = event_usage.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0);
        // OpenAI 系 prompt_tokens 已含缓存命中部分，即完整提示词（归一化口径与 input 相同）
        usage.prompt_tokens = usage.input_tokens;
        usage.output_tokens = event_usage.get("completion_tokens").and_then(Value::as_i64).unwrap_or(0);
        // DeepSeek 风格 prompt_cache_hit_tokens 与 OpenAI cached_tokens 语义相同，取较大值兜底
        let cached = event_usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let deepseek_hit = event_usage.get("prompt_cache_hit_tokens").and_then(Value::as_i64).unwrap_or(0);
        usage.cache_tokens = cached.max(deepseek_hit);
        usage.reasoning_tokens = event_usage
            .get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
    }
    if chunk.text.is_some() || chunk.reasoning.is_some() {
        Some(Ok(Some(chunk)))
    } else {
        Some(Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_join_strips_trailing_slash() {
        assert_eq!(join_url("https://api.x.com/v1"), "https://api.x.com/v1/chat/completions");
        assert_eq!(join_url("https://api.x.com/v1/"), "https://api.x.com/v1/chat/completions");
        assert_eq!(join_url("https://api.x.com/"), "https://api.x.com/chat/completions");
    }

    fn sample_request() -> Request {
        Request {
            model: "gpt-4o".into(),
            mode: "code".into(),
            system: vec![
                SystemBlock { content: "sys-a".into(), cache_control: false },
                SystemBlock { content: "sys-b".into(), cache_control: false },
            ],
            messages: vec![
                Message::new("user", "你好"),
                {
                    let mut m = Message::new("assistant", "好的");
                    m.tool_calls = vec![crate::types::ToolCall { id: "call_1".into(), name: "read".into(), args: serde_json::json!({"p": 1}) }];
                    m
                },
                {
                    let mut m = Message::new("tool", "文件内容");
                    m.tool_call_id = "call_1".into();
                    m
                },
            ],
            tools: vec![ToolSpec { name: "read".into(), description: "读文件".into(), schema: serde_json::json!({"type": "object"}) }],
            max_tokens: 1024,
            reasoning_kind: String::new(),
            reasoning_level: String::new(),
            session_id: "th_sample".into(),
        }
    }

    #[test]
    fn request_body_construction_message_shape_and_byte_stability() {
        let body = build_openai_body("gpt-4o", &sample_request());
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1024);
        let messages = body["messages"].as_array().unwrap();
        // system + user + assistant + tool
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "sys-a\n\nsys-b");
        assert_eq!(messages[1]["content"], "你好");
        assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[2]["tool_calls"][0]["function"]["arguments"], r#"{"p":1}"#);
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        // tools
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "read");
        // 字节稳定：两次构造完全一致
        assert_eq!(serde_json::to_string(&body).unwrap(), serde_json::to_string(&build_openai_body("gpt-4o", &sample_request())).unwrap());
    }

    #[test]
    fn reasoning_effort_injection_only_for_effort_models() {
        // effort 型：非 off 档位注入 reasoning_effort
        let mut req = sample_request();
        req.reasoning_kind = "effort".into();
        req.reasoning_level = "high".into();
        let body = build_openai_body("gpt-5", &req);
        assert_eq!(body["reasoning_effort"], "high");

        // off / 未选择 / 非 effort 类型：均不注入，请求体与既有一致
        for (kind, level) in [("effort", "off"), ("effort", ""), ("budget", "high"), ("", "high")] {
            let mut req = sample_request();
            req.reasoning_kind = kind.into();
            req.reasoning_level = level.into();
            let body = build_openai_body("gpt-5", &req);
            assert!(body.get("reasoning_effort").is_none(), "{kind}/{level} 不应注入");
        }
    }

    #[test]
    fn request_body_omits_fields_when_max_tokens_zero_and_tools_empty() {
        let mut req = sample_request();
        req.max_tokens = 0;
        req.tools = vec![];
        let body = build_openai_body("m", &req);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn event_decoding_text_and_usage() {
        let mut usage = Usage::empty();
        let mut usage_seen = false;
        let mut tool_calls = ToolCallBuilder::default();
        let mut text_seen = false;
        let mut finish_reason = None;
        let chunk = decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{"content":"hi"}}]}"#,
            &mut usage,
            &mut usage_seen,
            &mut tool_calls,
            &mut text_seen,
            &mut finish_reason,
        )
        .unwrap()
        .unwrap()
        .unwrap();
        assert_eq!(chunk.text.as_deref(), Some("hi"));
        let chunk = decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":3}}}"#,
            &mut usage,
            &mut usage_seen,
            &mut tool_calls,
            &mut text_seen,
            &mut finish_reason,
        )
        .unwrap()
        .unwrap();
        assert!(chunk.is_none());
        assert!(usage_seen);
        assert_eq!((usage.input_tokens, usage.output_tokens, usage.cache_tokens), (10, 5, 3));
        // OpenAI 系归一化口径与 input 相同（prompt_tokens 已含缓存命中）
        assert_eq!(usage.prompt_tokens, 10);
    }

    #[test]
    fn usage_null_payload_not_marked_seen() {
        // 部分网关在普通 chunk 里发 "usage": null：不得置 usage_seen（否则收尾产出全 0 用量行）
        let mut usage = Usage::empty();
        let mut usage_seen = false;
        let chunk = decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{}}],"usage":null}"#,
            &mut usage,
            &mut usage_seen,
            &mut ToolCallBuilder::default(),
            &mut false,
            &mut None,
        )
        .unwrap()
        .unwrap();
        assert!(chunk.is_none());
        assert!(!usage_seen);
        assert_eq!(usage.prompt_tokens, 0);
    }

    #[test]
    fn deepseek_cache_hit_tokens_recognized() {
        // DeepSeek 风格 prompt_cache_hit_tokens 与 OpenAI cached_tokens 语义相同，取较大值
        let mut usage = Usage::empty();
        let mut usage_seen = false;
        decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":100,"completion_tokens":8,"prompt_cache_hit_tokens":64}}"#,
            &mut usage,
            &mut usage_seen,
            &mut ToolCallBuilder::default(),
            &mut false,
            &mut None,
        )
        .unwrap()
        .unwrap();
        assert!(usage_seen);
        assert_eq!((usage.input_tokens, usage.cache_tokens, usage.prompt_tokens), (100, 64, 100));
    }

    #[test]
    fn finish_reason_extracted_from_empty_delta_and_kept_in_state() {
        // 带 finish_reason 的 delta 通常无正文（此前整块被丢弃），解析后记入外部状态
        let mut finish_reason = None;
        let chunk = decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
            &mut Usage::empty(),
            &mut false,
            &mut ToolCallBuilder::default(),
            &mut false,
            &mut finish_reason,
        )
        .unwrap()
        .unwrap();
        assert!(chunk.is_none());
        assert_eq!(finish_reason.as_deref(), Some("length"));
        // 后续无 finish_reason 的事件不覆盖
        decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{"content":"x"}}]}"#,
            &mut Usage::empty(),
            &mut false,
            &mut ToolCallBuilder::default(),
            &mut false,
            &mut finish_reason,
        )
        .unwrap()
        .unwrap();
        assert_eq!(finish_reason.as_deref(), Some("length"));
    }

    #[test]
    fn stream_end_done_via_distinguishes_done_line_and_eof() {
        let end = make_stream_end(Some("length".into()), true, Default::default(), String::new());
        assert_eq!(end.done_via, DoneVia::DoneLine);
        assert_eq!(end.finish_reason.as_deref(), Some("length"));
        let end = make_stream_end(None, false, Default::default(), String::new());
        assert_eq!(end.done_via, DoneVia::Eof);
        assert!(end.finish_reason.is_none());
    }

    #[test]
    fn event_decoding_in_stream_error() {
        let err = decode_openai_event(
            "p",
            r#"{"error":{"message":"bad","type":"auth"}}"#,
            &mut Usage::empty(),
            &mut false,
            &mut ToolCallBuilder::default(),
            &mut false,
            &mut None,
        )
        .unwrap()
        .unwrap_err();
        assert!(err.message.contains("bad"));
    }
}
