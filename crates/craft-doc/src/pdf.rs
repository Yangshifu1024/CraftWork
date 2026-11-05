//! 轻量 PDF 文本层扫描：不渲染、不执行脚本、无 OCR。
//! /Encrypt → 报错；FlateDecode 流 zlib 解压；内容流解析字面量/十六进制字符串与定位操作符。

use flate2::read::ZlibDecoder;
use std::io::Read;

use crate::{DocResult, MAX_TEXT_BYTES};

pub fn read(raw: &[u8]) -> Result<DocResult, String> {
    let head_len = raw.len().min(4096);
    let head = String::from_utf8_lossy(&raw[..head_len]).to_string();
    if !head.starts_with("%PDF") {
        return Ok(DocResult { kind: "pdf".into(), text: String::new(), note: "不是有效的 PDF 文件。".into(), truncated: false });
    }
    if find(raw, b"/Encrypt").is_some() {
        return Err("PDF 已加密，无法读取".into());
    }

    let latin = raw.iter().map(|b| *b as char).collect::<String>();
    let latin_bytes = latin.clone().into_bytes();
    let mut chunks: Vec<String> = Vec::new();
    let mut i = 0usize;
    while let Some(stream_start) = find_from(&latin_bytes, i, b"stream") {
        // 向前找最近的 << 字典判 FlateDecode
        let dict_start = rfind_before(&latin_bytes, stream_start, b"<<");
        let dict: String = match dict_start {
            Some(start) => latin[start..stream_start].to_string(),
            None => String::new(),
        };
        let is_flate = dict.contains("/FlateDecode");
        let mut data_start = stream_start + "stream".len();
        if latin_bytes.get(data_start) == Some(&b'\r') {
            data_start += 1;
        }
        if latin_bytes.get(data_start) == Some(&b'\n') {
            data_start += 1;
        }
        let Some(stream_end) = find_from(&latin_bytes, data_start, b"endstream") else { break };
        if is_flate {
            let data = &raw[data_start.min(raw.len())..stream_end.min(raw.len())];
            match inflate(data) {
                Ok(inflated) => chunks.push(inflated.iter().map(|b| *b as char).collect()),
                // 解压失败保留原始数据（可能是未压缩内容流）
                Err(_) => chunks.push(raw[data_start.min(raw.len())..stream_end.min(raw.len())].iter().map(|b| *b as char).collect()),
            }
        } else if !dict.contains("/Image") && !dict.contains("/XObject") {
            chunks.push(raw[data_start.min(raw.len())..stream_end.min(raw.len())].iter().map(|b| *b as char).collect());
        }
        i = stream_end + "endstream".len();
    }

    let text = extract_pdf_text(&chunks.join("\n"));
    if text.trim().is_empty() {
        return Ok(DocResult {
            kind: "pdf".into(),
            text: String::new(),
            note: "未检测到文本层，可能是扫描件；当前不支持 OCR。".into(),
            truncated: false,
        });
    }
    let mut out = text;
    let mut truncated = false;
    if out.len() > MAX_TEXT_BYTES {
        let mut byte_end = MAX_TEXT_BYTES;
        while byte_end > 0 && !out.is_char_boundary(byte_end) {
            byte_end -= 1;
        }
        out = format!("{}\n…（文本已截断）", &out[..byte_end]);
        truncated = true;
    }
    Ok(DocResult { kind: "pdf".into(), text: out, note: String::new(), truncated })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    find_from(haystack, 0, needle)
}

fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn rfind_before(haystack: &[u8], before: usize, needle: &[u8]) -> Option<usize> {
    let end = before.min(haystack.len());
    if end < needle.len() {
        return None;
    }
    haystack[..end]
        .windows(needle.len())
        .rposition(|w| w == needle)
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).map_err(|e| format!("zlib 解压失败：{e}"))?;
    Ok(out)
}

