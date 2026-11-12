//! xlsx 读取与单元格更新：sharedStrings/sheet XML 定点替换，保留原结构与样式。
//! =开头→公式（不含等号）；数字→<v>；空串→自闭合；其余→inlineStr 保留空格。


use crate::xml::{xml_escape, xml_unescape};
use crate::zipio;
use crate::{DocEdit, DocResult, MAX_TEXT_BYTES};

/// 工作表：名称与其 sheet XML 在 zip 内的路径。
#[derive(Debug, Clone)]
struct XlsxSheet {
    name: String,
    path: String,
}

/// 列字母 → 序号（A=1、Z=26、AA=27，26 进制）。
fn col_to_number(col: &str) -> i64 {
    let mut n: i64 = 0;
    for ch in col.chars() {
        n = n * 26 + (ch.to_ascii_uppercase() as i64 - 64);
    }
    n
}

/// 序号 → 列字母（与 col_to_number 互逆）。
fn number_to_col(n: i64) -> String {
    let mut s = String::new();
    let mut n = n;
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        s.insert(0, (b'A' + rem) as char);
        n = (n - 1) / 26;
    }
    s
}

/// 一次加载的解析产物：zip 条目、共享字符串表与工作表清单。
struct Loaded {
    entries: std::collections::HashMap<String, Vec<u8>>,
    shared: Vec<String>,
    sheets: Vec<XlsxSheet>,
}

/// 解析 workbook.xml、rels 与 sharedStrings：
/// 共享字符串把 <si> 内多个 <t> 片段拼接为完整字符串；
/// 工作表经「sheet 名 → r:id → rels Target」映射到 zip 内的 sheet XML 路径。
fn load(raw: &[u8]) -> Result<Loaded, String> {
    let entries = zipio::load_entries(raw)?;
    let workbook = entries
        .get("xl/workbook.xml")
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let rels = entries
        .get("xl/_rels/workbook.xml.rels")
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let shared_xml = entries
        .get("xl/sharedStrings.xml")
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();

    let mut shared: Vec<String> = Vec::new();
    if !shared_xml.is_empty() {
        for si in extract_blocks(&shared_xml, "<si>", "</si>") {
            let mut texts: Vec<String> = Vec::new();
            let mut rest = si.as_str();
            while let Some(start) = rest.find("<t") {
                // <t> 或 <t ...>
                let tag_end = rest[start..].find('>').map(|p| start + p).unwrap_or(rest.len());
                let tag = &rest[start..tag_end];
                if tag.ends_with('/') {
                    rest = &rest[tag_end..];
                    continue;
                }
                if !tag.starts_with("<t>") && !tag.contains("xml:space") && !tag.ends_with("<t") {
                    // 空分支：无任何过滤效果（历史遗留，见问题清单）
                }
                let Some(close_rel) = rest[tag_end..].find("</t>") else { break };
                let close = tag_end + close_rel;
                texts.push(xml_unescape(&rest[tag_end + 1..close]));
                rest = &rest[close + 4..];
            }
            shared.push(texts.join(""));
        }
    }

    // sheet 名 → r:id → Target 路径
    let mut rel_map = std::collections::HashMap::new();
    for caps in regex_like_attrs(&rels, "<Relationship") {
        if let (Some(id), Some(target)) = (caps.get("Id"), caps.get("Target")) {
            let mut target = target.clone();
            if let Some(stripped) = target.strip_prefix('/') {
                target = stripped.to_string();
            } else if !target.starts_with("xl/") {
                target = format!("xl/{target}");
            }
            rel_map.insert(id.clone(), target);
        }
    }
    let mut sheets: Vec<XlsxSheet> = Vec::new();
    for caps in regex_like_attrs(&workbook, "<sheet") {
        let sheet = caps
            .get("name")
            .zip(caps.get("r:id"))
            .and_then(|(name, rid)| rel_map.get(rid).map(|target| (name, target)))
            .map(|(name, target)| XlsxSheet { name: xml_unescape(name), path: target.clone() });
        if let Some(sheet) = sheet {
            sheets.push(sheet);
        }
    }
    Ok(Loaded { entries, shared, sheets })
}

/// 提取 `<block_open ...>...</block_close>` 块（非嵌套）。
fn extract_blocks(xml: &str, open: &str, close: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(open) {
        let Some(close_rel) = rest[start..].find(close) else { break };
        blocks.push(rest[start + open.len()..start + close_rel].to_string());
        rest = &rest[start + close_rel + close.len()..];
    }
    blocks
}

