//! 平台基础能力：原子写、哈希、二进制探测、环境清洗、子进程执行。
//! 安全语义必须逐条保留：环境清洗防止模型密钥泄漏给工具子进程；进程组杀防止孤儿进程；有界缓冲防止输出撑爆内存。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// 用户主目录：`HOME` → `USERPROFILE` → `HOMEDRIVE`+`HOMEPATH`。
///
/// 为什么不能只读 `HOME`：Windows 没有 `HOME`，只有 `USERPROFILE`（以及由它派生的
/// `HOMEDRIVE`/`HOMEPATH`）。此前 shell 环境接入、设置页 shell 清单、数据目录解析都直接
/// `std::env::var("HOME")`，在 Windows 上拿到空串——shell 因此读不到任何启动配置，
/// 数据目录也落进 `\\tmp` 这类兜底路径。这条回退链是这三处唯一的取法。
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var("HOME").ok().filter(|h| !h.trim().is_empty()) {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = std::env::var("USERPROFILE").ok().filter(|h| !h.trim().is_empty()) {
        return Some(PathBuf::from(profile));
    }
    // 旧式 Windows（域漫游配置偶尔只有拆分变量）与精简环境兜底
    let drive = std::env::var("HOMEDRIVE").ok().filter(|s| !s.trim().is_empty());
    let path = std::env::var("HOMEPATH").ok().filter(|s| !s.trim().is_empty());
    match (drive, path) {
        (Some(drive), Some(path)) => Some(PathBuf::from(format!("{drive}{path}"))),
        _ => None,
    }
}

/// [`home_dir`] 的字符串形式：shell 环境捕获与配置文件列表都以字符串传 home，
/// 各调用点不再各自 `var("HOME")`。取不到返回空串——与旧的 `var("HOME")` 行为一致，
/// 调用方按「无 home」处理。
pub fn home_dir_string() -> String {
    home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// SHA-256 摘要的小写十六进制串（写前备份与外部修改检测用）。
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

// ---------- 目录体积统计（存储页用） ----------
//
// 只服务「展示用体积估算」这一个消费者：统计结果要能解释「这一坨占了多少」，
// 不追求与 `du` 逐字节一致（不按 inode 去重硬链接、不计文件系统块对齐开销）。
//
// 三条纪律：
// 1. **不跟随符号链接**（`symlink_metadata` + 只在真实目录上下探）。工作树与技能目录
//    里可能有指回上层或指到远端的链接，跟随会成环、会统计到工作区之外，与「数据目录
//    占多少」这个问题的口径不符。
// 2. **条目数有上限**：`worktrees/` 下是整棵 git 工作树，可能是几十万个文件。超过上限
//    即停止下探并置 `truncated`——界面据此显示「≥」而不是一个假精确的数。
// 3. **永不报错**：路径不存在、无权限、读目录失败一律按「没读到」处理，累加继续。
//    体积统计不该让存储页报错，一个权限异常的子目录不该毁掉整张表。

/// 目录体积统计结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirUsage {
    /// 常规文件字节数之和（符号链接本身不计入，也不下探）。
    pub bytes: u64,
    /// 计入的常规文件数量。
    pub files: u64,
    /// 是否因条目上限提前截止（界面显示「≥」）。
    pub truncated: bool,
}

/// 递归统计目录体积，最多访问 `max_entries` 个目录项。
///
/// 参数是**目录**：传文件或不存在路径都返回零值。`max_entries` 为 0 按无上限处理
/// （调用方想全量统计时用；存储页固定传 [`DIR_USAGE_DEFAULT_MAX_ENTRIES`]）。
pub fn dir_usage(path: &Path, max_entries: u64) -> DirUsage {
    let mut usage = DirUsage::default();
    let limit = if max_entries == 0 { u64::MAX } else { max_entries };
    let mut visited: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries {
            if visited >= limit {
                usage.truncated = true;
                return usage;
            }
            visited += 1;
            let Ok(entry) = entry else { continue };
            // symlink_metadata 不解析链接：链接本身按「跳过」处理，既不统计大小也不下探
            let Ok(meta) = fs::symlink_metadata(entry.path()) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                usage.bytes += meta.len();
                usage.files += 1;
            }
        }
    }
    usage
}

/// 存储页统计目录体积时的默认条目上限：20 万项。足够覆盖日常数据目录，
/// 又能在工作树上把单次扫描控制在可接受的时间内。
pub const DIR_USAGE_DEFAULT_MAX_ENTRIES: u64 = 200_000;

/// 删除单个路径（文件或目录，目录递归删），返回是否真的删掉了东西。
///
/// 供存储页的清理动作使用：清理动的是**可丢弃数据**，失败只应记录、不应中断整批
/// （部分成功优于全盘放弃），所以本函数只回 bool，错误由调用方按需自己读。
pub fn remove_path(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else { return false };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path).is_ok()
    } else {
        fs::remove_file(path).is_ok()
    }
}

