//! Recorder：事件写入与广播的唯一出口。
//! 「写进历史」与「推给界面」的映射只存在这一份，不会漂移。
//! 对应 src/main/core/record.ts。

use std::sync::Arc;

use serde_json::Value;

use crate::db::Db;
use crate::events::{record_event, EventRecord};
use crate::runs;
use crate::threads;
use craft_core::bus::{part_event, Bus};
use craft_core::contract::event_type;

/// Arc 持有版：可跨任务克隆（壳层与 Runner 共享同一实例）。
#[derive(Clone)]
pub struct Recorder {
    db: Arc<Db>,
    bus: Arc<Bus>,
}

impl Recorder {
    pub fn new(db: Arc<Db>, bus: Arc<Bus>) -> Self {
        Self { db, bus }
    }

    pub fn bus(&self) -> &Arc<Bus> {
        &self.bus
    }

    /// 落库 + 广播；无 PART_EVENTS 映射的 kind 只入库不广播。载荷统一补 messageId（渲染层幂等键）。
    pub fn append(&self, rec: &EventRecord) -> crate::db::StoreResult<(String, i64)> {
        let result = record_event(&self.db, rec)?;
        if let Some(broadcast_type) = part_event(&rec.kind) {
            let mut base = match &rec.payload {
                Value::Object(map) => map.clone(),
                _ => serde_json::Map::new(),
            };
            base.insert("messageId".into(), Value::String(rec.message_id.clone()));
            self.bus.emit(
                broadcast_type,
                Some(&rec.thread_id),
                rec.run_id.as_deref(),
                Value::Object(base),
            );
        }
        Ok(result)
    }

    /// 运行中部件落库：失败只降级为纯广播，不中断运行。
    pub fn append_or_warn(&self, rec: &EventRecord) {
        if let Err(_err) = self.append(rec) {
            self.bus.emit(event_type::NOTICE, Some(&rec.thread_id), rec.run_id.as_deref(), rec.payload.clone());
        }
    }

    /// 只广播（增量/生命周期事件，不入库）。
    pub fn emit(&self, event_type_str: &str, thread_id: Option<&str>, run_id: Option<&str>, payload: Value) {
        self.bus.emit(event_type_str, thread_id, run_id, payload);
    }

    /// 会话状态变更：先 DB 后广播 thread.updated。
    pub fn set_thread_status(&self, thread_id: &str, run_id: Option<&str>, status: &str) -> crate::db::StoreResult<()> {
        self.db.with(|conn| threads::set_thread_status(conn, thread_id, status))?;
        self.bus.emit(
            event_type::THREAD_UPDATED,
            Some(thread_id),
            run_id,
            serde_json::json!({ "threadId": thread_id, "status": status }),
        );
        Ok(())
    }

    /// 启动收尾（interrupt_stale_runs 的 Recorder 包装）：事务内标记中断 + 逐条广播 error 事件。
    pub fn interrupt_stale_runs(&self) -> crate::db::StoreResult<usize> {
        let mut broadcast: Vec<EventRecord> = Vec::new();
        let count = runs::interrupt_stale_runs(&self.db, |rec| {
            // 事务回调里只收集；落库与广播在回调外统一处理
            broadcast.push(rec);
        })?;
        for rec in &broadcast {
            // error 事件落库（自带事务），失败不中断启动
            let _ = self.append(rec);
        }
        Ok(count)
    }
}
