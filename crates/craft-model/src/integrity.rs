//! 工具调用配对完整性（`crates/craft-model/src/integrity.rs` 模块头）。
//!
//! 交给提供商的消息序列必须满足的不变量：
//! - 每条带工具调用的 assistant 消息，其后**紧邻**的一组消息必须恰好是该消息全部
//!   `tool_call_id` 对应的工具结果，一条不少，且不含其他角色的消息；
//! - 工具结果消息的 `tool_call_id` 必须能在其之前最近的、带工具调用的 assistant
//!   消息里找到对应项。
//!
//! 违反不变量的历史（中止的 Run、历史注入顺序疏忽、压缩切分错位等）会让 OpenAI 系
//! 提供商直接拒绝整条请求（400：tool_calls 后必须紧跟全部 tool 结果），且此后每次
//! 请求都在同一位置失败——一次性污染、永久锁死。`repair_tool_call_integrity` 在把
//! 历史交给模型之前整理出合规序列，让已污染的会话恢复可用。
//!
//! 这是纯函数：不发起网络请求、不读盘、不改消息正文与调用 id，只做增删与重排；
//! 对已合规的序列幂等（输出与输入逐条相等、修复清单为空）。调用方为 Run 循环的
//! 请求构造出口与历史重建（两个入口共用同一份逻辑，不在协议适配器里各写一份）。

use std::collections::HashSet;

use crate::types::{Message, ROLE_ASSISTANT, ROLE_TOOL, ROLE_USER};

/// 缺失工具结果的占位正文：说明结果缺失、由系统补齐、不可作为结论依据。
/// 措辞中性——不写「调用失败」，避免模型把占位误读为执行出错而重复发起同一工具。
///
/// 为什么是「保留调用 + 补占位」而不是「删掉这条 tool_calls 重来」：删调用等于抹掉
/// 模型自己的动作记录，它会以为这项工具从没发起过，转而在下一轮重复执行同一动作
/// （写文件类工具就是重复写）；补一条中性占位既恢复配对合法性，又保留「这件事发生过」
/// 的事实，模型可自行决定要不要重取。
pub const MISSING_TOOL_RESULT_PLACEHOLDER: &str = "【系统补齐】这条工具调用的结果在会话历史中缺失，无法恢复原始输出。此占位由系统自动补齐以保持消息配对，不代表工具执行成功或失败，请勿将其作为结论依据；如需该信息请重新调用工具获取。";

/// 无主工具结果降级为用户文本时附加的来源说明行。
pub const ORPHAN_TOOL_RESULT_SOURCE_LINE: &str =
    "【系统整理】下面这段内容原本是一条工具结果，但在会话历史中找不到对应的工具调用，已降级为普通文本保留：";

/// 修复种类（会话日志 `history_integrity_repaired` 记录用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityFixKind {
    /// 某个工具调用的结果缺失，在该组结果末尾补占位
    PaddedMissingToolResult,
    /// 夹在调用与结果之间的消息移到全部结果之后（消息本体不丢，只挪位置）
    MovedSandwichedMessage,
    /// 无主工具结果（找不到对应调用）降级为用户文本，原输出保留
    DemotedOrphanToolResult,
}

impl IntegrityFixKind {
    /// 日志字段用短标识。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PaddedMissingToolResult => "pad_missing_result",
            Self::MovedSandwichedMessage => "move_sandwiched",
            Self::DemotedOrphanToolResult => "demote_orphan_result",
        }
    }
}

/// 单处修复记录：修复涉及的消息在**输出序列**中的下标与修复种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityFix {
    pub index: usize,
    pub kind: IntegrityFixKind,
}

/// 整理结果：整理后的消息序列 + 修复清单 + 来源映射。
pub struct IntegrityReport {
    pub messages: Vec<Message>,
    /// 每条输出消息的来源：`Some(输入下标)`；`None` = 本次新增（补占位）。
    /// 供历史重建把「模型消息 → 来源事件 seq」的平行数组一并重排。
    pub origins: Vec<Option<usize>>,
    pub fixes: Vec<IntegrityFix>,
}

