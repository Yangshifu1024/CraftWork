//! 输入预测（docs/input-prediction/spec.md）：一轮任务正常结束后，基于当前会话
//! 上下文与该用户的历史输入，调会话当前模型预计算「下一句建议提示词」。
//! sidecar 语义与自动命名一致（失败静默、不阻断运行、用量 best-effort 省略）；
//! 预测是瞬态 UI 辅助，不落事件表。

use craft_model::types::{single_system, Client, Message};

/// 预测请求的 max_tokens 上限。正文只要一句话，但思考模型（服务端默认开思考、
/// reasoning 不占正文却共享配额）需要余量：128 会被思考耗尽导致正文为空
/// （实测 glm-5.3-flashx 1.8s 返回 0 字符），与压缩摘要的 1024 同级。
const PREDICTION_MAX_TOKENS: i64 = 1024;
/// 建议文本的字符上限（超出截断加省略号）
const PREDICTION_LIMIT_RUNES: usize = 50;
/// 纳入上下文的用户历史输入条数上限（取最近）
const MAX_USER_INPUTS: usize = 10;
/// 单条用户输入的字符上限
const MAX_INPUT_CHARS: usize = 500;
/// 纳入上下文的最近对话消息条数
const MAX_RECENT_MESSAGES: usize = 8;
/// 转写中单条消息的字符上限
const MAX_TRANSCRIPT_ITEM_CHARS: usize = 800;
/// 转写总字符上限
const MAX_TRANSCRIPT_CHARS: usize = 8000;

const PREDICTION_SYSTEM_PROMPT: &str = "你是对话续写预测器。根据给定的会话上下文，预测提问的用户最有可能发出的下一条提示词。只输出这一句提示词本身：不要引号、编号、前缀或任何解释；一句话，不超过 50 个字；跟随该用户的历史输入语言与措辞习惯；内容是对话的自然延续（继续未完成的工作、追问结果或下一步操作）。不要回答、续写或评论对话内容。";

/// 提取该会话的用户历史输入（输入习惯信号）：user 消息文本 trim 非空，取最近
/// MAX_USER_INPUTS 条，每条截 MAX_INPUT_CHARS。
pub fn user_inputs(history: &[Message]) -> Vec<String> {
    let mut items: Vec<String> = history
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.content.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let cut = items.len().saturating_sub(MAX_USER_INPUTS);
    items.drain(..cut);
    items
        .into_iter()
        .map(|s| s.chars().take(MAX_INPUT_CHARS).collect())
        .collect()
}

/// 最近对话转写（上下文信号）：带角色标签的单行文本，每条与总长均有截断
/// （转写方式借鉴 context.rs 的 transcript_of，但只保留可读正文、跳过工具链细节）。
pub fn recent_transcript(history: &[Message]) -> String {
    let start = history.len().saturating_sub(MAX_RECENT_MESSAGES);
    let mut lines: Vec<String> = Vec::new();
    for msg in &history[start..] {
        let content = msg.content.trim();
        if content.is_empty() {
            continue;
        }
        let content: String = content.chars().take(MAX_TRANSCRIPT_ITEM_CHARS).collect();
        let tag = match msg.role.as_str() {
            "user" => "【用户】",
            "assistant" => "【助手】",
            "tool" => "【工具结果】",
            _ => continue,
        };
        lines.push(format!("{tag}{content}"));
    }
    let mut joined = lines.join("\n");
    if joined.chars().count() > MAX_TRANSCRIPT_CHARS {
        joined = joined.chars().take(MAX_TRANSCRIPT_CHARS).collect();
    }
    joined
}

/// 剥一层成对包裹引号（中英常用配对）；无配对原样返回。
fn strip_quotes(s: &str) -> String {
    const PAIRS: [(&str, &str); 4] = [("\"", "\""), ("“", "”"), ("「", "」"), ("『", "』")];
    let t = s.trim();
    for (open, close) in PAIRS {
        if let Some(inner) = t.strip_prefix(open).and_then(|x| x.strip_suffix(close)) {
            return inner.trim().to_string();
        }
    }
    t.to_string()
}

/// 模型输出清洗：取首行 → 剥「建议：」类前缀与包裹引号 → 拒绝对话式输出与
/// 纯符号串 → 超长截断。返回 None 表示输出不可用作建议（等价于无预测）。
pub fn sanitize_prediction(raw: &str) -> Option<String> {
    let mut s = raw.split('\n').next().unwrap_or("").trim().to_string();
    for prefix in ["建议：", "建议:", "建议 ", "提示词：", "提示词:"] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            s = stripped.trim().to_string();
            break;
        }
    }
    s = strip_quotes(&s);
    if !crate::title::has_content(&s) || crate::title::is_conversational_output(&s) {
        return None;
    }
    if s.chars().count() > PREDICTION_LIMIT_RUNES {
        let mut cut: String = s.chars().take(PREDICTION_LIMIT_RUNES).collect();
        cut = cut.trim_end().to_string();
        s = format!("{cut}…");
    }
    Some(s)
}

