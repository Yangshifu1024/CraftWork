//! 存储层测试：迁移、事件 seq、快照重建、审批并发保护、启动收尾。
//! 用例语义对应 tests/store.test.ts（vitest 版的 Rust 翻译）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use craft_store::db::{Db, StoreError, SCHEMA_VERSION};
use craft_store::events::{append_event, latest_context_summary, plan_mode_for_trigger, record_event, snapshot, EventRecord};
use craft_store::plans::{create_plan, latest_plan};
use craft_store::records::{add_audit, get_setting, list_audit, set_setting};
use craft_store::runs::{create_approval, create_run, finish_run, get_approval, resolve_approval};
use craft_store::threads::{
    auto_title_thread, create_thread, delete_thread, get_thread, list_threads, rename_thread, search_threads, set_thread_worktree,
    NewThread,
};
use craft_store::workspaces::{default_conversation_workspace_id, get_workspace, replace_workspace_folders, upsert_workspace};

fn tmp_db(tag: &str) -> (PathBuf, Db) {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cw-store-{}-{}-{}",
        tag,
        std::process::id(),
        n + 1000 * (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_millis() as u32)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Db::open(&dir.join("test.db")).expect("打开测试库失败");
    (dir, db)
}

fn thread(id: &str) -> NewThread<'_> {
    NewThread {
        id,
        mode: "code",
        title: "新对话",
        title_auto: true,
        model_id: "",
        reasoning_level: "",
        workspace_id: "",
        status: "idle",
        archived: false,
    }
}

fn user_message(thread_id: &str, message_id: &str, text: &str) -> EventRecord {
    EventRecord {
        thread_id: thread_id.into(),
        message_id: message_id.into(),
        run_id: None,
        kind: "user_message".into(),
        payload: serde_json::json!({ "text": text, "attachments": [] }),
    }
}

/// 新库直迁当前版本，关闭重开后版本稳定（迁移幂等）。
#[test]
fn migrate_to_current_version_and_reopen() {
    let (dir, db) = tmp_db("migrate");
    let version: i64 = db
        .with(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0)))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    drop(db);
    let reopened = Db::open(&dir.join("test.db")).unwrap();
    let again: i64 = reopened
        .with(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0)))
        .unwrap();
    assert_eq!(again, SCHEMA_VERSION);
    let _ = std::fs::remove_dir_all(&dir);
}

