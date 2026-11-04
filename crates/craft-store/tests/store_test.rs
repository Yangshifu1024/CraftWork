//! 存储层测试：迁移、事件 seq、快照重建、审批并发保护、启动收尾。
//! 用例语义对应 tests/store.test.ts（vitest 版的 Rust 翻译）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use craft_store::db::{Db, StoreError, SCHEMA_VERSION};
use craft_store::events::{append_event, latest_context_summary, record_event, snapshot, EventRecord};
use craft_store::records::{add_audit, get_setting, list_audit, set_setting};
use craft_store::runs::{create_approval, create_run, finish_run, get_approval, resolve_approval};
use craft_store::threads::{
    auto_title_thread, create_thread, delete_thread, get_thread, list_threads, rename_thread, search_threads, NewThread,
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

#[test]
fn 迁移到当前版本且可重开() {
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

#[test]
fn v7空会话清理语义_删空自动命名_保留有事件与手动改名() {
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
fn threads基本读写与删除() {
    let (dir, db) = tmp_db("threads");
    let t = create_thread(&db.lock_conn(), thread("th_1")).unwrap();
    assert_eq!(get_thread(&db.lock_conn(), &t.id).unwrap().title, "新对话");
    rename_thread(&db.lock_conn(), &t.id, "改名").unwrap();
    assert!(!get_thread(&db.lock_conn(), &t.id).unwrap().title_auto);
    delete_thread(&db, &t.id).unwrap();
    assert!(matches!(get_thread(&db.lock_conn(), &t.id), Err(StoreError::NotFound(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn autoTitle条件更新只在标题未被改动时生效() {
    let (dir, db) = tmp_db("autotitle");
    create_thread(&db.lock_conn(), thread("th_2")).unwrap();
    assert!(auto_title_thread(&db.lock_conn(), "th_2", "新标题", "新对话").unwrap());
    assert!(!auto_title_thread(&db.lock_conn(), "th_2", "再改", "新对话").unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 事件seq单调递增且快照重建消息() {
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

#[test]
fn 审批并发保护与重复决定拒绝() {
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

#[test]
fn 启动收尾_中断运行标记interrupted并写错误事件() {
    let (dir, db) = tmp_db("stale");
    create_thread(
        &db.lock_conn(),
        NewThread { id: "th_5", status: "running", ..thread("x") },
    )
    .unwrap();
    let run = create_run(&db.lock_conn(), "th_5", "gpt-x").unwrap();
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
fn run完成回填状态() {
    let (dir, db) = tmp_db("runfinish");
    create_thread(&db.lock_conn(), thread("th_6")).unwrap();
    let run = create_run(&db.lock_conn(), "th_6", "gpt-x").unwrap();
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
fn 工作区按path幂等upsert() {
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
fn 文件夹整体替换并同步主目录() {
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
fn 默认临时会话工作区解析_缺失时回落空串() {
    let (dir, db) = tmp_db("wsdefault");
    // 无 conversation 工作区时回落空串
    assert_eq!(default_conversation_workspace_id(&db.lock_conn()), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 设置upsert与读取() {
    let (dir, db) = tmp_db("settings");
    set_setting(&db.lock_conn(), "app", &serde_json::json!({ "theme": "dark" })).unwrap();
    assert_eq!(get_setting(&db.lock_conn(), "app").unwrap().unwrap()["theme"], "dark");
    set_setting(&db.lock_conn(), "app", &serde_json::json!({ "theme": "light" })).unwrap();
    assert_eq!(get_setting(&db.lock_conn(), "app").unwrap().unwrap()["theme"], "light");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 审计追加与倒序列表() {
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
fn 会话列表按模式过滤() {
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
fn 上下文摘要_最新一条自然淘汰旧摘要() {
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
fn append_event便捷入口_经record_event验证() {
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
