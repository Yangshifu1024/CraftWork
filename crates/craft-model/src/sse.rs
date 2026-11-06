//! SSE 公共骨架：POST JSON → 状态码校验 → data: 行解析 → 适配器解码。
//! 行拆分核心为同步纯函数（LineReader，可单测）；异步层在适配器内消费字节流。

use serde_json::Value;

use crate::errors::ModelError;

/// 单行长度上限：超过即截断保留尾部，防异常超长行撑爆内存。
const MAX_SSE_LINE_BYTES: usize = 4 * 1024 * 1024;

/// 按 SSE 帧拆行（容忍超长行截断；跨 chunk 缓冲拼接），同时记录传输质量统计。
///
/// 解码策略（P0 修复）：**字节域缓冲，按行边界解码**。多字节字符可能被网络
/// 分块从中间切开（如「划」= e5 88 92 被 TCP/TLS 分块劈成两半），按 chunk 独立
/// lossy 解码会把一个汉字损坏成 2~3 个 U+FFFD。字节域缓冲 + 行边界解码天然
/// 避免撕裂：\n 是单字节、永不参与多字节序列，行边界切割一定落在字符边界上。
/// `from_utf8_lossy` 只兜底服务器真的发出非法字节的场景（换行缺失的超长行受
/// MAX_SSE_LINE_BYTES 约束，不会无限缓冲）。
#[derive(Default)]
pub struct LineReader {
    /// 字节域缓冲：原始字节按序累积，在行边界（\n）处才解码成行
    buffer: Vec<u8>,
    /// 传输质量统计：排查内容损伤（UTF-8 跨块撕裂 / 超长行截断 / 尾部半行）
    pub stats: crate::types::SseStats,
}

impl LineReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入字节，返回本次切出的完整行（已去除行尾 \r）。
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(bytes);
        // 行内容超长保护：按字节截断保留尾部（此时 buffer 可能切开多字节字符，
        // 故意按 lossy 解码并在统计里留痕——超长行本身就是异常，损坏可接受）
        if self.buffer.len() > MAX_SSE_LINE_BYTES {
            let keep = 1024;
            let mut start = self.buffer.len().saturating_sub(keep);
            while start < self.buffer.len() && (self.buffer[start] & 0xC0) == 0x80 {
                start += 1; // 跳过 UTF-8 续字节，对齐字符边界
            }
            self.buffer.drain(..start);
            self.stats.oversized_lines += 1;
        }
        let mut lines = Vec::new();
        while let Some(idx) = self.buffer.iter().position(|&b| b == b'\n') {
            let mut line_bytes = self.buffer[..idx].to_vec();
            self.buffer.drain(..idx + 1);
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            // 行边界解码：\n 永不参与多字节序列，此处字节一定是完整字符；
            // lossy 只兜底服务器发送非法 UTF-8 的场景，并计入统计
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(text) => text.to_string(),
                Err(_) => {
                    self.stats.invalid_utf8_chunks += 1;
                    String::from_utf8_lossy(&line_bytes).into_owned()
                }
            };
            lines.push(line);
        }
        lines
    }

    /// 流结束：残留缓冲作为最后一行（无换行结尾）；残留字节数计入统计。
    pub fn finish(&mut self) -> Option<String> {
        self.stats.trailing_partial_bytes = self.buffer.len();
        if self.buffer.is_empty() {
            return None;
        }
        let mut line_bytes = std::mem::take(&mut self.buffer);
        if line_bytes.last() == Some(&b'\r') {
            line_bytes.pop();
        }
        // EOF 残留行同样行边界解码（此时是完整事件行，不应有撕裂）
        let line = match std::str::from_utf8(&line_bytes) {
            Ok(text) => text.to_string(),
            Err(_) => {
                self.stats.invalid_utf8_chunks += 1;
                String::from_utf8_lossy(&line_bytes).into_owned()
            }
        };
        Some(line)
    }
}

/// 原始事件行尾部缓冲：限长保留最近内容（字符边界安全），用于异常时留存截断瞬间的原始行为。
#[derive(Default)]
pub struct TailBuffer {
    buf: String,
    cap: usize,
}

impl TailBuffer {
    pub fn new(cap: usize) -> Self {
        Self { buf: String::new(), cap }
    }

    /// 追加一段原始文本（每段以换行分隔）；超容量时丢弃头部、对齐字符边界。
    pub fn push(&mut self, text: &str) {
        if !text.is_empty() {
            self.buf.push_str(text);
            self.buf.push('\n');
        }
        if self.buf.len() > self.cap {
            let mut start = self.buf.len() - self.cap;
            while start < self.buf.len() && !self.buf.is_char_boundary(start) {
                start += 1;
            }
            self.buf = self.buf[start..].to_string();
        }
    }

    /// 取当前尾部内容。
    pub fn as_str(&self) -> &str {
        &self.buf
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
    /// 累积一个增量：id / name 取最后一个非空值，参数按到达顺序串接。
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

/// 建立流（reqwest）：非 2xx 读取错误体（截断 8192 字符）归一化为 ModelError；
/// 成功返回响应，字节流由调用方消费。
pub async fn open_sse(builder: reqwest::RequestBuilder, provider: &str) -> Result<reqwest::Response, ModelError> {
    let resp = builder
        // reqwest 的整请求超时语义：从建连覆盖到响应体读完（超长流也会被掐断）
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
    fn multibyte_char_split_across_chunks_survives() {
        // P0 回归：「计划」两个字的多字节序列被网络分块从中间劈开。
        // 旧实现按 chunk 独立 lossy 解码，会把汉字损坏成 U+FFFD；字节域缓冲后必须完整还原。
        let mut reader = LineReader::new();
        // 「计划」= e8 ae a1 e5 88 92；在「划」的首字节 e5 之后切开（e5 在上一块、88 92 在下一块）
        let mut first_chunk = b"data: ".to_vec();
        first_chunk.extend_from_slice(&"计划".as_bytes()[..4]); // "计" + e5
        assert!(reader.push_bytes(&first_chunk).is_empty(), "半行不应被切出");
        let mut second_chunk = "计划".as_bytes()[4..].to_vec(); // 88 92
        second_chunk.extend_from_slice(b"\n\n");
        let lines = reader.push_bytes(&second_chunk);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "data: 计划", "跨块劈开的汉字必须完整还原");
        assert_eq!(reader.stats.invalid_utf8_chunks, 0, "正常撕裂不应计入非法统计");

        // 三字劈两刀：每个字节边界都可能是分块点
        let mut reader = LineReader::new();
        let text = "单行修复";
        for b in text.as_bytes() {
            let lines = reader.push_bytes(std::slice::from_ref(b));
            let _ = lines; // 中间块的输出允许为空或半行，最终一致性只看 EOF
        }
        assert_eq!(reader.finish().as_deref(), Some(text), "逐字节喂入必须完整还原");
        assert_eq!(reader.stats.invalid_utf8_chunks, 0);
    }

    #[test]
    fn genuinely_invalid_utf8_still_counted_and_lossy() {
        // 服务器真的发送非法字节：统计留痕 + lossy 兜底（不 panic、不丢行）
        let mut reader = LineReader::new();
        let lines = reader.push_bytes(&[b'd', b'a', b't', b'a', b':', b' ', 0xFF, b'\n']);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{FFFD}'));
        assert_eq!(reader.stats.invalid_utf8_chunks, 1);
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
