//! SSE 公共骨架：POST JSON → 状态码校验 → data: 行解析 → 适配器解码。
//! 行拆分核心为同步纯函数（LineReader，可单测）；异步层在适配器内消费字节流。

use serde_json::Value;

use crate::errors::ModelError;

const MAX_SSE_LINE_BYTES: usize = 4 * 1024 * 1024;

/// 按 SSE 帧拆行（容忍超长行截断；跨 chunk 缓冲拼接）。
#[derive(Default)]
pub struct LineReader {
    buffer: String,
}

impl LineReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入字节，返回本次切出的完整行（已去除行尾 \r）。
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<String> {
        // 直接按字节追加：SSE 行内容以 UTF-8 为主，超长行截断按字符边界处理成本高，
        // 采用与 TS 版一致的「按字符串长度截断」语义（lossy 转换交给行级处理）。
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        if self.buffer.len() > MAX_SSE_LINE_BYTES {
            // 保留尾部 1024 字节（TS 版语义）
            let keep = 1024;
            let start = self.buffer.len().saturating_sub(keep);
            // 从字符边界对齐
            let mut start = start;
            while start < self.buffer.len() && !self.buffer.is_char_boundary(start) {
                start += 1;
            }
            self.buffer = self.buffer[start..].to_string();
        }
        let mut lines = Vec::new();
        while let Some(idx) = self.buffer.find('\n') {
            let mut line = self.buffer[..idx].to_string();
            if line.ends_with('\r') {
                line.pop();
            }
            self.buffer.drain(..idx + 1);
            lines.push(line);
        }
        lines
    }

    /// 流结束：残留缓冲作为最后一行（无换行结尾）。
    pub fn finish(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        let mut line = std::mem::take(&mut self.buffer);
        if line.ends_with('\r') {
            line.pop();
        }
        Some(line)
    }
}

/// SSE 事件载荷：普通 data 行，或 [DONE]（None）。
pub enum SseEvent {
    Data(String),
    Done,
}

/// 从行流提取 data: 载荷（忽略注释/心跳与空行）。
pub fn line_to_event(line: &str) -> Option<SseEvent> {
    let rest = line.strip_prefix("data:")?;
    let payload = rest.trim();
    if payload == "[DONE]" {
        return Some(SseEvent::Done);
    }
    if payload.is_empty() {
        return None;
    }
    Some(SseEvent::Data(payload.to_string()))
}

/// 解析工具调用参数 JSON；失败保留原文到 _raw（避免丢弃模型输出）。
pub fn parse_arguments(raw: &str) -> Value {
    if raw.is_empty() {
        return serde_json::json!({});
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(parsed) if parsed.is_object() => parsed,
        Ok(parsed) => serde_json::json!({ "_raw": parsed }),
        Err(_) => serde_json::json!({ "_raw": raw }),
    }
}

/// 工具调用增量聚合：按 index 累积 {id, name, args 串接}，结束时按 index 升序交付。
#[derive(Default)]
pub struct ToolCallBuilder {
    items: std::collections::BTreeMap<i64, ToolCallItem>,
}

#[derive(Default)]
struct ToolCallItem {
    id: String,
    name: String,
    args: String,
}

impl ToolCallBuilder {
    pub fn add(&mut self, index: i64, id: &str, name: &str, args_delta: &str) {
        let item = self.items.entry(index).or_default();
        if !id.is_empty() {
            item.id = id.to_string();
        }
        if !name.is_empty() {
            item.name = name.to_string();
        }
        if !args_delta.is_empty() {
            item.args.push_str(args_delta);
        }
    }

    /// 交付全部调用；缺 ID 生成占位（保证顺序稳定）。
    pub fn flush(self) -> Vec<crate::types::ToolCall> {
        self.items
            .into_iter()
            .map(|(idx, item)| {
                let id = if item.id.is_empty() {
                    format!("call_{idx:04x}")
                } else {
                    item.id
                };
                crate::types::ToolCall {
                    id,
                    name: item.name,
                    args: parse_arguments(&item.args),
                }
            })
            .collect()
    }
}

/// 建立流（reqwest）：非 2xx 抛 ModelError；成功返回响应（字节流由调用方消费）。
pub async fn open_sse(builder: reqwest::RequestBuilder, provider: &str) -> Result<reqwest::Response, ModelError> {
    let resp = builder
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| crate::errors::transport_error(provider, &e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let truncated: String = body.chars().take(8192).collect();
        return Err(crate::errors::status_error(provider, status.as_u16(), &truncated));
    }
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_split_cross_chunk_buffering_and_cr_removal() {
        let mut reader = LineReader::new();
        assert_eq!(reader.push_bytes(b"data: hel"), Vec::<String>::new());
        let lines = reader.push_bytes(b"lo\r\ndata: world\n\n");
        assert_eq!(lines, vec!["data: hello", "data: world", ""]);
        assert_eq!(reader.finish(), None);
    }

    #[test]
    fn line_split_no_trailing_newline_remainder_as_last_line() {
        let mut reader = LineReader::new();
        assert_eq!(reader.push_bytes(b"abc"), Vec::<String>::new());
        assert_eq!(reader.finish(), Some("abc".into()));
    }

    #[test]
    fn line_split_oversized_line_truncation_keeps_tail() {
        let mut reader = LineReader::new();
        let big = vec![b'x'; MAX_SSE_LINE_BYTES + 4096];
        let lines = reader.push_bytes(&big);
        assert!(lines.is_empty());
        assert!(reader.buffer.len() <= 1024 + 4096);
    }

    #[test]
    fn data_line_extraction_done_and_empty_payload() {
        assert!(matches!(line_to_event("data: [DONE]"), Some(SseEvent::Done)));
        assert!(line_to_event("data: {}").is_some());
        assert!(line_to_event("data:").is_none()); // 空载荷忽略
        assert!(line_to_event(": comment").is_none());
        assert!(line_to_event("event: x").is_none());
    }

    #[test]
    fn tool_call_arguments_parse_failure_preserves_raw() {
        assert_eq!(parse_arguments(""), serde_json::json!({}));
        assert_eq!(parse_arguments(r#"{"a":1}"#), serde_json::json!({"a":1}));
        assert_eq!(parse_arguments("[1,2]"), serde_json::json!({"_raw": [1, 2]}));
        assert_eq!(parse_arguments("不是json"), serde_json::json!({"_raw": "不是json"}));
    }

    #[test]
    fn tool_call_incremental_aggregation_delivered_in_ascending_index_order() {
        let mut builder = ToolCallBuilder::default();
        builder.add(1, "", "read", "{\"p");
        builder.add(0, "call_a", "write", "{\"q\":");
        builder.add(0, "", "", "1}");
        builder.add(1, "", "", "ath\": \"x\"}");
        let calls = builder.flush();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].name, "write");
        assert_eq!(calls[0].args, serde_json::json!({"q": 1}));
        assert_eq!(calls[1].id, "call_0001");
        assert_eq!(calls[1].name, "read");
        assert_eq!(calls[1].args, serde_json::json!({"path": "x"}));
    }
}
