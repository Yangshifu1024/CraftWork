//! 后台命令任务注册表。
//!
//! `shell_exec` 的 `run_in_background` 与「前台超时转后台」两条路径都把进程交给这里托管：
//! 进程已脱离工具调用返回，生命周期由本模块记账。设计取舍（运行期任务注册表 +
//! 后台命令生命周期，见 `crates/craft-agent/src/shell_tasks.rs` 模块头）：
//!
//! - **只记账，不持有等待线程**：进程自己跑，输出直写文件。等待者（Runner）在任务终态时
//!   收到通知——用轮询等待会白占一个线程，且应用退出时会挂住。
//! - **进程组整组归属**：启动时 `process_group(0)`，终止走 `kill_process_group(pid)`，
//!   孙进程一并回收。
//! - **终态只结算一次**：`settle` 用状态检查做幂等，重复结算返回 None，避免同一条完成
//!   通知被重复投递（并发轮询与显式 kill 会同时到达）。
//! - **输出文件是唯一输出载体**：不缓存全文，读取时按需截断。进程退出后文件保留，
//!   供事后查看与终态通知引用。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 后台任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// 进程仍在跑（或尚未被观察到退出）
    Running,
    /// 正常退出（退出码可为非零——非零退出是命令结果，不是任务失败）
    Completed,
    /// 被 `kill_shell` 或应用退出终止
    Killed,
    /// 启动失败 / 观察失败（拿不到退出码）
    Failed,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, TaskState::Running)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Running => "running",
            TaskState::Completed => "completed",
            TaskState::Killed => "killed",
            TaskState::Failed => "failed",
        }
    }
}

/// 一条后台任务记录。
#[derive(Debug, Clone)]
pub struct ShellTask {
    /// `bash_<8hex>`；模型经 `bash_output` / `kill_shell` 引用它
    pub id: String,
    /// 所属会话（跨会话不允许读取/终止，防串号）
    pub thread_id: String,
    /// 进程组组长 pid
    pub pid: u32,
    /// 输出文件绝对路径（stdout + stderr 合并）
    pub output_path: String,
    /// 一行命令摘要（通知与工具卡展示用）
    pub command: String,
    pub state: TaskState,
    /// 退出码；`Killed` / 启动失败时为 None
    pub exit_code: Option<i32>,
    pub started_at: i64,
    /// 终态时刻；running 时为 0
    pub finished_at: i64,
    /// 是否由「前台超时转后台」产生（文案区分：超时转后台要说明原因）
    pub from_timeout: bool,
    /// 是否已投递过完成通知（幂等标记）
    pub notified: bool,
}

impl ShellTask {
    pub fn duration_ms(&self) -> i64 {
        let end = if self.finished_at > 0 { self.finished_at } else { now_millis() };
        (end - self.started_at).max(0)
    }
}

/// 任务注册表：进程句柄之外的唯一事实源。
///
/// 同时持有「已完成但还没被消费的通知」队列——结构照抄子代理的报告队列
/// （`craft_agent::subagent::SubagentRegistry::take_reports`）：等待线程只负责
/// 把通知书塞进队列，注入历史的责任完全由 Runner 在它的固定注入点完成。
/// 这样跨 crate 只传递纯数据（`ShellNotification`），不传进程句柄。
/// 通知到达时的唤醒回调类型：入参是会话 id。
pub type WakeHook = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
pub struct ShellTasks {
    tasks: Mutex<HashMap<String, ShellTask>>,
    /// 待投递的终态通知（按到达顺序）
    notifications: Mutex<Vec<ShellNotification>>,
    /// 通知到达时的唤醒回调（由 Runner 装配）：把「有任务完成」这件事推给会话，
    /// 而不是让会话在收尾时反复猜测。craft-tools 不认识 Runner，所以只存一个闭包。
    wake: Mutex<Option<WakeHook>>,
}