// ---------- 用户身份探测 ----------
//
// user_display_name（git 全局 user.name → 系统用户名 → "CraftWork"）已随 D13 迁移
// 落到 craft-git（`craft_git::user_display_name`，gix 进程内读取全局配置）：本 crate
// 不得依赖 craft-git（依赖方向是 craft-git → craft-core，反向成环），git 配置读取
// 又只允许出现在 craft-git，故该函数整体移出。调用方见 src-tauri services::get_info。

// ---------- 环境清洗 ----------
//
// 语义：**黑名单剔除**（决策 D18）。此前是纯白名单，只放行 12 个变量——
// 安全但废掉了用户 shell 配置里的一切（node/cargo/pnpm 全不可见），
// 进而逼出「用绝对路径绕过工具链管理」这类更危险的绕行。
//
// 现在改为：继承进程环境，只剔除**可能承载体密**的变量。判定规则全部可逐条解释
// （`shellenv::is_sensitive_key`），不用模糊匹配——裸 `TOKEN`/`AUTH`/`SESSION`
// 会误杀 `SSH_AUTH_SOCK`、`DBUS_SESSION_BUS_ADDRESS` 这类功能依赖。
//
// 注意：真正的凭据不进 app 进程环境（keys.json 只存在 Rust 侧、不写环境变量），
// 这条规则防的是「用户的 shell 配置里自己导出了密钥」这种情况。

/// Python UTF-8 保证：用户显式设置时尊重用户值，否则注入默认值，保证批量脚本的
/// 输出与文件名处理跨平台均为 UTF-8（非 UTF-8 默认编码的 Windows 尤其受益）。
fn ensure_python_utf8(env: &mut Vec<(String, String)>) {
    for (key, default) in [("PYTHONIOENCODING", "utf-8"), ("PYTHONUTF8", "1")] {
        let exists = env.iter().any(|(k, _)| k.eq_ignore_ascii_case(key));
        if !exists {
            env.push((key.to_string(), default.to_string()));
        }
    }
}

/// 清洗给定环境：继承全部变量，剔除敏感键，末尾追加哨兵与 Python UTF-8 保证。
///
/// 剔除规则见 [`crate::shellenv::is_sensitive_key`]；保留例外（`SSH_AUTH_SOCK` 等
/// 名字含敏感词但功能必需）也在那里维护，两处口径只有一份。
pub fn sanitize_env_from<I>(vars: I) -> Vec<(String, String)>
where
    I: Iterator<Item = (String, String)>,
{
    let mut env: Vec<(String, String)> = Vec::new();
    for (key, value) in vars {
        if crate::shellenv::is_sensitive_key(&key) {
            continue;
        }
        env.push((key, value));
    }
    // 哨兵变量：子进程可据此感知运行于 CraftWork 环境
    if !env.iter().any(|(k, _)| k == "CRAFTWORK") {
        env.push(("CRAFTWORK".to_string(), "1".to_string()));
    }
    ensure_python_utf8(&mut env);
    env
}

/// 清洗当前进程环境，规则见 [`sanitize_env_from`]。
pub fn sanitized_env() -> Vec<(String, String)> {
    sanitize_env_from(std::env::vars())
}

// ---------- 子进程执行 ----------

/// 子进程执行规格；执行命令时由调用方拼 [-c, command]。
///
/// `env` 为 `None` 时使用 [`sanitized_env`]（进程环境按黑名单清洗后的结果）；
/// 由 shell 环境接入装配时显式传入捕获结果——见 `shellenv` 与决策 D18。
#[derive(Debug, Clone, Default)]
pub struct RunSpec {
    pub file: String,
    pub args: Vec<String>,
    /// 空串视同未设置
    pub dir: Option<String>,
    pub timeout_ms: Option<u64>,
    /// 默认 32 KiB
    pub output_limit: Option<usize>,
    /// 子进程环境；`None` = 用进程环境的清洗结果（默认路径）。
    pub env: Option<Vec<(String, String)>>,
    /// 脱离控制终端（Unix：`setsid` 新会话；新会话同时是新的进程组，组长为子进程 pid）。
    ///
    /// 只给需要跑**交互式 shell** 的场景用（shell 环境捕获）：`-i` 的 shell 启动时会
    /// `tcsetpgrp` 把自身设为终端前台组；若它处在后台进程组，内核用 `SIGTTOU` 把它停住
    /// （状态 T），子进程永不退出，最终表现为「读取 shell 环境超时」。无控制终端时
    /// shell 会跳过这套 job control 初始化。
    pub new_session: bool,
}

impl RunSpec {
    /// 以可执行文件路径构造，其余字段取默认。
    pub fn new(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            ..Self::default()
        }
    }
}