/// 整理消息序列，使工具调用配对不变量成立。
///
/// 以「调用组」为单位处理：一条带工具调用的 assistant 消息，加上到下一个带调用的
/// assistant 之前（或序列尾）的全部消息。组内按固定顺序修复：
/// 1. 合法工具结果原序紧跟调用消息；
/// 2. 缺失的调用按声明顺序补占位工具消息；
/// 3. 其余消息（夹层消息 + 无主工具结果）按原相对顺序排在全部结果之后，
///    其中无主工具结果降级为用户文本（原输出保留 + 来源说明行）。
///
/// 对已合规序列幂等：夹层位判定保证「原本就在全部结果之后」的消息不被记为移动，
/// 输出与输入逐条相等、修复清单为空。
pub fn repair_tool_call_integrity(messages: Vec<Message>) -> IntegrityReport {
    let n = messages.len();
    let mut out: Vec<Message> = Vec::with_capacity(n);
    let mut origins: Vec<Option<usize>> = Vec::with_capacity(n);
    let mut fixes: Vec<IntegrityFix> = Vec::new();

    let mut i = 0usize;
    while i < n {
        if messages[i].role != ROLE_ASSISTANT || messages[i].tool_calls.is_empty() {
            // 不带调用的消息原样通过；唯一的例外是出现在第一个调用组之前的工具结果——
            // 它前面没有任何带调用的 assistant，必属无主，降级保留原文。
            if messages[i].role == ROLE_TOOL {
                demote_orphan(&messages[i], &mut out, &mut origins, &mut fixes, i);
            } else {
                out.push(messages[i].clone());
                origins.push(Some(i));
            }
            i += 1;
            continue;
        }

        // 调用组起点
        let call_start = i;
        let call_ids: Vec<&str> = messages[call_start].tool_calls.iter().map(|c| c.id.as_str()).collect();
        out.push(messages[call_start].clone());
        origins.push(Some(call_start));
        i += 1;

        // 组边界：到下一个带调用的 assistant 之前（或序列尾）
        let mut group_end = i;
        while group_end < n
            && !(messages[group_end].role == ROLE_ASSISTANT && !messages[group_end].tool_calls.is_empty())
        {
            group_end += 1;
        }

        // 组内分区：合法工具结果与「尾部」（夹层消息 + 无主结果），均保持原相对顺序。
        // 同一 id 只认第一次出现的结果，之后的重复视为无主（结果必须恰好一条对一调用）。
        let mut seen: HashSet<&str> = HashSet::new();
        let mut results: Vec<usize> = Vec::new();
        let mut tail: Vec<usize> = Vec::new();
        for (offset, m) in messages[i..group_end].iter().enumerate() {
            let k = i + offset;
            if m.role == ROLE_TOOL && call_ids.contains(&m.tool_call_id.as_str()) && seen.insert(m.tool_call_id.as_str()) {
                results.push(k);
            } else {
                tail.push(k);
            }
        }

        // 1) 合法结果原序输出
        for k in &results {
            out.push(messages[*k].clone());
            origins.push(Some(*k));
        }
        // 2) 缺失结果按调用声明顺序补占位（补在夹层之前，保证占位也是「紧跟调用」）
        for call in &messages[call_start].tool_calls {
            if !seen.contains(call.id.as_str()) {
                let mut tool_msg = Message::new(ROLE_TOOL, MISSING_TOOL_RESULT_PLACEHOLDER);
                tool_msg.tool_call_id = call.id.clone();
                out.push(tool_msg);
                origins.push(None);
                fixes.push(IntegrityFix {
                    index: out.len() - 1,
                    kind: IntegrityFixKind::PaddedMissingToolResult,
                });
            }
        }
        // 3) 尾部原序输出；无主结果降级为用户文本；夹层移动判定见下
        // 「被移动」= 消息原本位于最后一条合法结果之前；本组没有任何合法结果时
        // （全部缺失、必有占位插入），尾部消息一律从「调用之后」挪到了「占位之后」，均算移动。
        let last_result = results.last().copied();
        for k in &tail {
            let m = &messages[*k];
            if m.role == ROLE_TOOL {
                demote_orphan(m, &mut out, &mut origins, &mut fixes, *k);
            } else {
                out.push(m.clone());
                origins.push(Some(*k));
                let moved = match last_result {
                    Some(lr) => *k < lr,
                    None => true,
                };
                if moved {
                    fixes.push(IntegrityFix {
                        index: out.len() - 1,
                        kind: IntegrityFixKind::MovedSandwichedMessage,
                    });
                }
            }
        }
        i = group_end;
    }

    IntegrityReport { messages: out, origins, fixes }
}

