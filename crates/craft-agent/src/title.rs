//! 会话自动命名：首条消息可短名直用时直接用；否则调模型起标题（sidecar 调用计用量）。
//! 对应 src/main/agent/title.ts。

use craft_model::types::{single_system, Client};
use craft_store::db::Db;
use craft_store::records::add_usage;
use craft_store::threads::auto_title_thread;

const TITLE_MAX_TOKENS: i64 = 128;
const TITLE_LIMIT_RUNES: usize = 20;

const TITLE_SYSTEM_PROMPT: &str =
    "为下面的对话生成一个简短的中文标题。只输出标题本身，不要引号、句号或任何解释，不超过 20 个字。";

/// 取首行、合并空白、剥前缀装饰符号与反引号、去两侧标点引号。
pub fn normalize_title(text: &str) -> String {
    let mut s = text.split('\n').next().unwrap_or("").to_string();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s = s
        .trim_start_matches(['#', '>', '-', '*', '_', '`'])
        .trim()
        .to_string();
    s = s
        .trim_start_matches([
            ' ', '"', '\'', '“', '”', '‘', '’', '「', '」', '『', '』', '《', '》', '【', '】', '(', ')', '（', '）', '。',
            '，', ',', '、', '！', '!', '？', '?', '：', ':', '；', ';', '.', '-', '—',
        ])
        .to_string();
    s = s
        .trim_end_matches([
            ' ', '"', '\'', '“', '”', '‘', '’', '「', '」', '『', '』', '《', '》', '【', '】', '(', ')', '（', '）', '。',
            '，', ',', '、', '！', '!', '？', '?', '：', ':', '；', ';', '.', '-', '—',
        ])
        .to_string();
    s.trim().to_string()
}

fn has_content(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&c))
}

pub fn short_title(text: &str) -> String {
    let normalized = normalize_title(text);
    if !has_content(&normalized) {
        return String::new();
    }
    if normalized.chars().count() <= TITLE_LIMIT_RUNES {
        normalized
    } else {
        String::new()
    }
}

/// 模型标题清洗：剥「标题：/Title:」前缀；须含字母或数字；超长截断加省略号。
pub fn sanitize_model_title(raw: &str) -> String {
    let mut s = normalize_title(raw);
    for prefix in ["标题：", "标题:", "Title:", "title:", "Title：", "title："] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            s = stripped.trim().to_string();
            break;
        }
    }
    if !has_content(&s) {
        return String::new();
    }
    let runes: Vec<char> = s.chars().collect();
    if runes.len() <= TITLE_LIMIT_RUNES {
        return s;
    }
    let mut cut: String = runes[..TITLE_LIMIT_RUNES].iter().collect();
    cut = cut
        .trim_end_matches([
            ' ', '"', '\'', '“', '”', '‘', '’', '。', '，', ',', '、', '！', '!', '？', '?', '：', ':', '；', ';', '.', '-',
            '—',
        ])
        .to_string();
    format!("{cut}…")
}

/// 无模型兜底：首行有效文本；纯附件用文件名。
pub fn derive_title(text: &str, attachments: &[String]) -> String {
    let short = short_title(text);
    if !short.is_empty() {
        return short;
    }
    if let Some(first) = attachments.first() {
        let name = first.rsplit('/').next().unwrap_or(first);
        if !name.is_empty() {
            return name.chars().take(TITLE_LIMIT_RUNES).collect();
        }
    }
    let normalized = normalize_title(text);
    if !has_content(&normalized) {
        return String::new();
    }
    let runes: Vec<char> = normalized.chars().collect();
    if runes.len() <= TITLE_LIMIT_RUNES {
        return normalized;
    }
    let cut: String = runes[..TITLE_LIMIT_RUNES].iter().collect();
    format!(
        "{}…",
        cut.trim_end_matches([' ', '，', ',', '。', '；', ';', '：', ':'])
    )
}

pub struct AutoTitleInput<'a> {
    pub client: &'a dyn Client,
    pub model_id: &'a str,
    pub thread_id: &'a str,
    pub first_text: &'a str,
    pub first_attachments: &'a [craft_model::types::Attachment],
}

/// 生成并写入自动标题（并发防重复由调用方 pending 表保证；条件更新防覆盖手动改名）。
/// 同步执行（调用方放阻塞线程）；短名直用不调模型。
pub fn auto_title(db: &Db, input: AutoTitleInput<'_>) {
    let short = short_title(input.first_text);
    if !short.is_empty() {
        let _ = auto_title_thread(&db.lock_conn(), input.thread_id, &short, "新对话");
        return;
    }
    let attachment_text = input
        .first_attachments
        .iter()
        .filter(|a| a.skipped.is_empty() && !a.text.is_empty())
        .map(|a| a.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let combined: String = format!("{} {}", input.first_text, attachment_text).trim().chars().take(1500).collect();
    let prompt = if combined.is_empty() { String::new() } else { format!("用户：{combined}") };

    let req = craft_model::types::Request {
        model: input.model_id.to_string(),
        mode: "code".into(),
        system: single_system(TITLE_SYSTEM_PROMPT),
        messages: vec![craft_model::types::Message::new("user", prompt)],
        tools: vec![],
        max_tokens: TITLE_MAX_TOKENS,
    };
    let mut generated = String::new();
    // sidecar 流消费（无整体超时依赖：客户端层有 600s 上限；失败静默放弃）
    {
        let stream = input.client.stream(&req);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok();
        if let Some(rt) = rt {
            rt.block_on(async {
                let mut stream = stream;
                use futures_util::StreamExt;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(chunk) => {
                            if let Some(text) = chunk.text {
                                generated.push_str(&text);
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    }

    if !generated.trim().is_empty() {
        let title = sanitize_model_title(&generated);
        if !title.is_empty() {
            let _ = auto_title_thread(&db.lock_conn(), input.thread_id, &title, "新对话");
        }
        // sidecar 用量记账在 Runner 层（title 请求的 usage 由流末尾 chunk 提供——此处省略：
        // TS 版 sidecar 也不覆盖上下文水位，用量记账为 best-effort）
        let _ = add_usage;
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 标题清洗规则() {
        assert_eq!(normalize_title("# 你好，世界！"), "你好，世界");
        assert_eq!(normalize_title("  多   行\n第二行"), "多 行");
        assert_eq!(short_title("帮我写一个测试"), "帮我写一个测试");
        assert_eq!(short_title(&"很长的标题".repeat(10)), "");
        assert_eq!(sanitize_model_title("标题：聊天记录分析"), "聊天记录分析");
        assert!(sanitize_model_title(&"长".repeat(30)).ends_with('…'));
        // 与 TS 版一致：附件兜底取文件名部分
        assert_eq!(derive_title("", &["src/main.rs".into()]), "main.rs");
        // 尾部 '#' 不在剥离列表：与 TS 版一致返回原串（含有效中文即视为标题）
        assert_eq!(derive_title("纯符号！！！ ###", &[]), "纯符号！！！ ###");
    }
}