/// 后台任务终态通知：Runner 取走后注入会话历史并唤醒。
#[derive(Debug, Clone)]
pub struct ShellNotification {
    pub task_id: String,
    pub thread_id: String,
    pub command: String,
    pub state: TaskState,
    pub exit_code: Option<i32>,
    pub duration_ms: i64,
    pub output_path: String,
    /// 是否由前台超时转后台（文案要说明原因）
    pub from_timeout: bool,
}

impl ShellNotification {
    /// 模型可见的通知正文：短、结构化、只给路径不灌全文。
    ///
    /// 与同类后台任务通知同族但更简：只保留「哪个任务、结果如何、
    /// 输出去哪看」，不含 stdout 正文——日志可能几十 MB，灌进上下文就是灾难。
    pub fn render(&self) -> String {
        let status = match self.state {
            TaskState::Completed => match self.exit_code {
                Some(0) => "已完成（退出码 0）".to_string(),
                Some(code) => format!("已完成，但退出码 {code}（命令自身失败）"),
                None => "已结束".to_string(),
            },
            TaskState::Killed => "已被终止".to_string(),
            TaskState::Failed => "启动或观察失败".to_string(),
            TaskState::Running => "仍在运行".to_string(),
        };
        let origin = if self.from_timeout {
            "后台命令（前台超时转后台）"
        } else {
            "后台命令"
        };
        format!(
            "<task-notification>\n\
             <task-id>{}</task-id>\n\
             <status>{}</status>\n\
             <command>{}</command>\n\
             <output-file>{}</output-file>\n\
             </task-notification>\n\
             {origin}「{}」{status}，耗时 {}s。完整输出在 {}；需要看细节用 bash_output（task_id = {}）。\n\
             这是系统自动投递的任务完成通知，内容不在用户消息里。",
            self.task_id,
            self.state.as_str(),
            self.command,
            self.output_path,
            self.command,
            self.duration_ms / 1000,
            self.output_path,
            self.task_id
        )
    }
}

