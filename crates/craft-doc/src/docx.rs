//! docx 读取与文本替换：word/document.xml 流式扫描。
//! <w:p> 段落成行；表格 <w:tbl>/<w:tr>/<w:tc>（单元格内换行连接，单元格间 tab 连接）；
//! <w:t> 文本、<w:tab/>→\t、<w:br/>→\n。页眉页脚批注不在范围。

use crate::xml::{xml_escape, xml_unescape};
use crate::zipio;
use crate::{DocEdit, DocResult, MAX_TEXT_BYTES};

const DOCUMENT_XML: &str = "word/document.xml";

pub fn read(raw: &[u8]) -> Result<DocResult, String> {
    let entries = zipio::load_entries(raw)?;
    let Some(xml_bytes) = entries.get(DOCUMENT_XML) else {
        return Ok(DocResult {
            kind: "docx".into(),
            text: String::new(),
            note: "文档缺少正文内容。".into(),
            truncated: false,
        });
    };
    let xml = String::from_utf8_lossy(xml_bytes).to_string();
    let bytes = xml.as_bytes();
    let n = bytes.len();

    let mut lines: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < n {
        let Some(lt_rel) = find_byte(bytes, i, b"<") else { break };
        let lt = lt_rel;
        if starts_with_at(bytes, lt, b"<w:p ") || starts_with_at(bytes, lt, b"<w:p>") {
            // 解析一个段落：找配对的 </w:p>
            let mut j = lt;
            let mut depth = 0i32;
            let body_end;
            loop {
                let next_open = find_byte(bytes, j + 1, b"<").unwrap_or(n);
                let next_close = find_byte(bytes, j + 1, b"</w:p>").unwrap_or(usize::MAX);
                if next_close == usize::MAX {
                    j = n;
                    body_end = n;
                    break;
                }
                if next_open < next_close {
                    if starts_with_at(bytes, next_open, b"<w:p") {
                        depth += 1;
                    }
                    match find_byte(bytes, next_open, b">") {
                        Some(end) => j = end,
                        None => {
                            j = n;
                            body_end = n;
                            break;
                        }
                    }
                } else if depth == 0 {
                    // 段落体范围 [lt, next_close)
                    j = next_close + 6;
                    body_end = next_close;
                    break;
                } else {
                    depth -= 1;
                    j = next_close + 6;
                }
            }
            let body = &xml[lt..body_end.min(xml.len())];
            lines.push(extract_paragraph_text(body));
            i = j;
            continue;
        }
        if starts_with_at(bytes, lt, b"<w:tbl") {
            // 解析表格：找配对的 </w:tbl>
            let table_end = find_balanced(&xml, lt, "<w:tbl", "</w:tbl>");
            let Some(table_end) = table_end else { break };
            let body = &xml[lt..table_end];
            lines.extend(extract_table(body));
            i = table_end;
            continue;
        }
        i = find_byte(bytes, lt, b">").map(|p| p + 1).unwrap_or(n);
        if i <= lt {
            break;
        }
    }

    let mut text = lines.join("\n");
    let mut truncated = false;
    if text.len() > MAX_TEXT_BYTES {
        let mut byte_end = MAX_TEXT_BYTES;
        while byte_end > 0 && !text.is_char_boundary(byte_end) {
            byte_end -= 1;
        }
        text = format!("{}\n…（文本已截断）", &text[..byte_end]);
        truncated = true;
    }
    Ok(DocResult { kind: "docx".into(), text, note: String::new(), truncated })
}

fn find_byte(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|pos| pos + from)
}

fn starts_with_at(haystack: &[u8], at: usize, needle: &[u8]) -> bool {
    haystack.len() >= at + needle.len() && &haystack[at..at + needle.len()] == needle
}

/// 提取一个 <w:p>...</w:p> 体内的可见文本。
fn extract_paragraph_text(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    while let Some(open) = find_byte(bytes, i, b"<") {
        if starts_with_at(bytes, open, b"<w:t") {
            let Some(close) = find_byte(bytes, open, b"</w:t>") else { break };
            let tag_end = find_byte(bytes, open, b">").unwrap_or(close);
            let is_open = tag_end == 0 || bytes[tag_end - 1] != b'/';
            if is_open {
                out.push_str(&xml_unescape(&body[tag_end + 1..close]));
                i = close + 6;
                continue;
            }
            // 自闭合（如 <w:tab/> 误中 <w:t 前缀）：与 TS 一致落到下方 tab/br 检查
            let Some(end) = find_byte(bytes, open, b">") else { break };
            i = end + 1;
            if starts_with_at(bytes, open, b"<w:tab") {
                out.push('\t');
            } else if starts_with_at(bytes, open, b"<w:br") {
                out.push('\n');
            }
            continue;
        }
        if starts_with_at(bytes, open, b"<w:tab") {
            out.push('\t');
        } else if starts_with_at(bytes, open, b"<w:br") {
            out.push('\n');
        }
        let Some(end) = find_byte(bytes, open, b">") else { break };
        i = end + 1;
    }
    out
}

/// 找配对的闭合标签（处理同名嵌套），返回闭合结束位置。
fn find_balanced(xml: &str, from: usize, open_tag: &str, close_tag: &str) -> Option<usize> {
    let bytes = xml.as_bytes();
    let mut depth = 0i32;
    let mut i = from;
    loop {
        let open = find_byte(bytes, i, open_tag.as_bytes());
        let close = find_byte(bytes, i, close_tag.as_bytes())?;
        let open_is_real = match open {
            Some(o) => o < close && !starts_with_at(bytes, o + 1, b"/"),
            None => false,
        };
        if open_is_real {
            depth += 1;
            i = open.unwrap() + open_tag.len();
        } else {
            depth -= 1;
            i = close + close_tag.len();
            if depth == 0 {
                return Some(i);
            }
        }
    }
}

