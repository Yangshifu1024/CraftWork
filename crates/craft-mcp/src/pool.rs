//! 服务器池：把「配置 + 运行时状态」收成一个可被壳层与工具层共用的句柄。
//!
//! 分层：本模块只做编排与状态缓存（纯同步、无锁竞争热点），异步细节全在 runtime
//! 线程（worker）与 client 里。工具层拿到 `Arc<McpPool>` 就能调用工具，不需要知道
//! 底下是 rmcp 还是别的实现。

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use crate::config::ServerConfig;
use crate::worker::{CallOutcome, Command, McpRuntime, ServerSnapshot, ToolDescriptor};

/// 服务器的可见状态（供设置页与工具层判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    /// 已停用（enabled = false），不连接
    Disabled,
    /// 尚未连接（配置存在但未建立连接）
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

impl ServerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerStatus::Disabled => "disabled",
            ServerStatus::Disconnected => "disconnected",
            ServerStatus::Connecting => "connecting",
            ServerStatus::Connected => "connected",
            ServerStatus::Failed => "failed",
        }
    }
}

/// 一个服务器在池中的运行态。
#[derive(Debug, Clone)]
pub struct ServerRuntime {
    pub status: ServerStatus,
    /// 最近一次错误（失败或断连时保留，供界面解释）
    pub error: String,
    /// 已注册工具数
    pub tool_count: usize,
    /// 服务器自述（连上后才有）
    pub server_name: String,
    pub server_version: String,
}

impl Default for ServerRuntime {
    fn default() -> Self {
        Self {
            status: ServerStatus::Disconnected,
            error: String::new(),
            tool_count: 0,
            server_name: String::new(),
            server_version: String::new(),
        }
    }
}

/// 连接池：持有 runtime 线程句柄与每个服务器的运行态快照。
pub struct McpPool {
    runtime: McpRuntime,
    /// 配置快照（壳层写入，工具层读）
    configs: Mutex<Vec<ServerConfig>>,
    /// 运行态快照
    runtimes: Mutex<HashMap<String, ServerRuntime>>,
    /// 已连上服务器的工具描述符（server_id → 工具表）
    tools: Mutex<HashMap<String, Vec<ToolDescriptor>>>,
}

impl McpPool {
    pub fn new() -> Self {
        Self {
            runtime: McpRuntime::spawn(),
            configs: Mutex::new(Vec::new()),
            runtimes: Mutex::new(HashMap::new()),
            tools: Mutex::new(HashMap::new()),
        }
    }

    /// 用给定配置替换池内配置（壳层保存设置后调用）。不自动连接。
    pub fn set_configs(&self, configs: Vec<ServerConfig>) {
        *self.configs.lock().unwrap() = configs;
    }

    pub fn configs(&self) -> Vec<ServerConfig> {
        self.configs.lock().unwrap().clone()
    }

    pub fn config(&self, server_id: &str) -> Option<ServerConfig> {
        self.configs.lock().unwrap().iter().find(|c| c.id == server_id).cloned()
    }

    pub fn runtime_status(&self, server_id: &str) -> ServerRuntime {
        self.runtimes.lock().unwrap().get(server_id).cloned().unwrap_or_default()
    }

    /// 全部服务器的运行态快照（设置页列表用）。
    pub fn all_runtimes(&self) -> HashMap<String, ServerRuntime> {
        self.runtimes.lock().unwrap().clone()
    }

    /// 已连接服务器的全部工具（server_id, 描述符）。
    pub fn connected_tools(&self) -> Vec<(String, ToolDescriptor)> {
        let tools = self.tools.lock().unwrap();
        let mut out: Vec<(String, ToolDescriptor)> = Vec::new();
        for (server_id, descriptors) in tools.iter() {
            for descriptor in descriptors {
                out.push((server_id.clone(), descriptor.clone()));
            }
        }
        out
    }

    /// 工具的展示名（连接时记录的服务器名，缺省回落配置名）。
    pub fn server_display_name(&self, server_id: &str) -> String {
        let runtime = self.runtime_status(server_id);
        if !runtime.server_name.is_empty() {
            return runtime.server_name;
        }
        self.config(server_id).map(|c| c.name).unwrap_or_else(|| server_id.to_string())
    }

    /// 连接一个服务器并记录工具表。失败写运行态并返回可读错误。
    pub fn connect(&self, config: &ServerConfig) -> Result<ServerSnapshot, String> {
        self.set_status(&config.id, ServerStatus::Connecting, String::new());
        let result = self.runtime.request(
            |reply| Command::Connect { config: config.clone(), reply },
            config.timeout_ms,
        );
        match result {
            Ok(Ok(snapshot)) => {
                {
                    let mut runtimes = self.runtimes.lock().unwrap();
                    let entry = runtimes.entry(config.id.clone()).or_default();
                    entry.status = ServerStatus::Connected;
                    entry.error = String::new();
                    entry.tool_count = snapshot.tools.len();
                    entry.server_name = snapshot.server_name.clone();
                    entry.server_version = snapshot.server_version.clone();
                }
                self.tools.lock().unwrap().insert(config.id.clone(), snapshot.tools.clone());
                Ok(snapshot)
            }
            Ok(Err(message)) => {
                self.mark_failed(&config.id, message.clone());
                Err(message)
            }
            Err(message) => {
                self.mark_failed(&config.id, message.clone());
                Err(message)
            }
        }
    }

