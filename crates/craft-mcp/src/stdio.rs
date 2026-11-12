//! stdio 服务器的环境装配与 stderr 收尾。
//!
//! 环境合并顺序（安全语义，改动前先读 D16）：
//! 1. 基础环境：用户选中 shell 时用 `shellenv` 捕获并清洗的结果（与 `shell_exec`
//!    完全同一份，决策 D18），否则用 `platform::sanitized_env()`——两者的剔除规则
//!    相同（黑名单，剔除 `*_KEY/TOKEN/SECRET/PASSWORD/CREDENTIAL` 与厂商前缀）。
//! 2. 该服务器配置里用户显式填写的 `env`
//!
//! 顺序不可颠倒：用户 env 覆盖基础环境是无害的（用户主动填写），但基础环境覆盖
//! 用户 env 会让「配置看起来生效、实际被清洗掉」的排查变得极其困难。
//!
//! 为什么 MCP 要跟随 shell 环境：nvm/pyenv 装的 node/python 只在用户 shell 配置里
//! 可见，不跟随的话「MCP server 起不来」会成为下一个难查的问题。

use std::collections::BTreeMap;

use crate::config::ServerConfig;

/// 取用点刷新：过期才真跑 shell，失败沿用旧值（详见 `shellenv::ensure_fresh`）。
///
/// 这里从缓存里取 shell 路径——`configure` 已经把「用户选中哪个 shell」记在缓存里，
/// craft-mcp 不读 settings 表（分层约束）。缓存为空时 `configured_shell()` 返回 `None`，
/// 意味着用户没启用 shell 环境，刷新没有意义。
fn refresh_shell_env() {
    let Some(shell) = craft_core::shellenv::configured_shell() else { return };
    let home = craft_core::platform::home_dir_string();
    if !craft_core::shellenv::ensure_fresh(&shell, &home) {
        eprintln!("craftwork: MCP 子进程环境刷新失败，沿用上一次的 shell 环境");
    }
}

/// 合并后的子进程环境（按变量名排序，便于日志与测试断言稳定）。
pub fn stdio_env(config: &ServerConfig) -> Vec<(String, String)> {
    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    // 基础环境与 shell_exec 同源：选中 shell 用捕获结果，否则用进程环境清洗结果。
    // 装配前先尝试刷新（过期才真跑 shell），避免 MCP 子进程拿到陈旧环境。
    refresh_shell_env();
    let base = craft_core::shellenv::current_vars().unwrap_or_else(craft_core::platform::sanitized_env);
    for (key, value) in base {
        merged.insert(key, value);
    }
    for (key, value) in &config.env {
        merged.insert(key.clone(), value.clone());
    }
    merged.into_iter().collect()
}

/// 把 stderr 排空（读到 EOF 为止）。
///
/// 为什么必须读：`TokioChildProcess::builder().stderr(piped())` 之后若无人读取，
/// 服务器一旦往 stderr 写满管道缓冲区（几十 KB，日志型服务器很容易达到）就会
/// 阻塞在 write 上，表现为「连接一直能建立但工具调用全部超时」。
///
/// 实现：在 MCP runtime 线程上 spawn 一个 tokio 任务异步读（因此本函数必须在
/// runtime 上下文内调用——调用点正是 `connect` 里的异步代码）。保留末尾
/// [`STDERR_TAIL_BYTES`] 字节供报错展示，更早的内容直接丢弃。
pub fn spawn_stderr_drain(server_id: String, stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncReadExt;
    tokio::spawn(async move {
        let mut stderr = stderr;
        let mut tail: Vec<u8> = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    tail.extend_from_slice(&buffer[..read]);
                    if tail.len() > STDERR_TAIL_BYTES {
                        let drop_len = tail.len() - STDERR_TAIL_BYTES;
                        tail.drain(..drop_len);
                    }
                }
            }
        }
        if !tail.is_empty() {
            crate::record_stderr_tail(&server_id, &String::from_utf8_lossy(&tail));
        }
    });
}

/// stderr 保留的尾部字节数（足够容纳一段异常堆栈的开头）。
pub const STDERR_TAIL_BYTES: usize = 4096;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Transport;

    fn stdio_config(env: Vec<(String, String)>) -> ServerConfig {
        let mut config = ServerConfig::new("demo".into(), Transport::Stdio);
        config.command = "node".into();
        config.env = env;
        config
    }

    /// 用户显式填写的 env 必须出现（即使变量名命中清洗黑名单词）。
    /// 这条是 MCP 能用的前提：大量 MCP 服务器靠 `API_KEY` 这类变量鉴权。
    #[test]
    fn user_env_survives_sanitize_blacklist() {
        let config = stdio_config(vec![("DEMO_API_KEY".into(), "secret-value".into())]);
        let env = stdio_env(&config);
        assert!(
            env.iter().any(|(k, v)| k == "DEMO_API_KEY" && v == "secret-value"),
            "用户显式配置的 env 应原样进入子进程环境：{env:?}"
        );
    }

    /// 反向约束：用户没写进配置的敏感变量不得从父进程继承。
    #[test]
    fn parent_secrets_are_not_inherited() {
        // SAFETY: 单线程测试内设置，且立即清除
        unsafe {
            std::env::set_var("CRAFTWORK_TEST_PROVIDER_TOKEN", "should-not-leak");
        }
        let env = stdio_env(&stdio_config(vec![]));
        unsafe {
            std::env::remove_var("CRAFTWORK_TEST_PROVIDER_TOKEN");
        }
        assert!(
            !env.iter().any(|(k, _)| k == "CRAFTWORK_TEST_PROVIDER_TOKEN"),
            "provider 密钥不得泄漏给 MCP 子进程：{env:?}"
        );
    }

    #[test]
    fn env_is_sorted_for_stable_assertions() {
        let config = stdio_config(vec![("ZZZ".into(), "1".into()), ("AAA".into(), "2".into())]);
        let env = stdio_env(&config);
        let mut sorted = env.clone();
        sorted.sort();
        assert_eq!(env, sorted);
    }
}
