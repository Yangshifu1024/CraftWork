//! events 表读写与快照重建。
//! 事件表是唯一事实源：消息与部件从事件行重建，不另存副本。
//! seq 分配与插入必须同事务（并发安全）；写入后顺带推进 threads.updated_at。

use rusqlite::Connection;
use serde_json::Value;

use crate::db::{now_millis, other_err, Db, StoreResult};
use crate::threads::get_thread;
use crate::view::to_thread_view;
use craft_core::contract::{ApprovalCard, ApprovalFact, MessageSnapshot, PartSnapshot, Snapshot, ThreadView};
use craft_core::ids::{id_prefix, new_id};

/// events 表一行的读取形态（payload 为 JSON 字符串，读取侧按需解析）。
#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: String,
    pub thread_id: String,
    pub message_id: String,
    pub run_id: Option<String>,
    pub seq: i64,
    pub kind: String,
    pub payload: String,
    pub created_at: i64,
}

/// 待写入的一条事件（写入入参；id/seq/created_at 由存储层分配）。
#[derive(Debug, Clone)]
pub struct EventRecord {
    pub thread_id: String,
    pub message_id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub payload: Value,
}

/// 在连接上追加一条事件：分配会话内单调 seq（MAX+1）→ 插入 → 顺带推进 threads.updated_at。
/// 前提：调用方必须持锁并在事务内调用（seq 分配与插入同事务，否则并发下 seq 会错重）；
/// 无事务的便捷入口用 record_event。返回 (事件 id, 分配的 seq)。
pub fn append_event(conn: &Connection, rec: &EventRecord) -> StoreResult<(String, i64)> {
    let seq: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 AS next FROM events WHERE thread_id = ?1",
            [&rec.thread_id],
            |row| row.get(0),
        )
        .map_err(other_err)?;
    let id = new_id(id_prefix::PART);
    conn.execute(
        "INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            rec.thread_id,
            rec.message_id,
            rec.run_id,
            seq,
            rec.kind,
            serde_json::to_string(&rec.payload).unwrap_or_else(|_| "{}".into()),
            now_millis(),
        ],
    )
    .map_err(other_err)?;
    conn.execute("UPDATE threads SET updated_at = ?1 WHERE id = ?2", rusqlite::params![now_millis(), rec.thread_id])
        .map_err(other_err)?;
    Ok((id, seq))
}

/// 落库入口（含事务）：Db 层的便捷函数。
pub fn record_event(db: &Db, rec: &EventRecord) -> StoreResult<(String, i64)> {
    db.transaction(|conn| append_event(conn, rec))
}