/// 无主工具结果降级为用户文本：原输出保留 + 来源说明行；附件与消息 id 原样带过。
fn demote_orphan(
    m: &Message,
    out: &mut Vec<Message>,
    origins: &mut Vec<Option<usize>>,
    fixes: &mut Vec<IntegrityFix>,
    source_index: usize,
) {
    let mut demoted = Message::new(ROLE_USER, format!("{ORPHAN_TOOL_RESULT_SOURCE_LINE}\n{}", m.content));
    demoted.attachments = m.attachments.clone();
    demoted.id = m.id.clone();
    out.push(demoted);
    origins.push(Some(source_index));
    fixes.push(IntegrityFix {
        index: out.len() - 1,
        kind: IntegrityFixKind::DemotedOrphanToolResult,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;

    fn call(id: &str) -> ToolCall {
        ToolCall { id: id.into(), name: "fs_read".into(), args: serde_json::json!({}) }
    }

    fn calling_assistant(id: &str) -> Message {
        let mut m = Message::new(ROLE_ASSISTANT, "");
        m.tool_calls = vec![call(id)];
        m
    }

    fn tool_result(id: &str, output: &str) -> Message {
        let mut m = Message::new(ROLE_TOOL, output);
        m.tool_call_id = id.into();
        m
    }

    fn fix_kinds(report: &IntegrityReport) -> Vec<IntegrityFixKind> {
        report.fixes.iter().map(|f| f.kind).collect()
    }

    #[test]
    fn compliant_sequence_passes_through_unchanged_with_no_fixes() {
        // user → assistant(调用) → tool(结果) → user → assistant(普通) 的完整合规序列
        let input = vec![
            Message::new(ROLE_USER, "帮我看看"),
            calling_assistant("c1"),
            tool_result("c1", "内容"),
            Message::new(ROLE_USER, "谢谢，继续"),
            Message::new(ROLE_ASSISTANT, "好的"),
        ];
        let report = repair_tool_call_integrity(input.clone());
        assert_eq!(report.messages, input, "合规序列输出应与输入逐条相等");
        assert!(report.fixes.is_empty(), "合规序列不应产生修复记录");
        assert_eq!(report.origins, vec![Some(0), Some(1), Some(2), Some(3), Some(4)]);
        // 幂等：对输出再跑一次，结果不变、清单仍为空
        let again = repair_tool_call_integrity(report.messages);
        assert_eq!(again.messages, input);
        assert!(again.fixes.is_empty());
    }

    #[test]
    fn missing_tool_result_is_padded_with_placeholder_per_call() {
        // 两个调用只回了 c1：补 c2 的占位，id 与调用声明一致
        let mut assistant = Message::new(ROLE_ASSISTANT, "");
        assistant.tool_calls = vec![call("c1"), call("c2")];
        let input = vec![Message::new(ROLE_USER, "查两处"), assistant, tool_result("c1", "第一处内容")];
        let report = repair_tool_call_integrity(input);
        assert_eq!(report.messages.len(), 4);
        assert_eq!(report.messages[3].role, ROLE_TOOL);
        assert_eq!(report.messages[3].tool_call_id, "c2");
        assert!(report.messages[3].content.contains("缺失"), "占位应说明结果缺失");
        assert!(report.messages[3].content.contains("系统"), "占位应说明由系统补齐");
        assert!(report.messages[3].content.contains("结论依据"), "占位应声明不可作为结论依据");
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::PaddedMissingToolResult]);
        assert_eq!(report.fixes[0].index, 3);
        // 占位消息在来源映射中无来源
        assert_eq!(report.origins[3], None);
    }

    #[test]
    fn sandwiched_user_message_moves_after_all_results() {
        // 本次报错的直接形状：assistant(调用) → user(反馈) → tool(结果)
        // 修复后反馈挪到结果之后，内容原样保留
        let input = vec![
            Message::new(ROLE_USER, "做个计划"),
            calling_assistant("c1"),
            Message::new(ROLE_USER, "第一步拆细一点"),
            tool_result("c1", "用户没有批准这份计划"),
        ];
        let report = repair_tool_call_integrity(input);
        assert_eq!(report.messages.len(), 4);
        assert_eq!(report.messages[2].role, ROLE_TOOL);
        assert_eq!(report.messages[2].tool_call_id, "c1");
        assert_eq!(report.messages[3].role, ROLE_USER);
        assert_eq!(report.messages[3].content, "第一步拆细一点", "夹层消息内容不能被改写或丢弃");
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::MovedSandwichedMessage]);
        assert_eq!(report.fixes[0].index, 3);
        // 修复后重跑：序列合规、清单为空（幂等）
        let again = repair_tool_call_integrity(report.messages);
        assert!(again.fixes.is_empty());
    }

    #[test]
    fn partially_missing_batch_pads_only_missing_calls() {
        // 三调用缺 c2 与 c3 的结果：只补缺失的两条，已回的 c1 不动
        let mut assistant = Message::new(ROLE_ASSISTANT, "");
        assistant.tool_calls = vec![call("c1"), call("c2"), call("c3")];
        let input = vec![assistant, tool_result("c1", "A")];
        let report = repair_tool_call_integrity(input);
        let ids: Vec<&str> = report.messages[1..].iter().map(|m| m.tool_call_id.as_str()).collect();
        assert_eq!(ids, vec!["c1", "c2", "c3"]);
        assert_eq!(report.messages[2].content, MISSING_TOOL_RESULT_PLACEHOLDER);
        assert_eq!(report.messages[3].content, MISSING_TOOL_RESULT_PLACEHOLDER);
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::PaddedMissingToolResult, IntegrityFixKind::PaddedMissingToolResult]);
    }

    #[test]
    fn orphan_tool_result_demotes_to_user_text_keeps_output() {
        // 无主结果：前面没有带调用的 assistant。降级为用户文本，原输出保留 + 来源说明
        let input = vec![Message::new(ROLE_USER, "你好"), tool_result("c1", "孤儿输出")];
        let report = repair_tool_call_integrity(input);
        assert_eq!(report.messages.len(), 2);
        assert_eq!(report.messages[1].role, ROLE_USER);
        assert!(report.messages[1].content.contains(ORPHAN_TOOL_RESULT_SOURCE_LINE));
        assert!(report.messages[1].content.contains("孤儿输出"), "原输出必须保留");
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::DemotedOrphanToolResult]);
        // 幂等：降级后的序列重跑不再产生修复
        let again = repair_tool_call_integrity(report.messages);
        assert!(again.fixes.is_empty());
    }

    #[test]
    fn orphan_tool_result_before_any_call_group_also_demotes() {
        // 序列开头的孤立工具结果（前面连用户消息都可有可无）同样降级
        let input = vec![tool_result("c1", "开头的孤儿"), Message::new(ROLE_USER, "你好")];
        let report = repair_tool_call_integrity(input);
        assert_eq!(report.messages[0].role, ROLE_USER);
        assert!(report.messages[0].content.contains("开头的孤儿"));
        assert_eq!(report.messages[1].content, "你好");
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::DemotedOrphanToolResult]);
    }

    #[test]
    fn duplicate_result_for_same_call_demotes_the_second() {
        // 同一调用 id 出现两条结果：第一条合法，第二条按无主降级（恰好一对一）
        let input = vec![calling_assistant("c1"), tool_result("c1", "第一条"), tool_result("c1", "第二条")];
        let report = repair_tool_call_integrity(input);
        assert_eq!(report.messages[1].role, ROLE_TOOL);
        assert_eq!(report.messages[2].role, ROLE_USER);
        assert!(report.messages[2].content.contains("第二条"));
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::DemotedOrphanToolResult]);
    }

    #[test]
    fn empty_and_plain_sequences_are_untouched() {
        // 空序列
        let empty = repair_tool_call_integrity(Vec::new());
        assert!(empty.messages.is_empty());
        assert!(empty.fixes.is_empty());
        // 只有用户消息
        let users = vec![Message::new(ROLE_USER, "一"), Message::new(ROLE_USER, "二")];
        let report = repair_tool_call_integrity(users.clone());
        assert_eq!(report.messages, users);
        assert!(report.fixes.is_empty());
        // 连续多条不带调用的 assistant
        let assistants = vec![
            Message::new(ROLE_ASSISTANT, "甲"),
            Message::new(ROLE_ASSISTANT, "乙"),
            Message::new(ROLE_ASSISTANT, "丙"),
        ];
        let report = repair_tool_call_integrity(assistants.clone());
        assert_eq!(report.messages, assistants);
        assert!(report.fixes.is_empty());
    }

    #[test]
    fn trailing_messages_after_complete_results_are_not_counted_as_moved() {
        // 结果集齐之后跟 user/普通 assistant：它们本来就在全部结果之后，不算夹层移动
        let input = vec![
            calling_assistant("c1"),
            tool_result("c1", "结果"),
            Message::new(ROLE_USER, "下一句"),
            Message::new(ROLE_ASSISTANT, "好的"),
        ];
        let report = repair_tool_call_integrity(input.clone());
        assert_eq!(report.messages, input, "集齐结果的组内尾部消息顺序应保持原位");
        assert!(report.fixes.is_empty(), "合规尾部不应产生修复记录");
    }

    #[test]
    fn multi_group_history_repairs_each_group_independently() {
        // 第一组缺结果 + 第二组合法：各组独立整理，互不影响
        let input = vec![
            calling_assistant("c1"),
            Message::new(ROLE_USER, "插话"),   // 第一组的夹层
            calling_assistant("c2"),
            tool_result("c2", "B"),
            Message::new(ROLE_USER, "正常下一条"),
        ];
        let report = repair_tool_call_integrity(input);
        // 第一组：assistant(c1) → 占位 → user(插话)；第二组原样
        assert_eq!(report.messages[1].role, ROLE_TOOL);
        assert_eq!(report.messages[1].tool_call_id, "c1");
        assert_eq!(report.messages[2].content, "插话");
        assert_eq!(report.messages[3].tool_calls[0].id, "c2");
        assert_eq!(report.messages[4].tool_call_id, "c2");
        assert_eq!(report.messages[5].content, "正常下一条");
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::PaddedMissingToolResult, IntegrityFixKind::MovedSandwichedMessage]);
        // 修复后幂等
        let again = repair_tool_call_integrity(report.messages);
        assert!(again.fixes.is_empty());
    }

    #[test]
    fn calling_assistant_sandwiched_before_next_group_gets_results_padded() {
        // 两个带调用的 assistant 相邻、前一个无结果：前组补占位，不吞掉后一个 assistant
        let input = vec![calling_assistant("c1"), calling_assistant("c2"), tool_result("c2", "B")];
        let report = repair_tool_call_integrity(input);
        assert_eq!(report.messages.len(), 4);
        assert_eq!(report.messages[1].role, ROLE_TOOL);
        assert_eq!(report.messages[1].tool_call_id, "c1");
        assert_eq!(report.messages[2].tool_calls[0].id, "c2");
        assert_eq!(report.messages[3].tool_call_id, "c2");
        assert_eq!(fix_kinds(&report), vec![IntegrityFixKind::PaddedMissingToolResult]);
    }
}