impl ShellTasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条刚启动的后台任务。
    pub fn register(&self, task: ShellTask) {
        self.tasks.lock().unwrap().insert(task.id.clone(), task);
    }

    /// 启动一条任务的等待线程：阻塞在 `child.wait()` 上，进程一退出就结算并投递通知。
    ///
    /// 为什么用线程而不是轮询：`Child::wait` 是内核级等待，进程退出立即返回；
    /// 轮询则要么浪费 CPU，要么把通知延迟到一个轮询周期。代价是每个后台任务一个
    /// 阻塞线程——后台任务本就是低频操作（一次会话通常个位数），可以接受。
    ///
    /// 线程持有 `Arc<ShellTasks>`：注册表活多久线程就能安全写多久，不会悬垂。
    pub fn watch(self: &Arc<Self>, mut child: std::process::Child, task: ShellTask) {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let status = child.wait();
            let (state, exit_code) = match status {
                Ok(st) => (
                    TaskState::Completed,
                    // 被信号终止时 code() 为 None：按失败语义归一（进程没跑完）
                    st.code(),
                ),
                Err(_) => (TaskState::Failed, None),
            };
            // settle 幂等：若 kill_shell 已抢先结算成 Killed，这里返回 None，
            // 不再重复投递通知（用户主动杀的，结果已在 kill_shell 的结果里）。
            if let Some(settled) = this.settle(&task.id, state, exit_code) {
                this.push_notification(ShellNotification {
                    task_id: settled.id.clone(),
                    thread_id: settled.thread_id.clone(),
                    command: settled.command.clone(),
                    state: settled.state,
                    exit_code: settled.exit_code,
                    duration_ms: settled.duration_ms(),
                    output_path: settled.output_path.clone(),
                    from_timeout: settled.from_timeout,
                });
                this.mark_notified(&settled.id);
            }
        });
    }

    /// 投递一条终态通知到待消费队列，并立刻通知唤醒回调。
    ///
    /// 为什么必须在这里推：后台任务结束的时刻与会话 run 的收尾时刻完全无关——
    /// 命令可能在会话空闲很久之后才结束。只在 run 收尾处检查队列，就只有在
    /// 「命令恰好在 run 期间结束」时才会被消费；其余情况要等到用户下次发言，
    /// 那正是本功能要消灭的体验。
    fn push_notification(&self, notification: ShellNotification) {
        let thread_id = notification.thread_id.clone();
        self.notifications.lock().unwrap().push(notification);
        let wake = self.wake.lock().unwrap().clone();
        if let Some(wake) = wake {
            // 回调在等待线程里执行：它只负责起 run，不能在这里做重活
            wake(&thread_id);
        }
    }

    /// 装配唤醒回调（Runner 在构造时调用一次；重复调用以后者为准）。
    pub fn set_wake_hook(&self, hook: WakeHook) {
        *self.wake.lock().unwrap() = Some(hook);
    }

    /// 取走某会话的全部待投递通知（Runner 在注入点消费）。
    pub fn take_notifications(&self, thread_id: &str) -> Vec<ShellNotification> {
        let mut guard = self.notifications.lock().unwrap();
        let mut taken = Vec::new();
        let mut kept = Vec::with_capacity(guard.len());
        for item in guard.drain(..) {
            if item.thread_id == thread_id {
                taken.push(item);
            } else {
                kept.push(item);
            }
        }
        *guard = kept;
        taken
    }

    /// 某会话是否有待投递通知（空闲唤醒的判定条件）。
    pub fn has_notifications(&self, thread_id: &str) -> bool {
        self.notifications.lock().unwrap().iter().any(|n| n.thread_id == thread_id)
    }

    /// 按会话取任务（跨会话取不到：防串号读别人的输出）。
    pub fn get(&self, thread_id: &str, task_id: &str) -> Option<ShellTask> {
        let guard = self.tasks.lock().unwrap();
        guard.get(task_id).filter(|t| t.thread_id == thread_id).cloned()
    }

    /// 某会话下全部仍在跑的任务（取消级联、退出清理、状态展示用）。
    pub fn running_of(&self, thread_id: &str) -> Vec<ShellTask> {
        let guard = self.tasks.lock().unwrap();
        guard
            .values()
            .filter(|t| t.thread_id == thread_id && t.state == TaskState::Running)
            .cloned()
            .collect()
    }

    /// 全部仍在跑的任务（应用退出清理用）。
    pub fn running_all(&self) -> Vec<ShellTask> {
        let guard = self.tasks.lock().unwrap();
        guard.values().filter(|t| t.state == TaskState::Running).cloned().collect()
    }

    /// 结算为终态，返回更新后的记录；**幂等**——已终态的任务再次结算返回 None。
    ///
    /// 幂等不是洁癖：轮询观察与显式 `kill_shell` 会几乎同时到达同一任务，
    /// 没有这道检查就会给模型投递两条重复的完成通知。
    pub fn settle(&self, task_id: &str, state: TaskState, exit_code: Option<i32>) -> Option<ShellTask> {
        let mut guard = self.tasks.lock().unwrap();
        let task = guard.get_mut(task_id)?;
        if task.state.is_terminal() {
            return None;
        }
        task.state = state;
        task.exit_code = exit_code;
        task.finished_at = now_millis();
        Some(task.clone())
    }

    /// 标记通知已投递（防止同一终态重复唤醒）。
    pub fn mark_notified(&self, task_id: &str) -> bool {
        let mut guard = self.tasks.lock().unwrap();
        match guard.get_mut(task_id) {
            Some(task) if !task.notified => {
                task.notified = true;
                true
            }
            _ => false,
        }
    }

    /// 启动时把上一进程遗留的 running 记录改成 Killed（进程已随旧进程消失，
    /// 假装还在跑会让模型永远等一个不会来的通知）。
    pub fn reap_stale(&self) -> usize {
        let mut guard = self.tasks.lock().unwrap();
        let mut count = 0;
        for task in guard.values_mut() {
            if task.state == TaskState::Running {
                task.state = TaskState::Killed;
                task.finished_at = now_millis();
                count += 1;
            }
        }
        count
    }

    /// 丢弃全部已终态且已通知的记录（避免长期运行后无限增长）。
    pub fn prune_finished(&self) -> usize {
        let mut guard = self.tasks.lock().unwrap();
        let before = guard.len();
        guard.retain(|_, t| t.state == TaskState::Running || !t.notified);
        before - guard.len()
    }
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 生成任务 id：`bash_<16hex>`（前缀登记在 [`craft_core::ids::id_prefix::SHELL_TASK`]）。
pub fn new_task_id() -> String {
    craft_core::ids::new_id(craft_core::ids::id_prefix::SHELL_TASK)
}