/// 全量读取会话事件（seq 升序）；快照重建与历史回放的基础查询。
pub fn list_events(conn: &Connection, thread_id: &str) -> StoreResult<Vec<EventRow>> {
    let mut stmt = conn
        .prepare("SELECT * FROM events WHERE thread_id = ?1 ORDER BY seq ASC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([thread_id], |row| {
            Ok(EventRow {
                id: row.get("id")?,
                thread_id: row.get("thread_id")?,
                message_id: row.get("message_id")?,
                run_id: row.get("run_id")?,
                seq: row.get("seq")?,
                kind: row.get("kind")?,
                payload: row.get("payload")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 最近一次上下文压缩的摘要：返回 (summary, untilSeq)；无记录或摘要为空返回 None。
pub fn latest_context_summary(conn: &Connection, thread_id: &str) -> StoreResult<Option<(String, i64)>> {
    let mut stmt = conn
        .prepare("SELECT payload, seq FROM events WHERE thread_id = ?1 AND kind = 'context_summary' ORDER BY seq DESC LIMIT 1")
        .map_err(other_err)?;
    let row = stmt
        .query_row([thread_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .ok();
    let Some((payload, seq)) = row else {
        return Ok(None);
    };
    let parsed: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    let summary = parsed.get("summary").and_then(Value::as_str).unwrap_or_default().to_string();
    if summary.is_empty() {
        return Ok(None);
    }
    let until_seq = parsed.get("untilSeq").and_then(Value::as_i64).unwrap_or(seq);
    Ok(Some((summary, until_seq)))
}

/// 解析触发消息的计划模式标记（append 时随 payload 落库，run 启动时按 trigger 还原）：
/// trigger_seq 有值读该事件（普通发送 / steer 接力传首条引导消息 seq），
/// None 读最新一条用户消息（regenerate 重跑上一轮）。缺事件或无标记一律 false。
pub fn plan_mode_for_trigger(conn: &Connection, thread_id: &str, trigger_seq: Option<i64>) -> StoreResult<bool> {
    let payload: Option<String> = match trigger_seq {
        Some(seq) => conn
            .query_row(
                "SELECT payload FROM events WHERE thread_id = ?1 AND seq = ?2 AND kind = 'user_message'",
                rusqlite::params![thread_id, seq],
                |row| row.get(0),
            )
            .ok(),
        None => conn
            .query_row(
                "SELECT payload FROM events WHERE thread_id = ?1 AND kind = 'user_message' ORDER BY seq DESC LIMIT 1",
                [thread_id],
                |row| row.get(0),
            )
            .ok(),
    };
    let Some(payload) = payload else {
        return Ok(false);
    };
    let parsed: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    Ok(parsed.get("planMode").and_then(Value::as_bool).unwrap_or(false))
}

// ---------- 快照重建 ----------

/// 快照重建跳过的事件 kind：run 生命周期与压缩元数据只服务内部逻辑，不产生可见部件。
const SKIPPED_KINDS: [&str; 3] = ["run_started", "run_finished", "context_summary"];

/// DB kind → 快照部件 kind（approval_resolved 不产生部件，回填用）。
fn snapshot_part_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "user_message" => Some("text"),
        "text" => Some("text"),
        "reasoning" => Some("reasoning"),
        "approval" => Some("approval"),
        "todo" => Some("todo"),
        "error" => Some("error"),
        "notice" => Some("notice"),
        "draft" => Some("draft"),
        "callout" => Some("callout"),
        "tool_call" => Some("tool"),
        _ => None,
    }
}

struct RunInfo {
    id: String,
    message_id: String,
    status: String,
    started_at: i64,
    finished_at: Option<i64>,
    trigger_seq: Option<i64>,
}

fn list_runs(conn: &Connection, thread_id: &str) -> StoreResult<Vec<RunInfo>> {
    let mut stmt = conn
        .prepare("SELECT id, message_id, status, started_at, finished_at, trigger_seq FROM runs WHERE thread_id = ?1 ORDER BY started_at ASC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([thread_id], |row| {
            Ok(RunInfo {
                id: row.get(0)?,
                message_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                finished_at: row.get(4)?,
                trigger_seq: row.get(5)?,
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// run 状态 → 消息状态映射（快照重建用）。
fn run_message_status(run_status: &str) -> &'static str {
    match run_status {
        "running" => "streaming",
        "error" => "error",
        "canceled" | "interrupted" => "incomplete",
        _ => "complete",
    }
}

/// 快照的待决定审批：以 approvals 表（decision IS NULL）为准，而非事件流。
/// 富文本字段（target/detail/facts/args）在表里没有列，重建时留空——卡片回显仅保证可操作。
fn pending_approvals(conn: &Connection, thread_id: &str) -> StoreResult<Vec<ApprovalCard>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, tool_call_id, tool, risk, requested_at FROM approvals WHERE thread_id = ?1 AND decision IS NULL ORDER BY requested_at ASC",
        )
        .map_err(other_err)?;
    let rows = stmt
        .query_map([thread_id], |row| {
            Ok(ApprovalCard {
                part_id: String::new(),
                approval_id: row.get::<_, String>(0)?,
                tool_call_id: row.get::<_, String>(1)?,
                tool: row.get::<_, String>(2)?,
                risk: row.get::<_, String>(3)?,
                title: "需要你的批准".into(),
                target: String::new(),
                detail: String::new(),
                facts: Vec::<ApprovalFact>::new(),
                args: serde_json::json!({}),
                allow_permanent: false,
                status: "waiting".into(),
                decision: None,
                scope: None,
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 快照重建：从事件行还原消息列表。
/// before_seq > 0 时只保留「最后部件 seq < before_seq」的消息（向前翻页）。
pub fn snapshot(conn: &Connection, thread_id: &str, limit: usize, before_seq: i64) -> StoreResult<Snapshot> {
    let thread_row = get_thread(conn, thread_id)?;
    let thread: ThreadView = to_thread_view(conn, &thread_row)?;
    let events = list_events(conn, thread_id)?;
    let runs = list_runs(conn, thread_id)?;

    // 按 messageId 归组
    let mut order: Vec<String> = Vec::new();
    let mut messages: std::collections::HashMap<String, MessageSnapshot> = std::collections::HashMap::new();
    // (partId, message_id, 部件下标) 定位 approval 部件（approval_resolved 事件跨消息回填）
    let mut approval_part_pos: Vec<(String, String, usize)> = Vec::new();

    fn ensure_message<'m>(
        order: &mut Vec<String>,
        messages: &'m mut std::collections::HashMap<String, MessageSnapshot>,
        message_id: &str,
        role: &str,
        created_at: i64,
    ) -> &'m mut MessageSnapshot {
        if !messages.contains_key(message_id) {
            order.push(message_id.to_string());
            messages.insert(
                message_id.to_string(),
                MessageSnapshot {
                    id: message_id.to_string(),
                    role: role.to_string(),
                    parts: Vec::new(),
                    status: "complete".into(),
                    seq: 0,
                    created_at,
                    work_duration_ms: None,
                    finished_at: None,
                },
            );
        }
        messages.get_mut(message_id).unwrap()
    }

    for ev in &events {
        if SKIPPED_KINDS.contains(&ev.kind.as_str()) {
            continue;
        }

        if ev.kind == "approval_resolved" {
            // 回填对应 approval 部件的状态
            let payload: Value = serde_json::from_str(&ev.payload).unwrap_or(Value::Null);
            let part_id = payload.get("partId").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(part) = approval_part_pos
                .iter()
                .find(|(pid, _, _)| *pid == part_id)
                .and_then(|(pid, message_id, part_idx)| {
                    let _ = pid;
                    messages.get_mut(message_id).and_then(|msg| msg.parts.get_mut(*part_idx))
                })
            {
                let status = payload.get("status").and_then(Value::as_str).unwrap_or("canceled");
                part.payload["status"] = Value::String(status.to_string());
                part.payload["decision"] =
                    Value::String(payload.get("decision").and_then(Value::as_str).unwrap_or_default().into());
                part.payload["scope"] =
                    Value::String(payload.get("scope").and_then(Value::as_str).unwrap_or_default().into());
            }
            continue;
        }

        if ev.kind == "steer_delivered" {
            // 引导投递回执：摘除对应用户消息部件的 via（「待送达」标记）。
            // 事件行的 message_id 即引导消息 id，必然先于本事件落库（seq 有序）；
            // 不产生部件，后续输出的新 assistant 消息由事件表 message_id 自然分组。
            if let Some(msg) = messages.get_mut(&ev.message_id) {
                for part in &mut msg.parts {
                    part.payload.as_object_mut().map(|obj| obj.remove("via"));
                }
            }
            continue;
        }

        let Some(kind) = snapshot_part_kind(&ev.kind) else {
            continue;
        };

        let mut payload: Value = serde_json::from_str(&ev.payload).unwrap_or(Value::Null);
        if payload.is_null() {
            payload = serde_json::json!({});
        }
        if ev.kind == "user_message" {
            // user_message 只保留 text 与 attachments（via=steer 透传：前端「待送达」标记；
            // planMode 透传：计划 run 完成后前端展示「批准并执行」）
            let via = payload.get("via").cloned();
            let plan_mode = payload.get("planMode").cloned();
            payload = serde_json::json!({
                "text": payload.get("text").cloned().unwrap_or(Value::String(String::new())),
                "attachments": payload.get("attachments").cloned().unwrap_or(Value::Array(vec![])),
            });
            if let Some(via) = via {
                payload["via"] = via;
            }
            if let Some(plan_mode) = plan_mode {
                payload["planMode"] = plan_mode;
            }
        }

        let role = if ev.kind == "user_message" { "user" } else { "assistant" };
        let msg = ensure_message(&mut order, &mut messages, &ev.message_id, role, ev.created_at);
        // part.id 优先取 payload.partId：渲染层按 partId 归属后续增量事件（text.delta 等），
        // 用事件行 id 会导致快照恢复后的流式事件永远找不到归属消息。
        // 审批部件加 approval_ 前缀：审批请求复用工具调用的 partId，同一消息内两个部件
        // 同 id 会撞 key；前端事件流（app.ts）同规则，快照与实时流才能在同一键上合并。
        let base_part_id = payload
            .get("partId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ev.id.clone());
        let part_id = if kind == "approval" {
            format!("approval_{base_part_id}")
        } else {
            base_part_id
        };
        msg.parts.push(PartSnapshot {
            id: part_id,
            kind: kind.to_string(),
            seq: ev.seq,
            payload: payload.clone(),
        });
        if ev.seq > msg.seq {
            msg.seq = ev.seq;
        }
        if kind == "approval" {
            let part_id = payload
                .get("partId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !part_id.is_empty() {
                let idx = msg.parts.len() - 1;
                approval_part_pos.push((part_id, ev.message_id.clone(), idx));
            }
        }
    }

    // run 状态覆盖消息状态；没有事件的 run 补一条 assistant 消息
    let mut placeholder_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for run in &runs {
        if !run.message_id.is_empty() && messages.contains_key(&run.message_id) {
            let msg = messages.get_mut(&run.message_id).unwrap();
            msg.status = run_message_status(&run.status).to_string();
            if run.finished_at.is_some_and(|f| f > run.started_at) {
                msg.work_duration_ms = Some(run.finished_at.unwrap() - run.started_at);
            }
            // 回复完成时刻 = run 的 finished_at（界面时间戳显示它而非 created_at）
            msg.finished_at = run.finished_at;
        } else if !run.message_id.is_empty() && (run.status == "running" || run.status == "error") {
            // 补消息的 seq 取该 run 名下最早事件的 seq；run 尚无任何部件事件（运行中刚启动、
            // 或失败 run 无部件）时，回退到「run 启动前已落库事件的最大 seq」——即触发它的
            // 用户消息的 seq。若落 seq=0 会排到触发它的用户消息之前，时间线顺序颠倒。
            let run_first_seq = events
                .iter()
                .filter(|e| e.run_id.as_deref() == Some(run.id.as_str()))
                .map(|e| e.seq)
                .filter(|s| *s > 0)
                .min()
                .unwrap_or_else(|| {
                    // 权威信号：触发本 run 的用户消息 seq（V8 起落库）。+1 保证严格排在
                    // 触发消息之后；与「下一条消息 seq 同值」的情形由下方排序的占位优先
                    // 规则定序。不依赖毫秒时间戳——消息落库与 run 创建常落在同一毫秒，
                    // 时间戳区分不出先后。旧行（trigger_seq 为 NULL）回退时间戳兜底。
                    if let Some(t) = run.trigger_seq {
                        t + 1
                    } else {
                        events
                            .iter()
                            .filter(|e| e.created_at <= run.started_at)
                            .map(|e| e.seq)
                            .max()
                            .map(|s| s + 1)
                            .unwrap_or(0)
                    }
                });
            order.push(run.message_id.clone());
            placeholder_ids.insert(run.message_id.clone());
            messages.insert(
                run.message_id.clone(),
                MessageSnapshot {
                    id: run.message_id.clone(),
                    role: "assistant".into(),
                    parts: vec![],
                    status: run_message_status(&run.status).to_string(),
                    seq: run_first_seq,
                    created_at: run.started_at,
                    work_duration_ms: run
                        .finished_at
                        .filter(|f| *f > run.started_at)
                        .map(|f| f - run.started_at),
                    finished_at: run.finished_at,
                },
            );
        }
    }

    let mut list: Vec<MessageSnapshot> = order
        .iter()
        .filter_map(|id| messages.get(id).cloned())
        .filter(|m| !m.parts.is_empty() || m.status != "complete")
        .collect();
    // seq 相同（占位兜底 seq 会与下一条消息的 seq 同值）时，占位消息恒排在真实消息前：
    // 占位属于触发轮次的回复位，语义上先于下一轮用户输入。真实消息之间仍按起始时刻定序；
    // id 是随机的，只作最后兜底，避免同 seq 消息以随机顺序呈现。
    list.sort_by(|a, b| {
        a.seq.cmp(&b.seq).then_with(|| {
            let pa = placeholder_ids.contains(&a.id);
            let pb = placeholder_ids.contains(&b.id);
            pb.cmp(&pa)
        }).then_with(|| a.created_at.cmp(&b.created_at)).then_with(|| a.id.cmp(&b.id))
    });

    let mut has_more = false;
    if before_seq > 0 {
        list.retain(|m| m.seq > 0 && m.seq < before_seq);
    }
    if limit > 0 && list.len() > limit {
        has_more = true;
        list = list.split_off(list.len() - limit);
    }

    let last_seq = events.last().map(|e| e.seq).unwrap_or(0);
    let oldest_seq = list
        .iter()
        .find(|m| !m.parts.is_empty())
        .and_then(|m| m.parts.first())
        .map(|p| p.seq)
        .unwrap_or(0);

    Ok(Snapshot {
        thread,
        messages: list,
        pending_approvals: pending_approvals(conn, thread_id)?,
        last_seq,
        oldest_seq,
        has_more,
    })
}