/// 构造预测请求的用户提示词；无任何用户输入时返回 None（空上下文易诱导模型
/// 自我介绍，直接放弃不调模型——与自动命名的空输入守卫同思路）。
pub fn build_prompt(history: &[Message]) -> Option<String> {
    let inputs = user_inputs(history);
    if inputs.is_empty() {
        return None;
    }
    let inputs_block = inputs
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}. {s}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let transcript = recent_transcript(history);
    Some(format!(
        "【该用户的历史输入】\n{inputs_block}\n\n【最近对话】\n{transcript}\n\n请预测该用户最有可能发出的下一条提示词（只输出这一句提示词本身）。"
    ))
}

/// 生成输入预测（同步执行，调用方放阻塞线程）；无上下文、调用失败或输出无效
/// 均返回 None，调用方静默放弃。
pub fn generate_prediction(client: &dyn Client, model_id: &str, thread_id: &str, history: &[Message]) -> Option<String> {
    let prompt = build_prompt(history)?;

    let req = craft_model::types::Request {
        model: model_id.to_string(),
        mode: "code".into(),
        system: single_system(PREDICTION_SYSTEM_PROMPT),
        messages: vec![Message::new("user", prompt)],
        tools: vec![],
        max_tokens: PREDICTION_MAX_TOKENS,
        // sidecar 预测不需要思考档位
        reasoning_kind: String::new(),
        reasoning_level: String::new(),
        // sidecar 同样携带会话 ID（自定义请求头 {sessionId} 替换，网关路由归并）
        session_id: thread_id.to_string(),
    };
    let mut generated = String::new();
    // sidecar 流消费：失败静默放弃（客户端层有整体超时上限）
    {
        let stream = client.stream(&req);
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

    if generated.trim().is_empty() {
        return None;
    }
    sanitize_prediction(&generated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(content: &str) -> Message {
        Message::new("user", content)
    }

    fn assistant_msg(content: &str) -> Message {
        Message::new("assistant", content)
    }

    #[test]
    fn user_inputs_take_recent_and_truncate() {
        let history = vec![
            user_msg("第一条"),
            user_msg("   "), // 空白过滤
            assistant_msg("回答"),
            user_msg("第二条"),
        ];
        assert_eq!(user_inputs(&history), vec!["第一条".to_string(), "第二条".to_string()]);
        let long = "长".repeat(MAX_INPUT_CHARS + 10);
        let got = user_inputs(&[user_msg(&long)]);
        assert_eq!(got[0].chars().count(), MAX_INPUT_CHARS);
    }

    #[test]
    fn recent_transcript_tags_roles_and_limits() {
        let history = vec![user_msg("帮我跑测试"), assistant_msg("已通过"), Message::new("tool", "ok")];
        let t = recent_transcript(&history);
        assert!(t.contains("【用户】帮我跑测试"));
        assert!(t.contains("【助手】已通过"));
        assert!(t.contains("【工具结果】ok"));
        let long = "x".repeat(MAX_TRANSCRIPT_ITEM_CHARS + 5);
        let t = recent_transcript(&[user_msg(&long)]);
        assert!(t.chars().count() < MAX_TRANSCRIPT_ITEM_CHARS + 10);
    }

    #[test]
    fn sanitize_prediction_rules() {
        // 首行提取 + 建议前缀剥离 + 引号剥离
        assert_eq!(sanitize_prediction("建议：「先跑一遍完整测试」\n第二行忽略"), Some("先跑一遍完整测试".into()));
        assert_eq!(sanitize_prediction("\"继续修复剩余的报错\""), Some("继续修复剩余的报错".into()));
        // 对话式输出拒收（与标题生成同一套规则）
        assert_eq!(sanitize_prediction("你好！我是基于深度求索公司开发的DeepSeek模型。"), None);
        // 纯符号拒收
        assert_eq!(sanitize_prediction("。。。"), None);
        // 空输出拒收
        assert_eq!(sanitize_prediction(""), None);
        // 超长截断
        let long = "字".repeat(PREDICTION_LIMIT_RUNES + 5);
        let got = sanitize_prediction(&long).unwrap();
        assert!(got.chars().count() <= PREDICTION_LIMIT_RUNES + 1);
        assert!(got.ends_with('…'));
    }

    #[test]
    fn build_prompt_requires_user_input() {
        assert!(build_prompt(&[]).is_none());
        assert!(build_prompt(&[assistant_msg("只有助手消息")]).is_none());
        let prompt = build_prompt(&[user_msg("修复登录报错")]).unwrap();
        assert!(prompt.contains("修复登录报错"));
        assert!(prompt.contains("【最近对话】"));
    }
}
