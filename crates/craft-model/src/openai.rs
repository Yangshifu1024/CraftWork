//! OpenAI / OpenAI-compatible 适配器。
//! 请求体拼接规则固定不增减空白——前缀缓存依赖字节稳定。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;
use serde_json::{json, Value};

use crate::errors::{missing_endpoint_error, stream_error, ModelError};
use crate::sse::{open_sse, line_to_event, LineReader, SseEvent, ToolCallBuilder};
use crate::types::{attachment_prompt_text, Chunk, Client, Message, Request, SystemBlock, ToolSpec, Usage};

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

#[derive(Debug, Clone, PartialEq)]
enum ContentPart {
    Text(String),
    Parts(Vec<Value>),
}

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

#[derive(Clone)]
pub struct OpenAICompatClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    provider_name: String,
}

impl OpenAICompatClient {
    pub fn new(endpoint: &str, api_key: &str, provider_name: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            api_key: api_key.to_string(),
            provider_name: provider_name.to_string(),
        }
    }
}

impl Client for OpenAICompatClient {
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(OpenAICompatClient {
            http: self.http.clone(),
            endpoint: self.endpoint.clone(),
            api_key: self.api_key.clone(),
            provider_name: self.provider_name.clone(),
        })
    }

    fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
        let body = build_openai_body(&req.model, req);
        let endpoint = self.endpoint.clone();
        let api_key = self.api_key.clone();
        let provider = self.provider_name.clone();
        let http = self.http.clone();
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
                            match decode_openai_event(&provider, &payload, &mut usage, &mut usage_seen, &mut tool_calls, &mut text_seen) {
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
                            for call in tool_calls.flush() {
                                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
                            }
                            tool_calls = ToolCallBuilder::default();
                        }
                        Some(SseEvent::Data(payload)) => {
                            match decode_openai_event(&provider, &payload, &mut usage, &mut usage_seen, &mut tool_calls, &mut text_seen) {
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

            for call in tool_calls.flush() {
                yield Ok(Chunk { tool_call: Some(call), ..Default::default() });
            }
            yield Ok(Chunk { usage: Some(if usage_seen { usage } else { Usage::empty() }), ..Default::default() });
            yield Ok(Chunk { done: true, ..Default::default() });
        })
    }
}

/// 解码一个 OpenAI SSE 事件：返回 Some(Ok(Some(chunk))) 产出片段；
/// Some(Ok(None)) 无产出；Some(Err) 流内错误；None 非法载荷忽略。
#[allow(clippy::too_many_arguments)]
fn decode_openai_event(
    provider: &str,
    payload: &str,
    usage: &mut Usage,
    usage_seen: &mut bool,
    tool_calls: &mut ToolCallBuilder,
    text_seen: &mut bool,
) -> Option<Result<Option<Chunk>, ModelError>> {
    let Ok(event) = serde_json::from_str::<Value>(payload) else {
        return None; // 心跳/非标注释直接忽略
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
    if let Some(event_usage) = event.get("usage") {
        *usage_seen = true;
        usage.input_tokens = event_usage.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0);
        usage.output_tokens = event_usage.get("completion_tokens").and_then(Value::as_i64).unwrap_or(0);
        usage.cache_tokens = event_usage
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
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
    fn url拼接_去尾部斜杠() {
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
        }
    }

    #[test]
    fn 请求体构造_消息形状与字节稳定() {
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
    fn 请求体_零maxTokens与空tools时字段省略() {
        let mut req = sample_request();
        req.max_tokens = 0;
        req.tools = vec![];
        let body = build_openai_body("m", &req);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn 事件解码_文本与用量() {
        let mut usage = Usage::empty();
        let mut usage_seen = false;
        let mut tool_calls = ToolCallBuilder::default();
        let mut text_seen = false;
        let chunk = decode_openai_event(
            "p",
            r#"{"choices":[{"delta":{"content":"hi"}}]}"#,
            &mut usage,
            &mut usage_seen,
            &mut tool_calls,
            &mut text_seen,
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
        )
        .unwrap()
        .unwrap();
        assert!(chunk.is_none());
        assert!(usage_seen);
        assert_eq!((usage.input_tokens, usage.output_tokens, usage.cache_tokens), (10, 5, 3));
    }

    #[test]
    fn 事件解码_流内错误() {
        let err = decode_openai_event("p", r#"{"error":{"message":"bad","type":"auth"}}"#, &mut Usage::empty(), &mut false, &mut ToolCallBuilder::default(), &mut false)
            .unwrap()
            .unwrap_err();
        assert!(err.message.contains("bad"));
    }
}
