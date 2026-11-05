//! XML 转义工具（doc/index.ts 的 xmlEscape/xmlUnescape 直译）。
//! 供 docx/xlsx 的文本级替换使用：替换前对 old/new 转义、读取时对提取文本反转义，
//! 保证子串替换只命中正文文本而不会破坏标签结构。

/// XML 转义五个保留字符；`&` 必须最先替换，避免把后续已生成实体的 `&` 再转义一次。
pub fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 反转义：命名实体（lt/gt/quot/apos/amp）与 `&#十进制;` / `&#x十六进制;` 数字实体；
/// 未知实体或 `&` 后无分号时原样保留，不报错。
pub fn xml_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let Some(semi_rel) = after.find(';') else {
            // 无闭合分号：原样保留其余内容
            out.push_str(after);
            return out;
        };
        let entity = &after[..=semi_rel];
        match entity {
            "&lt;" => out.push('<'),
            "&gt;" => out.push('>'),
            "&quot;" => out.push('"'),
            "&apos;" => out.push('\''),
            "&amp;" => out.push('&'),
            _ => {
                let decoded = entity
                    .strip_prefix("&#")
                    .and_then(|s| s.strip_suffix(';'))
                    .and_then(|num| {
                        if let Some(hex) = num.strip_prefix(['x', 'X']) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            num.parse::<u32>().ok()
                        }
                    })
                    .and_then(char::from_u32);
                match decoded {
                    Some(c) => out.push(c),
                    None => out.push_str(entity), // 未知实体原样保留
                }
            }
        }
        rest = &rest[pos + semi_rel + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        assert_eq!(xml_escape("a<b&\"c\"'d'"), "a&lt;b&amp;&quot;c&quot;&apos;d&apos;");
        assert_eq!(xml_unescape("a&lt;b&amp;&quot;c&quot;&apos;d&apos;"), "a<b&\"c\"'d'");
        assert_eq!(xml_unescape("&#20013;&#x6587;"), "中文");
        assert_eq!(xml_unescape("&unknown;"), "&unknown;");
        assert_eq!(xml_unescape("no entity"), "no entity");
    }
}
