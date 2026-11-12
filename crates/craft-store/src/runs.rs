//! runs + approvals 表读写。
//! runs 记录一次 Agent 运行循环的生命周期（running → done/error/canceled/interrupted），
//! 创建时即分配 message_id 作为快照中占位回复消息的锚点；
//! approvals 记录审批请求与决定，决定列非空即终态。

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
    /// 触发本 run 的用户消息事件 seq（V8 起；旧行/无消息触发的 run 为 NULL）。
    /// 快照给无部件 run 的占位消息定序用，是权威先后信号——毫秒时间戳区分不出
    /// 同一毫秒内「消息落库 → run 创建」的先后。
    pub trigger_seq: Option<i64>,
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
        trigger_seq: row.get("trigger_seq")?,
    })
}

/// 创建 running 状态的 run：新分配 message_id（快照占位回复消息的锚点），
/// trigger_seq（触发消息事件 seq）透传落库供 V8 定序。
pub fn create_run(
    conn: &Connection,
    thread_id: &str,
    model_id: &str,
    trigger_seq: Option<i64>,
) -> StoreResult<RunRow> {
    let id = new_id(id_prefix::RUN);
    let message_id = new_id(id_prefix::MESSAGE);
    let now = now_millis();
    conn.execute(
        "INSERT INTO runs (id, thread_id, message_id, status, model_id, started_at, trigger_seq) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6)",
        rusqlite::params![id, thread_id, message_id, model_id, now, trigger_seq],
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
        trigger_seq,
    })
}

pub fn get_run(conn: &Connection, id: &str) -> StoreResult<RunRow> {
    conn.query_row("SELECT * FROM runs WHERE id = ?1", [id], to_run)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound("运行不存在".into()),
            other => other_err(other),
        })
}

/// 结束 run：落 finished_at 与终态；code/message 为空串时存 NULL。
pub fn finish_run(conn: &Connection, id: &str, status: &str, code: &str, message: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE runs SET status = ?1, finished_at = ?2, error_code = ?3, error_message = ?4 WHERE id = ?5",
        rusqlite::params![status, now_millis(), if code.is_empty() { None } else { Some(code) }, if message.is_empty() { None } else { Some(message) }, id],
    )
    .map_err(other_err)?;
    Ok(())
}

/// 该会话最后一个 run 的状态（无 run 返回 None）。新 run 启动时据此判断上一轮
/// 是否以失败收尾（error/interrupted → 消息流挂着未解决的错误卡片，需回填 error_resolved）。
/// runs 表无单调序号列，rowid 即插入顺序，比毫秒时间戳更能区分同毫秒创建的 run。
pub fn last_run_status(conn: &Connection, thread_id: &str) -> StoreResult<Option<String>> {
    let mut stmt = conn
        .prepare("SELECT status FROM runs WHERE thread_id = ?1 ORDER BY rowid DESC LIMIT 1")
        .map_err(other_err)?;
    let mut rows = stmt.query([thread_id]).map_err(other_err)?;
    rows.next()
        .map_err(other_err)?
        .map(|row| row.get::<_, String>(0))
        .transpose()
        .map_err(other_err)
}

/// 启动收尾时待取消的挂起审批（partId 供快照回填定位卡片）。
struct StaleApproval {
    id: String,
    thread_id: String,
    run_id: String,
    message_id: String,
    part_id: Option<String>,
}