/// 判定命令是否「首词就是 sleep」。
///
/// 命中时不享受「前台超时自动转后台」：那是拿 sleep 硬等外部任务的典型形态，
/// 转后台只会把空转从可见变不可见。判据是「比命令首词、不比整串」，已知边界是
/// `sh -c 'sleep 10'` 这类会绕开——见 spec「已知边界」。
pub fn is_sleep_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    let first = trimmed.split_whitespace().next().unwrap_or_default();
    // 兼容 `sleep` 带路径写法（/bin/sleep、/usr/bin/sleep）
    let name = first.rsplit('/').next().unwrap_or(first);
    name == "sleep"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, state: TaskState) -> ShellTask {
        ShellTask {
            id: id.into(),
            thread_id: "th_1".into(),
            pid: 1234,
            output_path: "/tmp/x.log".into(),
            command: "echo hi".into(),
            state,
            exit_code: None,
            started_at: 1000,
            finished_at: 0,
            from_timeout: false,
            notified: false,
        }
    }

    #[test]
    fn settle_is_idempotent_so_notifications_are_not_duplicated() {
        let tasks = ShellTasks::new();
        tasks.register(task("bash_a", TaskState::Running));
        assert!(tasks.settle("bash_a", TaskState::Completed, Some(0)).is_some());
        // 第二次结算（并发轮询与显式 kill 同时到达）必须返回 None
        assert!(tasks.settle("bash_a", TaskState::Killed, None).is_none());
        assert_eq!(tasks.get("th_1", "bash_a").unwrap().state, TaskState::Completed);
    }

    #[test]
    fn task_is_scoped_to_its_thread() {
        let tasks = ShellTasks::new();
        tasks.register(task("bash_a", TaskState::Running));
        assert!(tasks.get("th_1", "bash_a").is_some());
        // 跨会话读取一律落空：防串号拿到别的会话的输出
        assert!(tasks.get("th_2", "bash_a").is_none());
    }

    #[test]
    fn reap_stale_turns_leftover_running_into_killed() {
        let tasks = ShellTasks::new();
        tasks.register(task("bash_a", TaskState::Running));
        tasks.register(task("bash_b", TaskState::Completed));
        assert_eq!(tasks.reap_stale(), 1);
        assert_eq!(tasks.get("th_1", "bash_a").unwrap().state, TaskState::Killed);
        // 已终态的不会被改动
        assert_eq!(tasks.get("th_1", "bash_b").unwrap().state, TaskState::Completed);
    }

    #[test]
    fn mark_notified_allows_exactly_one_notification() {
        let tasks = ShellTasks::new();
        tasks.register(task("bash_a", TaskState::Running));
        assert!(tasks.mark_notified("bash_a"));
        assert!(!tasks.mark_notified("bash_a"), "第二次必须被拒绝");
    }

    #[test]
    fn sleep_detection_matches_first_token_only() {
        assert!(is_sleep_command("sleep 90"));
        assert!(is_sleep_command("  sleep   90  "));
        assert!(is_sleep_command("/bin/sleep 5"));
        assert!(!is_sleep_command("gh run watch 123"));
        assert!(!is_sleep_command("echo sleep"));
        // 已知边界：包在 sh -c 里绕开（与判据同口径，spec 有记录）
        assert!(!is_sleep_command("sh -c 'sleep 10'"));
        assert!(!is_sleep_command(""));
    }
}
