//! golden 数据库兼容测试：Electron 版（Node）生成的样本库在 Rust 侧原位打开、
//! 版本一致、快照重建结果一致。样本由 scripts/golden-db.mjs 生成（迁移链与写入全部走 Node 版代码）。

use std::path::PathBuf;

use craft_store::db::{Db, SCHEMA_VERSION};
use craft_store::events::snapshot;
use craft_store::records::get_setting;
use craft_store::threads::{get_thread, list_threads};
use craft_store::workspaces::list_workspaces;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/craftwork-golden.db")
}

fn copy_to_tmp() -> PathBuf {
    // 不改动 fixtures 原件：拷贝到临时目录再打开（打开会建 WAL 文件）
    let tmp = std::env::temp_dir().join(format!(
        "cw-golden-copy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let dest = tmp.join("craftwork-golden.db");
    std::fs::copy(golden_path(), &dest).unwrap();
    dest
}

#[test]
fn golden库_版本一致且会话列表可读() {
    let path = copy_to_tmp();
    let db = Db::open(&path).unwrap();
    let version: i64 = db
        .with(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0)))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION, "Node 版迁移链产物应恰为当前 schema 版本");

    // 会话两条，按 updated_at DESC
    let threads = list_threads(&db.lock_conn(), "code", false).unwrap();
    assert_eq!(threads.len(), 2);
    let named = threads.iter().find(|t| t.id == "th_gold1").unwrap();
    assert_eq!(named.title, "示例会话");
    assert_eq!(named.model_id, "openai/gpt-4o");

    // 工作区含主文件夹（V6 迁移产物）
    let workspaces = list_workspaces(&db.lock_conn()).unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].workspace.name, "示例项目");
    assert_eq!(workspaces[0].workspace.trust, "trusted");
    assert!(workspaces[0].folders.iter().any(|f| f.path == "/tmp/craftwork-demo"));

    // 设置 JSON 完整可读
    let settings = get_setting(&db.lock_conn(), "app").unwrap().unwrap();
    assert_eq!(settings["theme"], "dark");
}

#[test]
fn golden库_快照重建与Node侧基准一致() {
    let path = copy_to_tmp();
    let db = Db::open(&path).unwrap();
    let snap = snapshot(&db.lock_conn(), "th_gold1", 0, 0).unwrap();

    // Node 侧基准（scripts/golden-db.mjs 输出）：
    // events: msg_g1/user_message@1, msg_g2/text@2, msg_g2/tool_call@3
    assert_eq!(snap.messages.len(), 2);
    let user_msg = &snap.messages[0];
    assert_eq!(user_msg.id, "msg_g1");
    assert_eq!(user_msg.role, "user");
    assert_eq!(user_msg.parts.len(), 1);
    assert_eq!(user_msg.parts[0].payload["text"], "帮我看看这个仓库");

    let assistant = &snap.messages[1];
    assert_eq!(assistant.id, "msg_g2");
    assert_eq!(assistant.role, "assistant");
    assert_eq!(assistant.parts.iter().map(|p| p.kind.as_str()).collect::<Vec<_>>(), vec!["text", "tool"]);
    assert_eq!(assistant.seq, 3);
    // run done → 消息 complete，且有 workDurationMs（run started/finished 差值）
    assert_eq!(assistant.status, "complete");
    assert!(assistant.work_duration_ms.is_some());
    assert_eq!(snap.last_seq, 3);

    // 会话 2：已决定审批不出现在 pending
    let snap2 = snapshot(&db.lock_conn(), "th_gold2", 0, 0).unwrap();
    assert!(snap2.pending_approvals.is_empty());

    // 上下文占用来自 usage 行（input+output = 120+45）
    assert_eq!(snap.thread.context_tokens, 165);
    // 手动改名会话标题保留
    assert_eq!(get_thread(&db.lock_conn(), "th_gold2").unwrap().title, "被手动改名的会话");
}
