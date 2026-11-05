//! 内部事件总线：按会话分配内存内单调 seq（与 DB 的 seq 是两套，互不相干）；
//! 推送侧做 30ms 微批合并，界面层按帧渲染即可。
//!
//! push 回调由壳层提供（Tauri emit 到 craftwork:events 通道）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::contract::{Batch, Envelope};

const FLUSH_MS: u64 = 30;

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct BusState {
    seqs: HashMap<String, i64>,
    pending: Vec<Envelope>,
    /// 最早冲刷时刻（首个 pending 进入时设置）
    flush_at: Option<Instant>,
    closed: bool,
}

struct BusShared {
    state: Mutex<BusState>,
    cv: Condvar,
    push: Box<dyn Fn(Batch) + Send + Sync>,
    /// 事件全序兜底：跨会话的内存内递增（用于无 threadId 事件的排序稳定性）
    global_seq: AtomicI64,
}

/// 可克隆的事件总线句柄；冲刷线程随首次创建启动，随最后一个句柄关闭并冲刷余量。
#[derive(Clone)]
pub struct Bus {
    shared: Arc<BusShared>,
}

impl Bus {
    pub fn new(push: impl Fn(Batch) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(BusShared {
            state: Mutex::new(BusState {
                seqs: HashMap::new(),
                pending: Vec::new(),
                flush_at: None,
                closed: false,
            }),
            cv: Condvar::new(),
            push: Box::new(push),
            global_seq: AtomicI64::new(0),
        });
        // 冲刷线程：等待至最早 flush_at 时刻取走整批推送
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("craftwork-bus".into())
            .spawn(move || flush_loop(worker))
            .expect("craftwork-bus 线程启动失败");
        Self { shared }
    }

    /// 广播一个事件（立即进入合并窗口）。
    pub fn emit(&self, event_type: &str, thread_id: Option<&str>, run_id: Option<&str>, data: Value) {
        let ts = now_millis();
        let mut state = self.shared.state.lock().unwrap();
        if state.closed {
            return;
        }
        let seq = match thread_id {
            Some(tid) => {
                let next = state.seqs.get(tid).copied().unwrap_or(0) + 1;
                state.seqs.insert(tid.to_string(), next);
                next
            }
            // 无会话事件：用全局序（TS 版恒为 0；这里取负全局序避免与会话序混淆，
            // 渲染层仅在同 threadId 内使用 seq，跨会话值不参与比较）
            None => self.shared.global_seq.fetch_sub(1, Ordering::Relaxed),
        };
        state.pending.push(Envelope {
            seq,
            event_type: event_type.to_string(),
            thread_id: thread_id.map(str::to_string),
            run_id: run_id.map(str::to_string),
            ts,
            data,
        });
        if state.flush_at.is_none() {
            state.flush_at = Some(Instant::now() + Duration::from_millis(FLUSH_MS));
        }
        drop(state);
        self.shared.cv.notify_all();
    }

    /// 关闭：停止接收新事件，冲刷剩余事件（尽力而为）。
    pub fn close(&self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.closed {
                return;
            }
            state.closed = true;
            state.flush_at = Some(Instant::now());
        }
        self.shared.cv.notify_all();
        // 等待冲刷线程取走余量（closed 后 push 仍执行一次）
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let state = self.shared.state.lock().unwrap();
            if state.pending.is_empty() || Instant::now() > deadline {
                return;
            }
            let _ = self.shared.cv.wait_timeout(state, Duration::from_millis(50));
        }
    }
}

fn flush_loop(shared: Arc<BusShared>) {
    // 每轮循环：持锁判定「推送 / 等待到点 / 等待新事件」，动作在锁外执行。
    loop {
        let action: Option<Batch>;
        {
            let mut state = shared.state.lock().unwrap();
            if state.closed && state.pending.is_empty() {
                shared.cv.notify_all();
                return;
            }
            match state.flush_at {
                Some(at) if at <= Instant::now() && !state.pending.is_empty() => {
                    state.flush_at = None;
                    let events = std::mem::take(&mut state.pending);
                    let thread_id = events.first().and_then(|e| e.thread_id.clone());
                    action = Some(Batch {
                        v: 1,
                        thread_id,
                        events,
                    });
                }
                Some(at) => {
                    // 未到点：等到 at（被 emit/close 的 notify 提前唤醒则复查）
                    let wait = at.saturating_duration_since(Instant::now()).min(Duration::from_millis(50));
                    let _ = shared.cv.wait_timeout(state, wait);
                    action = None;
                }
                None => {
                    // 无排定冲刷：等新事件的 notify
                    let _ = shared.cv.wait_timeout(state, Duration::from_millis(200));
                    action = None;
                }
            }
        }
        if let Some(batch) = action {
            (shared.push)(batch);
        }
    }
}

/// 部件落库后广播哪个完成事件（原 record.partEvents，契约只存在一份）。
pub fn part_event(kind: &str) -> Option<&'static str> {
    match kind {
        "user_message" => Some(crate::contract::event_type::USER_MESSAGE),
        "text" => Some(crate::contract::event_type::TEXT_COMPLETED),
        "reasoning" => Some(crate::contract::event_type::REASONING_COMPLETED),
        "tool_call" => Some(crate::contract::event_type::TOOL_COMPLETED),
        "approval" => Some(crate::contract::event_type::APPROVAL_REQUESTED),
        // 审批终态：落库（快照回填状态）+ 广播（前端翻卡）。只 emit 不落库会让
        // 重启后的已决定审批卡回显「等待决定」成僵尸卡（migration-parity 票 03）。
        "approval_resolved" => Some(crate::contract::event_type::APPROVAL_RESOLVED),
        "todo" => Some(crate::contract::event_type::TODO_UPDATED),
        "error" => Some(crate::contract::event_type::ERROR),
        "notice" => Some(crate::contract::event_type::NOTICE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn 事件经微批合并推送_会话seq单调递增() {
        let (tx, rx) = mpsc::channel::<Batch>();
        let bus = Bus::new(move |batch| tx.send(batch).unwrap());
        bus.emit("text.delta", Some("t1"), None, serde_json::json!({"a": 1}));
        bus.emit("text.delta", Some("t1"), None, serde_json::json!({"a": 2}));
        bus.emit("notice", None, None, serde_json::json!("hi"));
        // 连续 emit 在同一 30ms 窗口内：合成一批
        let batch = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(batch.events.len(), 3);
        assert_eq!(batch.events[0].seq, 1);
        assert_eq!(batch.events[1].seq, 2);
        assert_eq!(batch.thread_id.as_deref(), Some("t1"));
        // 窗口关闭后无新事件：不再有批次
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn close_冲刷剩余事件() {
        let (tx, rx) = mpsc::channel::<Batch>();
        let bus = Bus::new(move |batch| tx.send(batch).unwrap());
        bus.emit("notice", Some("t2"), None, serde_json::json!("x"));
        bus.close();
        let batch = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(batch.events.len(), 1);
    }

    #[test]
    fn part_event_映射完整() {
        assert_eq!(part_event("user_message"), Some("user.message"));
        assert_eq!(part_event("tool_call"), Some("tool.completed"));
        assert_eq!(part_event("draft"), None);
    }
}

