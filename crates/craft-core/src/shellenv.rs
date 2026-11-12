//! Shell 环境接入：检测可用 shell、算出它会读的配置文件、让它自己跑一遍取回真实环境，
//! 再按黑名单剔除可能暴露凭据的变量。
//!
//! 为什么必须「跑一遍」而不是静态解析配置文件：`.zshenv` 可以 `source` 别处、可以走条件
//! 分支、可以用 `$(...)` 算出值。只有让 shell 自己执行启动文件，才能得到它真实的最终环境；
//! 静态解析只会得到一份幻觉。代价是每次捕获都要执行用户的 shell 启动配置文件
//! （.zshenv / .zshrc 这类，Unix 行话简称 rc 文件），因此结果带缓存与超时。
//!
//! 安全边界（决策 D18）：本模块只负责「取回」，是否放行由 [`sanitize_inherited`] 裁决。
//! 二者的分离是刻意的——取回逻辑要尽可能还原用户环境，剔除规则要尽可能保守可解释。
//!
//! Windows（决策 D24）：探测走「PATH 扫描 + 常见安装位置」（Windows 没有 `/etc/shells`），
//! 候选是 Windows PowerShell 5.1 与 PowerShell 7（pwsh）；捕获不用 `-l -i -c` + `env -0`
//! 那套 POSIX 写法，改用 PowerShell 自己的语法（见 [`powershell_env_script`]）。
//!
//! 缓存与失效（2026-11-10 修订）：捕获结果带 TTL（[`CACHE_TTL_SECS`]），过期后
//! [`current_vars`] **仍返回上一次的值**，由取用点的 [`ensure_fresh`] 负责刷新。
//! 原实现过期即返回 `None`，而唯一的重捕点在 run 启动时——长 run 跨过 TTL 线后剩余
//! 命令静默落进最小环境（`PATH` 丢 `/opt/homebrew/bin` 却留着 `/usr/bin`，可能静默
//! 命中系统旧版本工具），且不产生任何日志。现在「陈旧但可用」取代「过期即失效」。

//! 执行器（决策 D25）：本模块同时决定「用哪个 shell 解释命令」——[`command_invocation`]。
//! 用户选了 shell 就用它，未选择时回落平台默认（Unix `/bin/sh -c`、Windows `cmd /C`）。
//! 执行期不加载启动配置（环境已由捕获阶段取好注入），因此 profile 里的函数与别名在
//! 命令里不可直接用；PowerShell 需要显式 `-NoProfile` 与退出码尾才能对齐 POSIX 语义。
//!
//! 各家族除「自己的调用约定」外，还要**抹平与 sh 的展开差异**，否则同一条按 POSIX 写的
//! 命令换个执行器就从成功变失败（见 [`invocation_args`] 的 zsh / csh 分支）：
//! zsh 加 `-f -o noequals -o nonomatch`（关 `=cmd` 展开与 glob 无匹配中止），
//! csh 加 `-f` 并在脚本前置 `set nonomatch`。因此 zsh/csh 的执行期同样**不加载
//! 启动配置**——用户 `.zshenv` 里 `export` 的变量照常可用（来源是捕获，不是执行期读
//! 文件），只有 alias / 函数 / `setopt` 这类解释期状态丢失，属 D25 已接受的让步。
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 捕获结果的缓存有效期：执行启动配置文件可能有副作用（nvm/pyenv 初始化可达数百毫秒），
/// 每条命令都重跑既慢又可能重复触发那些副作用。
///
/// 2026-11-10 由 5 分钟提到 30 分钟：5 分钟对一次长 run 太短——run 内跨过 TTL 线后，
/// 取用点若无人重捕，剩余全部命令会静默落进最小环境（实测丢了 CARGO_HOME，工具按默认
/// 路径把缓存写到 home）。有效期只决定「何时去问 shell 要新环境」，不再决定「取不到新值
/// 就交白卷」——见 [`current_vars`]。
pub const CACHE_TTL_SECS: u64 = 1800;

/// 捕获超时：启动配置文件卡住（等输入、连网超时）时必须能打断，不能让对话挂死。
pub const CAPTURE_TIMEOUT_MS: u64 = 5000;

/// shell 家族：决定「它会读哪些配置文件」，也决定启动参数的写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    Csh,
    Nu,
    Sh,
    /// PowerShell 家族：pwsh（7+）与 Windows PowerShell（5.1）同一家族——配置文件目录
    /// 不同但都按「存在才列」处理（见 [`config_files`]），启动参数同一套。
    Powershell,
    #[default]
    Other,
}

impl ShellKind {
    /// 由可执行文件名推断家族（不看路径：`/opt/homebrew/bin/bash` 也是 bash）。
    pub fn from_program(program: &str) -> Self {
        // 去掉目录与常见后缀、连字符形式（`/bin/zsh`、`zsh-5.9`、`bash.exe`、`pwsh.exe`）
        let base = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
        let name = base.split(['-', '.']).next().unwrap_or(&base);
        match name {
            "zsh" => Self::Zsh,
            "bash" | "sh" if name == "bash" => Self::Bash,
            "fish" => Self::Fish,
            "csh" | "tcsh" => Self::Csh,
            "nu" => Self::Nu,
            "sh" | "dash" | "ksh" | "ash" => Self::Sh,
            "pwsh" | "powershell" => Self::Powershell,
            _ => Self::Other,
        }
    }

    /// 稳定标识：进入设置契约与提示词。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::Csh => "csh",
            Self::Nu => "nu",
            Self::Sh => "sh",
            Self::Powershell => "powershell",
            Self::Other => "other",
        }
    }
}

/// 一个可选的 shell：绝对路径 + 家族 + 是否当前登录 shell。
#[derive(Debug, Clone)]
pub struct ShellCandidate {
    pub path: String,
    pub kind: ShellKind,
    /// 与进程的 `$SHELL` 指向同一文件（realpath 相同）时为 true。
    pub current: bool,
}

/// 系统默认 PATH：捕获时的基线。给全的最小集合，保证 `/bin/zsh` 这类绝对路径能起得来，
/// 同时不让 app 进程那套被裁剪过的 PATH 影响启动配置文件的判断逻辑。
#[cfg(not(windows))]
const BASE_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// 常见 shell 的候选路径（`/etc/shells` 之外补的：包管理器装的 fish/nu 常不在其中）。
#[cfg(not(windows))]
const EXTRA_SHELLS: &[&str] = &[
    "/opt/homebrew/bin/fish",
    "/usr/local/bin/fish",
    "/opt/homebrew/bin/nu",
    "/usr/local/bin/nu",
    "/usr/bin/fish",
];

/// Windows 的 shell 候选：MSI 装的 PowerShell 7（含 preview 与旧大版本）与系统自带的
/// Windows PowerShell 5.1。商店版 pwsh 装在 `WindowsApps` 下且目录名带版本号，枚举不可靠，
/// 由下面的 PATH 扫描兜住（商店安装会把自己目录加进用户 PATH）。
#[cfg(windows)]
const WINDOWS_SHELLS: &[&str] = &[
    r"C:\Program Files\PowerShell\7\pwsh.exe",
    r"C:\Program Files\PowerShell\7-preview\pwsh.exe",
    r"C:\Program Files\PowerShell\6\pwsh.exe",
    r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
];

