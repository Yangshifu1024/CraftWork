//! events 表读写与快照重建（src/main/store/events.ts 的直译）。
//! 事件表是唯一事实源：消息与部件从事件行重建，不另存副本。
//! seq 分配与插入必须同事务（并发安全）；写入后顺带推进 threads.updated_at。

use rusqlite::Connection;
use serde_json::Value;

use crate::db::{now_millis, other_err, Db, StoreResult};
use crate::threads::get_thread;
use crate::view::to_thread_view;
use craft_core::contract::{ApprovalCard, ApprovalFact, MessageSnapshot, PartSnapshot, Snapshot, ThreadView};
use craft_core::ids::{id_prefix, new_id};

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

#[derive(Debug, Clone)]
pub struct EventRecord {
    pub thread_id: String,
    pub message_id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub payload: Value,
}

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

// ---------- 快照重建 ----------

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
}

fn list_runs(conn: &Connection, thread_id: &str) -> StoreResult<Vec<RunInfo>> {
    let mut stmt = conn
        .prepare("SELECT id, message_id, status, started_at, finished_at FROM runs WHERE thread_id = ?1 ORDER BY started_at ASC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([thread_id], |row| {
            Ok(RunInfo {
                id: row.get(0)?,
                message_id: row.get(1)?,
                status: row.get(2)?,
                started_at: row.get(3)?,
                finished_at: row.get(4)?,
            })
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

fn run_message_status(run_status: &str) -> &'static str {
    match run_status {
        "running" => "streaming",
        "error" => "error",
        "canceled" | "interrupted" => "incomplete",
        _ => "complete",
    }
}

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

        let Some(kind) = snapshot_part_kind(&ev.kind) else {
            continue;
        };

        let mut payload: Value = serde_json::from_str(&ev.payload).unwrap_or(Value::Null);
        if payload.is_null() {
            payload = serde_json::json!({});
        }
        if ev.kind == "user_message" {
            // user_message 只保留 text 与 attachments（via=steer 透传：前端「待送达」标记）
            let via = payload.get("via").cloned();
            payload = serde_json::json!({
                "text": payload.get("text").cloned().unwrap_or(Value::String(String::new())),
                "attachments": payload.get("attachments").cloned().unwrap_or(Value::Array(vec![])),
            });
            if let Some(via) = via {
                payload["via"] = via;
            }
        }

        let role = if ev.kind == "user_message" { "user" } else { "assistant" };
        let msg = ensure_message(&mut order, &mut messages, &ev.message_id, role, ev.created_at);
        // part.id 优先取 payload.partId：渲染层按 partId 归属后续增量事件（text.delta 等），
        // 用事件行 id 会导致快照恢复后的流式事件永远找不到归属消息。
        let part_id = payload
            .get("partId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ev.id.clone());
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
    for run in &runs {
        if !run.message_id.is_empty() && messages.contains_key(&run.message_id) {
            let msg = messages.get_mut(&run.message_id).unwrap();
            msg.status = run_message_status(&run.status).to_string();
            if run.finished_at.is_some_and(|f| f > run.started_at) {
                msg.work_duration_ms = Some(run.finished_at.unwrap() - run.started_at);
            }
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
                    // +1 保证严格排在触发它的用户消息之后（即便两者同毫秒）；
                    // 后续真实部件事件的 seq 归属同一消息，不与该占位值冲突。
                    events
                        .iter()
                        .filter(|e| e.created_at <= run.started_at)
                        .map(|e| e.seq)
                        .max()
                        .map(|s| s + 1)
                        .unwrap_or(0)
                });
            order.push(run.message_id.clone());
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
                },
            );
        }
    }

    let mut list: Vec<MessageSnapshot> = order
        .iter()
        .filter_map(|id| messages.get(id).cloned())
        .filter(|m| !m.parts.is_empty() || m.status != "complete")
        .collect();
    // seq 相同（如运行中 run 的占位消息兜底 seq == 触发它的用户消息 seq）时按消息起始时刻定序：
    // id 是随机的，用 id 定序会以随机顺序呈现同 seq 消息。
    list.sort_by(|a, b| {
        a.seq
            .cmp(&b.seq)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
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