/// 子进程执行结果；非零退出码属于正常输出而非错误（error 只承载无法执行类失败）。
#[derive(Debug)]
pub struct RunResult {
    pub output: String,
    /// 超时/信号 = -1
    pub exit_code: i32,
    pub timed_out: bool,
    /// 非零退出码不算错
    pub error: Option<String>,
    /// 输出是否已被 `output_limit` 截断（缓冲溢出）。
    ///
    /// 只报事实、不追加标记文本：截断提示由调用方统一渲染一份。历史上这里直接往正文
    /// 追加「…（输出已截断）」，该标记把长度顶过调用方的同类上限，随后被切掉，
    /// 于是模型看到的「原始长度」是截断后的长度（D29）。
    pub output_truncated: bool,
    /// 因 `output_limit` 被丢弃的字节数（`output_truncated` 为 false 时恒 0）。
    ///
    /// 由缓冲累计写入量与保留量相减算得，是**真实丢弃量**；调用方申报截断时直接引用它。
    pub dropped_bytes: usize,
}

/// 有界缓冲：满后丢弃后续写入，读取时追加截断说明。
struct LimitedBuffer {
    inner: std::sync::Mutex<LimitedState>,
}

struct LimitedState {
    bytes: Vec<u8>,
    overflow: bool,
    /// 子进程累计写出的字节数（含被丢弃部分）：截断后仍能报出真实丢弃量。
    total: usize,
}

impl LimitedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(LimitedState {
                bytes: Vec::with_capacity(limit.min(8192)),
                overflow: false,
                total: 0,
            }),
        }
    }

    fn write(&self, data: &[u8], limit: usize) {
        let mut state = self.inner.lock().unwrap();
        // 总量照常累计：满了也要记，否则算不出真实丢弃量
        state.total += data.len();
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

    /// 取出正文，并报告截断事实：`(正文, 是否溢出, 丢弃字节数)`。
    ///
    /// 丢弃量在这里算准（`total - 已保留字节`），调用方不必用「截断后的长度」反推——
    /// 那个反推正是历史上「原始长度」报错的来源（D29）。
    fn take_output(&self) -> (String, bool, usize) {
        let state = self.inner.lock().unwrap();
        let text = String::from_utf8_lossy(&state.bytes).to_string();
        let dropped = state.total.saturating_sub(state.bytes.len());
        (text, state.overflow, dropped)
    }
}

