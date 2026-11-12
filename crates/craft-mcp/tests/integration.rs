//! 端到端集成测试：用真实子进程跑 `tests/fixtures/mcp/` 下的 Python 假服务器。
//!
//! 这些测试覆盖单测覆盖不到的部分——rmcp 与真实 stdio 子进程的握手、NDJSON 分帧、
//! 工具发现、文本/错误/图片三条结果通道，以及不存在的可执行文件这类失败路径。
//! 退出清理用 `stubborn_server.py` 验证（stdin EOF 后不退出，只有显式杀进程组才会
//! 消失）；其余测试用行为一致的 `fake_server.py`。
//!
//! 依赖系统 Python 3（`python3`）。缺失时跳过而不是失败：它只是测试便利，
//! 不是应用运行时的依赖（用户自己选的 MCP 服务器才需要解释器）。

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use craft_mcp::config::{ServerConfig, Transport};
use craft_mcp::pool::{McpPool, ServerStatus};

/// fixture 脚本的绝对路径（cargo 从 crate 目录运行测试，工作区根在其上两级）。
fn fixture_script(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // crates
    path.pop(); // 工作区根
    path.push("tests/fixtures/mcp");
    path.push(name);
    path
}

/// 系统是否有可用的 python3。
fn python_available() -> bool {
    std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn server_config(name: &str, script: &str) -> ServerConfig {
    let mut config = ServerConfig::new(name.to_string(), Transport::Stdio);
    config.id = format!("mcp_{name}");
    config.command = "python3".into();
    config.args = vec![fixture_script(script).to_string_lossy().to_string()];
    config.timeout_ms = 15_000;
    config
}

fn fake_server_config(name: &str) -> ServerConfig {
    server_config(name, "fake_server.py")
}

/// 系统里命令行含 stubborn_server.py 的进程 pid 集合。
///
/// Unix 用 `ps`（完整命令行）；Windows 的 Git Bash `ps` 只显示命令名不带参数
/// （python3 后面的脚本路径匹配不到），改用 PowerShell 查 Win32_Process 的
/// CommandLine——CI runner 自带，每次约一秒，轮询预算相应放宽。
#[cfg(unix)]
fn stubborn_server_pids() -> HashSet<u32> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .expect("ps 应可用");
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pids = HashSet::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        let command = parts.collect::<Vec<_>>().join(" ");
        if command.contains("stubborn_server.py") {
            pids.insert(pid);
        }
    }
    pids
}

#[cfg(windows)]
fn stubborn_server_pids() -> HashSet<u32> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -like '*stubborn_server.py*' } | Select-Object -ExpandProperty ProcessId",
        ])
        .output()
        .expect("powershell 应可用");
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().filter_map(|line| line.trim().parse::<u32>().ok()).collect()
}

#[test]
fn connects_lists_tools_and_calls_text_tool() {
    if !python_available() {
        eprintln!("跳过：系统没有 python3");
        return;
    }
    let pool = McpPool::new();
    let config = fake_server_config("fake");
    let snapshot = pool.connect(&config).expect("连接假 MCP 服务器应成功");

    // 服务器自述
    assert_eq!(snapshot.server_name, "craftwork-fake-mcp");
    assert_eq!(snapshot.server_version, "0.1.0");
    // 工具表：三个工具，名称与描述来自服务器
    let names: Vec<&str> = snapshot.tools.iter().map(|t| t.tool_name.as_str()).collect();
    assert_eq!(names, vec!["echo", "fail", "screenshot"]);
    // 注解透传：echo 声明了 readOnlyHint
    let echo = snapshot.tools.iter().find(|t| t.tool_name == "echo").expect("有 echo");
    assert!(echo.read_only_hint, "readOnlyHint 应透传到描述符");
    assert!(!echo.description.is_empty());

    // 运行态
    let runtime = pool.runtime_status("mcp_fake");
    assert_eq!(runtime.status, ServerStatus::Connected);
    assert_eq!(runtime.tool_count, 3);
    assert_eq!(pool.connected_tools().len(), 3);

    // 调文本工具
    let outcome = pool
        .call_tool("mcp_fake", "echo", serde_json::json!({"text": "你好"}), 15_000)
        .expect("调用 echo 应成功");
    assert!(!outcome.is_error);
    assert_eq!(outcome.text, "回显：你好");
    assert!(outcome.media.is_empty());

    pool.shutdown();
}

#[test]
fn error_tool_result_is_marked_as_error() {
    if !python_available() {
        eprintln!("跳过：系统没有 python3");
        return;
    }
    let pool = McpPool::new();
    pool.connect(&fake_server_config("fake")).expect("连接应成功");
    let outcome = pool
        .call_tool("mcp_fake", "fail", serde_json::json!({}), 15_000)
        .expect("调用本身应成功返回（错误在结果里）");
    assert!(outcome.is_error, "isError=true 必须透传");
    assert!(outcome.text.contains("预期内的失败"));
    pool.shutdown();
}

