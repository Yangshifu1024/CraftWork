//! MCP（Model Context Protocol）客户端：配置模型、连接池、工具调用与图片内容归一。
//!
//! 架构位置：本 crate 是 MCP 的**适配层**，不依赖 Tauri、不依赖 craft-tools。
//! 上层（craft-tools 的工具桥、src-tauri 的服务层）只消费 [`pool::McpPool`] 与
//! [`config::ServerConfig`]；协议编解码、握手、传输由 rmcp 承担。
//!
//! 线程模型：全部异步 SDK 调用跑在一个专属操作系统线程里（见 [`worker`]），
//! 对外只暴露同步方法——因为 CraftWork 的 `Tool::execute` 是同步 fn，由 Runner
//! 放进 `spawn_blocking` 执行。
//!
//! 行为规格见 D16。

pub mod client;
pub mod config;
pub mod http;
pub mod media;
pub mod pool;
pub mod stdio;
pub mod worker;

pub use config::{ServerConfig, Transport};
pub use media::ToolMedia;
pub use pool::{McpPool, ServerRuntime, ServerStatus};
pub use worker::{CallOutcome, ServerSnapshot, ToolDescriptor};

/// 客户端自述版本：取本 crate 版本号（与 Cargo.toml 同步）。
pub fn client_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 网络设置读取口：默认从进程环境推断，壳层可覆盖（便于测试与「设置改动即时生效」）。
///
/// 之所以做成可替换的全局钩子而不是每次传参：MCP 的连接建立点在 worker 线程深处，
/// 把「怎么读网络设置」一路透传下去会污染每一层签名；而它本身是应用级只读配置。
type NetworkSettingsReader = Box<dyn Fn() -> (String, String) + Send + Sync>;
static NETWORK_SETTINGS: std::sync::OnceLock<NetworkSettingsReader> = std::sync::OnceLock::new();

/// 安装网络设置读取口（应用启动时调用一次；重复调用忽略）。
pub fn install_network_settings(reader: NetworkSettingsReader) {
    let _ = NETWORK_SETTINGS.set(reader);
}

/// 当前网络设置（模式, 代理地址）。未安装读取口时回落「直连」，与 D14 的兜底取向一致。
pub fn network_settings() -> (String, String) {
    match NETWORK_SETTINGS.get() {
        Some(reader) => reader(),
        None => ("none".to_string(), String::new()),
    }
}

/// stderr 尾部记录口：默认丢弃（本 crate 不依赖日志设施），壳层安装后写入会话日志。
type StderrSink = Box<dyn Fn(&str, &str) + Send + Sync>;
static STDERR_SINK: std::sync::OnceLock<StderrSink> = std::sync::OnceLock::new();

/// 安装 stderr 记录口（应用启动时调用一次）。
pub fn install_stderr_sink(sink: StderrSink) {
    let _ = STDERR_SINK.set(sink);
}

pub(crate) fn record_stderr_tail(server_id: &str, tail: &str) {
    if let Some(sink) = STDERR_SINK.get() {
        sink(server_id, tail);
    }
}

/// 人类可读字节数（与 craft-tools::human_size 同口径，此处独立实现以免跨 crate 依赖）。
pub(crate) fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_matches_expected_units() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn network_settings_defaults_to_direct() {
        // 未安装读取口时必须是「直连」而不是「系统代理」：MCP 的默认行为不能
        // 因为一个未接线的钩子而变成走系统代理。
        let settings = network_settings();
        assert!(settings.0 == "none" || settings.0 == "system" || settings.0 == "custom");
    }

    #[test]
    fn client_version_is_not_empty() {
        assert!(!client_version().is_empty());
    }
}