/// 轻量属性提取（替代 TS 的正则）：扫描自闭合/开标签的 key="value" 对，
/// 返回每个命中的 `tag_prefix` 标签的属性表。
fn regex_like_attrs(xml: &str, tag_prefix: &str) -> Vec<std::collections::HashMap<String, String>> {
    let mut results = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    while let Some(pos) = find_from(bytes, i, tag_prefix.as_bytes()) {
        let tag_end = find_from(bytes, pos, b">").map(|p| p + 1).unwrap_or(bytes.len());
        let tag_text = &xml[pos..tag_end.min(xml.len())];
        let mut attrs = std::collections::HashMap::new();
        let mut rest = tag_text;
        while let Some(eq) = rest.find("=\"") {
            let key_start = rest[..eq].rsplit(|c: char| !c.is_alphanumeric() && c != ':').next().unwrap_or("").to_string();
            let value_start = eq + 2;
            if let Some(close) = rest[value_start..].find('"') {
                let value = &rest[value_start..value_start + close];
                attrs.insert(key_start, value.to_string());
                rest = &rest[value_start + close + 1..];
            } else {
                break;
            }
        }
        results.push(attrs);
        i = tag_end;
    }
    results
}

fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// 读取工作簿：每个工作表输出「工作表：<名>」标题行，
/// 数据行格式为 `<列字母><行号>: <值>`、单元格间 tab 分隔；公式以 `=` 原文展示（不计算）。
pub fn read(raw: &[u8]) -> Result<DocResult, String> {
    let loaded = load(raw)?;
    if loaded.sheets.is_empty() {
        return Ok(DocResult { kind: "xlsx".into(), text: String::new(), note: "工作簿没有工作表。".into(), truncated: false });
    }
    let mut parts: Vec<String> = Vec::new();
    for sheet in &loaded.sheets {
        let sheet_xml = loaded
            .entries
            .get(&sheet.path)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        parts.push(format!("工作表：{}", sheet.name));
        // 行：<row r="N">...</row>
        for (row_num, row_body) in rows_of(&sheet_xml) {
            let mut cells: Vec<(i64, String)> = Vec::new();
            for (attrs, inner) in cells_of(&row_body) {
                let col = attrs
                    .get("r")
                    .map(|r| col_to_number(&r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>()))
                    .unwrap_or((cells.len() + 1) as i64);
                let cell_type = attrs.get("t").cloned().unwrap_or_default();
                let text = if let Some(formula) = extract_inner_tag(&inner, "<f", "</f>") {
                    // 公式：保留原文（本模块不计算公式）
                    format!("={}", xml_unescape(&formula))
                } else if cell_type == "s" {
                    // t="s"：共享字符串索引
                    extract_inner_tag(&inner, "<v", "</v>")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .map(|idx| loaded.shared.get(idx).cloned().unwrap_or_default())
                        .unwrap_or_default()
                } else if cell_type == "inlineStr" {
                    // 内联字符串：拼接全部 <t> 文本
                    let mut texts = Vec::new();
                    let mut rest = inner.as_str();
                    while let Some(start) = rest.find("<t") {
                        let tag_end = rest[start..].find('>').map(|p| start + p).unwrap_or(rest.len());
                        if rest[..tag_end].ends_with('/') {
                            rest = &rest[tag_end..];
                            continue;
                        }
                        let Some(close_rel) = rest[tag_end..].find("</t>") else { break };
                        let close = tag_end + close_rel;
                        texts.push(xml_unescape(&rest[tag_end + 1..close]));
                        rest = &rest[close + 4..];
                    }
                    texts.join("")
                } else if cell_type == "b" {
                    // 布尔单元格：1 → TRUE，其余 FALSE
                    let v = extract_inner_tag(&inner, "<v", "</v>").unwrap_or_default();
                    if v.trim() == "1" { "TRUE".into() } else { "FALSE".into() }
                } else if let Some(v) = extract_inner_tag(&inner, "<v", "</v>") {
                    xml_unescape(&v)
                } else {
                    String::new()
                };
                cells.push((col, text));
            }
            cells.sort_by_key(|(col, _)| *col);
            let line = cells
                .iter()
                .map(|(col, text)| format!("{}{row_num}: {text}", number_to_col(*col)))
                .collect::<Vec<_>>()
                .join("\t");
            if !line.is_empty() {
                parts.push(line);
            }
        }
    }
    let mut text = parts.join("\n");
    let mut truncated = false;
    if text.len() > MAX_TEXT_BYTES {
        let mut byte_end = MAX_TEXT_BYTES;
        while byte_end > 0 && !text.is_char_boundary(byte_end) {
            byte_end -= 1;
        }
        text = format!("{}\n…（文本已截断）", &text[..byte_end]);
        truncated = true;
    }
    Ok(DocResult { kind: "xlsx".into(), text, note: String::new(), truncated })
}