/// 表格提取：行内单元格多段落 \n 连接，单元格间 \t 连接。
fn extract_table(body: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0usize;
    while let Some(tr_open) = body.find("<w:tr") {
        let tr_open = tr_open.max(i);
        if tr_open != body.find("<w:tr").unwrap() && tr_open >= body.len() { break }
        let Some(tr_end) = find_balanced(body, tr_open, "<w:tr", "</w:tr>") else { break };
        let tr_body = &body[tr_open..tr_end];
        let mut cells: Vec<String> = Vec::new();
        let mut j = 0usize;
        while let Some(tc_open) = find_byte(tr_body.as_bytes(), j, b"<w:tc") {
            let Some(tc_end) = find_balanced(tr_body, tc_open, "<w:tc", "</w:tc>") else { break };
            let tc_body = &tr_body[tc_open..tc_end];
            let mut paragraphs: Vec<String> = Vec::new();
            let mut k = 0usize;
            while let Some(p_open) = find_byte(tc_body.as_bytes(), k, b"<w:p") {
                let Some(p_end_rel) = find_byte(tc_body.as_bytes(), p_open, b"</w:p>") else { break };
                paragraphs.push(extract_paragraph_text(&tc_body[p_open..p_end_rel]));
                k = p_end_rel + 6;
            }
            cells.push(paragraphs.join("\n"));
            j = tc_end;
        }
        lines.push(cells.join("\t"));
        i = tr_end;
    }
    lines
}

pub fn replace(path: &str, edit: &DocEdit) -> Result<(), String> {
    if edit.kind != "text" {
        return Err("docx 只支持文本替换".into());
    }
    let Some(old) = &edit.old else {
        return Err("docx 只支持文本替换".into());
    };
    if old.is_empty() {
        return Err("docx 只支持文本替换".into());
    }
    let raw = std::fs::read(path).map_err(|e| format!("读取失败：{e}"))?;
    let entries = zipio::load_entries(&raw)?;
    let Some(xml_bytes) = entries.get(DOCUMENT_XML) else {
        return Err("文档缺少正文内容".into());
    };
    let xml = String::from_utf8_lossy(xml_bytes).to_string();
    let old_escaped = xml_escape(old);
    let new_escaped = xml_escape(edit.new.as_deref().unwrap_or(""));
    let count = xml.matches(&old_escaped).count();
    if count == 0 {
        return Err("没有找到要替换的文本；文本可能被拆分到多个格式片段中，请调整原文".into());
    }
    if count > 1 {
        return Err(format!("找到 {count} 处相同文本，请提供更长的唯一片段"));
    }
    let updated = xml.replacen(&old_escaped, &new_escaped, 1);
    zipio::rewrite_single(&raw, path, DOCUMENT_XML, &updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小 docx（zip 含 word/document.xml）
    pub(crate) fn minimal_docx(document_xml: &str) -> Vec<u8> {
        let buf = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(buf);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file(DOCUMENT_XML, options).unwrap();
        std::io::Write::write_all(&mut writer, document_xml.as_bytes()).unwrap();
        writer.finish().unwrap().into_inner()
    }

    pub(crate) const SAMPLE: &str = r#"<?xml version="1.0"?>
<w:document xmlns:w="urn:x">
  <w:body>
    <w:p><w:r><w:t>标题段落</w:t></w:r></w:p>
    <w:p><w:r><w:t>带</w:t></w:r><w:r><w:tab/><w:t>制表与</w:t><w:br/><w:t>换行</w:t></w:r></w:p>
    <w:tbl>
      <w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr>
      <w:tr><w:tc><w:p><w:r><w:t>A2</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B2</w:t></w:r></w:p></w:tc></w:tr>
    </w:tbl>
  </w:body>
</w:document>"#;

    #[test]
    fn 读取_段落与表格() {
        let raw = minimal_docx(SAMPLE);
        let result = read(&raw).unwrap();
        assert_eq!(result.kind, "docx");
        // 段落成行；段内 <w:br/> 也产生换行（第二段拆成两行）
        assert_eq!(
            result.text,
            "标题段落\n带\t制表与\n换行\nA1\tB1\nA2\tB2"
        );
        assert!(!result.truncated);
    }

    #[test]
    fn 替换_唯一命中与多次拒绝() {
        let tmp = std::env::temp_dir().join(format!(
            "cw-docx-{}-{}.docx",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::write(&tmp, minimal_docx(SAMPLE)).unwrap();
        let path = tmp.to_string_lossy().to_string();
        let path: &str = &path;
        replace(&path, &DocEdit { kind: "text".into(), old: Some("标题段落".to_string()), new: Some("改后标题".to_string()), sheet: None, cell: None, value: None }).unwrap();
        let result = read(std::fs::read(&path).unwrap().as_slice()).unwrap();
        assert!(result.text.contains("改后标题"));
        assert!(!result.text.contains("标题段落"));

        // 两处命中拒绝
        let err = replace(&path, &DocEdit { kind: "text".into(), old: Some("A".to_string()), new: Some("X".to_string()), sheet: None, cell: None, value: None }).unwrap_err();
        assert!(err.contains("2 处"));
        // 无命中提示
        let err = replace(&path, &DocEdit { kind: "text".into(), old: Some("不存在的内容".to_string()), new: Some("X".into()), sheet: None, cell: None, value: None }).unwrap_err();
        assert!(err.contains("没有找到"));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn 读取_转义实体还原() {
        let xml = r#"<w:p><w:r><w:t>a&lt;b &amp; c</w:t></w:r></w:p>"#;
        let raw = minimal_docx(xml);
        let result = read(&raw).unwrap();
        assert_eq!(result.text, "a<b & c");
    }
}
