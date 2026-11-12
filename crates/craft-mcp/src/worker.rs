//! MCP 客户端运行时：一个专属操作系统线程 + 单线程 tokio 运行时，承载全部异步 SDK 调用。
//!
//! 为什么单独开线程：CraftWork 的工具层是同步 `fn execute`，由 Runner 放进
//! `spawn_blocking`；而 rmcp 是异步 SDK。若直接在阻塞线程里 `block_on`，一旦该线程
//! 本身就处于某个 runtime 上下文就会 panic。把异步世界收进自己的线程后，工具层只经
//! `std::sync::mpsc` 收发命令，与调用方在不在 runtime 里完全无关。
//!
//! 命令与结果都是纯数据（`Command` / `Reply`），异步连接对象（rmcp 的 `RunningService`）
//! 只存在于本线程内部，不跨越线程边界发送。

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

/// 一次工具调用的结果快照（可跨线程传输，不含任何异步句柄）。
#[derive(Debug, Clone, Default)]
pub struct CallOutcome {
    /// 文本结果（已按 MCP 语义拼接）
    pub text: String,
    /// 图片等媒体（见 media 模块）
    pub media: Vec<crate::media::ToolMedia>,
    pub is_error: bool,
}

/// 服务器的在连接结果：工具描述符与服务器自述信息。
#[derive(Debug, Clone, Default)]
pub struct ServerSnapshot {
    pub server_name: String,
    pub server_version: String,
    pub protocol_version: String,
    /// 服务器在 initialize 响应里给的说明（仅记录，是否注入提示词由上层决定）
    pub instructions: String,
    pub tools: Vec<ToolDescriptor>,
}

/// 从 MCP `tools/list` 取出的工具描述符（已归一为纯数据）。
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    /// 服务器侧原始工具名（调用时回传给服务器）
    pub tool_name: String,
    pub title: Option<String>,
    pub description: String,
    /// 输入 schema（原样，未做 object 强制——归一在工具层做）
    pub input_schema: Value,
    pub read_only_hint: bool,
    pub destructive_hint: bool,
    pub idempotent_hint: bool,
    /// 工具级 `_meta.timeoutMs` 覆盖（毫秒）
    pub meta_timeout_ms: Option<i64>,
}

/// 发往 runtime 线程的命令。
pub enum Command {
    /// 建立连接并拉取工具列表
    Connect {
        config: crate::config::ServerConfig,
        reply: Sender<Result<ServerSnapshot, String>>,
    },
    /// 调用工具
    Call {
        server_id: String,
        tool_name: String,
        arguments: Value,
        timeout_ms: i64,
        reply: Sender<Result<CallOutcome, String>>,
    },
    /// 该服务器是否仍处于连接状态（存活探测）
    IsAlive {
        server_id: String,
        reply: Sender<bool>,
    },
    /// 断开并清理一个服务器
    Disconnect {
        server_id: String,
        reply: Sender<()>,
    },
    /// 断开全部（应用退出）
    CloseAll {
        reply: Sender<()>,
    },
    /// 线程退出
    Shutdown,
}

/// runtime 线程句柄：跨线程可克隆共享，内部经 mpsc 串行化全部命令。
#[derive(Clone)]
pub struct McpRuntime {
    tx: Arc<Mutex<Option<Sender<Command>>>>,
}

impl McpRuntime {
    /// 启动 runtime 线程。线程内部 panic 不会波及调用方，后续命令会收到「已退出」错误。
    pub fn spawn() -> Self {
        let (tx, rx) = channel::<Command>();
        thread::Builder::new()
            .name("craftwork-mcp".into())
            .spawn(move || run_thread(rx))
            .expect("craftwork-mcp 线程启动失败");
        Self { tx: Arc::new(Mutex::new(Some(tx))) }
    }

    fn send(&self, command: Command) -> Result<(), String> {
        let guard = self.tx.lock().unwrap();
        match guard.as_ref() {
            Some(tx) => tx.send(command).map_err(|_| "MCP 运行时已退出".to_string()),
            None => Err("MCP 运行时已关闭".to_string()),
        }
    }

    /// 通用命令-应答：发出命令后阻塞等待应答。
    pub fn request<R>(
        &self,
        build: impl FnOnce(Sender<R>) -> Command,
        timeout_ms: i64,
    ) -> Result<R, String> {
        let (reply_tx, reply_rx) = channel::<R>();
        self.send(build(reply_tx))?;
        // 应答超时在调用方语义上属于「服务器没响应」；多留 5 秒给 runtime 侧收尾
        let wait = std::time::Duration::from_millis(timeout_ms.max(1) as u64 + 5_000);
        reply_rx.recv_timeout(wait).map_err(|_| "等待 MCP 运行时应答超时".to_string())
    }

    /// 通知型命令（不需要应答内容，但仍等一个完成信号以保证顺序）。
    pub fn notify(&self, build: impl FnOnce(Sender<()>) -> Command, timeout_ms: i64) -> Result<(), String> {
        self.request(build, timeout_ms)
    }

    /// 关闭 runtime：断开全部连接并结束线程。
    ///
    /// CloseAll 的等待超时必须盖过逐个断开的实际耗时（单个最坏约 15 秒，
    /// 见 client::disconnect）：等待提前结束的话调用方（退出回调）返回后进程终结，
    /// runtime 线程被强杀，剩余子进程成孤儿。极慢的收尾仍由队列里排队的 Shutdown
    /// 命令兜底——线程处理完 CloseAll 后会收到它并自行退出。
    pub fn shutdown(&self) {
        let _ = self.notify(|reply| Command::CloseAll { reply }, 30_000);
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(Command::Shutdown);
        }
    }
}

/// runtime 线程主体：建单线程 runtime，跑命令循环。
fn run_thread(rx: Receiver<Command>) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return,
    };
    runtime.block_on(async move {
        let mut state = crate::client::RuntimeState::default();
        while let Ok(command) = rx.recv() {
            match command {
                Command::Shutdown => break,
                Command::CloseAll { reply } => {
                    state.close_all().await;
                    let _ = reply.send(());
                }
                Command::Disconnect { server_id, reply } => {
                    state.disconnect(&server_id).await;
                    let _ = reply.send(());
                }
                Command::IsAlive { server_id, reply } => {
                    let _ = reply.send(state.is_alive(&server_id));
                }
                Command::Connect { config, reply } => {
                    let id = config.id.clone();
                    let result = state.connect(config).await;
                    // 连接失败不留在表里，避免后续调用拿到半死连接
                    if result.is_err() {
                        state.disconnect(&id).await;
                    }
                    let _ = reply.send(result);
                }
                Command::Call { server_id, tool_name, arguments, timeout_ms, reply } => {
                    let result = state.call(&server_id, &tool_name, arguments, timeout_ms).await;
                    let _ = reply.send(result);
                }
            }
        }
        // 线程退出前兜底清理（CloseAll 已清过一次，重复调用无害）
        state.close_all().await;
    });
}