    /// 断开一个服务器（无论当前是否连着）。
    pub fn disconnect(&self, server_id: &str) {
        let timeout = self.config(server_id).map(|c| c.timeout_ms).unwrap_or(crate::config::DEFAULT_TIMEOUT_MS);
        let _ = self.runtime.notify(|reply| Command::Disconnect { server_id: server_id.to_string(), reply }, timeout);
        self.tools.lock().unwrap().remove(server_id);
        let mut runtimes = self.runtimes.lock().unwrap();
        if let Some(entry) = runtimes.get_mut(server_id) {
            entry.status = ServerStatus::Disconnected;
            entry.tool_count = 0;
        }
    }

    /// 调用工具。
    pub fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: Value,
        timeout_ms: i64,
    ) -> Result<CallOutcome, String> {
        let result = self.runtime.request(
            |reply| Command::Call {
                server_id: server_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments,
                timeout_ms,
                reply,
            },
            timeout_ms,
        );
        match result {
            Ok(inner) => inner,
            Err(message) => {
                self.mark_failed(server_id, message.clone());
                Err(message)
            }
        }
    }

    /// 服务器是否仍连着（工具调用前用于决定是否尝试重连）。
    pub fn is_alive(&self, server_id: &str) -> bool {
        self.runtime
            .request(|reply| Command::IsAlive { server_id: server_id.to_string(), reply }, 5_000)
            .unwrap_or(false)
    }

    /// 应用退出：断开全部并结束 runtime 线程。
    ///
    /// 等待超时（30 秒）必须盖过 CloseAll 的实际耗时：单个断开最坏约 15 秒
    /// （rmcp 的 EOF/SIGTERM/SIGKILL 序列，见 client::disconnect），提前返回的话
    /// 进程终结会强杀 runtime 线程，还没断完的服务器就成了孤儿进程。
    pub fn shutdown(&self) {
        self.runtime.shutdown();
        self.tools.lock().unwrap().clear();
    }

    fn set_status(&self, server_id: &str, status: ServerStatus, error: String) {
        let mut runtimes = self.runtimes.lock().unwrap();
        let entry = runtimes.entry(server_id.to_string()).or_default();
        entry.status = status;
        if !error.is_empty() {
            entry.error = error;
        }
    }

    fn mark_failed(&self, server_id: &str, error: String) {
        let mut runtimes = self.runtimes.lock().unwrap();
        let entry = runtimes.entry(server_id.to_string()).or_default();
        entry.status = ServerStatus::Failed;
        entry.error = error;
        entry.tool_count = 0;
        drop(runtimes);
        self.tools.lock().unwrap().remove(server_id);
    }

    /// 标记为停用（配置里 enabled=false）。
    pub fn mark_disabled(&self, server_id: &str) {
        self.tools.lock().unwrap().remove(server_id);
        let mut runtimes = self.runtimes.lock().unwrap();
        let entry = runtimes.entry(server_id.to_string()).or_default();
        entry.status = ServerStatus::Disabled;
        entry.tool_count = 0;
    }
}

impl Default for McpPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 未连接的服务器状态是 Disconnected，且没有工具。
    #[test]
    fn unknown_server_reports_disconnected() {
        let pool = McpPool::new();
        let runtime = pool.runtime_status("mcp_missing");
        assert_eq!(runtime.status, ServerStatus::Disconnected);
        assert_eq!(runtime.tool_count, 0);
        assert!(pool.connected_tools().is_empty());
        pool.shutdown();
    }

    /// 连不上时状态转 Failed 且错误可读——设置页据此显示红灯，而不是永远转圈。
    #[test]
    fn failed_connect_marks_failed_with_reason() {
        let pool = McpPool::new();
        let mut config = ServerConfig::new("broken".into(), crate::config::Transport::Stdio);
        config.id = "mcp_broken".into();
        config.command = "/nonexistent/craftwork-mcp-test-binary".into();
        config.timeout_ms = 3_000;
        let err = pool.connect(&config).unwrap_err();
        assert!(!err.is_empty(), "失败应带可读原因：{err}");
        let runtime = pool.runtime_status("mcp_broken");
        assert_eq!(runtime.status, ServerStatus::Failed);
        assert!(!runtime.error.is_empty());
        pool.shutdown();
    }

    #[test]
    fn configs_round_trip() {
        let pool = McpPool::new();
        assert!(pool.configs().is_empty());
        let mut config = ServerConfig::new("demo".into(), crate::config::Transport::Stdio);
        config.id = "mcp_demo".into();
        config.command = "node".into();
        pool.set_configs(vec![config.clone()]);
        assert_eq!(pool.configs(), vec![config.clone()]);
        assert_eq!(pool.config("mcp_demo"), Some(config));
        assert_eq!(pool.config("mcp_other"), None);
        pool.shutdown();
    }

    /// 停用后状态是 Disabled（不是 Failed）——界面文案不同：一个是用户主动停，
    /// 一个是连接出错。
    #[test]
    fn disabled_is_a_distinct_status() {
        let pool = McpPool::new();
        pool.mark_disabled("mcp_demo");
        assert_eq!(pool.runtime_status("mcp_demo").status, ServerStatus::Disabled);
        pool.shutdown();
    }

    #[test]
    fn display_name_prefers_server_self_report() {
        let pool = McpPool::new();
        let mut config = ServerConfig::new("配置里的名字".into(), crate::config::Transport::Stdio);
        config.id = "mcp_demo".into();
        config.command = "node".into();
        pool.set_configs(vec![config]);
        assert_eq!(pool.server_display_name("mcp_demo"), "配置里的名字");
        pool.shutdown();
    }
}
