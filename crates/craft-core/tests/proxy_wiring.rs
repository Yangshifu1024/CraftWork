//! 出站代理接线集成测试（D14）：
//! 用本地 std TcpListener mock 服务器分别扮演「HTTP 代理」与「目标服务」，
//! 断言 blocking 客户端经 `craft_core::network::ProxyConfig::apply_reqwest_blocking`
//! 应用后的真实流量走向（壳层 provider_test / provider_fetch_models 的生产出口）：
//!
//! - none 模式：环境变量设了代理仍直连（显式 no_proxy 清空环境探测）；
//! - custom 模式：流量经 mock HTTP 代理转发（请求以绝对 URI 到达代理）；
//! - custom 模式内置排除：localhost / 127.0.0.1 目标直连，代理零流量；
//! - system 模式：环境变量代理被读出并显式应用，且不注入内置本地排除。
//!
//! socks5 握手级 wire 测试落在 craft-model（craft_model::net::build_http_client 出口）：
//! reqwest 的内置 socks 连接器要先经 HttpConnector 连代理，而 HttpConnector 仅在启用
//! TLS feature 的连接器构造路径里放开非 http scheme（connect.rs 的 enforce_http(false)
//! 只在 TLS 构造路径调用）；craft-core 单独测试时无 TLS feature，socks 必然失败，
//! 与生产配置不符，故不在本文件测。craft-core 侧的 socks 保障是 URL 构造断言
//! （mod.rs 单测 socks_urls_construct_reqwest_proxy）。
//!
//! 测试进程内可能并行执行，环境变量属进程全局——所有用例持同一把串行锁，
//! 保证改环境变量的用例不污染其他用例的客户端构建。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_core::network::{ProxyConfig, ProxyMode};

/// 串行锁：改进程环境变量的用例与构建客户端的用例互斥（环境变量是进程全局的）。
static SERIAL: Mutex<()> = Mutex::new(());

const RESPONSE_BODY: &str = "craftwork-proxy-ok";

// ---------- mock 服务器 ----------

/// 通用 HTTP mock 服务器：记录收到的请求行与完整请求头，对每个连接回固定 200 响应。
/// 可选 `first_line_filter`（HTTP 代理场景下代理可能收到多个 host 的请求，只对
/// 匹配目标的连接回响应，其余连接直接关闭让客户端快速失败）。
struct MockServer {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    heads: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    fn http() -> Self {
        Self::spawn(None)
    }

    /// 只对请求行包含指定片段的连接回 HTTP 响应（其余连接立即关闭）。
    fn http_filtered(filter: &'static str) -> Self {
        Self::spawn(Some(filter))
    }

    fn spawn(first_line_filter: Option<&'static str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        let port = listener.local_addr().expect("读取本地端口失败").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let heads = Arc::new(Mutex::new(Vec::new()));
        let requests_clone = Arc::clone(&requests);
        let heads_clone = Arc::clone(&heads);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = serve_http(
                    stream,
                    &requests_clone,
                    &heads_clone,
                    first_line_filter,
                );
            }
        });
        Self { port, requests, heads }
    }

    /// 收到的请求行快照（首行，如 `GET http://host/path HTTP/1.1`）。
    fn request_lines(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    /// 收到的完整请求头快照（含请求行，按到达顺序）。
    fn request_heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }
}

/// 处理单个 HTTP 连接：读请求头 → 记录请求行与完整头 →（可选过滤）回固定响应。
fn serve_http(
    mut stream: TcpStream,
    requests: &Mutex<Vec<String>>,
    heads: &Mutex<Vec<String>>,
    first_line_filter: Option<&str>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let head = read_until_head_end(&mut stream)?;
    let head_text = String::from_utf8_lossy(&head).to_string();
    let first_line = head_text.lines().next().unwrap_or_default().to_string();
    let matched = first_line_filter.is_none_or(|f| first_line.contains(f));
    requests.lock().unwrap().push(first_line);
    heads.lock().unwrap().push(head_text);
    if !matched {
        // 过滤未命中：直接关闭连接，让客户端在错误路径上快速失败而不是拿到假响应
        return Ok(());
    }
    write_ok_response(&mut stream)
}