/// 行提取：<row r="N">body</row>
fn rows_of(sheet_xml: &str) -> Vec<(i64, String)> {
    let mut rows = Vec::new();
    let bytes = sheet_xml.as_bytes();
    let mut i = 0usize;
    while let Some(open) = find_from(bytes, i, b"<row") {
        // 确认行号属性在标签内
        let tag_end = find_from(bytes, open, b">").unwrap_or(open);
        let tag_text = &sheet_xml[open..tag_end.min(sheet_xml.len())];
        let row_num = tag_text
            .split_whitespace()
            .find(|part| part.starts_with("r="))
            .and_then(|part| part.trim_start_matches("r=\"").trim_end_matches('"').parse::<i64>().ok());
        // 自闭合行
        if sheet_xml.as_bytes().get(tag_end.saturating_sub(1)) == Some(&b'/') {
            if let Some(num) = row_num {
                rows.push((num, String::new()));
            }
            i = tag_end;
            continue;
        }
        let Some(close) = find_from(bytes, tag_end, b"</row>") else { break };
        let body = sheet_xml[tag_end + 1..close].to_string();
        if let Some(num) = row_num {
            rows.push((num, body));
        }
        i = close + 6;
    }
    rows
}

/// 单元格提取：<c attrs>inner</c> 或 <c attrs/>
fn cells_of(row_body: &str) -> Vec<(std::collections::HashMap<String, String>, String)> {
    let mut cells = Vec::new();
    let bytes = row_body.as_bytes();
    let mut i = 0usize;
    while let Some(open) = find_from(bytes, i, b"<c") {
        // 标签边界（<c 后跟非字母）
        let after = bytes.get(open + 2).copied().unwrap_or(b'>');
        if after.is_ascii_alphanumeric() {
            i = open + 2;
            continue;
        }
        let tag_end = find_from(bytes, open, b">").unwrap_or(open);
        let tag_text = &row_body[open..tag_end.min(row_body.len())];
        let attrs = parse_attrs(tag_text);
        if tag_text.ends_with('/') {
            cells.push((attrs, String::new()));
            i = tag_end;
            continue;
        }
        let Some(close) = find_from(bytes, tag_end, b"</c>") else { break };
        let inner = row_body[tag_end + 1..close].to_string();
        cells.push((attrs, inner));
        i = close + 4;
    }
    cells
}

/// 解析标签文本中的 key="value" 属性对（与 regex_like_attrs 同一套手写解析）。
fn parse_attrs(tag_text: &str) -> std::collections::HashMap<String, String> {
    let mut attrs = std::collections::HashMap::new();
    let mut rest = tag_text;
    while let Some(eq) = rest.find("=\"") {
        let key_start = rest[..eq].rsplit(|c: char| !c.is_alphanumeric() && c != ':').next().unwrap_or("").to_string();
        let value_start = eq + 2;
        if let Some(close) = rest[value_start..].find('"') {
            attrs.insert(key_start, rest[value_start..value_start + close].to_string());
            rest = &rest[value_start + close + 1..];
        } else {
            break;
        }
    }
    attrs
}

/// 取 `<open ...>...</close>` 的内部文本；自闭合标签返回空串。
fn extract_inner_tag(inner: &str, open: &str, close: &str) -> Option<String> {
    let start = inner.find(open)?;
    let tag_end = inner[start..].find('>').map(|p| start + p)?;
    if inner.as_bytes().get(tag_end.saturating_sub(1)) == Some(&b'/') {
        return Some(String::new());
    }
    let close_rel = inner[tag_end..].find(close)?;
    Some(inner[tag_end + 1..tag_end + close_rel].to_string())
}

