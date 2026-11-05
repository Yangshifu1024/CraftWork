//! 平台基础能力：原子写、哈希、二进制探测、环境清洗、子进程执行。
//! 安全语义必须逐条保留：环境清洗防止模型密钥泄漏给工具子进程；进程组杀防止孤儿进程；有界缓冲防止输出撑爆内存。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

// ---------- 原子写 ----------

fn random_suffix() -> String {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 原子写文件：同目录临时文件 → 写入 → 原地 rename。
pub fn write_file_atomic(path: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("路径缺少父目录：{}", path.display()))?;
    let tmp: PathBuf = dir.join(format!(".craftwork-{}.tmp", random_suffix()));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("创建临时文件失败：{e}"))?;
        file.write_all(data).map_err(|e| format!("写入临时文件失败：{e}"))?;
        set_mode(&tmp, mode);
        fs::rename(&tmp, path).map_err(|e| format!("原子替换失败：{e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// 写文本（UTF-8）。
pub fn write_file_atomic_str(path: &Path, data: &str, mode: u32) -> Result<(), String> {
    write_file_atomic(path, data.as_bytes(), mode)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {
    // Windows 无 POSIX mode 语义
}

// ---------- 哈希与探测 ----------

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// 启发式二进制判定：前 8000 字节含 NUL 即视为二进制。
pub fn is_binary(data: &[u8]) -> bool {
    let n = data.len().min(8000);
    data[..n].contains(&0)
}

/// 备份文件名：<base>.<yyyy-MM-dd-HH-mm-ss.SSS>（毫秒级时间戳，本地时区）。
pub fn backup_name(abs_path: &Path) -> String {
    let base = abs_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let now = chrono::Local::now();
    format!("{base}.{}", now.format("%Y%m%d-%H%M%S%.3f"))
}

// ---------- 环境清洗 ----------

/// 子进程环境白名单：只保留无害变量。
const ENV_ALLOW: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "TERM", "TMPDIR", "TZ", "PWD", "COLORTERM", "SSH_AUTH_SOCK",
];

const ENV_DENY_SUBSTR: &[&str] = &["API_KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];
const ENV_DENY_PREFIX: &[&str] = &[
    "AWS_", "GITHUB_", "GH_", "OPENAI_", "ANTHROPIC_", "DEEPSEEK_", "ZHIPU_", "DASHSCOPE_",
];

/// 清洗当前进程环境：白名单 + 敏感词剔除 + 云厂商标识前缀剔除。
/// 目的：防止模型密钥通过环境变量泄漏给工具子进程。
pub fn sanitized_env() -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    for (key, value) in std::env::vars() {
        let upper = key.to_uppercase();
        // Windows 环境变量名不区分大小写（返回 "Path" 而非 "PATH"）：白名单按忽略大小写比较
        if ENV_ALLOW.iter().any(|allowed| allowed.eq_ignore_ascii_case(&key)) || upper.starts_with("LC_") {
            env.push((key, value));
            continue;
        }
        if ENV_DENY_SUBSTR.iter().any(|s| upper.contains(s)) {
            continue;
        }
        if ENV_DENY_PREFIX.iter().any(|p| upper.starts_with(p)) {
            continue;
        }
    }
    env.push(("CRAFTWORK".to_string(), "1".to_string()));
    env
}

// ---------- 子进程执行 ----------

/// 子进程执行规格；执行命令时由调用方拼 [-c, command]。
pub struct RunSpec {
    pub file: String,
    pub args: Vec<String>,
    /// 空串视同未设置
    pub dir: Option<String>,
    pub timeout_ms: Option<u64>,
    /// 默认 32 KiB
    pub output_limit: Option<usize>,
}

#[derive(Debug)]
pub struct RunResult {
    pub output: String,
    /// 超时/信号 = -1
    pub exit_code: i32,
    pub timed_out: bool,
    /// 非零退出码不算错
    pub error: Option<String>,
}

/// 有界缓冲：满后丢弃后续写入，读取时追加截断说明。
struct LimitedBuffer {
    inner: std::sync::Mutex<LimitedState>,
}

struct LimitedState {
    bytes: Vec<u8>,
    overflow: bool,
}

impl LimitedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(LimitedState {
                bytes: Vec::with_capacity(limit.min(8192)),
                overflow: false,
            }),
        }
    }

    fn write(&self, data: &[u8], limit: usize) {
        let mut state = self.inner.lock().unwrap();
        if state.overflow {
            return;
        }
        let remaining = limit.saturating_sub(state.bytes.len());
        if data.len() > remaining {
            state.bytes.extend_from_slice(&data[..remaining]);
            state.overflow = true;
            return;
        }
        state.bytes.extend_from_slice(data);
    }

    fn to_string_limited(&self) -> String {
        let state = self.inner.lock().unwrap();
        let base = String::from_utf8_lossy(&state.bytes).to_string();
        if state.overflow {
            format!("{base}\n…（输出已截断）")
        } else {
            base
        }
    }
}

fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            // 子进程以独立进程组启动：先杀整组，失败回落单进程
            if libc::kill(-(pid as i32), libc::SIGKILL) != 0 {
                let _ = libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

/// 执行子进程：独立进程组、清洗环境、共享 stdout/stderr 有界缓冲、超时优先上报。
pub fn run(spec: RunSpec) -> RunResult {
    let limit = spec.output_limit.unwrap_or(32 * 1024);
    let timeout = Duration::from_millis(spec.timeout_ms.unwrap_or(120_000));

    let mut command = Command::new(&spec.file);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    for (k, v) in sanitized_env() {
        command.env(k, v);
    }
    if let Some(dir) = &spec.dir {
        command.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组，便于整组杀（对应 Node detached: true）
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return RunResult {
                output: String::new(),
                exit_code: -1,
                timed_out: false,
                error: Some(err.to_string()),
            };
        }
    };
    let pid = child.id();

    let buffer = std::sync::Arc::new(LimitedBuffer::new(limit));
    let mut stdout = child.stdout.take().map(|mut pipe| {
        let buffer = std::sync::Arc::clone(&buffer);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            use std::io::Read;
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buffer.write(&chunk[..n], limit);
            }
        })
    });
    let mut stderr = child.stderr.take().map(|mut pipe| {
        let buffer = std::sync::Arc::clone(&buffer);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            use std::io::Read;
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buffer.write(&chunk[..n], limit);
            }
        })
    });

    // 带超时等待：try_wait 轮询（50ms 粒度）
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    timed_out = true;
                    kill_process_group(pid);
                    // 给读线程留出收尾时间
                    std::thread::sleep(Duration::from_millis(200));
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                let _ = stdout.take();
                let _ = stderr.take();
                return RunResult {
                    output: buffer.to_string_limited(),
                    exit_code: -1,
                    timed_out: false,
                    error: Some(err.to_string()),
                };
            }
        }
    };

    let _ = stdout.take();
    let _ = stderr.take();
    let exit_code = match (&status, timed_out) {
        (_, true) => -1,
        // None = 被信号终止
        (Some(status), false) => status.code().unwrap_or(-1),
        (None, false) => -1,
    };
    RunResult {
        output: buffer.to_string_limited(),
        exit_code,
        timed_out,
        error: None,
    }
}

/// 同步执行的异步包装（供 async 壳层在阻塞线程池调用）。
pub async fn run_async(spec: RunSpec) -> RunResult {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(run(spec));
    });
    rx.recv().unwrap_or(RunResult {
        output: String::new(),
        exit_code: -1,
        timed_out: false,
        error: Some("子进程结果通道关闭".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_overwrites_existing_file() {
        let dir = std::env::temp_dir().join(format!("craftwork-atomic-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        write_file_atomic_str(&path, "one", 0o644).unwrap();
        write_file_atomic_str(&path, "two", 0o644).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        // 同目录无残留临时文件
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".craftwork-"))
            .collect();
        assert!(leftovers.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn binary_probe_contains_nul_is_binary() {
        assert!(!is_binary(b"plain text"));
        assert!(is_binary(&[0x50, 0x4b, 0x00, 0x01]));
    }

    #[cfg(unix)]
    #[test]
    fn child_process_exec_captures_output_and_exit_code() {
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), "echo hello; echo err >&2; exit 3".into()],
            dir: None,
            timeout_ms: None,
            output_limit: None,
        });
        assert_eq!(result.exit_code, 3);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("err"));
        assert!(result.error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn child_process_timeout_kills_group_and_reports() {
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), "echo start; sleep 30".into()],
            dir: None,
            timeout_ms: Some(500),
            output_limit: None,
        });
        assert!(result.timed_out);
        assert_eq!(result.exit_code, -1);
        assert!(result.output.contains("start"));
    }

    #[cfg(unix)]
    #[test]
    fn child_process_grandchild_terminated_with_process_group() {
        // 孙进程 sleep 30：若未整组杀，输出会超时后仍等不到收尾
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), "sh -c 'sleep 30' & wait".into()],
            dir: None,
            timeout_ms: Some(500),
            output_limit: None,
        });
        assert!(result.timed_out);
        assert_eq!(result.exit_code, -1);
    }

    #[test]
    fn sanitized_env_strips_secrets_keeps_allowlist() {
        // 本测试进程注入敏感变量，验证白名单与剔除规则
        unsafe { std::env::set_var("CRAFTWORK_TEST_API_KEY", "x") };
        unsafe { std::env::set_var("AWS_SECRET_ACCESS_KEY", "y") };
        let env = sanitized_env();
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(!keys.iter().any(|k| k.contains("API_KEY")));
        assert!(!keys.iter().any(|k| k.starts_with("AWS_")));
        assert!(keys.iter().any(|k| k.eq_ignore_ascii_case("PATH")), "PATH 应保留（Windows 下为 Path）");
        assert!(keys.contains(&"CRAFTWORK"));
        unsafe { std::env::remove_var("CRAFTWORK_TEST_API_KEY") };
        unsafe { std::env::remove_var("AWS_SECRET_ACCESS_KEY") };
    }
}