/// V7 迁移语义直验：重放迁移 SQL，只清「空 + 自动命名 + 新对话」的会话。
#[test]
fn v7_empty_thread_cleanup_semantics_delete_empty_auto_named_keep_thread_with_events_and_manual_rename() {
    let (dir, db) = tmp_db("v7");
    create_thread(&db.lock_conn(), thread("th_v7_empty")).unwrap();
    create_thread(&db.lock_conn(), thread("th_v7_events")).unwrap();
    create_thread(
        &db.lock_conn(),
        NewThread { id: "th_v7_named", title: "我的会话", title_auto: false, ..thread("x") },
    )
    .unwrap();
    record_event(&db, &user_message("th_v7_events", "m_v7", "你好")).unwrap();

    // 重放 V7 迁移 SQL，直接验证语义
    db.with(|conn| {
        conn.execute_batch(craft_store::db::SCHEMA_V7).map_err(craft_store::db::other_err)?;
        Ok(())
    })
    .unwrap();

    // 空自动命名会话被删除
    assert!(matches!(get_thread(&db.lock_conn(), "th_v7_empty"), Err(StoreError::NotFound(_))));
    // 有事件的会话保留
    assert_eq!(get_thread(&db.lock_conn(), "th_v7_events").unwrap().id, "th_v7_events");
    // 手动改名（非自动命名）会话保留
    assert_eq!(get_thread(&db.lock_conn(), "th_v7_named").unwrap().title, "我的会话");
    // 升级后搜索「新对话」不再命中任何空会话
    let hits = search_threads(&db.lock_conn(), "code", "新对话").unwrap();
    assert_eq!(hits.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["th_v7_events"]);
    // 清理后可正常继续读写
    let fresh = create_thread(
        &db.lock_conn(),
        NewThread { id: "th_v7_new", title: "续会话", title_auto: false, ..thread("x") },
    )
    .unwrap();
    assert_eq!(get_thread(&db.lock_conn(), &fresh.id).unwrap().title, "续会话");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn threads_basic_read_write_and_delete() {
    let (dir, db) = tmp_db("threads");
    let t = create_thread(&db.lock_conn(), thread("th_1")).unwrap();
    assert_eq!(get_thread(&db.lock_conn(), &t.id).unwrap().title, "新对话");
    rename_thread(&db.lock_conn(), &t.id, "改名").unwrap();
    assert!(!get_thread(&db.lock_conn(), &t.id).unwrap().title_auto);
    delete_thread(&db, &t.id).unwrap();
    assert!(matches!(get_thread(&db.lock_conn(), &t.id), Err(StoreError::NotFound(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 删除会话级联清理计划行：plans 不留孤儿（批准提示条以 plans 为准，残留会误指向已删会话）。
#[test]
fn delete_thread_cascades_plan_rows() {
    let (dir, db) = tmp_db("thread-plan-cascade");
    create_thread(&db.lock_conn(), thread("th_plan")).unwrap();
    let plan = create_plan(&db.lock_conn(), "th_plan", "run_plan").unwrap();
    assert!(latest_plan(&db.lock_conn(), "th_plan").unwrap().is_some());
    delete_thread(&db, "th_plan").unwrap();
    assert!(latest_plan(&db.lock_conn(), "th_plan").unwrap().is_none());
    // 行已物理移除，而非仅 latest_plan 查不到
    let count: i64 = db
        .with(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM plans WHERE id = ?1", [&plan.id], |row| row.get(0)).unwrap_or(-1)))
        .unwrap();
    assert_eq!(count, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_title_conditional_update_only_applies_to_auto_named_threads_and_flips_flag() {
    let (dir, db) = tmp_db("autotitle");
    create_thread(&db.lock_conn(), thread("th_2")).unwrap();
    // 命名落地并翻转 title_auto=0：后续 run 不再重复命名（migration-parity 票 03；
    // 旧条件要求 title=「新对话」，而首发路径标题是 derive_title，模型命名永不落地）
    assert!(auto_title_thread(&db.lock_conn(), "th_2", "新标题").unwrap());
    assert!(!get_thread(&db.lock_conn(), "th_2").unwrap().title_auto);
    assert!(!auto_title_thread(&db.lock_conn(), "th_2", "再改").unwrap());
    // 手动改名保护：rename 后 title_auto=0，自动命名不覆盖
    create_thread(&db.lock_conn(), thread("th_3")).unwrap();
    rename_thread(&db.lock_conn(), "th_3", "手动名").unwrap();
    assert!(!auto_title_thread(&db.lock_conn(), "th_3", "模型名").unwrap());
    assert_eq!(get_thread(&db.lock_conn(), "th_3").unwrap().title, "手动名");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 同会话事件 seq 连续递增；快照按 messageId 分组重建出用户/助手两条消息。
#[test]
fn event_seq_monotonically_increasing_and_snapshot_rebuilds_messages() {
    let (dir, db) = tmp_db("events");
    create_thread(&db.lock_conn(), thread("th_3")).unwrap();
    let r1 = record_event(&db, &user_message("th_3", "m1", "你好")).unwrap();
    let r2 = record_event(
        &db,
        &EventRecord {
            thread_id: "th_3".into(),
            message_id: "m2".into(),
            run_id: None,
            kind: "text".into(),
            payload: serde_json::json!({ "partId": "p1", "text": "回" }),
        },
    )
    .unwrap();
    assert_eq!(r2.1, r1.1 + 1);
    let snap = snapshot(&db.lock_conn(), "th_3", 0, 0).unwrap();
    // 不同 messageId 各自成一条消息：用户消息 + 助手回复
    assert_eq!(snap.messages.len(), 2);
    assert_eq!(snap.messages[0].role, "user");
    assert_eq!(snap.messages[1].role, "assistant");
    assert_eq!(snap.messages[1].parts[0].payload["text"], "回");
    assert_eq!(snap.last_seq, r2.1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// steer 快照重建：steer_delivered 回执摘除「待送达」标记（via），
/// 消费切换的新旧 assistant 消息按事件序夹住引导消息。
#[test]
fn snapshot_steer_delivered_removes_via_flag_and_splits_assistant_messages() {
    let (dir, db) = tmp_db("steer-snap");
    create_thread(&db.lock_conn(), thread("th_steer")).unwrap();
    // 事件序：旧 assistant 输出 → 引导用户消息（via=steer）→ 新 assistant 输出 → 投递回执
    let events = [
        EventRecord {
            thread_id: "th_steer".into(),
            message_id: "m_a1".into(),
            run_id: None,
            kind: "text".into(),
            payload: serde_json::json!({ "partId": "p_a1", "text": "前半段" }),
        },
        EventRecord {
            thread_id: "th_steer".into(),
            message_id: "m_s".into(),
            run_id: None,
            kind: "user_message".into(),
            payload: serde_json::json!({ "text": "还要考虑 Windows", "attachments": [], "via": "steer", "planMode": false }),
        },
        EventRecord {
            thread_id: "th_steer".into(),
            message_id: "m_a2".into(),
            run_id: None,
            kind: "text".into(),
            payload: serde_json::json!({ "partId": "p_a2", "text": "后半段" }),
        },
        EventRecord {
            thread_id: "th_steer".into(),
            message_id: "m_s".into(),
            run_id: None,
            kind: "steer_delivered".into(),
            payload: serde_json::json!({ "nextMessageId": "m_a2" }),
        },
    ];
    for rec in &events {
        record_event(&db, rec).unwrap();
    }
    let snap = snapshot(&db.lock_conn(), "th_steer", 0, 0).unwrap();
    assert_eq!(snap.messages.len(), 3);
    assert_eq!(snap.messages[0].id, "m_a1");
    assert_eq!(snap.messages[0].parts[0].payload["text"], "前半段");
    assert_eq!(snap.messages[1].id, "m_s");
    assert_eq!(snap.messages[1].role, "user");
    assert!(
        snap.messages[1].parts[0].payload.get("via").is_none(),
        "投递回执后「待送达」标记应被摘除"
    );
    assert_eq!(snap.messages[1].parts[0].payload["text"], "还要考虑 Windows");
    assert_eq!(snap.messages[2].id, "m_a2");
    assert_eq!(snap.messages[2].parts[0].payload["text"], "后半段");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 尚未投递的引导（无 steer_delivered 回执）保留 via 标记。
#[test]
fn snapshot_keeps_via_flag_for_undelivered_steer() {
    let (dir, db) = tmp_db("steer-keep");
    create_thread(&db.lock_conn(), thread("th_steer2")).unwrap();
    record_event(
        &db,
        &EventRecord {
            thread_id: "th_steer2".into(),
            message_id: "m_s2".into(),
            run_id: None,
            kind: "user_message".into(),
            payload: serde_json::json!({ "text": "排队中", "attachments": [], "via": "steer" }),
        },
    )
    .unwrap();
    let snap = snapshot(&db.lock_conn(), "th_steer2", 0, 0).unwrap();
    assert_eq!(snap.messages.len(), 1);
    assert_eq!(snap.messages[0].parts[0].payload["via"], "steer");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 审批决定幂等保护：首个决定生效，重复决定返回 NotFound（WHERE decision IS NULL）。
#[test]
fn approval_concurrency_protection_and_repeated_decision_rejected() {
    let (dir, db) = tmp_db("approvals");
    create_thread(&db.lock_conn(), thread("th_4")).unwrap();
    let ap = create_approval(
        &db.lock_conn(),
        craft_store::runs::NewApproval { thread_id: "th_4", run_id: "run_1", tool_call_id: "call_1", tool: "fs_write", risk: "R1" },
    )
    .unwrap();
    resolve_approval(&db.lock_conn(), &ap.id, "allow", "session").unwrap();
    assert_eq!(get_approval(&db.lock_conn(), &ap.id).unwrap().decision.as_deref(), Some("allow"));
    assert!(matches!(
        resolve_approval(&db.lock_conn(), &ap.id, "deny", "once"),
        Err(StoreError::NotFound(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 启动收尾主路径：遗留 running 的 run 标记 interrupted，会话拿到可重试的 error 部件。
#[test]
fn startup_finalization_marks_stale_runs_interrupted_and_writes_error_event() {
    let (dir, db) = tmp_db("stale");
    create_thread(
        &db.lock_conn(),
        NewThread { id: "th_5", status: "running", ..thread("x") },
    )
    .unwrap();
    let run = create_run(&db.lock_conn(), "th_5", "gpt-x", None).unwrap();
    // Recorder 的 interrupt_stale_runs 包装负责落库；此处直接用 append_event 等价回调
    let count = craft_store::runs::interrupt_stale_runs(&db, |rec| {
        record_event(&db, &rec).unwrap();
    })
    .unwrap();
    assert_eq!(count, 1);
    let (status, error_code): (String, Option<String>) = db
        .with(|conn| {
            Ok(conn.query_row("SELECT status, error_code FROM runs WHERE id = ?1", [&run.id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?)
        })
        .unwrap();
    assert_eq!(status, "interrupted");
    assert_eq!(error_code.as_deref(), Some("interrupted"));
    let snap = snapshot(&db.lock_conn(), "th_5", 0, 0).unwrap();
    // 中断消息标记 incomplete，且带有可重试的 error 部件
    let interrupted = snap.messages.iter().find(|m| m.parts.iter().any(|p| p.kind == "error")).unwrap();
    assert_eq!(interrupted.status, "incomplete");
    let error_part = interrupted.parts.iter().find(|p| p.kind == "error").unwrap();
    assert_eq!(error_part.payload["retryable"], serde_json::json!(true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_finish_backfills_status() {
    let (dir, db) = tmp_db("runfinish");
    create_thread(&db.lock_conn(), thread("th_6")).unwrap();
    let run = create_run(&db.lock_conn(), "th_6", "gpt-x", None).unwrap();
    finish_run(&db.lock_conn(), &run.id, "done", "", "").unwrap();
    let (status, finished_at): (String, i64) = db
        .with(|conn| {
            Ok(conn.query_row("SELECT status, finished_at FROM runs WHERE id = ?1", [&run.id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?)
        })
        .unwrap();
    assert_eq!(status, "done");
    assert!(finished_at > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn workspace_idempotent_upsert_by_path() {
    let (dir, db) = tmp_db("wsupsert");
    let a = upsert_workspace(&db.lock_conn(), "项目", "/tmp/ws-a", "trusted", "project").unwrap();
    let b = upsert_workspace(&db.lock_conn(), "项目改名", "/tmp/ws-a", "restricted", "conversation").unwrap();
    assert_eq!(a.id, b.id);
    // purpose 由首次创建决定
    let ws = get_workspace(&db.lock_conn(), &a.id).unwrap();
    assert_eq!(ws.workspace.purpose, "project");
    assert_eq!(ws.workspace.trust, "restricted");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folders_replaced_wholesale_and_main_path_synced() {
    let (dir, db) = tmp_db("wsfolders");
    let ws = upsert_workspace(&db.lock_conn(), "项目", "/tmp/ws-b", "restricted", "project").unwrap();
    replace_workspace_folders(&db, &ws.id, &["/tmp/ws-b".into(), "/tmp/ws-c".into()]).unwrap();
    let loaded = get_workspace(&db.lock_conn(), &ws.id).unwrap();
    let paths: Vec<_> = loaded.folders.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["/tmp/ws-b", "/tmp/ws-c"]);
    assert_eq!(loaded.workspace.path, "/tmp/ws-b");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_conversation_workspace_resolution_falls_back_to_empty_string_when_missing() {
    let (dir, db) = tmp_db("wsdefault");
    // 无 conversation 工作区时回落空串
    assert_eq!(default_conversation_workspace_id(&db.lock_conn()), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_upsert_and_read() {
    let (dir, db) = tmp_db("settings");
    set_setting(&db.lock_conn(), "app", &serde_json::json!({ "theme": "dark" })).unwrap();
    assert_eq!(get_setting(&db.lock_conn(), "app").unwrap().unwrap()["theme"], "dark");
    set_setting(&db.lock_conn(), "app", &serde_json::json!({ "theme": "light" })).unwrap();
    assert_eq!(get_setting(&db.lock_conn(), "app").unwrap().unwrap()["theme"], "light");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_append_and_descending_list() {
    let (dir, db) = tmp_db("audit");
    add_audit(
        &db.lock_conn(),
        craft_store::records::AuditRecord {
            ts: None,
            thread_id: None,
            run_id: None,
            tool: "fs_write",
            risk: "R1",
            decision: "allow",
            scope: Some("user"),
            detail: Some("a.txt"),
        },
    )
    .unwrap();
    add_audit(
        &db.lock_conn(),
        craft_store::records::AuditRecord {
            ts: Some(craft_store::db::now_millis() + 5),
            thread_id: None,
            run_id: None,
            tool: "shell_exec",
            risk: "R2",
            decision: "deny",
            scope: None,
            detail: None,
        },
    )
    .unwrap();
    let list = list_audit(&db.lock_conn(), 10).unwrap();
    assert_eq!(list.len(), 2);
    // 最新在前
    assert_eq!(list[0].tool, "shell_exec");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thread_list_filtered_by_mode() {
    let (dir, db) = tmp_db("threadmode");
    create_thread(&db.lock_conn(), NewThread { id: "th_c1", title: "c1", ..thread("x") }).unwrap();
    create_thread(
        &db.lock_conn(),
        NewThread { id: "th_w1", mode: "work", title: "w1", ..thread("x") },
    )
    .unwrap();
    assert_eq!(list_threads(&db.lock_conn(), "code", false).unwrap().len(), 1);
    assert_eq!(list_threads(&db.lock_conn(), "work", false).unwrap()[0].id, "th_w1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn context_summary_latest_one_naturally_evicts_older_summaries() {
    let (dir, db) = tmp_db("summary");
    create_thread(&db.lock_conn(), thread("th_7")).unwrap();
    record_event(
        &db,
        &EventRecord {
            thread_id: "th_7".into(),
            message_id: "s0".into(),
            run_id: None,
            kind: "context_summary".into(),
            payload: serde_json::json!({ "summary": "旧", "untilSeq": 3 }),
        },
    )
    .unwrap();
    record_event(
        &db,
        &EventRecord {
            thread_id: "th_7".into(),
            message_id: "s1".into(),
            run_id: None,
            kind: "context_summary".into(),
            payload: serde_json::json!({ "summary": "新", "untilSeq": 9 }),
        },
    )
    .unwrap();
    let summary = latest_context_summary(&db.lock_conn(), "th_7").unwrap().unwrap();
    assert_eq!(summary.0, "新");
    assert_eq!(summary.1, 9);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_event_convenience_entry_verified_via_record_event() {
    let (dir, db) = tmp_db("append");
    create_thread(&db.lock_conn(), thread("th_8")).unwrap();
    // 直接在 with 内调用 append_event 与 record_event 等价（record_event 自带事务）
    let (id, seq) = db
        .with(|conn| append_event(conn, &user_message("th_8", "m8", "hi")))
        .unwrap();
    assert!(id.starts_with("part_"));
    assert_eq!(seq, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_running_run_placeholder_message_after_triggering_user_message() {
    let (dir, db) = tmp_db("run-order");
    create_thread(&db.lock_conn(), thread("th_order")).unwrap();
    // 用户消息先落库（快照里 seq 更小）；seq 随 run 透传为 trigger_seq（V8）
    let (_, seq1) = record_event(&db, &user_message("th_order", "msg_u1", "你好")).unwrap();
    // 创建运行：running 且尚无任何部件事件 → 快照补占位消息
    let run = create_run(&db.lock_conn(), "th_order", "model-x", Some(seq1)).unwrap();
    let snap = snapshot(&db.lock_conn(), "th_order", 0, 0).unwrap();
    let ids: Vec<&str> = snap.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["msg_u1", run.message_id.as_str()],
        "运行中 run 的占位消息必须排在触发它的用户消息之后"
    );
    // 失败且无部件的 run 同理
    finish_run(&db.lock_conn(), &run.id, "error", "", "模型调用失败").unwrap();
    let (_, seq2) = record_event(&db, &user_message("th_order", "msg_u2", "再来")).unwrap();
    let run2 = create_run(&db.lock_conn(), "th_order", "model-x", Some(seq2)).unwrap();
    finish_run(&db.lock_conn(), &run2.id, "error", "", "模型调用失败").unwrap();
    let snap = snapshot(&db.lock_conn(), "th_order", 0, 0).unwrap();
    let ids: Vec<&str> = snap.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["msg_u1", run.message_id.as_str(), "msg_u2", run2.message_id.as_str()]);
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_placeholder_ordering_is_deterministic_within_same_millisecond() {
    // 同毫秒确定性回归（V8 trigger_seq + 占位优先 tie-break）：消息落库与 run 创建
    // 常落在同一毫秒，旧的时间戳兜底（created_at <= started_at）分不出先后，占位
    // 消息会随机排到下一条用户消息之后。本用例把 run.started_at 人为对齐到消息
    // 落库毫秒，强制复现该场景，断言顺序恒定。
    let (dir, db) = tmp_db("run-order-same-ms");
    create_thread(&db.lock_conn(), thread("th_same_ms")).unwrap();
    let (_, seq1) = record_event(&db, &user_message("th_same_ms", "msg_u1", "你好")).unwrap();
    let t1: i64 = db
        .lock_conn()
        .query_row(
            "SELECT created_at FROM events WHERE message_id = 'msg_u1' AND kind = 'user_message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let run = create_run(&db.lock_conn(), "th_same_ms", "model-x", Some(seq1)).unwrap();
    db.lock_conn()
        .execute("UPDATE runs SET started_at = ?1 WHERE id = ?2", rusqlite::params![t1, run.id])
        .unwrap();
    finish_run(&db.lock_conn(), &run.id, "error", "", "模型调用失败").unwrap();
    let (_, seq2) = record_event(&db, &user_message("th_same_ms", "msg_u2", "再来")).unwrap();
    let run2 = create_run(&db.lock_conn(), "th_same_ms", "model-x", Some(seq2)).unwrap();
    // run2 同样对齐到同一毫秒（比旧时间戳兜底更苛刻）
    db.lock_conn()
        .execute("UPDATE runs SET started_at = ?1 WHERE id = ?2", rusqlite::params![t1, run2.id])
        .unwrap();
    finish_run(&db.lock_conn(), &run2.id, "error", "", "模型调用失败").unwrap();

    let snap = snapshot(&db.lock_conn(), "th_same_ms", 0, 0).unwrap();
    let ids: Vec<&str> = snap.messages.iter().map(|m| m.id.as_str()).collect();
    // run1 占位（seq=seq1+1）与 msg_u2（seq=seq2）同值时，占位优先规则保证占位在前；
    // run2 占位（seq=seq2+1）严格在 msg_u2 之后——全序确定，与毫秒采样无关。
    assert_eq!(
        ids,
        vec!["msg_u1", run.message_id.as_str(), "msg_u2", run2.message_id.as_str()],
        "同毫秒下占位消息顺序必须确定：紧跟各自触发轮次，不得漂移到后续用户消息之后"
    );
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_placeholder_falls_back_to_timestamp_for_legacy_rows() {
    // 旧行兼容：V8 之前的 run 行 trigger_seq 为 NULL，快照回退时间戳兜底——
    // started_at 严格晚于消息落库毫秒时，占位仍排在已落库消息之后。
    let (dir, db) = tmp_db("run-order-legacy");
    create_thread(&db.lock_conn(), thread("th_legacy")).unwrap();
    let (_, seq1) = record_event(&db, &user_message("th_legacy", "msg_u1", "你好")).unwrap();
    let run = create_run(&db.lock_conn(), "th_legacy", "model-x", None).unwrap();
    let t1: i64 = db
        .lock_conn()
        .query_row(
            "SELECT created_at FROM events WHERE message_id = 'msg_u1' AND kind = 'user_message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.lock_conn()
        .execute("UPDATE runs SET started_at = ?1 WHERE id = ?2", rusqlite::params![t1 + 1000, run.id])
        .unwrap();
    finish_run(&db.lock_conn(), &run.id, "error", "", "模型调用失败").unwrap();

    let snap = snapshot(&db.lock_conn(), "th_legacy", 0, 0).unwrap();
    let ids: Vec<&str> = snap.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["msg_u1", run.message_id.as_str()], "NULL trigger_seq 回退时间戳兜底，占位仍排触发消息之后");
    assert_eq!(snap.messages[1].seq, seq1 + 1, "兜底 seq = 已落库事件最大 seq + 1");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn startup_interrupt_cleanup_writes_canceled_terminal_event_for_pending_approvals() {
    let (dir, db) = tmp_db("stale-approval");
    create_thread(&db.lock_conn(), thread("th_sa")).unwrap();
    let run = create_run(&db.lock_conn(), "th_sa", "model-x", None).unwrap();
    let approval = create_approval(
        &db.lock_conn(),
        craft_store::runs::NewApproval {
            thread_id: "th_sa",
            run_id: &run.id,
            tool_call_id: "call_1",
            tool: "shell_exec",
            risk: "R2",
        },
    )
    .unwrap();
    record_event(
        &db,
        &EventRecord {
            thread_id: "th_sa".into(),
            message_id: "msg_a".into(),
            run_id: Some(run.id.clone()),
            kind: "approval".into(),
            payload: serde_json::json!({ "partId": "part_ap1", "approvalId": approval.id, "status": "waiting" }),
        },
    )
    .unwrap();

    // 回调内即落库（对齐生产 Recorder 包装：append = 落库 + 广播）
    let mut resolved_count = 0;
    craft_store::runs::interrupt_stale_runs(&db, |rec| {
        if rec.kind == "approval_resolved" {
            resolved_count += 1;
            assert_eq!(rec.payload["partId"], "part_ap1");
            assert_eq!(rec.payload["approvalId"], approval.id.as_str());
            assert_eq!(rec.payload["status"], "canceled");
        }
        let _ = record_event(&db, &rec).unwrap();
    }).unwrap();
    assert_eq!(resolved_count, 1, "应写审批取消终态事件");

    // 快照回填：卡片不再是 waiting（此前只改库不发事件，重启后卡片僵尸等待）
    let snap = snapshot(&db.lock_conn(), "th_sa", 0, 0).unwrap();
    let card = snap
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .find(|p| p.kind == "approval")
        .expect("快照应有审批卡片");
    assert_eq!(card.payload["status"], "canceled");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// worktree 绑定读写往返：落库后读取一致，重开库不丢（V10 迁移列的持久性）。
#[test]
fn worktree_binding_roundtrip_and_survives_reopen() {
    let (dir, db) = tmp_db("worktree");
    create_thread(&db.lock_conn(), thread("th_wt1")).unwrap();
    set_thread_worktree(&db.lock_conn(), "th_wt1", "/tmp/cw-wt/th_wt1", "craftwork/a1b2c3d4").unwrap();
    let row = get_thread(&db.lock_conn(), "th_wt1").unwrap();
    assert_eq!(row.worktree_path, "/tmp/cw-wt/th_wt1");
    assert_eq!(row.worktree_branch, "craftwork/a1b2c3d4");
    // 未绑定的会话字段为空串
    create_thread(&db.lock_conn(), thread("th_wt2")).unwrap();
    let plain = get_thread(&db.lock_conn(), "th_wt2").unwrap();
    assert!(plain.worktree_path.is_empty() && plain.worktree_branch.is_empty());
    drop(db);
    let reopened = Db::open(&dir.join("test.db")).unwrap();
    let again = get_thread(&reopened.lock_conn(), "th_wt1").unwrap();
    assert_eq!(again.worktree_branch, "craftwork/a1b2c3d4");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 计划模式标记还原：trigger_seq 命中读该事件；None 读最新一条用户消息；无事件/无标记 = false。
#[test]
fn plan_mode_for_trigger_reads_seq_or_latest_user_message() {
    let (dir, db) = tmp_db("planmode");
    create_thread(&db.lock_conn(), thread("th_pm")).unwrap();
    // 第一条：planMode=false；第二条：planMode=true
    let (_, seq1) = record_event(
        &db,
        &EventRecord {
            thread_id: "th_pm".into(),
            message_id: "msg_pm1".into(),
            run_id: None,
            kind: "user_message".into(),
            payload: serde_json::json!({ "text": "一", "attachments": [], "planMode": false }),
        },
    )
    .unwrap();
    let (_, seq2) = record_event(
        &db,
        &EventRecord {
            thread_id: "th_pm".into(),
            message_id: "msg_pm2".into(),
            run_id: None,
            kind: "user_message".into(),
            payload: serde_json::json!({ "text": "二", "attachments": [], "planMode": true }),
        },
    )
    .unwrap();
    assert!(plan_mode_for_trigger(&db.lock_conn(), "th_pm", Some(seq2)).unwrap());
    assert!(!plan_mode_for_trigger(&db.lock_conn(), "th_pm", Some(seq1)).unwrap());
    // regenerate（None）：最新一条用户消息带标记
    assert!(plan_mode_for_trigger(&db.lock_conn(), "th_pm", None).unwrap());
    // 老数据无 planMode 字段：false
    record_event(
        &db,
        &EventRecord {
            thread_id: "th_pm".into(),
            message_id: "msg_pm3".into(),
            run_id: None,
            kind: "user_message".into(),
            payload: serde_json::json!({ "text": "三", "attachments": [] }),
        },
    )
    .unwrap();
    assert!(!plan_mode_for_trigger(&db.lock_conn(), "th_pm", None).unwrap());
    // 无任何用户消息的会话：false
    create_thread(&db.lock_conn(), thread("th_pm_empty")).unwrap();
    assert!(!plan_mode_for_trigger(&db.lock_conn(), "th_pm_empty", None).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 快照重建透传 user_message 的 planMode（与 via 同款白名单），前端「批准并执行」依赖它。
#[test]
fn snapshot_user_message_part_carries_plan_mode() {
    let (dir, db) = tmp_db("snapplan");
    create_thread(&db.lock_conn(), thread("th_sp")).unwrap();
    record_event(
        &db,
        &EventRecord {
            thread_id: "th_sp".into(),
            message_id: "msg_sp1".into(),
            run_id: None,
            kind: "user_message".into(),
            payload: serde_json::json!({ "text": "出个计划", "attachments": [], "planMode": true }),
        },
    )
    .unwrap();
    let snap = snapshot(&db.lock_conn(), "th_sp", 0, 0).unwrap();
    let part = snap
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .find(|p| p.kind == "text")
        .expect("快照应有 user 文本部件");
    assert_eq!(part.payload["planMode"], true);
    let _ = std::fs::remove_dir_all(&dir);
}