/// 读到请求头结束（\r\n\r\n）为止，返回已读字节。
fn read_until_head_end(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    Ok(buf)
}

/// 写固定 200 响应（带 Content-Length 与 Connection: close，读体即收尾）。
fn write_ok_response(stream: &mut TcpStream) -> std::io::Result<()> {
    let body = RESPONSE_BODY;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

// ---------- 配置与客户端构造 ----------

/// custom 模式配置（地址已按生产路径经 normalize 归一）。
fn custom_config(url: &str) -> ProxyConfig {
    ProxyConfig {
        mode: ProxyMode::Custom,
        effective_url: Some(craft_core::network::normalize_proxy_url(url).expect("代理地址应合法")),
    }
}

/// none 模式配置（显式直连）。
fn none_config() -> ProxyConfig {
    ProxyConfig { mode: ProxyMode::None, effective_url: None }
}

/// 按配置构建 blocking 客户端（走与生产 `apply_reqwest_blocking` 相同的出口），
/// 统一 5 秒超时保证配错时快速失败而不是挂死测试。
fn blocking_client(config: &ProxyConfig) -> reqwest::blocking::Client {
    config
        .apply_reqwest_blocking(reqwest::blocking::Client::builder())
        .timeout(Duration::from_secs(5))
        .build()
        .expect("客户端构建失败")
}

/// 取一个当前空闲的本地端口（绑定后立即释放）：用作「必死」代理地址，
/// 任何试图连它的流量都会快速失败。
fn dead_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
    listener.local_addr().expect("读取本地端口失败").port()
}

/// 环境变量清理守卫：无论用例断言成败，Drop 时移除设置过的变量（防污染后续用例）。
struct EnvGuard {
    names: &'static [&'static str],
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for name in self.names {
            // edition 2024 起置/删环境变量为 unsafe（进程全局状态，串行锁内调用安全）
            unsafe { std::env::remove_var(name) };
        }
    }
}

// ---------- 用例 ----------

/// custom 模式：流量经 mock HTTP 代理转发——请求以绝对 URI 形式到达代理并被应答。
#[test]
fn custom_mode_routes_traffic_through_mock_proxy() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let proxy = MockServer::http_filtered("craftwork-proxy-test.invalid");
    let client = blocking_client(&custom_config(&format!("127.0.0.1:{}", proxy.port)));
    // 目标 host 不存在（.invalid 保留域），本地 DNS 必失败——能拿到响应只能说明流量走了代理
    let resp = client
        .get("http://craftwork-proxy-test.invalid/hello")
        .send()
        .expect("经代理的请求应成功");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().expect("读响应体失败"), RESPONSE_BODY);
    let lines = proxy.request_lines();
    assert_eq!(lines.len(), 1, "代理应恰好收到一次请求");
    assert!(
        lines[0].contains("http://craftwork-proxy-test.invalid/hello"),
        "代理收到的应是绝对 URI 请求行，实际：{}",
        lines[0]
    );
}

/// custom 模式内置排除：127.0.0.1 目标直连——请求到达本地目标服务，代理零流量。
#[test]
fn custom_mode_excludes_localhost_target_direct() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let proxy = MockServer::http();
    let target = MockServer::http();
    let client = blocking_client(&custom_config(&format!("127.0.0.1:{}", proxy.port)));
    let resp = client
        .get(format!("http://127.0.0.1:{}/ping", target.port))
        .send()
        .expect("本地目标应直连成功");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().expect("读响应体失败"), RESPONSE_BODY);
    assert!(
        target.request_lines().iter().any(|l| l.contains("/ping")),
        "请求应到达本地目标服务"
    );
    assert!(
        proxy.request_lines().is_empty(),
        "内置排除应让本地目标绕过代理，代理不应收到流量"
    );
}