/// 解析 PDF 内容流中的文本操作。
fn extract_pdf_text(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0usize;
    let mut pending_space = false;

    while i < n {
        let ch = chars[i];
        if ch == '(' {
            // 字面量字符串（支持嵌套括号与转义）
            let mut depth = 1i32;
            let mut j = i + 1;
            let mut literal: Vec<u8> = Vec::new();
            while j < n && depth > 0 {
                let c = chars[j];
                if c == '\\' {
                    let next = chars.get(j + 1).copied();
                    j += 2;
                    match next {
                        Some('n') => literal.push(b'\n'),
                        Some('r') => literal.push(b'\r'),
                        Some('t') => literal.push(b'\t'),
                        Some('b') => literal.push(8),
                        Some('f') => literal.push(12),
                        Some('(') => literal.push(b'('),
                        Some(')') => literal.push(b')'),
                        Some('\\') => literal.push(b'\\'),
                        Some(d) if ('0'..='7').contains(&d) => {
                            // 八进制 ≤3 位
                            let mut oct = String::new();
                            let mut d = d;
                            while oct.len() < 3 {
                                oct.push(d);
                                match chars.get(j) {
                                    Some(c2) if ('0'..='7').contains(c2) => {
                                        d = *c2;
                                        j += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if let Ok(v) = u8::from_str_radix(&oct, 8) {
                                literal.push(v);
                            }
                        }
                        // 行续接：空
                        Some('\n') => {}
                        Some(other) => literal.push(other as u8),
                        None => {}
                    }
                    continue;
                }
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                literal.push(c as u8);
                j += 1;
            }
            append_text(&mut out, &mut pending_space, &decode_pdf_string(&literal));
            i = j + 1;
            continue;
        }
        if ch == '<' && chars.get(i + 1) != Some(&'<') {
            let mut close = i + 1;
            while close < n && chars[close] != '>' {
                close += 1;
            }
            if close >= n {
                break;
            }
            let hex: String = chars[i + 1..close].iter().filter(|c| c.is_ascii_hexdigit()).collect();
            let hex = if hex.len().is_multiple_of(2) { hex } else { format!("{hex}0") };
            let buf: Vec<u8> = (0..hex.len() / 2)
                .filter_map(|k| u8::from_str_radix(&hex[k * 2..k * 2 + 2], 16).ok())
                .collect();
            append_text(&mut out, &mut pending_space, &decode_pdf_string(&buf));
            i = close + 1;
            continue;
        }
        let next_is_digit = chars.get(i + 1).map(|c| c.is_ascii_digit()).unwrap_or(false);
        if ch == '-' && next_is_digit && !out.is_empty() {
            // 数组里的负数间距（TJ）：≤-100 近视为词间空格
            let mut num_end = i + 1;
            while num_end < n && chars[num_end].is_ascii_digit() {
                num_end += 1;
            }
            let num: String = chars[i + 1..num_end].iter().collect();
            if num.parse::<i64>().map(|v| v >= 100).unwrap_or(false) {
                pending_space = true;
            }
            i = num_end;
            continue;
        }
        // 操作符
        let two: String = chars[i..(i + 2).min(n)].iter().collect();
        if two == "Td" || two == "TD" || two == "T*" || two == "ET" || ch == '\'' || ch == '"' {
            out.push('\n');
            i += if two == "Td" || two == "TD" || two == "T*" || two == "ET" { 2 } else { 1 };
            continue;
        }
        if ch == 'T' && chars.get(i + 1) == Some(&'J') {
            pending_space = false;
            i += 2;
            continue;
        }
        i += 1;
    }
    out.split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_text(out: &mut String, pending_space: &mut bool, s: &str) {
    if *pending_space && !out.is_empty() && !out.ends_with(|c: char| c.is_whitespace()) {
        out.push(' ');
    }
    *pending_space = false;
    out.push_str(s);
}

/// 解码 PDF 字符串字节：FE FF → UTF-16BE；EF BB BF → UTF-8；其余 latin1。
fn decode_pdf_string(buf: &[u8]) -> String {
    if buf.len() >= 2 && buf[0] == 0xfe && buf[1] == 0xff {
        // UTF-16BE：两两组合
        let units: Vec<u16> = buf[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if buf.len() >= 3 && buf[0] == 0xef && buf[1] == 0xbb && buf[2] == 0xbf {
        return String::from_utf8_lossy(&buf[3..]).to_string();
    }
    buf.iter().map(|b| *b as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flate_stream(content: &str) -> Vec<u8> {
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(content.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut out = b"<< /Length 10 /Filter /FlateDecode >>\nstream\n".to_vec();
        out.extend_from_slice(&compressed);
        out.extend_from_slice(b"\nendstream");
        out
    }

    #[test]
    fn 读取_压缩内容流的文本层() {
        let content = "BT /F1 12 Tf\n(Hello \\(world\\)) Tj\n0 -20 Td\n(Second line) Tj\nET";
        let mut raw = b"%PDF-1.4\n".to_vec();
        raw.extend_from_slice(&flate_stream(content));
        raw.extend_from_slice(b"\n%%EOF");
        let result = read(&raw).unwrap();
        assert_eq!(result.kind, "pdf");
        assert!(result.text.contains("Hello (world)"));
        assert!(result.text.contains("Second line"));
        assert!(result.note.is_empty());
    }

    #[test]
    fn 加密pdf报错() {
        let raw = b"%PDF-1.4\n/Encrypt 1 0 R\n%%EOF".to_vec();
        assert!(read(&raw).is_err());
    }

    #[test]
    fn 无文本层提示扫描件() {
        let raw = b"%PDF-1.4\n<< /Subtype /Image >>\nstream\n\x89PNG\nendstream\n%%EOF".to_vec();
        let result = read(&raw).unwrap();
        assert!(result.note.contains("扫描件"));
    }

    #[test]
    fn 十六进制与utf16字符串() {
        // <FEFF0041 0042> = UTF-16BE "AB"
        let content = "BT <FE FF 0041 0042> Tj ET";
        assert_eq!(extract_pdf_text(content), "AB");
        let content2 = "BT <48656C6C6F> Tj ET";
        assert_eq!(extract_pdf_text(content2), "Hello");
    }

    #[test]
    fn 负间距视为词间空格() {
        let content = "BT (Word) Tj -150 (Word) Tj ET";
        assert_eq!(extract_pdf_text(content), "Word Word");
    }
}