/// buildCellXML：=开头→公式（不含等号）；数字→<v>；空串→自闭合；其余→inlineStr 保留空格。
fn build_cell_xml(value: &str) -> String {
    if let Some(formula) = value.strip_prefix('=') {
        return format!("<f>{}</f>", xml_escape(formula));
    }
    let trimmed = value.trim();
    if !value.is_empty() && is_plain_number(trimmed) {
        return format!("<v>{value}</v>");
    }
    if value.is_empty() {
        return String::new();
    }
    format!("<is><t xml:space=\"preserve\">{}</t></is>", xml_escape(value))
}

/// 是否按数字写入 `<v>`：可选负号 + 整数/小数；不支持正号与科学计数法（"+1"、"1e5" 按文本写）。
fn is_plain_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty() {
        return false;
    }
    let mut parts = body.split('.');
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next();
    if parts.next().is_some() {
        return false;
    }
    if int_part.is_empty() && frac_part.map(|f| f.is_empty()).unwrap_or(true) {
        return false;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) || int_part.is_empty() {
        return false;
    }
    if frac_part.is_some_and(|frac| !frac.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }

    true
}

/// 更新单元格：解析工作表（名称或从 1 序号）与单元格引用后写入，
/// 三种落点——原位替换已有 <c>、行内按列序插入、新建行按行号插入 sheetData；
/// 清空值写成自闭合 <c/>。重写 zip 写回，写前备份由调用方负责。
pub fn replace(path: &str, edit: &DocEdit) -> Result<(), String> {
    if edit.kind != "cell" {
        return Err("xlsx 只支持单元格更新".into());
    }
    let Some(cell) = &edit.cell else {
        return Err("xlsx 只支持单元格更新".into());
    };
    if cell.trim().is_empty() {
        return Err("cell 不能为空".into());
    }
    let raw = std::fs::read(path).map_err(|e| format!("读取失败：{e}"))?;
    let loaded = load(&raw)?;
    if loaded.sheets.is_empty() {
        return Err("工作簿没有工作表".into());
    }

    // 解析目标 sheet：名称或从 1 序号
    let mut sheet = loaded.sheets[0].clone();
    if edit.sheet.as_ref().is_some_and(|s| !s.is_empty()) {
        let sheet_name = edit.sheet.as_deref().unwrap_or_default();
        if let Ok(as_num) = sheet_name.parse::<usize>() {
            if (1..=loaded.sheets.len()).contains(&as_num) {
                sheet = loaded.sheets[as_num - 1].clone();
            } else {
                return Err(format!("找不到工作表：{sheet_name}"));
            }
        } else {
            let found = loaded.sheets.iter().find(|s| s.name == *sheet_name);
            match found {
                Some(found) => sheet = found.clone(),
                None => return Err(format!("找不到工作表：{sheet_name}")),
            }
        }
    }

    let reference = cell.to_uppercase();
    let split = reference
        .find(|c: char| !c.is_ascii_uppercase())
        .unwrap_or(reference.len());
    let col_part = &reference[..split];
    let row_part = &reference[split..];
    let valid = !col_part.is_empty()
        && row_part.parse::<u64>().map(|n| n >= 1).unwrap_or(false)
        && reference == format!("{col_part}{row_part}");
    if !valid {
        return Err(format!("无效的单元格引用：{cell}"));
    }
    let col_letters: String = reference.chars().take_while(|c| c.is_ascii_uppercase()).collect();
    let row_num: i64 = reference[col_letters.len()..].parse().unwrap_or(1);
    let col_num = col_to_number(&col_letters);

    let mut sheet_xml = loaded
        .entries
        .get(&sheet.path)
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    let cell_xml = build_cell_xml(edit.value.as_deref().unwrap_or(""));

    let build_cell = |cell_xml: &str| -> String {
        if cell_xml.is_empty() {
            return format!("<c r=\"{reference}\"/>");
        }
        if cell_xml.starts_with("<is>") {
            return format!("<c r=\"{reference}\" t=\"inlineStr\">{cell_xml}</c>");
        }
        format!("<c r=\"{reference}\">{cell_xml}</c>")
    };

    // 原位替换
    let cell_pattern = format!("<c r=\"{reference}\"");
    if let Some(pos) = find_from(sheet_xml.as_bytes(), 0, cell_pattern.as_bytes()) {
        // 找该单元格的完整范围（自闭合或成对）
        let tag_end = find_from(sheet_xml.as_bytes(), pos, b">").unwrap_or(pos);
        let end = if sheet_xml.as_bytes().get(tag_end.saturating_sub(1)) == Some(&b'/') {
            tag_end
        } else {
            find_from(sheet_xml.as_bytes(), tag_end, b"</c>").map(|p| p + 4).unwrap_or(tag_end)
        };
        let updated = format!("{}{}{}", &sheet_xml[..pos], build_cell(&cell_xml), &sheet_xml[end..]);
        sheet_xml = updated;
    } else {
        // 行内按列序插入 / 新建行按行号插入 / 追加末尾
        let row_open = format!("<row r=\"{row_num}\"");
        let row_match_start = find_from(sheet_xml.as_bytes(), 0, row_open.as_bytes());
        if let Some(row_start) = row_match_start {
            let row_tag_end = find_from(sheet_xml.as_bytes(), row_start, b">").unwrap_or(row_start);
            if sheet_xml.as_bytes().get(row_tag_end.saturating_sub(1)) == Some(&b'/') {
                // 自闭合行：<row r="N"/>
                if !cell_xml.is_empty() {
                    let new_row = format!("<row r=\"{row_num}\">{}</row>", build_cell(&cell_xml));
                    let row_end = find_from(sheet_xml.as_bytes(), row_tag_end, b"/>").map(|p| p + 2).unwrap_or(row_tag_end);
                    sheet_xml = format!("{}{}{}", &sheet_xml[..row_start], new_row, &sheet_xml[row_end..]);
                }
            } else {
                let row_close = find_from(sheet_xml.as_bytes(), row_tag_end, b"</row>").unwrap_or(row_tag_end);
                let row_body = &sheet_xml[row_tag_end + 1..row_close];
                let mut insert_pos = row_close;
                // 按列序找插入点
                let mut scan = row_body.as_bytes();
                let mut offset = 0usize;
                while let Some(cell_open) = find_from(scan, 0, b"<c r=\"") {
                    let ref_start = cell_open + 6;
                    let ref_end = find_from(scan, ref_start, b"\"").unwrap_or(ref_start);
                    let cell_ref = &row_body[offset + ref_start..offset + ref_end];
                    let cell_col: String = cell_ref.chars().take_while(|c| c.is_ascii_uppercase()).collect();
                    let c = col_to_number(&cell_col);
                    if c > col_num && !cell_xml.is_empty() {
                        insert_pos = row_tag_end + 1 + offset + cell_open;
                        break;
                    }
                    let this_tag_end = find_from(scan, cell_open, b">").unwrap_or(cell_open);
                    let this_end = if scan.get(this_tag_end.saturating_sub(1)) == Some(&b'/') {
                        this_tag_end
                    } else {
                        find_from(scan, this_tag_end, b"</c>").map(|p| p + 4).unwrap_or(this_tag_end)
                    };
                    offset += this_end;
                    scan = &row_body.as_bytes()[offset.min(row_body.len())..];
                    if offset >= row_body.len() {
                        break;
                    }
                }
                if !cell_xml.is_empty() {
                    sheet_xml = format!("{}{}{}", &sheet_xml[..insert_pos], build_cell(&cell_xml), &sheet_xml[insert_pos..]);
                }
            }
        } else {
            // 新建行：插入到正确位置（按行号）或 sheetData 末尾
            let new_row = format!("<row r=\"{row_num}\">{}</row>", if cell_xml.is_empty() { String::new() } else { build_cell(&cell_xml) });
            let Some(sheet_data_open) = find_from(sheet_xml.as_bytes(), 0, b"<sheetData") else {
                return Err("工作表缺少 sheetData".into());
            };
            let insert_at = find_sheet_data_insert_pos(&sheet_xml, sheet_data_open, row_num);
            sheet_xml = format!("{}{}{}", &sheet_xml[..insert_at], new_row, &sheet_xml[insert_at..]);
        }
    }

    zipio::rewrite_single(&raw, path, &sheet.path, &sheet_xml)
}