/// none 模式：环境变量设了代理仍直连——显式 no_proxy 清空环境探测，请求直达本地目标。
#[test]
fn none_mode_connects_direct_even_with_env_proxy() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 环境变量指向必死地址：若 none 模式没有清空环境探测，请求会连这个死端口而失败
    let dead = dead_port();
    let guard = EnvGuard { names: &["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"] };
    for name in guard.names {
        unsafe { std::env::set_var(name, format!("http://127.0.0.1:{dead}")) };
    }
    let target = MockServer::http();
    let client = blocking_client(&none_config());
    let resp = client
        .get(format!("http://127.0.0.1:{}/direct", target.port))
        .send()
        .expect("none 模式应无视环境代理直连成功");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().expect("读响应体失败"), RESPONSE_BODY);
    assert!(target.request_lines().iter().any(|l| l.contains("/direct")));
    drop(guard);
}

/// system 模式回落链路（环境变量优先）：环境代理被读出并显式应用，localhost 不在内置
/// 排除内（系统模式尊重环境自身排除），因此请求经环境代理转发。
#[test]
fn system_mode_applies_env_proxy_without_builtin_exclusions() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let proxy = MockServer::http();
    let guard = EnvGuard { names: &["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"] };
    for name in guard.names {
        unsafe { std::env::set_var(name, format!("http://127.0.0.1:{}", proxy.port)) };
    }
    let config = ProxyConfig::from_settings("system", "").expect("system 模式解析应成功");
    assert_eq!(config.effective_url.as_deref(), Some(format!("http://127.0.0.1:{}", proxy.port)).as_deref());
    let client = blocking_client(&config);
    // 目标 host 不在本地可解析（.invalid 域），拿到响应即证明流量走了环境代理
    let resp = client
        .get("http://craftwork-proxy-test.invalid/via-env")
        .send()
        .expect("system 模式应经环境代理转发");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().expect("读响应体失败"), RESPONSE_BODY);
    assert!(proxy.request_lines().iter().any(|l| l.contains("/via-env")));
    drop(guard);
}

// ---------- 出站 UA ----------

/// 从原始请求头文本里取指定头的值（大小写不敏感的头名匹配）。
fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
    })
}

/// 所有 reqwest 出口在线上真实带统一 UA：blocking 出口（壳层探活 / 拉取模型）与异步
/// 出口（模型调用 / 远程 MCP）都由 `craft_core::network` 注入 `CraftWork/<版本>`，
/// 不依赖各调用点各自设置。craft-model / craft-mcp 的客户端也只经这两个出口构造，
/// 故本用例覆盖它们的 UA 语义。
#[test]
fn both_reqwest_exits_send_app_user_agent() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let expected = craft_core::network::APP_USER_AGENT.to_string();
    assert!(expected.starts_with("CraftWork/"), "UA 应为 CraftWork/<版本>，实际：{expected}");

    // blocking 出口
    let target = MockServer::http();
    let client = blocking_client(&none_config());
    client
        .get(format!("http://127.0.0.1:{}/ua-blocking", target.port))
        .send()
        .expect("blocking 请求应成功");
    let head = target.request_heads().join("\n");
    assert_eq!(
        header_value(&head, "user-agent").as_deref(),
        Some(expected.as_str()),
        "blocking 出口应带统一 UA，实际请求头：\n{head}"
    );

    // 异步出口（与 craft-model / craft-mcp 同一条 `apply_reqwest` 路径）
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("测试用 runtime 应可构建");
    let target = MockServer::http();
    let client = none_config()
        .apply_reqwest(reqwest::Client::builder())
        .timeout(Duration::from_secs(5))
        .build()
        .expect("异步客户端构建失败");
    runtime.block_on(async {
        client
            .get(format!("http://127.0.0.1:{}/ua-async", target.port))
            .send()
            .await
            .expect("异步请求应成功");
    });
    let head = target.request_heads().join("\n");
    assert_eq!(
        header_value(&head, "user-agent").as_deref(),
        Some(expected.as_str()),
        "异步出口应带统一 UA，实际请求头：\n{head}"
    );
}