/// 检测可用 shell：`/etc/shells` + 常见路径（Unix）/ PATH 扫描 + 常见安装位置（Windows），
/// 按 realpath 去重。
///
/// 顺序稳定：`$SHELL` 指向的排第一，其后按路径字典序——设置页的下拉因此不抖动。
/// Windows 没有 `$SHELL` 与「登录 shell」概念，全部 `current: false`，纯字典序。
pub fn detect_shells() -> Vec<ShellCandidate> {
    let mut raw: Vec<PathBuf> = Vec::new();

    #[cfg(not(windows))]
    {
        // 1) /etc/shells：系统权威清单（只取绝对路径、跳过注释）
        if let Ok(text) = std::fs::read_to_string("/etc/shells") {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || !line.starts_with('/') {
                    continue;
                }
                raw.push(PathBuf::from(line));
            }
        }
        // 2) /bin 下最常见的几个（容器/精简系统里 /etc/shells 可能缺失）
        for name in ["sh", "bash", "zsh", "dash", "ksh"] {
            raw.push(PathBuf::from("/bin").join(name));
        }
        // 3) 包管理器路径
        for p in EXTRA_SHELLS {
            raw.push(PathBuf::from(p));
        }
    }

    #[cfg(windows)]
    {
        // 1) PATH 扫描：scoop / 商店版等非标准安装位置（用户 PATH 里有就找得到）
        if let Ok(path_var) = std::env::var("PATH") {
            for name in ["pwsh.exe", "powershell.exe"] {
                for dir in path_var.split(';').filter(|d| !d.trim().is_empty()) {
                    raw.push(PathBuf::from(dir.trim()).join(name));
                }
            }
        }
        // 2) 常见安装位置（PATH 被裁剪过的 GUI 进程也能找到系统自带 PowerShell）
        for p in WINDOWS_SHELLS {
            raw.push(PathBuf::from(p));
        }
    }

    let current = current_login_shell();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out: Vec<ShellCandidate> = Vec::new();
    for path in raw {
        if !path.is_file() {
            continue;
        }
        // realpath 去重：/bin/sh 与 /bin/dash 可能是同一个文件，只留先出现的
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real.clone());
        let program = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        out.push(ShellCandidate {
            path: path.to_string_lossy().to_string(),
            kind: ShellKind::from_program(program),
            current: current.as_deref() == Some(real.as_path()),
        });
    }

    // 排第一：当前登录 shell
    out.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

/// 进程的登录 shell（已解析真实路径）。
fn current_login_shell() -> Option<PathBuf> {
    let shell = std::env::var("SHELL").ok()?;
    if shell.is_empty() {
        return None;
    }
    std::fs::canonicalize(&shell).ok()
}