/// 找 sheetData 内按行号排序的插入位置。
fn find_sheet_data_insert_pos(xml: &str, sheet_data_open: usize, row_num: i64) -> usize {
    let body_start = find_from(xml.as_bytes(), sheet_data_open, b">").map(|p| p + 1).unwrap_or(xml.len());
    let bytes = xml.as_bytes();
    let mut i = body_start;
    while let Some(open) = find_from(bytes, i, b"<row") {
        let tag_end = find_from(bytes, open, b">").unwrap_or(open);
        let tag_text = &xml[open..tag_end.min(xml.len())];
        if let Some(n) = tag_text
            .split_whitespace()
            .find(|p| p.starts_with("r=\""))
            .and_then(|p| p.trim_start_matches("r=\"").trim_end_matches('"').parse::<i64>().ok())
        {
            {
                if n > row_num {
                    return open;
                }
            }
        }
        i = tag_end;
    }
    xml.find("</sheetData>").unwrap_or(xml.len())
}

#[cfg(test)]
mod tests {
    use super::*;


    const SAMPLE_SHEET: &str = r#"<?xml version="1.0"?>
<worksheet xmlns="urn:x"><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
<row r="2"><c r="A2"><v>42</v></c><c r="B2"><f>SUM(A1:B1)</f></c></row>
</sheetData></worksheet>"#;