/// 图片链路的核心断言：MCP 的 image 块必须变成 ToolMedia，且结构化内容进文本。
#[test]
fn image_tool_result_carries_media() {
    if !python_available() {
        eprintln!("跳过：系统没有 python3");
        return;
    }
    let pool = McpPool::new();
    pool.connect(&fake_server_config("fake")).expect("连接应成功");
    let outcome = pool
        .call_tool("mcp_fake", "screenshot", serde_json::json!({}), 15_000)
        .expect("调用截图工具应成功");

    assert_eq!(outcome.media.len(), 1, "应收到一张图片：{:?}", outcome.media);
    let media = &outcome.media[0];
    assert_eq!(media.mime, "image/png");
    assert_eq!(media.size, 70, "1x1 PNG 解码后应为 70 字节");
    assert_eq!(media.name, "mcp-image-1.png");
    // 文本块保留，结构化内容追加
    assert!(outcome.text.contains("截图如下"));
    assert!(outcome.text.contains("Structured content:"));
    assert!(outcome.text.contains("\"width\": 1"));
    pool.shutdown();
}

#[test]
fn unknown_tool_reports_error_result() {
    if !python_available() {
        eprintln!("跳过：系统没有 python3");
        return;
    }
    let pool = McpPool::new();
    pool.connect(&fake_server_config("fake")).expect("连接应成功");
    let outcome = pool
        .call_tool("mcp_fake", "no-such-tool", serde_json::json!({}), 15_000)
        .expect("服务器返回结果即算调用成功");
    assert!(outcome.is_error);
    assert!(outcome.text.contains("未知工具"));
    pool.shutdown();
}

/// 调用不存在的服务器：必须是可读错误而不是 panic。
#[test]
fn call_on_unknown_server_fails_readably() {
    let pool = McpPool::new();
    let err = pool
        .call_tool("mcp_missing", "echo", serde_json::json!({}), 1_000)
        .expect_err("未连接的服务器应报错");
    assert!(err.contains("未连接"), "错误文案应可读：{err}");
    pool.shutdown();
}

/// 断开后状态回到 Disconnected，工具表清空。
#[test]
fn disconnect_clears_tools_and_status() {
    if !python_available() {
        eprintln!("跳过：系统没有 python3");
        return;
    }
    let pool = McpPool::new();
    pool.connect(&fake_server_config("fake")).expect("连接应成功");
    assert_eq!(pool.connected_tools().len(), 3);
    pool.disconnect("mcp_fake");
    assert!(pool.connected_tools().is_empty(), "断开后不应残留工具");
    assert_eq!(pool.runtime_status("mcp_fake").status, ServerStatus::Disconnected);
    // 断开后调用：明确报错
    let err = pool
        .call_tool("mcp_fake", "echo", serde_json::json!({"text": "x"}), 1_000)
        .expect_err("断开后调用应报错");
    assert!(err.contains("未连接"), "{err}");
    pool.shutdown();
}

/// 连不上的进程：状态 Failed 且错误可读（设置页据此显示红灯而不是永久转圈）。
#[test]
fn missing_binary_marks_server_failed() {
    let pool = McpPool::new();
    let mut config = ServerConfig::new("broken".into(), Transport::Stdio);
    config.id = "mcp_broken".into();
    config.command = "/nonexistent/craftwork-mcp-does-not-exist".into();
    config.timeout_ms = 3_000;
    let err = pool.connect(&config).expect_err("不存在的可执行文件应连接失败");
    assert!(err.contains("启动") || err.contains("连接失败"), "错误应说明原因：{err}");
    assert_eq!(pool.runtime_status("mcp_broken").status, ServerStatus::Failed);
    pool.shutdown();
}

/// 退出清理的核心断言：`shutdown` 之后 stubborn 子进程必须真的消失。
///
/// fake_server 在 stdin EOF 后自行退出，无法区分「应用显式关闭」与「父进程消失」；
/// stubborn 版本（`tests/fixtures/mcp/stubborn_server.py`）EOF 后继续空转，只有
/// `close_all`（rmcp 的 transport.close 触发杀进程组）能让它消失。pid 的观测点用
/// 系统进程表按命令行匹配（连接快照里没有 pid，也不为此侵入产品代码）：
/// 连接前记录基线、连接后取差集，shutdown 后轮询断言差集清空。
#[test]
fn shutdown_kills_stubborn_child_process() {
    if !python_available() {
        eprintln!("跳过：系统没有 python3");
        return;
    }
    let before = stubborn_server_pids();
    let pool = McpPool::new();
    let snapshot = pool
        .connect(&server_config("stubborn", "stubborn_server.py"))
        .expect("连接 stubborn MCP 服务器应成功");
    assert_eq!(snapshot.server_name, "craftwork-stubborn-mcp");

    let spawned: Vec<u32> = stubborn_server_pids().difference(&before).copied().collect();
    assert!(!spawned.is_empty(), "连接后应能观察到 stubborn_server.py 子进程");

    pool.shutdown();

    // Windows 上每次观测要起一个 PowerShell（约一秒），预算相应放宽；
    // Unix 上正常 1-2 个轮询周期（进程组被 SIGTERM 后立即退出）就能收敛。
    let deadline = Instant::now() + Duration::from_secs(30);
    while spawned.iter().any(|pid| stubborn_server_pids().contains(pid)) {
        assert!(
            Instant::now() < deadline,
            "shutdown 后 stubborn 子进程仍存活：{spawned:?}（剩余：{:?}）",
            stubborn_server_pids()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