/// 上次退出遗留的 running 运行列表 (run_id, thread_id)（启动恢复的观测记录用；
/// 读取失败返回空列表——观测路径不向上传播错误）。
pub fn list_stale_runs(db: &Db) -> Vec<(String, String)> {
    db.with(|conn| {
        let mut stmt = conn.prepare("SELECT id, thread_id FROM runs WHERE status = 'running'")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .unwrap_or_default()
}

/// 启动收尾：把上次退出遗留的 running 运行标记为 interrupted、挂起审批标记 canceled、
/// 会话状态复位，并为每个中断 run 写一条可重试的 error 事件。返回中断数量。
pub fn interrupt_stale_runs(db: &Db, mut append_event_fn: impl FnMut(EventRecord)) -> StoreResult<usize> {    let stale: Vec<(String, String, String)> = db.with(|conn| {
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

    // 取消前收集挂起审批（id/thread/run/message + 卡片 partId 反查）：
    // approval_resolved 事件回填快照状态时按 partId 定位卡片，缺了会留下「等待决定」僵尸卡。
    let stale_approvals: Vec<StaleApproval> = db.with(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT a.id, a.thread_id, a.run_id FROM approvals a JOIN runs r ON r.id = a.run_id \
                 WHERE r.status = 'running' AND a.decision IS NULL",
            )
            .map_err(other_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(other_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(other_err)?;
        let mut result = Vec::with_capacity(rows.len());
        for (id, thread_id, run_id) in rows {
            // 卡片的 partId / message_id 从审批事件行反查（approvals 表没有这两列）
            let (part_id, message_id) = find_approval_event(conn, &thread_id, &id).unwrap_or((None, String::new()));
            result.push(StaleApproval { id, thread_id, run_id, message_id, part_id });
        }
        Ok(result)
    })?;

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
    // 事件在事务外逐条写入（append_event 自带事务）：可重试的 error + 挂起审批的取消终态
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
    // 子代理卡片收尾（决策 D15）：启动时不存在任何在途运行，故 event 表里仍是
    // status=running 的 subagent 部件必然是上次退出的残留——补写 interrupted 终态，
    // 否则主会话里会留下永远转圈的僵尸卡（与审批僵尸卡同一类问题）。
    for record in stale_subagent_events(db).unwrap_or_default() {
        append_event_fn(record);
    }
    for sa in &stale_approvals {
        append_event_fn(EventRecord {
            thread_id: sa.thread_id.clone(),
            message_id: sa.message_id.clone(),
            run_id: Some(sa.run_id.clone()),
            kind: "approval_resolved".into(),
            payload: serde_json::json!({
                "partId": sa.part_id.clone().unwrap_or_default(),
                "approvalId": sa.id,
                "status": "canceled",
                "decision": "canceled",
                "scope": "",
            }),
        });
    }
    Ok(stale.len())
}

/// 在指定会话的事件行里反查审批卡片（payload.approvalId 匹配），返回 (partId, message_id)。
fn find_approval_event(conn: &rusqlite::Connection, thread_id: &str, approval_id: &str) -> StoreResult<(Option<String>, String)> {
    let mut stmt = conn
        .prepare("SELECT payload, message_id FROM events WHERE kind = 'approval' AND thread_id = ?1 ORDER BY seq DESC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([thread_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows
        .iter()
        .filter_map(|(payload, message_id)| serde_json::from_str::<serde_json::Value>(payload).ok().map(|p| (p, message_id.clone())))
        .find(|(p, _)| p.get("approvalId").and_then(serde_json::Value::as_str) == Some(approval_id))
        .map(|(p, message_id)| (p.get("partId").and_then(serde_json::Value::as_str).map(String::from), message_id))
        .unwrap_or((None, String::new())))
}

/// 上次退出遗留的「仍在运行」子代理卡片，转成 interrupted 终态事件（决策 D15）。
/// 扫描全库的 subagent 部件事件，取每个 partId 的最后一条，遗留状态 running 即产出
/// 一条同 partId 的终态事件（快照按 partId 合并，卡片就此收口）。
/// 读取失败返回空列表——启动恢复是观测/兜底路径，不向上传播错误。
pub fn stale_subagent_events(db: &Db) -> StoreResult<Vec<EventRecord>> {
    let rows: Vec<(String, String, Option<String>, String, String)> = db.with(|conn| {
        let mut stmt = conn
            .prepare("SELECT thread_id, message_id, run_id, payload, id FROM events WHERE kind = 'subagent' ORDER BY thread_id, seq ASC")
            .map_err(other_err)?;
        let collected = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(other_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(other_err)?;
        Ok(collected)
    })?;
    // (thread, partId) → 最后一条事件载荷；事件按 seq 升序遍历，后者覆盖前者
    let mut latest: std::collections::HashMap<(String, String), (String, Option<String>, String)> =
        std::collections::HashMap::new();
    for (thread_id, message_id, run_id, payload, _event_id) in rows {
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let part_id = parsed.get("partId").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        if part_id.is_empty() {
            continue;
        }
        latest.insert((thread_id, part_id), (message_id, run_id, payload));
    }
    let mut out = Vec::new();
    for ((thread_id, _part_id), (message_id, _run_id, payload)) in latest {
        let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        if parsed.get("status").and_then(serde_json::Value::as_str) != Some("running") {
            continue;
        }
        parsed["status"] = serde_json::Value::String("interrupted".into());
        parsed["result"] = serde_json::Value::String("应用退出导致子代理运行中断。".into());
        parsed["notify"] = serde_json::Value::Bool(false);
        out.push(EventRecord {
            thread_id,
            message_id,
            run_id: None,
            kind: "subagent".into(),
            payload: parsed,
        });
    }
    Ok(out)
}

// ---------- 审批 ----------

/// approvals 一行；decision/scope 为 None 表示等待决定。
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

/// 创建审批的入参（risk 为 R0~R4 风险等级字符串）。
pub struct NewApproval<'a> {
    pub thread_id: &'a str,
    pub run_id: &'a str,
    pub tool_call_id: &'a str,
    pub tool: &'a str,
    pub risk: &'a str,
}

/// 登记一次审批请求（decision/scope 留空等待决定）。
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

/// 去重待审批会话：`decision IS NULL` 的 distinct thread_id。
///
/// 子代理审批落库时 `thread_id` 已是父会话（D15），调用方无需再做父子映射。
pub fn pending_approval_threads(conn: &Connection) -> StoreResult<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT thread_id FROM approvals WHERE decision IS NULL")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 取消会话下全部待决定审批（run 取消/用户中断时调用；已决定的行不受影响）。
pub fn cancel_thread_approvals(conn: &Connection, thread_id: &str) -> StoreResult<()> {
    conn.execute(
        "UPDATE approvals SET decision = 'canceled', decided_at = ?1 WHERE thread_id = ?2 AND decision IS NULL",
        rusqlite::params![now_millis(), thread_id],
    )
    .map_err(other_err)?;
    Ok(())
}