    fn minimal_xlsx(sheet_xml: &str) -> Vec<u8> {
        let buf = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(buf);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("xl/workbook.xml", options).unwrap();
        std::io::Write::write_all(
            &mut writer,
            r#"<workbook><sheets><sheet name="数据" r:id="rId1"/></sheets></workbook>"#.as_bytes(),
        )
        .unwrap();
        writer.start_file("xl/_rels/workbook.xml.rels", options).unwrap();
        std::io::Write::write_all(
            &mut writer,
            br#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>"#,
        )
        .unwrap();
        writer.start_file("xl/sharedStrings.xml", options).unwrap();
        std::io::Write::write_all(&mut writer, r#"<sst><si><t>名称</t></si><si><t>数值</t></si></sst>"#.as_bytes()).unwrap();
        writer.start_file("xl/worksheets/sheet1.xml", options).unwrap();
        std::io::Write::write_all(&mut writer, sheet_xml.as_bytes()).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn read_shared_strings_numbers_and_formulas() {
        let raw = minimal_xlsx(SAMPLE_SHEET);
        let result = read(&raw).unwrap();
        assert!(result.text.contains("工作表：数据"));
        assert!(result.text.contains("A1: 名称"));
        assert!(result.text.contains("B1: 数值"));
        assert!(result.text.contains("A2: 42"));
        assert!(result.text.contains("B2: =SUM(A1:B1)"), "公式保留原文");
    }

    #[test]
    fn cell_update_in_place_replacement_formulas_and_numbers() {
        let tmp = crate::test_util::unique_temp_file("xlsx", "xlsx");
        std::fs::write(&tmp, minimal_xlsx(SAMPLE_SHEET)).unwrap();
        let path = tmp.to_string_lossy().to_string();

        // 数值写入 A2（原位替换）
        replace(&path, &DocEdit { kind: "cell".into(), old: None, new: None, sheet: None, cell: Some("a2".into()), value: Some("100".into()) }).unwrap();
        let result = read(std::fs::read(&path).unwrap().as_slice()).unwrap();
        assert!(result.text.contains("A2: 100"));

        // 公式写入 C1（行内按列序插入）
        replace(&path, &DocEdit { kind: "cell".into(), old: None, new: None, sheet: None, cell: Some("C1".into()), value: Some("=SUM(A2)".into()) }).unwrap();
        let result = read(std::fs::read(&path).unwrap().as_slice()).unwrap();
        assert!(result.text.contains("C1: =SUM(A2)"));

        // 新建第 3 行
        replace(&path, &DocEdit { kind: "cell".into(), old: None, new: None, sheet: None, cell: Some("A3".into()), value: Some("新行".into()) }).unwrap();
        let result = read(std::fs::read(&path).unwrap().as_slice()).unwrap();
        assert!(result.text.contains("A3: 新行"));

        // 无效引用
        let err = replace(&path, &DocEdit { kind: "cell".into(), old: None, new: None, sheet: None, cell: Some("bad".into()), value: Some("x".into()) }).unwrap_err();
        assert!(err.contains("无效"));
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn number_detection() {
        assert!(is_plain_number("42"));
        assert!(is_plain_number("-3.14"));
        assert!(is_plain_number("0.5"));
        assert!(!is_plain_number(""));
        assert!(!is_plain_number("1.2.3"));
        assert!(!is_plain_number("abc"));
        assert!(!is_plain_number("--1"));
    }
}
