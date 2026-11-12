//! 会话自动命名：首条消息可短名直用时直接用；否则调模型起标题（sidecar 调用，
//! 无输出或输出不可用时回退本地兜底；sidecar 用量当前为 best-effort，未记账）。

use craft_model::types::{single_system, Client};
use craft_store::db::Db;
use craft_store::records::add_usage;
use craft_store::threads::auto_title_thread;

// 思考模型的思考内容与正文共享 max_tokens 配额：过小时思考耗尽配额、正文为空
// （实测 glm-5.3-flashx 在 128 下 1.8s 返回 0 字符，与输入预测的修正同因；
// 上限与压缩摘要的 1024 同级）
const TITLE_MAX_TOKENS: i64 = 1024;
const TITLE_LIMIT_RUNES: usize = 20;

const TITLE_SYSTEM_PROMPT: &str = "你是一个标题生成器。根据输入的对话内容生成一个简短的中文标题：只输出标题本身，不要引号、句号或任何解释，不超过 20 个字。不要回答、续写或评论对话内容。";

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

/// 是否含有效内容字符（ASCII 字母数字或中文），过滤纯符号/空白串。
pub(crate) fn has_content(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 拒收对话式输出：弱指令遵循的模型可能无视任务指令，把请求当对话
/// 开始自我介绍或回答用户问题（实测 DeepSeek 系输出「你好！我是基于…」）。
/// 保守特征：问候语开头或自我介绍短语；正常标题/建议是名词短语，几乎不会命中。
pub(crate) fn is_conversational_output(s: &str) -> bool {
    let lower = s.to_lowercase();
    if ["你好", "您好", "hello"].iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    const SELF_INTRO_PHRASES: [&str; 5] =
        ["我是基于", "我是一个", "我是由", "developed by", "trained by"];
    SELF_INTRO_PHRASES.iter().any(|p| lower.contains(p))
}

/// 思考模型的开标签形态（网关把思考内容以内联标签混进正文时出现）。
/// 互不为前缀（`<think>` 与 `<thinking>` 在 `>`/`i` 处分叉），匹配顺序无关紧要；
/// 列表保持长标签在前仅是稳定的书写约定。
const THINKING_OPEN_TAGS: [&str; 3] = ["<reasoning>", "<thinking>", "<think>"];

/// ASCII 大小写不敏感的子串查找：标签均为 ASCII，逐字节比较即可，
/// 避免 `to_lowercase()` 对个别 Unicode 字符改变字节长度的索引错位问题。
fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len()).find(|&i| {
        h[i..i + n.len()]
            .iter()
            .zip(n.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// 剥离开头的连续思考块（`<think>…</think>` 等，大小写不敏感），返回剩余文本。
/// 开标签未闭合视为整段输出都是思考内容，返回 None（调用方按「不可用作标题」处理）。
/// 只剥开头：正文中途出现的同名标签不碰，避免误删恰好含此字样的正常标题。
pub(crate) fn strip_leading_thinking_blocks(raw: &str) -> Option<String> {
    let mut rest = raw;
    loop {
        let start = rest.trim_start();
        let open = THINKING_OPEN_TAGS.iter().find(|tag| {
            start.len() >= tag.len()
                && start.as_bytes()[..tag.len()]
                    .iter()
                    .zip(tag.as_bytes())
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
        });
        let Some(open) = open else { break };
        let close = format!("</{}", &open[1..]);
        let pos = find_ignore_ascii_case(start, &close)?;
        rest = &start[pos + close.len()..];
    }
    Some(rest.to_string())
}

/// 首条消息可直用作短标题时返回规范化结果（不超过 20 字且含有效字符）；否则返回空串。
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

/// 模型标题清洗：剥开头的思考块（`<think>…</think>` 等——sidecar 请求不带思考参数，
/// 网关按模型默认开启思考且可能把思考内容以内联标签混进正文，实测 glm-5.3-flash
/// 产出「<thinking>The user w…」落库成标题）；未闭合的思考块视为整段思考、拒收。
/// 剥「标题：/Title:」前缀；拒收对话式输出（自我介绍/回答）；
/// 须含字母或数字；超长截断加省略号。返回空串表示输出不可用作标题。
pub fn sanitize_model_title(raw: &str) -> String {
    let Some(stripped) = strip_leading_thinking_blocks(raw) else {
        return String::new();
    };
    let mut s = normalize_title(&stripped);
    for prefix in ["标题：", "标题:", "Title:", "title:", "Title：", "title："] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            s = stripped.trim().to_string();
            break;
        }
    }
    if !has_content(&s) || is_conversational_output(&s) {
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

/// 无模型兜底：仅从文本生成（短名直用 → 规范化截断）。附件名不参与自动命名：
/// 落盘名可能是 att_<hex> 内部 ID（粘贴附件清洗后无可读部分），显示为标题不可读。
pub fn derive_title(text: &str) -> String {
    let short = short_title(text);
    if !short.is_empty() {
        return short;
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

/// 自动命名的输入参数包：模型客户端 + 会话标识 + 首条消息内容与附件。
pub struct AutoTitleInput<'a> {
    pub client: &'a dyn Client,
    pub model_id: &'a str,
    pub thread_id: &'a str,
    pub first_text: &'a str,
    pub first_attachments: &'a [craft_model::types::Attachment],
}

/// 本地兜底标题：用户消息截断（附件名不参与自动命名，理由见 derive_title）。
fn fallback_title(input: &AutoTitleInput<'_>) -> String {
    derive_title(input.first_text)
}

/// 生成并写入自动标题（并发防重复由调用方 pending 表保证；条件更新防覆盖手动改名）。
/// 同步执行（调用方放阻塞线程）；短名直用不调模型；模型无输出或输出被判为
/// 对话式（自我介绍/回答）时回退本地兜底，保证不写入明显不是标题的内容。
///
/// 返回是否真的改写了标题（条件更新只在仍是自动命名会话时生效）：调用方据此决定
/// 要不要广播 thread.updated——标题没变就没必要让界面重拉会话列表。
pub fn auto_title(db: &Db, input: AutoTitleInput<'_>) -> bool {
    let short = short_title(input.first_text);
    if !short.is_empty() {
        return auto_title_thread(&db.lock_conn(), input.thread_id, &short).unwrap_or(false);
    }
    let attachment_text = input
        .first_attachments
        .iter()
        .filter(|a| a.skipped.is_empty() && !a.text.is_empty())
        .map(|a| a.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    let combined: String = format!("{} {}", input.first_text, attachment_text).trim().chars().take(1500).collect();
    if combined.is_empty() {
        // 无有效输入内容：空输入易诱导模型自我介绍，直接走本地兜底不调模型
        let fallback = fallback_title(&input);
        if !fallback.is_empty() {
            return auto_title_thread(&db.lock_conn(), input.thread_id, &fallback).unwrap_or(false);
        }
        return false;
    }
    let prompt = format!("请为下面的对话生成简短标题（不要回答对话内容）：\n\n用户：{combined}");

    let req = craft_model::types::Request {
        model: input.model_id.to_string(),
        mode: "code".into(),
        system: single_system(TITLE_SYSTEM_PROMPT),
        messages: vec![craft_model::types::Message::new("user", prompt)],
        tools: vec![],
        max_tokens: TITLE_MAX_TOKENS,
        // sidecar 起标题不需要思考档位
        reasoning_kind: String::new(),
        reasoning_level: String::new(),
        // sidecar 同样携带会话 ID（自定义请求头 {sessionId} 替换，网关路由归并）
        session_id: input.thread_id.to_string(),
        // sidecar 不需要图片
        supports_vision: false,
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
            return auto_title_thread(&db.lock_conn(), input.thread_id, &title).unwrap_or(false);
        }
    }
    // 模型无输出或输出不可用作标题：回退本地兜底
    let fallback = fallback_title(&input);
    if !fallback.is_empty() {
        return auto_title_thread(&db.lock_conn(), input.thread_id, &fallback).unwrap_or(false);
    }
    // sidecar 用量记账在 Runner 层（title 请求的 usage 由流末尾 chunk 提供——此处省略：
    // TS 版 sidecar 也不覆盖上下文水位，用量记账为 best-effort）
    let _ = add_usage;
    false
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_sanitization_rules() {
        assert_eq!(normalize_title("# 你好，世界！"), "你好，世界");
        assert_eq!(normalize_title("  多   行\n第二行"), "多 行");
        assert_eq!(short_title("帮我写一个测试"), "帮我写一个测试");
        assert_eq!(short_title(&"很长的标题".repeat(10)), "");
        assert_eq!(sanitize_model_title("标题：聊天记录分析"), "聊天记录分析");
        assert!(sanitize_model_title(&"长".repeat(30)).ends_with('…'));
        // 附件名不参与自动命名：无文本时兜底为空（不产出 att_ 内部 ID 标题）
        assert_eq!(derive_title(""), "");
        // 尾部 '#' 不在剥离列表：与 TS 版一致返回原串（含有效中文即视为标题）
        assert_eq!(derive_title("纯符号！！！ ###"), "纯符号！！！ ###");
    }

    #[test]
    fn conversational_model_output_rejected() {
        // 实测故障样本：弱指令模型把标题请求当对话，输出自我介绍 → 拒收返回空
        assert_eq!(sanitize_model_title("你好！我是基于深度求索公司开发的DeepSeek模型。"), "");
        assert_eq!(sanitize_model_title("我是一个智能助手"), "");
        assert_eq!(sanitize_model_title("Developed by DeepSeek"), "");
        // 「标题：」前缀剥掉后仍是对话式输出 → 同样拒收
        assert_eq!(sanitize_model_title("标题：你好，我是AI助手"), "");
        // 正常标题不受影响
        assert_eq!(sanitize_model_title("指令文件与工具说明"), "指令文件与工具说明");
    }

    #[test]
    fn strips_leading_thinking_block_and_keeps_real_title() {
        // 闭合思考块剥掉后取正文标题；大小写不敏感
        assert_eq!(sanitize_model_title("<think>让我想想</think>分析会话错误"), "分析会话错误");
        assert_eq!(sanitize_model_title("<THINK>hmm</think>API 设计评审"), "API 设计评审");
        assert_eq!(sanitize_model_title("<Thinking>…</THINKING>指令文件与工具说明"), "指令文件与工具说明");
        // 连续多个思考块全部剥掉
        assert_eq!(sanitize_model_title("<think>a</think><reasoning>b</reasoning>依赖注入讲解"), "依赖注入讲解");
    }

    #[test]
    fn unclosed_thinking_block_rejected_entirely() {
        // 实测故障样本：glm-5.3-flash 标题 sidecar 输出未闭合 <thinking> 块，
        // 过去落库成标题「<thinking>The user w…」；现应拒收（auto_title 回退本地兜底）
        assert_eq!(sanitize_model_title("<thinking>The user is reporting that session just had an error"), "");
        assert_eq!(sanitize_model_title("<think>"), "");
        assert_eq!(sanitize_model_title("<think>只有思考没有正文"), "");
    }

    #[test]
    fn thinking_tag_mid_text_not_stripped() {
        // 只剥开头的思考块：非思考标签与正文中途的同名标签不碰
        // （21 字符超 20 字上限，按既有规则截断加省略号）
        assert_eq!(sanitize_model_title("<task>plan</task>迁移步骤"), "<task>plan</task>迁移步…");
        assert_eq!(sanitize_model_title("详解 think 标签"), "详解 think 标签");
    }

    #[test]
    fn fallback_uses_first_text_when_model_output_invalid() {
        // 模型输出被拒收 → 兜底走 derive_title 取用户消息截断（本故障案例的期望修复行为）
        let fallback = derive_title("你遵循了哪些文件的指令？以及有哪些工具可用？");
        assert!(fallback.starts_with("你遵循了哪些文件的指令"));
        assert!(fallback.ends_with('…'));
    }
}
