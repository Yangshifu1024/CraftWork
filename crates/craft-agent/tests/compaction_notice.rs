//! 上下文压缩事件的落点契约（回归测试）。
//!
//! 背景（真实缺陷）：压缩通知曾用 `new_id()` 自成一条新消息，它的 seq 取事件 seq
//! （时间线最末端）、消息内又没有正文部件，渲染层因此把它当成「尾段」永久固定在
//! 对话流底部——压缩提示与它描述的那段历史完全脱节。
//! 修正后 notice 挂在本轮正在写的 assistant 消息上，随该消息的折叠区落位。
//!
//! 这里直接用 store 层验证落库契约（不需要跑完整 run）：压缩写入的三条事件中，
//! 只有 notice 会产生可见部件，它必须落在传入的 message_id 上。

use std::sync::Arc;

use craft_core::bus::Bus;
use craft_core::contract::Snapshot;
use craft_model::types::{Chunk, Client, Message, Request};
use craft_store::db::Db;
use craft_store::events::snapshot;
use craft_store::recorder::Recorder;

/// 假客户端：压缩 sidecar 只要求流式返回一段文本。
struct SummaryClient;

impl Client for SummaryClient {
    fn stream<'a>(
        &'a self,
        _req: &'a Request,
    ) -> futures_core::stream::BoxStream<'a, Result<Chunk, craft_model::errors::ModelError>> {
        Box::pin(futures_util::stream::iter(vec![Ok(Chunk {
            text: Some("早期对话的摘要".into()),
            reasoning: None,
            tool_call: None,
            usage: None,
            done: true,
            end: None,
        })]))
    }
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(SummaryClient)
    }
}

/// 装配一个隔离库，塞入足够长的历史让压缩必然触发。
fn seeded_db(thread_id: &str, message_id: &str) -> (tempfile::TempDir, Arc<Db>) {
    let dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    db.with(|conn| {
        craft_store::threads::create_thread(
            conn,
            craft_store::threads::NewThread {
                id: thread_id,
                mode: "code",
                title: "压缩落点",
                title_auto: false,
                model_id: "stub/m1",
                reasoning_level: "",
                workspace_id: "",
                status: "idle",
                archived: false,
            },
        )
        .unwrap();
        // 30 条大消息：远超 3000 窗口的 70%
        for i in 0..30 {
            let message_id_owned = if i % 2 == 0 { format!("msg_u{i}") } else { message_id.to_string() };
            craft_store::events::append_event(
                conn,
                &craft_store::events::EventRecord {
                    thread_id: thread_id.into(),
                    message_id: message_id_owned,
                    run_id: None,
                    kind: if i % 2 == 0 { "user_message".into() } else { "text".into() },
                    payload: serde_json::json!({ "text": "汉".repeat(200) }),
                },
            )
            .unwrap();
        }
        Ok(())
    })
    .unwrap();
    (dir, db)
}

#[test]
fn compression_notice_lands_on_current_run_message() {
    let thread_id = "th_compact";
    let run_message_id = "msg_run";
    let (_dir, db) = seeded_db(thread_id, run_message_id);
    let recorder = Recorder::new(Arc::clone(&db), Arc::new(Bus::new(|_| {})));
    let snap: Snapshot = db.with(|conn| snapshot(conn, thread_id, 0, 0)).unwrap();

    let messages: Vec<Message> = (0..30).map(|i| Message::new("user", format!("{i}：{}", "汉".repeat(200)))).collect();
    let after = craft_agent::context::run_compression_with(
        craft_agent::context::CompressionArgs {
            db: &db,
            recorder: &recorder,
            snap: &snap,
            thread_id,
            run_id: "run_compact",
            message_id: run_message_id,
            client: &SummaryClient,
            model_id: "stub/m1",
            context_k: 3,
            approved_plan: None,
            measured_prompt_tokens: 0,
        },
        messages,
    );
    assert!(after.len() < 30, "应发生压缩");

    // notice 必须落在本轮 assistant 消息上，而不是自成一条新消息
    let rows: Vec<(String, String)> = db
        .with(|conn| {
            let mut stmt = conn
                .prepare("SELECT message_id, payload FROM events WHERE thread_id = ?1 AND kind = 'notice'")
                .map_err(craft_store::db::other_err)?;
            let rows = stmt
                .query_map([thread_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(craft_store::db::other_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(craft_store::db::other_err)?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(rows.len(), 1, "压缩应写一条 notice");
    assert_eq!(rows[0].0, run_message_id, "notice 的 message_id 必须是本轮 assistant 消息");
    let payload: serde_json::Value = serde_json::from_str(&rows[0].1).unwrap();
    assert!(payload["text"].as_str().unwrap().contains("上下文已压缩"));

    // 快照里：notice 作为本轮消息的过程行存在，且没有新的孤立消息被造出来
    let snap_after: Snapshot = db.with(|conn| snapshot(conn, thread_id, 0, 0)).unwrap();
    let run_msg = snap_after
        .messages
        .iter()
        .find(|m| m.id == run_message_id)
        .expect("本轮 assistant 消息应在快照里");
    assert!(
        run_msg.parts.iter().any(|p| p.kind == "notice"),
        "notice 部件应落在本轮消息内，实际部件：{:?}",
        run_msg.parts.iter().map(|p| &p.kind).collect::<Vec<_>>()
    );
    // 消息条数保持原样（notice 没有新增消息）
    let user_like = snap_after.messages.iter().filter(|m| m.role == "user").count();
    assert_eq!(user_like, 15, "不应多出孤立消息");
}