/// Windows 子进程创建标志：不分配控制台窗口。
///
/// 为什么每条路径都要带：GUI 应用（`windows_subsystem = "windows"`，见 `src-tauri/src/main.rs`）
/// 自己没有控制台，而 `std::process::Command` 在 Windows 上会为新进程分配一个，于是每执行
/// 一次命令屏幕上就闪一个黑窗。前台 [`run`]、后台 [`spawn_detached`]、shell 环境捕获三条
/// 路径共用 [`build_command`]，必须在那里统一加，不要各调用点各自补；MCP 的 stdio 子进程
/// 走自己的 tokio `Command`（`craft-mcp`），单独同步这一标志。
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// SIGKILL 整个进程组（含孙进程）；组杀失败回落单进程杀。
pub fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    {
        unsafe {
            // 子进程以独立进程组启动：先杀整组，失败回落单进程
            if libc::kill(-(pid as i32), libc::SIGKILL) != 0 {
                let _ = libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        // Windows 无进程组语义：taskkill /T 沿 PID 杀整棵进程树（含孙进程），/F 强制。
        // 必须等 taskkill 退出再返回——外层 sleep(200ms) + child.wait() 依赖「杀已生效」，
        // 否则 wait 会阻塞在一个不会被杀的子进程上（杀树失效，工具调用永久挂死）。
        // CREATE_NO_WINDOW 防止闪命令行窗口（与 build_command 同一常量）。
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

/// 按 RunSpec 装配一个已就绪、尚未启动的子进程。
///
/// 前台 [`run`] 与后台 [`spawn_detached`] 共用这份装配：独立进程组、环境清洗、
/// stdin 关闭、Windows 不分配控制台窗口。各条路径的**安全语义必须完全一致**——任何
/// 一侧漏掉 `env_clear` + 清洗都会把凭据泄漏给子进程；漏掉 Windows 的
/// `CREATE_NO_WINDOW` 则会让命令执行时闪出黑窗（见该常量注释）。
fn build_command(spec: &RunSpec, stdout: Stdio, stderr: Stdio) -> Command {
    let mut command = Command::new(&spec.file);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .env_clear();
    // 显式传入的环境（shell 环境接入）已由调用方清洗过；否则用进程环境的清洗结果。
    let env = match &spec.env {
        Some(env) => env.clone(),
        None => sanitized_env(),
    };
    for (k, v) in env {
        command.env(k, v);
    }
    if let Some(dir) = &spec.dir {
        command.current_dir(dir);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        if spec.new_session {
            // 新会话：同时是新的进程组（组长 = 子进程 pid），并且**没有控制终端**。
            // 标准库的 setpgid 在 pre_exec 之前执行，二者同开会因「已是组长」让 setsid
            // 失败（EPERM），所以这里只调 setsid，不再 process_group(0)。
            // 杀整组仍按 pid 走：pid 就是新进程组的组长。
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        } else {
            // 独立进程组，便于整组杀（对应 Node detached: true）
            command.process_group(0);
        }
    }
    #[cfg(windows)]
    {
        // 不分配控制台窗口：GUI 进程没有自己的控制台，命令子进程默认会弹一个黑窗
        // 一闪而过。放在这份共用装配里，前台 / 后台 / 环境捕获三条路径一次覆盖。
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// 后台子进程句柄：调用方持有 pid、输出文件路径与可等待的子进程句柄。
///
/// 为什么要交出 `Child`：Runner 的观察线程必须能 `wait()` 到**准确退出码**。
/// 只给 pid 的话，进程被回收后退出码就永远拿不到（Unix 上 `waitpid` 竞态、Windows
/// 上干脆没有等价物）。`Child` 交给观察线程后，进程退出即线程结束，不轮询、不白占。
pub struct BgProcess {
    /// 进程组组长 pid（整组杀做 `kill(-pid)`）
    pub pid: u32,
    /// 输出文件绝对路径（stdout 与 stderr 合并直写）
    pub output_path: String,
    /// 子进程句柄：调用方负责 `wait()`（或交给观察线程）
    pub child: std::process::Child,
}

/// 启动一个后台子进程：stdout/stderr 合并直写 `output_path`，立即返回句柄，不等待退出。
///
/// 与 [`run`] 的差异只在「谁来等」：`run` 在本线程轮询到进程结束，这里把等待责任交给
/// 上层（任务注册表 + 完成通知）。因此这里**不设超时**——后台任务的生命周期由调用方
/// 显式终止（`kill_process_group`）或自然结束。
///
/// 安全语义与 `run` 完全一致（同一份 [`build_command`] 装配）：独立进程组、环境白名单
/// 清洗、stdin 关闭。输出不经过有界缓冲——文件本身就是有界手段，读取时按需截断。
pub fn spawn_detached(spec: &RunSpec, output_path: &Path) -> Result<BgProcess, String> {
    use std::fs::File;
    let file = File::options()
        .create(true)
        .append(true)
        .open(output_path)
        .map_err(|e| format!("创建任务输出文件失败：{e}"))?;
    let stderr_file = file.try_clone().map_err(|e| format!("复制输出句柄失败：{e}"))?;
    let mut command = build_command(spec, Stdio::from(file), Stdio::from(stderr_file));
    let child = command.spawn().map_err(|e| format!("启动后台命令失败：{e}"))?;
    let pid = child.id();
    Ok(BgProcess {
        pid,
        output_path: output_path.to_string_lossy().to_string(),
        child,
    })
}

/// 等 stdout/stderr 读线程把管道数据搬进缓冲（有界等待 500ms）。
///
/// 子进程退出 ≠ 读线程已经跑完：`try_wait` 返回后直接读缓冲，会在慢机器上偶发
/// 空输出（CI 实测 `/bin/sh -c 'echo hello'` 的 output 不含 hello）。若孙进程仍
/// 持有管道导致读线程收不到 EOF，最多等 500ms 就带着已读到的快照返回，不把整个
/// 工具调用挂死。
fn wait_for_readers(stdout_done: &AtomicBool, stderr_done: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if stdout_done.load(Ordering::SeqCst) && stderr_done.load(Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// 执行子进程：独立进程组、清洗环境、共享 stdout/stderr 有界缓冲、超时优先上报。
pub fn run(spec: RunSpec) -> RunResult {
    let limit = spec.output_limit.unwrap_or(32 * 1024);
    let timeout = Duration::from_millis(spec.timeout_ms.unwrap_or(120_000));

    let mut command = build_command(&spec, Stdio::piped(), Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return RunResult {
                output: String::new(),
                exit_code: -1,
                timed_out: false,
                error: Some(err.to_string()),
                output_truncated: false,
                dropped_bytes: 0,
            };
        }
    };
    let pid = child.id();

    let buffer = std::sync::Arc::new(LimitedBuffer::new(limit));
    let stdout_done = std::sync::Arc::new(AtomicBool::new(false));
    let stderr_done = std::sync::Arc::new(AtomicBool::new(false));
    let mut stdout = child.stdout.take().map(|mut pipe| {
        let buffer = std::sync::Arc::clone(&buffer);
        let done = std::sync::Arc::clone(&stdout_done);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            use std::io::Read;
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buffer.write(&chunk[..n], limit);
            }
            done.store(true, Ordering::SeqCst);
        })
    });
    let mut stderr = child.stderr.take().map(|mut pipe| {
        let buffer = std::sync::Arc::clone(&buffer);
        let done = std::sync::Arc::clone(&stderr_done);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            use std::io::Read;
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                buffer.write(&chunk[..n], limit);
            }
            done.store(true, Ordering::SeqCst);
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
                wait_for_readers(&stdout_done, &stderr_done);
                let _ = stdout.take();
                let _ = stderr.take();
                let (output, output_truncated, dropped_bytes) = buffer.take_output();
                return RunResult {
                    output,
                    exit_code: -1,
                    timed_out: false,
                    error: Some(err.to_string()),
                    output_truncated,
                    dropped_bytes,
                };
            }
        }
    };

    wait_for_readers(&stdout_done, &stderr_done);
    let _ = stdout.take();
    let _ = stderr.take();
    let exit_code = match (&status, timed_out) {
        (_, true) => -1,
        // None = 被信号终止
        (Some(status), false) => status.code().unwrap_or(-1),
        (None, false) => -1,
    };
    let (output, output_truncated, dropped_bytes) = buffer.take_output();
    RunResult {
        output,
        exit_code,
        timed_out,
        error: None,
        output_truncated,
        dropped_bytes,
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
        output_truncated: false,
        dropped_bytes: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_usage_sums_nested_files() {
        let dir = std::env::temp_dir().join(format!("craftwork-dirusage-{}", random_suffix()));
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("root.txt"), b"12345").unwrap();
        fs::write(dir.join("a/one.txt"), b"123").unwrap();
        fs::write(dir.join("a/b/two.txt"), b"12").unwrap();
        let usage = dir_usage(&dir, 0);
        assert_eq!(usage.bytes, 10, "三层文件字节应全部累加");
        assert_eq!(usage.files, 3);
        assert!(!usage.truncated);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dir_usage_reports_truncation_at_cap() {
        let dir = std::env::temp_dir().join(format!("craftwork-dirusage-cap-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            fs::write(dir.join(format!("f{i}.txt")), b"1234").unwrap();
        }
        // 上限 2：必然在读完全部 5 个文件前截止
        let usage = dir_usage(&dir, 2);
        assert!(usage.truncated, "达到条目上限应置 truncated");
        assert!(usage.files <= 5);
        // 无上限时能读全
        let full = dir_usage(&dir, 0);
        assert_eq!(full.files, 5);
        assert!(!full.truncated);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dir_usage_missing_dir_is_zero() {
        let missing = std::env::temp_dir().join(format!("craftwork-dirusage-none-{}", random_suffix()));
        assert_eq!(dir_usage(&missing, 0), DirUsage::default(), "不存在的目录返回零值而不报错");
        // 传文件同样按零值处理，不下探
        let dir = std::env::temp_dir().join(format!("craftwork-dirusage-file-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.txt");
        fs::write(&file, b"123").unwrap();
        assert_eq!(dir_usage(&file, 0), DirUsage::default());
        fs::remove_dir_all(&dir).ok();
    }

    /// 符号链接既不计入自身字节，也不下探到目标——否则统计会跑到数据目录之外、并可能成环。
    #[cfg(unix)]
    #[test]
    fn dir_usage_does_not_follow_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("craftwork-dirusage-link-{}", random_suffix()));
        let outside = std::env::temp_dir().join(format!("craftwork-dirusage-out-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("big.bin"), vec![0u8; 4096]).unwrap();
        symlink(&outside, dir.join("link-dir")).unwrap();
        fs::write(dir.join("real.txt"), b"12").unwrap();
        let usage = dir_usage(&dir, 0);
        assert_eq!(usage.bytes, 2, "链接目标的内容不得计入");
        assert_eq!(usage.files, 1);
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn remove_path_handles_files_and_directories() {
        let dir = std::env::temp_dir().join(format!("craftwork-removepath-{}", random_suffix()));
        fs::create_dir_all(dir.join("sub/deep")).unwrap();
        fs::write(dir.join("sub/deep/x.txt"), b"1").unwrap();
        assert!(remove_path(&dir.join("sub")), "目录应整棵删除");
        assert!(!dir.join("sub").exists());
        assert!(!remove_path(&dir.join("sub")), "已不存在的路径返回 false");
        fs::remove_dir_all(&dir).ok();
    }

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

    // user_display_name 相关测试随函数迁移至 craft-git（见本文件「用户身份探测」注释）。

    #[cfg(unix)]
    #[test]
    fn child_process_exec_captures_output_and_exit_code() {
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), "echo hello; echo err >&2; exit 3".into()],
            dir: None,
            timeout_ms: None,
            output_limit: None,
        env: None,
        new_session: false,
        });
        assert_eq!(result.exit_code, 3);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("err"));
        assert!(result.error.is_none());
    }

    /// 回归（D29）：截断时只报事实与真实丢弃量，不往正文追加标记文本。
    ///
    /// 历史行为：`LimitedBuffer` 在正文后追加「…（输出已截断）」，该标记把长度顶过
    /// 调用方的同类上限，随后被切掉；调用方再拿「截断后的长度」当原始长度报给模型，
    /// 于是 50000 字节的输出被告知「原始长度 32793 字节」。
    #[cfg(unix)]
    #[test]
    fn truncated_output_reports_dropped_bytes_without_marker_text() {
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            // 输出 200000 字节，远超 1000 字节上限
            args: vec!["-c".into(), "head -c 200000 /dev/zero | tr '\\0' 'x'".into()],
            dir: None,
            timeout_ms: None,
            output_limit: Some(1000),
            env: None,
            new_session: false,
        });
        assert!(result.output_truncated, "应报告已截断");
        // 正文不得含标记文本（标记由调用方唯一渲染）
        assert!(!result.output.contains("已截断"), "正文不应自带标记: {}", &result.output[result.output.len().saturating_sub(80)..]);
        assert!(result.output.len() <= 1000, "保留量不得超过上限: {}", result.output.len());
        // 丢弃量是真实值（总量 − 保留量），不是保留量本身
        assert_eq!(result.dropped_bytes, 200_000 - result.output.len());
        assert!(result.dropped_bytes >= 199_000, "丢弃量应接近 199000，实际 {}", result.dropped_bytes);
    }

    /// 未超上限时不报截断、丢弃量为 0。
    #[cfg(unix)]
    #[test]
    fn untruncated_output_reports_zero_dropped_bytes() {
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), "echo short".into()],
            dir: None,
            timeout_ms: None,
            output_limit: Some(1000),
            env: None,
            new_session: false,
        });
        assert!(!result.output_truncated);
        assert_eq!(result.dropped_bytes, 0);
        assert!(result.output.contains("short"));
    }

    /// 回归：子进程退出后读线程可能还没被调度，直接读缓冲会偶发空输出。
    /// 连跑 30 次，每次都必须拿到 hello（CI Linux 上曾偶发）。
    #[cfg(any(unix, windows))]
    #[test]
    fn child_process_exec_output_is_not_lost_after_fast_exit() {
        #[cfg(unix)]
        let (file, args) = ("/bin/sh", vec!["-c".to_string(), "echo hello".to_string()]);
        #[cfg(windows)]
        let (file, args) = ("cmd", vec!["/C".to_string(), "echo hello".to_string()]);
        for attempt in 0..30 {
            let result = run(RunSpec {
                file: file.to_string(),
                args: args.clone(),
                dir: None,
                timeout_ms: None,
                output_limit: None,
                env: None,
                new_session: false,
            });
            assert_eq!(result.exit_code, 0, "第 {attempt} 次退出码异常：{result:?}");
            assert!(result.output.contains("hello"), "第 {attempt} 次输出丢失：{result:?}");
            assert!(result.error.is_none(), "第 {attempt} 次执行报错：{result:?}");
        }
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
        env: None,
        new_session: false,
        });
        assert!(result.timed_out);
        assert_eq!(result.exit_code, -1);
        assert!(result.output.contains("start"));
    }

    // ---------- 后台子进程（`crates/craft-agent/src/shell_tasks.rs` 模块头） ----------

    #[cfg(unix)]
    #[test]
    fn spawn_detached_writes_merged_output_to_file_and_returns_exit_code() {
        let dir = std::env::temp_dir().join(format!("craftwork-bg-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("task.log");
        let mut spawned = spawn_detached(
            &RunSpec {
                file: "/bin/sh".into(),
                args: vec!["-c".into(), "echo out-line; echo err-line >&2; exit 7".into()],
                dir: None,
                timeout_ms: None,
                output_limit: None,
            env: None,
            new_session: false,
            },
            &out,
        )
        .expect("启动应成功");
        // 句柄可等待：拿到准确退出码（这正是只给 pid 做不到的）
        let status = spawned.child.wait().unwrap();
        assert_eq!(status.code(), Some(7));
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("out-line"), "stdout 应直写文件：{text}");
        assert!(text.contains("err-line"), "stderr 与 stdout 合并同一文件：{text}");
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn spawn_detached_output_file_exists_before_command_writes() {
        // 启动前就要建好文件：模型拿到路径后可以立刻读取，不必等第一行输出
        let dir = std::env::temp_dir().join(format!("craftwork-bg-early-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("task.log");
        let mut spawned = spawn_detached(
            &RunSpec {
                file: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                dir: None,
                timeout_ms: None,
                output_limit: None,
            env: None,
            new_session: false,
            },
            &out,
        )
        .unwrap();
        assert!(out.exists(), "文件应在命令产生输出之前就存在");
        kill_process_group(spawned.pid);
        let _ = spawned.child.wait();
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn spawn_detached_child_dies_with_process_group_kill() {
        // 孙进程场景：整组杀必须带走后代，否则后台任务会留孤儿
        let dir = std::env::temp_dir().join(format!("craftwork-bg-group-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("task.log");
        let mut spawned = spawn_detached(
            &RunSpec {
                file: "/bin/sh".into(),
                args: vec!["-c".into(), "sh -c 'sleep 30' & wait".into()],
                dir: None,
                timeout_ms: None,
                output_limit: None,
            env: None,
            new_session: false,
            },
            &out,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        kill_process_group(spawned.pid);
        let status = spawned.child.wait().unwrap();
        assert!(status.code().is_none(), "被信号终止时没有退出码：{status:?}");
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn child_process_grandchild_terminated_with_process_group() {        // 孙进程 sleep 30：若未整组杀，输出会超时后仍等不到收尾
        let result = run(RunSpec {
            file: "/bin/sh".into(),
            args: vec!["-c".into(), "sh -c 'sleep 30' & wait".into()],
            dir: None,
            timeout_ms: Some(500),
            output_limit: None,
        env: None,
        new_session: false,
        });
        assert!(result.timed_out);
        assert_eq!(result.exit_code, -1);
    }

    // Windows 等价测试（无真机，CI 的 windows runner 是唯一验证场）。
    // 命令等价：`echo x; echo e >&2; exit n` → `echo x & echo e >&2 & exit /b n`；
    // `sleep N` → `ping -n N+1 127.0.0.1 > nul`（cmd 的 `timeout /t` 在无 stdin 环境
    // 报「输入重定向不受支持」，不可用）；多级进程树用 cmd 嵌套 cmd。

    #[cfg(windows)]
    #[test]
    fn spawn_detached_writes_merged_output_to_file_and_returns_exit_code_windows() {
        let dir = std::env::temp_dir().join(format!("craftwork-bg-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("task.log");
        let mut spawned = spawn_detached(
            &RunSpec {
                file: "cmd".into(),
                args: vec!["/C".into(), "echo out-line & echo err-line >&2 & exit /b 7".into()],
                dir: None,
                timeout_ms: None,
                output_limit: None,
            env: None,
            new_session: false,
            },
            &out,
        )
        .expect("启动应成功");
        let status = spawned.child.wait().unwrap();
        assert_eq!(status.code(), Some(7));
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("out-line"), "stdout 应直写文件：{text}");
        assert!(text.contains("err-line"), "stderr 与 stdout 合并同一文件：{text}");
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn spawn_detached_output_file_exists_before_command_writes_windows() {
        let dir = std::env::temp_dir().join(format!("craftwork-bg-early-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("task.log");
        let mut spawned = spawn_detached(
            &RunSpec {
                file: "cmd".into(),
                args: vec!["/C".into(), "ping -n 31 127.0.0.1 > nul".into()],
                dir: None,
                timeout_ms: None,
                output_limit: None,
            env: None,
            new_session: false,
            },
            &out,
        )
        .unwrap();
        assert!(out.exists(), "文件应在命令产生输出之前就存在");
        // 杀树失效时这里的 wait() 会永久阻塞——挂死即暴露（CI 超时兜底）
        kill_process_group(spawned.pid);
        let _ = spawned.child.wait();
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn spawn_detached_child_dies_with_process_group_kill_windows() {
        // 三级进程树（cmd → cmd → ping）：taskkill /T 必须整树终止。
        // Windows 与 Unix 断言不同：taskkill /F 强杀的退出码是 1，不是「无退出码」；
        // 核心判据是 wait() 能返回（杀树失效则永久阻塞在等一个不会被杀的子进程）。
        let dir = std::env::temp_dir().join(format!("craftwork-bg-group-{}", random_suffix()));
        fs::create_dir_all(&dir).unwrap();
        let out = dir.join("task.log");
        let mut spawned = spawn_detached(
            &RunSpec {
                file: "cmd".into(),
                args: vec!["/C".into(), "cmd /C ping -n 31 127.0.0.1 > nul".into()],
                dir: None,
                timeout_ms: None,
                output_limit: None,
            env: None,
            new_session: false,
            },
            &out,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        kill_process_group(spawned.pid);
        let status = spawned.child.wait().unwrap();
        assert_ne!(status.code(), None, "进程应已终止（强杀退出码 1）：{status:?}");
        fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn child_process_grandchild_terminated_with_process_group_windows() {
        // cmd → cmd → ping 三级树：若 /T 不递归杀，超时收尾会等不到孙进程退出
        let result = run(RunSpec {
            file: "cmd".into(),
            args: vec!["/C".into(), "cmd /C ping -n 31 127.0.0.1 > nul".into()],
            dir: None,
            timeout_ms: Some(500),
            output_limit: None,
        env: None,
        new_session: false,
        });
        assert!(result.timed_out);
        assert_eq!(result.exit_code, -1);
    }

    // ---------- Windows 子进程执行（无真机，CI 的 windows runner 是唯一验证场） ----------

    /// Windows 命令执行基础链路冒烟：cmd /C 解析、输出捕获、退出码透传。
    /// shell_exec 的 Windows 分支写死 cmd /C——这条链路此前零覆盖。
    #[cfg(windows)]
    #[test]
    fn child_process_exec_captures_output_and_exit_code_windows() {
        let result = run(RunSpec {
            file: "cmd".into(),
            args: vec!["/C".into(), "echo hello & exit /b 3".into()],
            dir: None,
            timeout_ms: None,
            output_limit: None,
        env: None,
        new_session: false,
        });
        assert_eq!(result.exit_code, 3);
        assert!(result.output.contains("hello"), "应捕获 stdout：{}", result.output);
        assert!(result.error.is_none());
    }

    /// Windows 超时杀树：cmd → ping 天然两级进程树，taskkill /T 必须整树终止。
    /// 杀树失效时 child.wait() 会永久阻塞——本测试挂死即暴露问题（无真机，靠 CI 逮）。
    #[cfg(windows)]
    #[test]
    fn child_process_timeout_kills_tree_and_reports_windows() {
        let started = std::time::Instant::now();
        let result = run(RunSpec {
            file: "cmd".into(),
            args: vec!["/C".into(), "ping -n 30 127.0.0.1".into()],
            dir: None,
            timeout_ms: Some(1_000),
            output_limit: None,
        env: None,
        new_session: false,
        });
        assert!(result.timed_out, "超时必须被上报");
        assert_eq!(result.exit_code, -1);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "超时后应在杀树生效后及时返回（taskkill 未生效则此处永久阻塞），实际 {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn sanitized_env_strips_secrets_keeps_allowlist() {
        // 本测试进程注入敏感变量，验证剔除规则（决策 D18：黑名单语义）
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

    /// 黑名单语义的核心断言：非敏感变量**要**被继承，否则用户 shell 配的工具链全不可见
    /// （这正是 D18 要修的问题）。
    #[test]
    fn sanitize_env_inherits_benign_variables() {
        let env = sanitize_env_from(
            [
                ("PNPM_HOME".to_string(), "/home/u/Library/pnpm".to_string()),
                ("CARGO_HOME".to_string(), "/home/u/Developer/cargo".to_string()),
                ("JAVA_HOME".to_string(), "/home/u/jdk".to_string()),
                ("PYTHONPATH".to_string(), "/tmp/x".to_string()),
                ("OPENAI_API_KEY".to_string(), "sk-x".to_string()),
            ]
            .into_iter(),
        );
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        for kept in ["PNPM_HOME", "CARGO_HOME", "JAVA_HOME", "PYTHONPATH"] {
            assert!(keys.contains(&kept), "{kept} 属无害变量，应继承");
        }
        assert!(!keys.contains(&"OPENAI_API_KEY"), "密钥类变量必须剔除");
    }

    #[test]
    fn sanitize_env_keeps_ssh_socket_despite_token_like_name() {
        // SSH_AUTH_SOCK 名字不含敏感词，但同一类「看似凭据」的变量必须明确保住：
        // 一旦被剔除，git over ssh 在工具子进程里会直接失效。
        let env = sanitize_env_from(
            [
                ("SSH_AUTH_SOCK".to_string(), "/tmp/agent.sock".to_string()),
                ("DBUS_SESSION_BUS_ADDRESS".to_string(), "unix:path=/tmp/bus".to_string()),
            ]
            .into_iter(),
        );
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"SSH_AUTH_SOCK"));
        assert!(keys.contains(&"DBUS_SESSION_BUS_ADDRESS"));
    }

    #[test]
    fn sanitize_env_injects_python_utf8_defaults() {
        let env = sanitize_env_from([("PATH".to_string(), "/usr/bin".to_string())].into_iter());
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("PYTHONIOENCODING"), Some("utf-8"));
        assert_eq!(get("PYTHONUTF8"), Some("1"));
        assert_eq!(get("CRAFTWORK"), Some("1"));
    }

    #[test]
    fn sanitize_env_respects_user_python_utf8_values() {
        let env = sanitize_env_from(
            [
                ("PATH".to_string(), "/usr/bin".to_string()),
                ("PYTHONIOENCODING".to_string(), "gbk".to_string()),
                ("PYTHONUTF8".to_string(), "0".to_string()),
            ]
            .into_iter(),
        );
        let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("PYTHONIOENCODING"), Some("gbk"), "用户显式设置时保留用户值");
        assert_eq!(get("PYTHONUTF8"), Some("0"));
        // 不重复注入：各变量只出现一次
        assert_eq!(env.iter().filter(|(k, _)| k == "PYTHONUTF8").count(), 1);
    }

    #[test]
    fn sanitize_env_non_utf8_keeps_pythonpath_but_injects_utf8() {
        // 旧白名单语义下 PYTHONPATH 会被丢弃；D18 改为继承，但 UTF-8 保证仍然注入。
        let env = sanitize_env_from([("PYTHONPATH".to_string(), "/tmp/x".to_string())].into_iter());
        assert!(env.iter().any(|(k, _)| k == "PYTHONPATH"), "PYTHONPATH 无害，应继承");
        assert!(env.iter().any(|(k, v)| k == "PYTHONIOENCODING" && v == "utf-8"));
        assert!(env.iter().any(|(k, v)| k == "PYTHONUTF8" && v == "1"));
    }
}
