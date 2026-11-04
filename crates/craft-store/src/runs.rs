//! runs + approvals 表读写（src/main/store/runs.ts 的直译）。

use rusqlite::Connection;

use crate::db::{not_found, now_millis, other_err, Db, StoreError, StoreResult};
use crate::events::EventRecord;
use craft_core::ids::{id_prefix, new_id};

#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: String,
    pub thread_id: String,
    pub message_id: String,
    pub status: String,
    pub model_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

fn to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get("id")?,
        thread_id: row.get("thread_id")?,
        message_id: row.get("message_id")?,
        status: row.get("status")?,
        model_id: row.get("model_id")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        error_code: row.get("error_code")?,
        error_message: row.get("error_message")?,
    })
}

pub fn create_run(conn: &Connection, thread_id: &str, model_id: &str) -> StoreResult<RunRow> {
    let id = new_id(id_prefix::RUN);
    let message_id = new_id(id_prefix::MESSAGE);
    let now = now_millis();
    conn.execute(
        "INSERT INTO runs (id, thread_id, message_id, status, model_id, started_at) VALUES (?1, ?2, ?3, 'running', ?4, ?5)",
        rusqlite::params![id, thread_id, message_id, model_id, now],
    )
    .map_err(other_err)?;
    Ok(RunRow {
        id,
        thread_id: thread_id.into(),
        message_id,
        status: "running".into(),
        model_id: model_id.into(),
        started_at: now,
        finished_at: None,
        error_code: None,
        error_message: None,
    })
}

pub fn get_run(conn: &Connection, id: &str) -> StoreResult<RunRow> {
    conn.query_row("SELECT * FROM runs WHERE id = ?1", [id], to_run)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound("运行不存在".into()),
            other => other_err(other),
        })
}

pub fn finish_run(conn: &Connection, id: &str, status: &str, code: &str, message: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE runs SET status = ?1, finished_at = ?2, error_code = ?3, error_message = ?4 WHERE id = ?5",
        rusqlite::params![status, now_millis(), if code.is_empty() { None } else { Some(code) }, if message.is_empty() { None } else { Some(message) }, id],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 启动收尾：把上次退出遗留的 running 运行标记为 interrupted、挂起审批标记 canceled、
/// 会话状态复位，并为每个中断 run 写一条可重试的 error 事件。返回中断数量。
pub fn interrupt_stale_runs(db: &Db, mut append_event_fn: impl FnMut(EventRecord)) -> StoreResult<usize> {
    let stale: Vec<(String, String, String)> = db.with(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, thread_id, message_id FROM runs WHERE status = 'running'")
            .map_err(other_err)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(other_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(other_err)?;
        Ok(rows)
    })?;
    if stale.is_empty() {
        return Ok(0);
    }
    let message = "应用退出导致运行中断，已保留已产生的消息，可以重试。";
    db.transaction(|conn| {
        for (run_id, thread_id, _message_id) in &stale {
            conn.execute(
                "UPDATE runs SET status = 'interrupted', finished_at = ?1, error_code = 'interrupted', error_message = ?2 WHERE id = ?3",
                rusqlite::params![now_millis(), message, run_id],
            )
            .map_err(other_err)?;
            conn.execute(
                "UPDATE approvals SET decision = 'canceled', decided_at = ?1 WHERE thread_id = ?2 AND decision IS NULL",
                rusqlite::params![now_millis(), thread_id],
            )
            .map_err(other_err)?;
            conn.execute(
                "UPDATE threads SET status = 'error', updated_at = ?1 WHERE id = ?2 AND status IN ('running','awaiting')",
                rusqlite::params![now_millis(), thread_id],
            )
            .map_err(other_err)?;
        }
        Ok(())
    })?;
    // error 事件在事务外逐条写入（append_event 自带事务）
    for (run_id, thread_id, message_id) in &stale {
        append_event_fn(EventRecord {
            thread_id: thread_id.clone(),
            message_id: message_id.clone(),
            run_id: Some(run_id.clone()),
            kind: "error".into(),
            payload: serde_json::json!({
                "partId": new_id(id_prefix::PART),
                "code": "interrupted",
                "message": message,
                "retryable": true,
            }),
        });
    }
    Ok(stale.len())
}

// ---------- 审批 ----------

#[derive(Debug, Clone)]
pub struct ApprovalRow {
    pub id: String,
    pub thread_id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub tool: String,
    pub risk: String,
    pub decision: Option<String>,
    pub scope: Option<String>,
    pub requested_at: i64,
}

pub struct NewApproval<'a> {
    pub thread_id: &'a str,
    pub run_id: &'a str,
    pub tool_call_id: &'a str,
    pub tool: &'a str,
    pub risk: &'a str,
}

pub fn create_approval(conn: &Connection, input: NewApproval<'_>) -> StoreResult<ApprovalRow> {
    let id = new_id(id_prefix::APPROVAL);
    conn.execute(
        "INSERT INTO approvals (id, thread_id, run_id, tool_call_id, tool, risk, requested_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, input.thread_id, input.run_id, input.tool_call_id, input.tool, input.risk, now_millis()],
    )
    .map_err(other_err)?;
    Ok(ApprovalRow {
        id,
        thread_id: input.thread_id.into(),
        run_id: input.run_id.into(),
        tool_call_id: input.tool_call_id.into(),
        tool: input.tool.into(),
        risk: input.risk.into(),
        decision: None,
        scope: None,
        requested_at: now_millis(),
    })
}

/// 并发保护：WHERE decision IS NULL——重复决定 RowsAffected=0 → NotFound。
pub fn resolve_approval(conn: &Connection, id: &str, decision: &str, scope: &str) -> StoreResult<()> {
    let changes = conn
        .execute(
            "UPDATE approvals SET decision = ?1, scope = ?2, decided_at = ?3 WHERE id = ?4 AND decision IS NULL",
            rusqlite::params![decision, if scope.is_empty() { None } else { Some(scope) }, now_millis(), id],
        )
        .map_err(other_err)?;
    if changes == 0 {
        return not_found("审批不存在或已处理");
    }
    Ok(())
}

pub fn get_approval(conn: &Connection, id: &str) -> StoreResult<ApprovalRow> {
    conn.query_row("SELECT * FROM approvals WHERE id = ?1", [id], |row| {
        Ok(ApprovalRow {
            id: row.get("id")?,
            thread_id: row.get("thread_id")?,
            run_id: row.get("run_id")?,
            tool_call_id: row.get("tool_call_id")?,
            tool: row.get("tool")?,
            risk: row.get("risk")?,
            decision: row.get("decision")?,
            scope: row.get("scope")?,
            requested_at: row.get("requested_at")?,
        })
    })
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound("审批不存在".into()),
        other => other_err(other),
    })
}

pub fn cancel_thread_approvals(conn: &Connection, thread_id: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE approvals SET decision = 'canceled', decided_at = ?1 WHERE thread_id = ?2 AND decision IS NULL",
        rusqlite::params![now_millis(), thread_id],
    )
    .map_err(other_err)?;
    Ok(())
}