/// 该 shell 会读的配置文件（只返回**实际存在**的，按执行顺序）。
///
/// 只用于展示与提示词，不用于解析——值一律经 [`capture_env`] 让 shell 自己算。
/// HOME 由调用方传入，测试可注入。
pub fn config_files(kind: ShellKind, home: &str) -> Vec<String> {
    let home = Path::new(home);
    let names: &[&str] = match kind {
        // zsh 启动顺序：.zshenv（所有实例）→ .zprofile（登录）→ .zshrc（交互）→ .zlogin（登录）
        ShellKind::Zsh => &[".zshenv", ".zprofile", ".zshrc", ".zlogin"],
        // bash：登录读 profile 系，交互读 .bashrc；顺序与 bash 实际行为一致
        ShellKind::Bash => &[".bash_profile", ".bash_login", ".profile", ".bashrc"],
        ShellKind::Fish => &[".config/fish/config.fish"],
        // .cshrc 每次启动都读；.login 仅登录 shell（argv[0] 带 `-`）执行，捕获时不可达
        ShellKind::Csh => &[".cshrc"],
        ShellKind::Nu => &[".config/nushell/env.nu", ".config/nushell/config.nu"],
        // PowerShell 用户级顺序：全宿主 profile.ps1 → 当前宿主 Microsoft.PowerShell_profile.ps1；
        // 7（pwsh）用 Documents\PowerShell，5.1 用 Documents\WindowsPowerShell——两套都列，
        // 存在哪个列哪个（捕获哪个 shell 就读哪个目录，这里只做展示）。
        ShellKind::Powershell => &[
            "Documents/PowerShell/profile.ps1",
            "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
            "Documents/WindowsPowerShell/profile.ps1",
            "Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
        ],
        ShellKind::Sh | ShellKind::Other => &[".profile"],
    };
    names
        .iter()
        .map(|n| home.join(n))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

/// 捕获结果：真实环境变量 + 该 shell 会读的配置文件。
#[derive(Debug, Clone, Default)]
pub struct Captured {
    pub shell_path: String,
    pub kind: ShellKind,
    pub configs: Vec<String>,
    /// 已剔除敏感键的变量（按名排序，便于断言与提示词稳定）
    pub vars: Vec<(String, String)>,
}

/// 捕获标记：用于从启动配置文件的任何输出里切出 env 段落。
///
/// 为什么需要：交互式 shell 可能在启动配置文件里 `echo` 提示、`nvm` 打印版本、甚至 `neofetch`。
/// 不划边界就只能靠「第一行是否是 KEY=VALUE」猜，而值里可以含换行。
const MARK_BEGIN: &str = "__CRAFTWORK_ENV_BEGIN__";
const MARK_END: &str = "__CRAFTWORK_ENV_END__";

/// PowerShell 的捕获脚本：输出两标记之间的 NUL 分隔 `KEY=VALUE` 段落（与 Unix 的
/// `env -0` 同一格式，[`parse_env_block`] 不用区分平台）。
///
/// 三个细节：
/// - `-Command`（不带 `-NoProfile`）会先执行用户 profile——这正是捕获的目的，别加 `-NoProfile`；
///   `-NonInteractive` 防止 profile 里的交互提示把捕获挂到超时。
/// - `[Console]::OutputEncoding` 设 UTF-8：重定向下 PS 5.1 默认走 OEM 代码页，
///   值里的非 ASCII（中文路径很常见）会变 mojibake；设置失败（个别精简环境）也不致命。
/// - 用 `[char]0`（NUL）做分隔符：值可以含换行，与 Unix 侧的解析格式完全一致。
fn powershell_env_script() -> String {
    format!(
        "$ErrorActionPreference='SilentlyContinue'; \
         try{{[Console]::OutputEncoding=[Text.Encoding]::UTF8}}catch{{}}; \
         Write-Output '{MARK_BEGIN}'; \
         $o=[Console]::Out; \
         foreach($e in [Environment]::GetEnvironmentVariables().GetEnumerator()){{ \
            $o.Write($e.Key); $o.Write('='); $o.Write($e.Value); $o.Write([char]0); \
         }}; \
         Write-Output '{MARK_END}'"
    )
}

/// 捕获超时（PowerShell 单列）：Windows PowerShell 冷启动 + 杀毒扫盘常见 2~8 秒，
/// 5 秒的 Unix 值在 Windows 上会误报超时；给双倍余量，卡死仍能打断。
const CAPTURE_TIMEOUT_MS_PS: u64 = 15000;

/// 让指定 shell 自己跑一遍启动文件，取回它最终的完整环境。
///
/// 参数：`login` + `interactive`——用户的 PATH 常在 `.zprofile`/`.zshrc` 里配置，
/// 两者都要才覆盖完整。
pub fn capture_env(shell_path: &str, home: &str) -> Result<Captured, String> {
    let program = Path::new(shell_path).file_name().and_then(|s| s.to_str()).unwrap_or("");
    let kind = ShellKind::from_program(program);
    if !Path::new(shell_path).is_file() {
        return Err(format!("shell 不存在：{shell_path}"));
    }
    let mut spec = crate::platform::RunSpec::new(shell_path);
    spec.args = match kind {
        // PowerShell：加载 profile 地跑一段输出脚本；不能复用 POSIX 的 `-l -i -c` + env -0。
        ShellKind::Powershell => {
            vec!["-NoLogo".into(), "-NonInteractive".into(), "-Command".into(), powershell_env_script()]
        }
        // csh/tcsh 不认 `-l`（直接报 Unknown option），但 `-c` 仍会读 ~/.cshrc；
        // 登录级配置 ~/.login 只在 argv[0] 带 `-` 的登录 shell 里执行，这里不做。
        ShellKind::Csh => vec!["-c".into(), format!("printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}")],
        ShellKind::Nu => vec!["-c".into(), format!("printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}")],
        _ => vec![
            "-l".into(),
            "-i".into(),
            "-c".into(),
            format!("printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}"),
        ],
    };
    // 交互式 shell（`-i`）启动时会把自己设为控制终端的前台进程组；若它处在后台组，
    // `tcsetpgrp` 会收到 SIGTTOU 把进程停住（状态 T），永远等不到退出——表现为
    // 5 秒超时。新会话没有控制终端，这套 job control 初始化会被跳过。
    spec.new_session = true;
    let timeout_ms = match kind {
        ShellKind::Powershell => CAPTURE_TIMEOUT_MS_PS,
        _ => CAPTURE_TIMEOUT_MS,
    };
    spec.timeout_ms = Some(timeout_ms);
    spec.output_limit = Some(1024 * 1024);
    spec.env = Some(baseline_env(home, shell_path));
    let result = crate::platform::run(spec);
    if result.timed_out {
        return Err(format!("读取 shell 环境超时（{} 秒）：{shell_path}", timeout_ms / 1000));
    }
    if result.exit_code != 0 && !result.output.contains(MARK_BEGIN) {
        return Err(format!("读取 shell 环境失败（退出码 {}）：{}", result.exit_code, result.output.trim()));
    }
    let vars = parse_env_block(&result.output)
        .into_iter()
        .filter(|(k, _)| !is_sensitive_key(k))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();

    Ok(Captured {
        shell_path: shell_path.to_string(),
        kind,
        configs: config_files(kind, home),
        vars,
    })
}

/// 捕获用的基线环境：最小但足以启动 shell 并找到它自己的配置。
///
/// Windows 不能照搬 Unix 的最小 PATH：PowerShell 启动依赖 `SystemRoot`/`COMSPEC` 等
/// 系统变量，缺了会起不来或行为残缺。所以以进程环境的清洗结果为底，只覆盖 HOME 系
/// 变量（profile 定位与 `$HOME` 语义一致）；清洗已剔除凭据类，不会把密钥带进捕获。
fn baseline_env(home: &str, shell_path: &str) -> Vec<(String, String)> {
    // 两个 cfg 只有一个参与编译，非当前平台的 let 不存在；值统一在尾部交出。
    #[cfg(windows)]
    let env = {
        let _ = shell_path; // Windows 没有登录 shell 概念，不设 SHELL 变量
        let mut env = crate::platform::sanitized_env();
        env.retain(|(k, _)| !matches!(k.as_str(), "HOME" | "USERPROFILE"));
        env.push(("HOME".to_string(), home.to_string()));
        env.push(("USERPROFILE".to_string(), home.to_string()));
        env
    };
    #[cfg(not(windows))]
    let env = {
        let mut env = vec![
            ("PATH".to_string(), BASE_PATH.to_string()),
            ("HOME".to_string(), home.to_string()),
            ("SHELL".to_string(), shell_path.to_string()),
            ("TERM".to_string(), "dumb".to_string()),
            ("LANG".to_string(), "en_US.UTF-8".to_string()),
        ];
        if let Ok(user) = std::env::var("USER") {
            env.push(("USER".to_string(), user));
        }
        env
    };
    env
}

/// 从捕获输出里切出两个标记之间的 NUL 分隔 env 段落。
///
/// 容错：标记缺失时回落「整段按 NUL 解析」——某些 shell 的 `printf` 会被配置里的
/// `set -e`/别名干扰，此时宁可拿一份可能含噪声的结果，也不要整体失败。
pub fn parse_env_block(output: &str) -> Vec<(String, String)> {
    let body = match (output.find(MARK_BEGIN), output.find(MARK_END)) {
        (Some(b), Some(e)) if e > b => &output[b + MARK_BEGIN.len()..e],
        _ => output,
    };
    body.split('\0')
        .filter_map(|entry| {
            let entry = entry.trim_start_matches('\n');
            if entry.is_empty() {
                return None;
            }
            let (key, value) = entry.split_once('=')?;
            let key = key.rsplit('\n').next().unwrap_or(key);
            if key.is_empty() || !is_valid_env_name(key) {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// POSIX 环境变量名的宽松校验：字母/数字/下划线，且不以数字开头。
fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 必须保留的变量：名字命中剔除规则，但它们是功能依赖（git over ssh、桌面集成）。
const SANITIZE_KEEP: &[&str] = &[
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GPG_AGENT_INFO",
    "DBUS_SESSION_BUS_ADDRESS",
    "SESSION_MANAGER",
];

/// 按后缀剔除（强信号：这些后缀几乎只用于承载体密）。
const DROP_SUFFIX: &[&str] = &[
    "_KEY",
    "_TOKEN",
    "_SECRET",
    "_PASSWORD",
    "_PASSWD",
    "_PASSPHRASE",
    "_CREDENTIAL",
    "_CREDENTIALS",
    "_PRIVATE_KEY",
    "_ACCESS_KEY",
    "_ACCOUNT_KEY",
];

/// 按子串剔除（只留高置信组合，不用裸 TOKEN/AUTH/SESSION 以免误伤）。
const DROP_SUBSTR: &[&str] = &[
    "API_KEY",
    "APIKEY",
    "APITOKEN",
    "AUTH_TOKEN",
    "ACCESS_TOKEN",
    "REFRESH_TOKEN",
    "SECRET_KEY",
    "PRIVATE_KEY",
    "CLIENT_SECRET",
    "PASSWORD",
    "PASSWD",
];

/// 按前缀剔除：云厂商与常见服务的凭据命名空间。
const DROP_PREFIX: &[&str] = &[
    "AWS_",
    "GITHUB_",
    "GH_",
    "GITLAB_",
    "OPENAI_",
    "ANTHROPIC_",
    "DEEPSEEK_",
    "ZHIPU_",
    "DASHSCOPE_",
    "MOONSHOT_",
    "GEMINI_",
    "GOOGLE_",
    "AZURE_",
    "HF_",
    "HUGGINGFACE_",
    "MISTRAL_",
    "COHERE_",
    "REPLICATE_",
    "STRIPE_",
    "SLACK_",
    "TWILIO_",
    "SENDGRID_",
    "NPM_TOKEN",
    "DOCKER_",
    "HEROKU_",
    "VERCEL_",
    "NETLIFY_",
];

/// 该变量名是否属于「可能暴露信息」而必须剔除。
///
/// 语义：黑名单剔除（决策 D18）。白名单天然安全但废掉了用户 shell 的大部分能力，
/// 这里改为「只删可能承载体密的」，并把规则做成可逐条解释的形式。
pub fn is_sensitive_key(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if SANITIZE_KEEP.iter().any(|keep| upper == *keep) {
        return false;
    }
    if DROP_SUFFIX.iter().any(|s| upper.ends_with(s)) {
        return true;
    }
    if DROP_SUBSTR.iter().any(|s| upper.contains(s)) {
        return true;
    }
    DROP_PREFIX.iter().any(|p| upper.starts_with(p))
}

/// 进程级缓存：设置变更时重建，其余情况按 TTL 复用。
///
/// 用 `RwLock` 而不是 `Mutex`：读路径（每条命令装配环境）远多于写路径（用户改设置）。
static CACHE: std::sync::RwLock<Option<Cached>> = std::sync::RwLock::new(None);

struct Cached {
    shell_path: String,
    captured: Captured,
    at: std::time::Instant,
}

/// 配置当前生效的 shell 环境（空串 = 不使用，回落最小环境）。
///
/// 返回 `Ok(())` 或可读错误；错误时**清空缓存**并让调用方展示——不静默保留旧值，
/// 否则用户会以为切换生效了。
pub fn configure(shell_path: &str, home: &str) -> Result<(), String> {
    if shell_path.is_empty() {
        clear();
        return Ok(());
    }
    let captured = capture_env(shell_path, home)?;
    let mut guard = CACHE.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some(Cached {
        shell_path: shell_path.to_string(),
        captured,
        at: std::time::Instant::now(),
    });
    Ok(())
}

/// 清空缓存（切回「不使用」或设置变更前的重置）。
pub fn clear() {
    let mut guard = CACHE.write().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// 当前生效的环境变量快照；未配置时返回 `None`（调用方回落到最小环境）。
///
/// **过期不返回 `None`**（2026-11-10 修订）：只要曾经捕获成功过，就返回上一次的值。
/// 原实现在超过 [`CACHE_TTL_SECS`] 后直接交白卷，导致长 run 的剩余命令静默落进最小
/// 环境——`PATH` 丢掉 `/opt/homebrew/bin` 却仍留着 `/usr/bin` 时，某些命令会静默命中
/// 系统旧版本，比直接失败更难查。代价是值可能落后于用户新改的 rc，这由取用点的
/// [`ensure_fresh`] 与设置页的 [`configure`] 负责刷新；「环境慢一会儿」远好于「环境
/// 突然变空」。
pub fn current_vars() -> Option<Vec<(String, String)>> {
    let guard = CACHE.read().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(|cached| cached.captured.vars.clone())
}

/// 当前生效 shell 的元信息（路径 / 家族 / 配置文件 / 变量名），供提示词与设置页展示。
///
/// 与 [`current_vars`] 同一条修订：过期仍返回上一次的捕获结果，不交白卷。
pub fn current_meta() -> Option<Captured> {
    let guard = CACHE.read().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(|cached| cached.captured.clone())
}

/// 缓存是否仍在有效期内（只读，不触发捕获）。
///
/// run 启动前与取用点（[`ensure_fresh`]）用它判断是否需要重捕。注意：过期**不再**让
/// [`current_vars`] 交白卷，`is_fresh` 现在纯粹是「该不该去问 shell 要新值」的判断。
pub fn is_fresh() -> bool {
    let guard = CACHE.read().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .is_some_and(|cached| cached.at.elapsed().as_secs() <= CACHE_TTL_SECS)
}

/// 重捕串行锁：多命令同时跨过 TTL 时，只让一个去跑 shell 启动文件，其余等它结束后
/// 直接命中新鲜缓存（双检）。捕获最坏 5 秒，串行排队远好过各捕一次。
static REFRESH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 取用点重捕：过期则重新捕获，成功返回 `true`。
///
/// 这是「B」——把重捕从「只在 run 启动」下移到每条命令装配环境之前。原设计只有 run
/// 启动时那一处重捕点，同一个 run 跨过 TTL 后剩余命令就没人管了。
///
/// 语义要点：
/// - **失败不改变已有缓存**：沿用上一次的值（[`current_vars`] 不再因过期交白卷），
///   命令照常执行——环境陈旧远好于命令因环境为空而找不到工具；
/// - **未配置 shell**（缓存本就为空）时直接返回 `true`：没有「过期」可言，也不该在这里
///   凭空创造一个捕获（是否启用 shell 是用户设置，由 [`configure`] 负责）；
/// - **并发双检**：拿到 [`REFRESH_LOCK`] 后重查 `is_fresh()`，避免一批并行命令各捕一次。
pub fn ensure_fresh(shell_path: &str, home: &str) -> bool {
    if shell_path.is_empty() {
        return true;
    }
    if is_fresh() {
        return true;
    }
    let _serial = REFRESH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 双检：等锁期间别的调用可能已经捕完了
    if is_fresh() {
        return true;
    }
    match capture_env(shell_path, home) {
        Ok(captured) => {
            let mut guard = CACHE.write().unwrap_or_else(|e| e.into_inner());
            *guard = Some(Cached {
                shell_path: shell_path.to_string(),
                captured,
                at: std::time::Instant::now(),
            });
            true
        }
        // 捕获失败保留旧值：调用方据此写一条日志，但不阻断命令
        Err(_) => false,
    }
}

/// 缓存里的 shell 路径（不受 TTL 影响）：设置页需要显示「已配置但待刷新」。
/// 同时也是执行器解析的入口——见 [`command_invocation`]。
pub fn configured_shell() -> Option<String> {
    let guard = CACHE.read().unwrap_or_else(|e| e.into_inner());
    guard.as_ref().map(|c| c.shell_path.clone())
}

// ---------- 执行器解析 ----------

/// 未选择 shell 时的平台默认执行器：Unix 走 `/bin/sh -c`，Windows 走 `cmd /C`。
///
/// 为什么保留这个默认：它是本模块之外唯一被广泛依赖的命令语义——POSIX 命令串在三端
/// 都能被它解释，且开销最小。用户没有表达偏好时不要替他选 shell。
pub fn default_invocation(command: &str) -> (String, Vec<String>) {
    #[cfg(unix)]
    {
        ("/bin/sh".to_string(), vec!["-c".to_string(), command.to_string()])
    }
    #[cfg(windows)]
    {
        ("cmd".to_string(), vec!["/C".to_string(), command.to_string()])
    }
}

/// 用当前生效的 shell 执行一条命令：返回（可执行文件, argv）。
///
/// **未选择 shell 时回落 [`default_invocation`]**（Unix `/bin/sh -c`、Windows `cmd /C`）。
/// 选中时用该 shell 自己的调用约定解释同一条命令串——执行器与环境必须指向同一个 shell，
/// 否则「设置里选了 pwsh」只改环境变量、命令仍被 cmd 解释，用户看到的语义与设置不符。
///
/// 执行期**不加载启动配置**：环境已由 [`capture_env`] 事先取好并作为子进程 env 注入，
/// 这里再跑一遍 profile 只会让每条命令多出数百毫秒、并把 profile 的输出混进命令结果。
/// 代价是 profile 里定义的函数与别名在命令里不可直接用（见 D25）。
///
/// 各家族的 argv 还要抹平与 sh 的展开差异（zsh 的 `=cmd` / glob 无匹配中止，见
/// [`invocation_args`]）：本函数是唯一解析点，命令串原样传入、不做转义或改写。
pub fn command_invocation(command: &str) -> (String, Vec<String>) {
    let Some(shell_path) = configured_shell() else {
        return default_invocation(command);
    };
    let program = Path::new(&shell_path).file_name().and_then(|s| s.to_str()).unwrap_or("");
    let kind = ShellKind::from_program(program);
    let args = invocation_args(kind, &shell_path, command);
    (shell_path, args)
}

/// 各家族解释命令串的 argv（不含可执行文件本身）。
///
/// POSIX 家族一律 `-c`：fish / nu / csh 都认这个形式，且执行期不需要 `-l -i`
/// （登录、交互语义只影响启动配置，而启动配置在捕获阶段已经执行过）。
///
/// zsh 与 csh 各自还要压掉本家族的默认展开行为，见下面两条分支的注释。
fn invocation_args(kind: ShellKind, shell_path: &str, command: &str) -> Vec<String> {
    match kind {
        ShellKind::Powershell => {
            let mut script = String::new();
            // Windows PowerShell 5.1 重定向下默认走 OEM 代码页，中文输出会变 mojibake；
            // pwsh 7 默认就是 UTF-8，无需设置。放在最前以便尽量早生效。
            if is_windows_powershell(shell_path) {
                script.push_str("[Console]::OutputEncoding=[Text.Encoding]::UTF8; ");
            }
            // 7+ 错误信息默认带 ANSI 彩色转义，落进工具有界缓冲后是乱码；切成纯文本。
            // 5.1 没有 $PSStyle，赋值会报错污染 stderr——按版本判定，不用 try/catch 兜。
            script.push_str("if ($PSVersionTable.PSVersion.Major -ge 7) { $PSStyle.OutputRendering='PlainText' }; ");
            script.push_str(command);
            // 退出码传播（实测确定，见 D25）：PowerShell 的 `-Command` **不**传播原生进程
            // 退出码（实测 `cmd /c exit 5` 得 0），且 cmdlet 失败只写 stderr 不影响返回码。
            // `$?` 必须紧跟命令捕获——任何中间语句（含赋值本身）都会把它重置为 true。
            script.push_str(
                "\n$__cw_ok = $?\n$__cw_code = $LASTEXITCODE\n\
                 if (-not $__cw_ok) { if ($null -ne $__cw_code -and $__cw_code -ne 0) { exit $__cw_code }; exit 1 }\n\
                 exit 0",
            );
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                script,
            ]
        }
        // zsh：`-c` 之外还要压掉两个**默认开启**的展开行为，否则「命令串按 POSIX 写、
        // 换执行器就变失败」——实测同一条命令在 /bin/sh 正常、在 zsh 报错退出 1：
        //
        // - `nonomatch`：glob 无匹配时原样交给命令（POSIX 语义）。zsh 默认 NOMATCH
        //   直接报 `no matches found` 并**中止整条命令**，`grep --include=*.rs`、
        //   `ls *.nope` 这类常规写法会让 `&&` 链后续语句全部丢失。
        // - `noequals`：关闭 `=cmd` 展开。zsh 默认把命令位的裸 `===` 当成「找名为 `==`
        //   的命令」并报 `== not found`；模型用 `echo ===` 做分隔是常规写法。
        //
        // `-f`（NO_RCS）是这两个 `-o` 能生效的前提：`zsh -c` **只**读 `~/.zshenv`
        // （实测不读 .zprofile / .zshrc / .zlogin），而 `.zshenv` 在命令行选项**之后**
        // 执行，里面一句 `setopt equals` 就会推翻 `-o noequals`。`-f` 同时落实 D25
        // 「执行期不加载启动配置」，顺带挡掉 `.zshenv` 的三个已知雷——`cd` 会覆盖
        // 工具传入的 cwd、`set -e` 泄漏成命令提前退出、任意 stderr 噪音混进命令输出。
        //
        // 用户 `.zshenv` 里 `export` 的变量**不受影响**：变量的来源是捕获阶段
        // （[`capture_env`] 用 `-l -i -c` 取回完整环境后注入子进程），与执行期读不读
        // 文件无关。只有 alias / 函数 / `setopt` 这类解释期状态会丢失——D25 已接受。
        //
        // 为什么不加 `--emulate sh`：实测数组索引语义随之改变（`${a[1]}` 从首元素变成
        // 第二个），把 zsh 用户选的 shell 换成另一套语义，与本决策「用用户选中的 shell
        // 解释命令」相悖。
        ShellKind::Zsh => vec![
            "-f".to_string(),
            "-o".to_string(),
            "noequals".to_string(),
            "-o".to_string(),
            "nonomatch".to_string(),
            "-c".to_string(),
            command.to_string(),
        ],
        // csh/tcsh 不认 `-o`（报 `Unknown option`），只能把选项写成脚本前缀。
        // 它默认同样是 NOMATCH 语义：实测 `grep -n x --include=*.toml f` 会打印
        // `grep: No match.` 且**整条命令不执行**；`set nonomatch` 让无匹配 glob 原样传递。
        // csh 没有 `=cmd` 展开（实测 `echo =ls` 输出字面量），故只需压 glob。
        // `-f` 同理跳过 `~/.cshrc`，与 zsh 分支一个口径。
        ShellKind::Csh => vec![
            "-f".to_string(),
            "-c".to_string(),
            format!("set nonomatch; {command}"),
        ],
        _ => vec!["-c".to_string(), command.to_string()],
    }
}

/// 是否为 Windows PowerShell 5.1（`powershell.exe`），与 pwsh 7+ 区分。
///
/// 只按可执行文件名判断：5.1 与 7 的编码默认值不同，而两者的 profile 与启动参数一致。
fn is_windows_powershell(shell_path: &str) -> bool {
    // 入参可能是 Windows 路径（`C:\Windows\…\powershell.exe`），而本函数在 Unix 上也会
    // 被测试直接调用——`Path::file_name` 在 Unix 不把反斜杠当分隔符，整串会被当成文件名，
    // 判定恒 false。两种分隔符都切一刀，跨平台语义一致。
    let name = shell_path.rsplit(['/', '\\']).next().unwrap_or(shell_path);
    name.to_ascii_lowercase().starts_with("powershell")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全局缓存是进程级静态量：测试默认并行跑，任何一个 `clear()` / `configure()` 都会
    /// 干扰其他用例（并发双检用例曾因此读到「已被别人清空」的状态而失败）。
    /// 所有碰缓存的用例先拿这把锁，让它们串行；锁体内的多线程并发仍在用例内部发生。
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 串行进入缓存相关用例。poison 不致命：用例之间没有共享不变量要保护。
    fn serial_cache_test() -> std::sync::MutexGuard<'static, ()> {
        CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn sensitive_keys_dropped_and_functional_ones_kept() {
        for key in [
            "OPENAI_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "GITHUB_TOKEN",
            "ANTHROPIC_AUTH_TOKEN",
            "MY_PASSWORD",
            "DB_PASSWD",
            "SERVICE_CLIENT_SECRET",
            "DEPLOY_KEY",
            "NPM_TOKEN",
        ] {
            assert!(is_sensitive_key(key), "{key} 应被剔除");
        }
        for key in [
            "PATH",
            "HOME",
            "PNPM_HOME",
            "JAVA_HOME",
            "ANDROID_HOME",
            "SSH_AUTH_SOCK",
            "DBUS_SESSION_BUS_ADDRESS",
            "SESSION_MANAGER",
            "CARGO_HOME",
        ] {
            assert!(!is_sensitive_key(key), "{key} 应保留");
        }
    }

    #[test]
    fn parse_env_block_ignores_rc_noise_outside_markers() {
        let output = format!(
            "nvm: v20.0.0 loaded\nWelcome!\n{MARK_BEGIN}\nA=1\0B=has=equals\0C=line1\nline2\0{MARK_END}\nbye\n"
        );
        let vars = parse_env_block(&output);
        let map: BTreeMap<_, _> = vars.into_iter().collect();
        assert_eq!(map.get("A").map(String::as_str), Some("1"));
        assert_eq!(map.get("B").map(String::as_str), Some("has=equals"));
        assert_eq!(map.get("C").map(String::as_str), Some("line1\nline2"));
        assert!(!map.contains_key("bye"), "标记外内容不应解析");
    }

    #[test]
    fn parse_env_block_drops_invalid_names() {
        let output = format!("{MARK_BEGIN}\0GOOD=1\0=empty\0 1BAD=x\0{MARK_END}");
        let map: BTreeMap<_, _> = parse_env_block(&output).into_iter().collect();
        assert_eq!(map.len(), 1, "只应保留 GOOD：{map:?}");
        assert!(map.contains_key("GOOD"));
    }

    #[test]
    fn shell_kind_from_program_handles_paths_and_suffixes() {
        assert_eq!(ShellKind::from_program("/bin/zsh"), ShellKind::Zsh);
        assert_eq!(ShellKind::from_program("bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_program("/opt/homebrew/bin/bash"), ShellKind::Bash);
        assert_eq!(ShellKind::from_program("/usr/bin/fish"), ShellKind::Fish);
        assert_eq!(ShellKind::from_program("/bin/sh"), ShellKind::Sh);
        assert_eq!(ShellKind::from_program("/usr/bin/tcsh"), ShellKind::Csh);
    }

    #[test]
    fn shell_kind_from_program_recognizes_powershell_family() {
        assert_eq!(ShellKind::from_program("pwsh"), ShellKind::Powershell);
        assert_eq!(ShellKind::from_program("pwsh.exe"), ShellKind::Powershell);
        assert_eq!(ShellKind::from_program("powershell.exe"), ShellKind::Powershell);
        // Windows 绝对路径（反斜杠分隔）也要能认
        assert_eq!(ShellKind::from_program(r"C:\Program Files\PowerShell\7\pwsh.exe"), ShellKind::Powershell);
        assert_eq!(
            ShellKind::from_program(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            ShellKind::Powershell
        );
    }

    #[test]
    fn powershell_env_script_wraps_markers_and_outputs_nul_entries() {
        let script = powershell_env_script();
        assert!(script.contains(MARK_BEGIN), "脚本必须含起始标记");
        assert!(script.contains(MARK_END), "脚本必须含结束标记");
        assert!(script.contains("[char]0"), "条目分隔符必须是 NUL，值含换行才切得开");
        assert!(!script.contains("-NoProfile"), "捕获要执行 profile，不能加 -NoProfile");
    }

    #[test]
    fn config_files_lists_powershell_profiles_when_present() {
        let dir = crate::testutil::unique_temp_dir("shellenv-ps-profiles");
        let ps7 = dir.join("Documents").join("PowerShell");
        let ps5 = dir.join("Documents").join("WindowsPowerShell");
        std::fs::create_dir_all(&ps7).unwrap();
        std::fs::create_dir_all(&ps5).unwrap();
        std::fs::write(ps7.join("profile.ps1"), "").unwrap();
        std::fs::write(ps5.join("Microsoft.PowerShell_profile.ps1"), "").unwrap();
        let home = dir.to_string_lossy().to_string();
        let files = config_files(ShellKind::Powershell, &home);
        assert_eq!(files.len(), 2, "只列真实存在的：{files:?}");
        assert!(files[0].ends_with("PowerShell\\profile.ps1") || files[0].ends_with("PowerShell/profile.ps1"));
        assert!(
            files[1].ends_with("WindowsPowerShell\\Microsoft.PowerShell_profile.ps1")
                || files[1].ends_with("WindowsPowerShell/Microsoft.PowerShell_profile.ps1")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn detect_shells_has_no_duplicates_and_includes_sh() {
        let shells = detect_shells();
        assert!(!shells.is_empty(), "至少要能检出 /bin/sh");
        assert!(shells.iter().any(|s| s.path == "/bin/sh"), "应包含 /bin/sh");
        let mut paths: Vec<&str> = shells.iter().map(|s| s.path.as_str()).collect();
        paths.sort_unstable();
        let before = paths.len();
        paths.dedup();
        assert_eq!(before, paths.len(), "去重后不应有重复路径");
    }

    #[cfg(windows)]
    #[test]
    fn detect_shells_on_windows_finds_system_powershell() {
        let shells = detect_shells();
        assert!(
            shells.iter().any(|s| {
                let p = s.path.replace('/', "\\");
                p.ends_with("WindowsPowerShell\\v1.0\\powershell.exe")
            }),
            "至少要检出系统自带的 Windows PowerShell：{shells:?}"
        );
        assert!(
            shells.iter().all(|s| s.kind == ShellKind::Powershell),
            "Windows 候选应全部是 PowerShell 家族：{shells:?}"
        );
    }

    #[test]
    fn config_files_only_returns_existing_entries() {
        let dir = crate::testutil::unique_temp_dir("shellenv-cfg");
        std::fs::create_dir_all(&dir).unwrap();
        let home = dir.to_string_lossy().to_string();
        // 空目录：一个都没有
        assert!(config_files(ShellKind::Zsh, &home).is_empty());
        // 建两个：只返回存在的两个，且按 zsh 执行顺序
        std::fs::write(dir.join(".zshenv"), "").unwrap();
        std::fs::write(dir.join(".zshrc"), "").unwrap();
        let files = config_files(ShellKind::Zsh, &home);
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with(".zshenv"));
        assert!(files[1].ends_with(".zshrc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_env_rejects_missing_shell() {
        let err = capture_env("/nonexistent/shell", "/tmp").unwrap_err();
        assert!(err.contains("不存在"), "错误应可读：{err}");
    }

    #[cfg(windows)]
    #[test]
    fn capture_env_reads_powershell_environment() {
        let Some(pwsh) = detect_shells().into_iter().find(|s| s.kind == ShellKind::Powershell) else {
            // 极端精简环境（无任何 PowerShell）：探测为空就不验捕获，不能因此挂测试
            return;
        };
        let captured = capture_env(&pwsh.path, &crate::platform::home_dir_string()).expect("PowerShell 捕获应成功");
        assert!(!captured.vars.is_empty(), "至少要取回环境变量");
        // Windows 环境变量名不区分大小写，捕获到的是块内原样大小写（常为全大写），按不区分大小写断言
        assert!(
            captured
                .vars
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("SystemRoot") || k.eq_ignore_ascii_case("windir")),
            "Windows 系统变量应取回（证明走的是系统基线环境）：{:?}",
            captured.vars.iter().take(5).collect::<Vec<_>>()
        );
    }

    #[cfg(unix)]
    #[test]
    fn capture_env_times_out_on_hanging_shell() {
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-hang");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("hanging-shell");
        let mut f = std::fs::File::create(&script).unwrap();
        // 忽略所有参数、直接挂住：必须由超时杀死，而不是等满 5 秒以上（这里只断言「有界」）
        writeln!(f, "#!/bin/sh\nsleep 30").unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let started = std::time::Instant::now();
        let err = capture_env(&script.to_string_lossy(), &dir.to_string_lossy()).unwrap_err();
        assert!(started.elapsed().as_secs() < 15, "超时必须生效，不能挂满整个 sleep");
        assert!(err.contains("超时"), "应报超时：{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn cache_freshness_follows_configure_and_clear() {
        let _serial = serial_cache_test();
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-fresh");
        std::fs::create_dir_all(&dir).unwrap();
        let shell = dir.join("fake-shell");
        let mut f = std::fs::File::create(&shell).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        clear();
        assert!(!is_fresh(), "清空后缓存应判为失效");
        assert!(current_vars().is_none(), "从未捕获过时没有值可用");
        configure(&shell.to_string_lossy(), &dir.to_string_lossy()).unwrap();
        assert!(is_fresh(), "刚捕获完应判为有效");
        assert!(current_vars().is_some(), "有效期内应能取到变量");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 过期不再交白卷（2026-11-10 修订）：缓存过期后 `current_vars` 仍返回上一次的值。
    ///
    /// 这是「环境静默变空」的回归——原实现在超过 TTL 后返回 `None`，长 run 的剩余命令
    /// 全部落进最小环境，且无处可观测。这里把 `at` 手工拨到过期，断言值仍在。
    #[cfg(unix)]
    #[test]
    fn stale_cache_still_serves_last_captured_vars() {
        let _serial = serial_cache_test();
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-stale");
        std::fs::create_dir_all(&dir).unwrap();
        let shell = dir.join("fake-shell");
        let mut f = std::fs::File::create(&shell).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        configure(&shell.to_string_lossy(), &dir.to_string_lossy()).unwrap();
        let expected = current_vars().expect("捕获后应有值");

        // 手工把时间戳拨到 TTL 之外：模拟长 run 跨过有效期
        {
            let mut guard = CACHE.write().unwrap();
            let cached = guard.as_mut().unwrap();
            cached.at = std::time::Instant::now()
                - std::time::Duration::from_secs(CACHE_TTL_SECS + 1);
        }
        assert!(!is_fresh(), "拨过 TTL 后应判为不新鲜（取用点据此重捕）");
        assert_eq!(
            current_vars().as_deref(),
            Some(expected.as_slice()),
            "过期后必须沿用上一次的值，不能变空"
        );
        assert!(current_meta().is_some(), "元信息同样沿用旧值");
        clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `ensure_fresh`：新鲜时不重捕，过期时重捕并刷新时间戳。
    ///
    /// 用「每次执行往计数文件追加一行」的假 shell 数捕获次数，避免靠耗时推断。
    #[cfg(unix)]
    #[test]
    fn ensure_fresh_recaptures_only_when_stale() {
        let _serial = serial_cache_test();
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-ensure");
        std::fs::create_dir_all(&dir).unwrap();
        let counter = dir.join("count.txt");
        let shell = dir.join("counting-shell");
        let mut f = std::fs::File::create(&shell).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        // 计数：每被执行一次追加一行；环境里带上标记值供断言
        writeln!(f, "echo x >> {}", counter.display()).unwrap();
        writeln!(f, "printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shell_path = shell.to_string_lossy().to_string();
        let home = dir.to_string_lossy().to_string();

        clear();
        // 空缓存 + 非空 shell_path：视作过期，会捕一次
        assert!(ensure_fresh(&shell_path, &home), "过期应重捕成功");
        assert_eq!(std::fs::read_to_string(&counter).unwrap().lines().count(), 1);

        // 刚捕完：新鲜，不该再捕
        assert!(ensure_fresh(&shell_path, &home));
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().count(),
            1,
            "新鲜时不应重复执行 shell"
        );

        // 拨过 TTL：再调一次应捕第二次
        {
            let mut guard = CACHE.write().unwrap();
            guard.as_mut().unwrap().at =
                std::time::Instant::now() - std::time::Duration::from_secs(CACHE_TTL_SECS + 1);
        }
        assert!(ensure_fresh(&shell_path, &home));
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().count(),
            2,
            "过期后应重新捕获一次"
        );
        clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 未配置 shell（空 cache）时 `ensure_fresh` 不该凭空捕获，也不该判失败。
    #[test]
    fn ensure_fresh_is_noop_when_shell_not_configured() {
        let _serial = serial_cache_test();
        clear();
        assert!(ensure_fresh("", ""), "空 shell_path 表示不使用 shell，直接视为可用");
        clear();
    }

    /// 并发双检：多线程同时跨过 TTL 时只捕获一次。
    ///
    /// 不加双检的后果是一批并行命令各跑一次 shell 启动文件（最坏 5 秒 × N 串行），
    /// 而 nvm/pyenv 这类初始化本身就有副作用，重复执行代价不止是慢。
    #[cfg(unix)]
    #[test]
    fn concurrent_ensure_fresh_captures_once() {
        let _serial = serial_cache_test();
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-race");
        std::fs::create_dir_all(&dir).unwrap();
        let counter = dir.join("count.txt");
        let shell = dir.join("slow-shell");
        let mut f = std::fs::File::create(&shell).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        // 故意慢一点，让并发窗口真实存在（否则先到的那个瞬间就跑完了）
        writeln!(f, "echo x >> {}", counter.display()).unwrap();
        writeln!(f, "sleep 0.2").unwrap();
        writeln!(f, "printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shell_path = shell.to_string_lossy().to_string();
        let home = dir.to_string_lossy().to_string();
        clear();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let shell_path = shell_path.clone();
            let home = home.clone();
            handles.push(std::thread::spawn(move || ensure_fresh(&shell_path, &home)));
        }
        for handle in handles {
            assert!(handle.join().unwrap(), "并发刷新都应成功");
        }
        let count = std::fs::read_to_string(&counter).unwrap().lines().count();
        assert_eq!(count, 1, "并发跨 TTL 时应只捕获一次，实际执行 {count} 次");
        clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn capture_env_child_is_session_leader_without_controlling_terminal() {
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-session");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("session-shell");
        let pid_file = dir.join("session-shell.pid");
        let mut f = std::fs::File::create(&script).unwrap();
        // 假 shell：先写下自己的 pid 并短暂存活（供测试查会话号），再按约定输出 env 段。
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "echo $$ > \"{0}.pid\"", script.to_string_lossy()).unwrap();
        writeln!(f, "sleep 0.5").unwrap();
        writeln!(f, "printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let shell = script.to_string_lossy().to_string();
        let home = dir.to_string_lossy().to_string();
        let handle = std::thread::spawn(move || capture_env(&shell, &home));
        // 在子进程存活期间查会话号：进程一旦退出就被回收，getsid 只会返回 -1。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let sid = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                let sid = unsafe { libc::getsid(pid) };
                if sid != -1 {
                    break (pid, sid);
                }
            }
            assert!(std::time::Instant::now() < deadline, "等不到捕获子进程的 pid");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        // 会话组长 = setsid 生效。若退回到 process_group(0)，会话仍是测试进程的，
        // 交互式 shell 就会在 tcsetpgrp 上被 SIGTTOU 停住（读取环境超时的真因）。
        assert_eq!(sid.1, sid.0, "捕获子进程必须是新会话的组长");

        let captured = handle.join().unwrap().unwrap();
        assert!(!captured.vars.is_empty(), "假 shell 也应产出环境变量");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- 执行器解析（D25） ----------

    /// 未选择 shell 时必须回落平台默认执行器：Unix `/bin/sh -c`、Windows `cmd /C`。
    #[test]
    fn command_invocation_falls_back_to_platform_default_when_unconfigured() {
        let _serial = serial_cache_test();
        clear();
        let (file, args) = command_invocation("echo hi");
        #[cfg(unix)]
        {
            assert_eq!(file, "/bin/sh");
            assert_eq!(args, vec!["-c".to_string(), "echo hi".to_string()]);
        }
        #[cfg(windows)]
        {
            assert_eq!(file, "cmd");
            assert_eq!(args, vec!["/C".to_string(), "echo hi".to_string()]);
        }
    }

    /// 选中 POSIX 家族 shell 后，命令交给它自己的 `-c` 解释；命令串原样传给 argv，
    /// 不做任何转义或重写（引号、分号、换行都由该 shell 自己解析）。
    ///
    /// 用 bash 代表「无额外展开差异」的 POSIX 家族。zsh / csh 会多出压展开的选项，
    /// 分别由 [`command_invocation_zsh_disables_equals_and_nomatch_and_skips_rcs`] 与
    /// [`command_invocation_csh_prefixes_set_nonomatch`] 覆盖。
    #[cfg(unix)]
    #[test]
    fn command_invocation_uses_posix_dash_c_for_posix_shells() {
        let _serial = serial_cache_test();
        use std::io::Write;
        let dir = crate::testutil::unique_temp_dir("shellenv-invocation");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-bash");
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "printf '%s\\n' {MARK_BEGIN}; env -0; printf '%s\\n' {MARK_END}").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let shell = script.to_string_lossy().to_string();
        let home = dir.to_string_lossy().to_string();
        configure(&shell, &home).unwrap();

        let (file, args) = command_invocation("echo 'a; b'");
        assert_eq!(file, shell, "执行器必须是选中的 shell 路径");
        assert_eq!(args, vec!["-c".to_string(), "echo 'a; b'".to_string()]);

        clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// zsh 必须压掉 `=cmd` 展开与 glob 无匹配中止，且跳过启动配置。
    ///
    /// 三条都是同一个目标：命令串按 POSIX 写就，换执行器不该改变成功/失败模式。
    /// `-f` 不是可选优化——`.zshenv` 在命令行选项之后执行，里面一句 `setopt equals`
    /// 就能推翻 `-o noequals`（实测）。
    #[test]
    fn command_invocation_zsh_disables_equals_and_nomatch_and_skips_rcs() {
        let args = invocation_args(ShellKind::Zsh, "/bin/zsh", "echo ===; grep -n x --include=*.rs");
        assert_eq!(
            args,
            vec![
                "-f".to_string(),
                "-o".to_string(),
                "noequals".to_string(),
                "-o".to_string(),
                "nonomatch".to_string(),
                "-c".to_string(),
                "echo ===; grep -n x --include=*.rs".to_string(),
            ],
            "zsh 的 argv 顺序被改动时，命令串必须仍在最后且原样：{args:?}"
        );
    }

    /// csh/tcsh 不认 `-o`（`Unknown option`），只能把 `set nonomatch` 写成脚本前缀；
    /// 它没有 `=cmd` 展开，故不需要 equals 相关处理。
    #[test]
    fn command_invocation_csh_prefixes_set_nonomatch() {
        let args = invocation_args(ShellKind::Csh, "/bin/tcsh", "grep -n x --include=*.rs");
        assert_eq!(
            args,
            vec![
                "-f".to_string(),
                "-c".to_string(),
                "set nonomatch; grep -n x --include=*.rs".to_string(),
            ],
            "tcsh 前缀被改动时无匹配 glob 会再次让整条命令不执行：{args:?}"
        );
    }

    /// 真机回归：会话 `th_96178d5f9ab3f6df` 里高频失败的两条命令，在真实 zsh 下必须成功。
    ///
    /// 这两条分别踩中 zsh 的 `EQUALS`（`echo ===`）与 `NOMATCH`（裸 `--include=*.rs`）。
    /// 断言的是「stderr 不再出现那两句报错」而不是具体输出——报错文案才是本用例要守的东西。
    #[cfg(unix)]
    #[test]
    fn zsh_dash_c_survives_echo_equals_and_bare_glob() {
        let Some(zsh) = detect_shells().into_iter().find(|s| s.kind == ShellKind::Zsh) else {
            // 精简环境里没有 zsh（如部分 Linux 容器）：无 shell 可验，跳过而不挂测试
            return;
        };
        let dir = crate::testutil::unique_temp_dir("shellenv-zsh-real");
        std::fs::create_dir_all(&dir).unwrap();
        // 造一个会推翻命令行选项的 .zshenv：证明 -f 是必要条件而非可选优化
        std::fs::write(dir.join(".zshenv"), "setopt equals nomatch\n").unwrap();
        // 再放一个真实存在的文件，让 glob 有可匹配对象（与 grep 的实际用法一致）
        std::fs::write(dir.join("probe.rs"), "fn probe() {}\n").unwrap();

        let run = |command: &str| {
            let args = invocation_args(ShellKind::Zsh, &zsh.path, command);
            let mut spec = crate::platform::RunSpec::new(zsh.path.clone());
            spec.args = args;
            spec.dir = Some(dir.to_string_lossy().to_string());
            spec.timeout_ms = Some(10_000);
            // HOME 指到临时目录，确保测的是我们造的那份 .zshenv，而不是宿主的
            let mut env = crate::platform::sanitized_env();
            env.retain(|(k, _)| k != "HOME");
            env.push(("HOME".to_string(), dir.to_string_lossy().to_string()));
            spec.env = Some(env);
            crate::platform::run(spec)
        };

        let cat = run("echo ===; grep -rn probe --include=*.rs . | head -3");
        assert_eq!(cat.exit_code, 0, "zsh 执行失败：{}", cat.output);
        assert!(!cat.output.contains("== not found"), "`===` 不应触发 =cmd 展开：{}", cat.output);
        assert!(!cat.output.contains("no matches found"), "无匹配 glob 不应中止命令：{}", cat.output);
        assert!(cat.output.contains("probe.rs"), "命令链应真正执行到 grep：{}", cat.output);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `-f` 顺带挡掉 `.zshenv` 的两类副作用，且不牺牲用户 export 的变量。
    ///
    /// 三件事一起验，因为它们共同构成「跳过启动配置是安全的」这个判断：
    /// `.zshenv` 里的 `cd` 不得覆盖工具传入的 cwd（否则 `shell_exec` 的 `cwd` 参数失效），
    /// `.zshenv` 的 stderr 噪音不得混进命令输出，而 `export` 的变量必须仍然可用——
    /// 变量的来源是捕获阶段注入，不是执行期读文件，这是 `-f` 方案成立的前提。
    #[cfg(unix)]
    #[test]
    fn zsh_execution_ignores_zshenv_side_effects_and_keeps_captured_vars() {
        let Some(zsh) = detect_shells().into_iter().find(|s| s.kind == ShellKind::Zsh) else {
            return;
        };
        let dir = crate::testutil::unique_temp_dir("shellenv-zsh-zshenv");
        let work = dir.join("work");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        // .zshenv 同时做三件事：跳目录、写噪音、导出变量
        std::fs::write(
            dir.join(".zshenv"),
            format!(
                "cd '{}'\necho ZSHENV_NOISE >&2\nexport CW_PROBE_VAR=kept\n",
                elsewhere.to_string_lossy()
            ),
        )
        .unwrap();

        let args = invocation_args(ShellKind::Zsh, &zsh.path, "pwd; echo \"var=$CW_PROBE_VAR\"");
        let mut spec = crate::platform::RunSpec::new(zsh.path.clone());
        spec.args = args;
        spec.dir = Some(work.to_string_lossy().to_string());
        spec.timeout_ms = Some(10_000);
        let mut env = crate::platform::sanitized_env();
        env.retain(|(k, _)| k != "HOME");
        env.push(("HOME".to_string(), dir.to_string_lossy().to_string()));
        // 捕获阶段已把 export 的变量注入进来（真实链路如此，这里用显式注入模拟）
        env.push(("CW_PROBE_VAR".to_string(), "kept".to_string()));
        spec.env = Some(env);
        let result = crate::platform::run(spec);

        assert_eq!(result.exit_code, 0, "命令应正常执行：{}", result.output);
        assert!(
            result.output.contains(&work.to_string_lossy().to_string()),
            ".zshenv 里的 cd 不得覆盖工具传入的 cwd：{}",
            result.output
        );
        assert!(
            !result.output.contains(&elsewhere.to_string_lossy().to_string()),
            "cwd 仍停在 .zshenv 跳到的地方：{}",
            result.output
        );
        assert!(!result.output.contains("ZSHENV_NOISE"), ".zshenv 的 stderr 不得混进输出：{}", result.output);
        assert!(
            result.output.contains("var=kept"),
            "捕获注入的变量必须仍然可用（-f 不该影响它）：{}",
            result.output
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 且脚本带退出码尾——否则原生进程的退出码会丢失（实测 `cmd /c exit 5` 得 0）。
    #[test]
    fn command_invocation_powershell_skips_profile_and_propagates_exit_code() {
        let _serial = serial_cache_test();
        let args = invocation_args(ShellKind::Powershell, "C:\\Program Files\\PowerShell\\7\\pwsh.exe", "Write-Output hi");
        assert!(args.contains(&"-NoProfile".to_string()), "执行期不得重跑启动配置：{args:?}");
        assert!(args.contains(&"-Command".to_string()));
        let script = args.last().expect("脚本应在最后");
        assert!(script.contains("Write-Output hi"), "命令原文必须保留");
        assert!(script.contains("$LASTEXITCODE"), "必须参考原生命令退出码：{script}");
        assert!(script.trim_end().ends_with("exit 0"), "无失败时显式给 0：{script}");
        // pwsh 7 默认 UTF-8，不应插入 5.1 专属的编码设置
        assert!(!script.contains("OutputEncoding"), "pwsh 7 不需要 OEM 代码页修正：{script}");
    }

    /// Windows PowerShell 5.1 重定向下默认走 OEM 代码页，中文输出会 mojibake：
    /// 必须在脚本里先设 UTF-8；pwsh 7 天然 UTF-8，不加这段。
    #[test]
    fn command_invocation_only_ps51_needs_utf8_output_encoding() {
        let ps51 = invocation_args(ShellKind::Powershell, "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe", "Write-Output hi");
        assert!(ps51.last().unwrap().contains("OutputEncoding"), "5.1 必须设 UTF-8 输出编码");
        assert!(is_windows_powershell("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
        assert!(!is_windows_powershell("C:\\Program Files\\PowerShell\\7\\pwsh.exe"));
    }
}
