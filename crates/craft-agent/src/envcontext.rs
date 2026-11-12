//! 环境上下文快照：日期 / 平台 / Shell + Git 摘要 + 指令文件（AGENTS.md 两级）。
//! 任何一步失败都降级省略，不阻断运行——环境公告是增强信息。
//! 同步采集，调用方放阻塞线程。
//!
//! Git 摘要（D13 迁移后）走 craft-git：分支名 / `status --short` / `log --oneline -n 5`
//! 三段与迁移前进程调用输出同构（分支行与装饰本就不在此处使用）；不在 Git 仓库或
//! 采集失败（含 3s 超时，经 craft-git 的协作式中断等价物）时整体降级省略，子项失败
//! 空串——与迁移前逐条对齐。

use craft_core::platform::{run, RunSpec};

const GIT_TIMEOUT_MS: u64 = 3000;
const MAX_INSTRUCTION_LINES: usize = 200;
const MAX_INSTRUCTION_CHARS: usize = 25000;
const RUNTIME_TIMEOUT_MS: u64 = 3000;
const RUNTIME_OUTPUT_LIMIT: usize = 1024;

/// 一份指令文件（AGENTS.md）的内容：scope 标明层级，超长时截断并标记。
#[derive(Debug, Clone)]
pub struct EnvInstruction {
    pub path: String,
    /// 指令层级："global"（用户全局）/ "project"（项目）
    pub scope: &'static str,
    pub body: String,
    /// 因超行/超字被截断时为 true（截断标记会随内容一起写入提示词）
    pub truncated: bool,
}

/// Git 工作区摘要（分支 / 未提交变更 / 最近提交）；各字段采集失败时为空串。
#[derive(Debug, Clone, Default)]
pub struct GitSnapshot {
    pub branch: String,
    pub status: String,
    pub commits: String,
}

/// 环境上下文快照：注入系统提示词动态块；各步失败降级省略，字段可能为空。
#[derive(Debug, Clone, Default)]
pub struct EnvContextSnapshot {
    pub today: String,
    pub platform: String,
    /// 生效的 shell：选中时是「路径 + 家族」，未选中时是 $SHELL 的短名（D18）
    pub shell: String,
    /// 选中 shell 会执行的配置文件（绝对路径，只含实际存在的）
    pub shell_configs: Vec<String>,
    /// 清洗后保留的环境变量**名**（不含值——值可能含凭据，且提示词会发往提供商）
    pub shell_env_names: Vec<String>,
    /// 可用的脚本运行时（如 "python3 3.12.4"）；探测失败为空，模型可据此决定批量操作方式
    pub runtimes: Vec<String>,
    /// Git 摘要；不在 Git 工作区内为 None
    pub git: Option<GitSnapshot>,
    pub instructions: Vec<EnvInstruction>,
}

/// 采集 Git 摘要的分支名：符号 HEAD 的分支短名（unborn HEAD 也算，与
/// `symbolic-ref --short -q HEAD` 一致）；分离 HEAD / 读取失败为空串。
fn git_branch_name(repo: &craft_git::Repository) -> String {
    repo.head_branch_name().unwrap_or_default()
}

/// 探测单个运行时：`<file> <flag>` 成功退出即视为可用，取输出首行作版本描述（如 "python3 3.12.4"）。
fn detect_runtime(file: &str, flag: &str) -> Option<String> {
    let result = run(RunSpec {
        file: file.to_string(),
        args: vec![flag.to_string()],
        dir: None,
        timeout_ms: Some(RUNTIME_TIMEOUT_MS),
        output_limit: Some(RUNTIME_OUTPUT_LIMIT),
        // 用选中的 shell 环境探测：nvm/pyenv 装的解释器只有这样才能被找到（D18）
        env: craft_core::shellenv::current_vars(),
        new_session: false,
    });
    if result.timed_out || result.exit_code != 0 {
        return None;
    }
    let line = result.output.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    // "Python 3.12.4" → "3.12.4"，避免 "python3 Python 3.12.4" 式重复
    let version = line.strip_prefix("Python ").unwrap_or(line);
    Some(format!("{file} {version}"))
}

/// 探测可用的脚本运行时：优先 python3，缺失时回落 python；全部失败静默省略。
fn detect_runtimes() -> Vec<String> {
    let mut runtimes = Vec::new();
    if let Some(desc) = detect_runtime("python3", "--version") {
        runtimes.push(desc);
    } else if let Some(desc) = detect_runtime("python", "--version") {
        runtimes.push(desc);
    }
    runtimes
}


/// 未选择 shell 时实际解释命令的程序名（提示词用）：Unix `/bin/sh`、Windows `cmd`。
///
/// 与 `shellenv::default_invocation` 同源判定，但这里只要一个短名用于展示。
fn default_executor_name() -> String {
    #[cfg(unix)]
    {
        "sh".to_string()
    }
    #[cfg(windows)]
    {
        "cmd".to_string()
    }
}

/// 采集环境快照（Git 失败或超时返回空串并跳过；任何失败都降级省略）。
pub fn collect_env_context(data_dir: &str, workspace: &str) -> EnvContextSnapshot {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    // shell 信息：选中时用真实生效的 shell（路径 + 家族），否则报平台默认执行器。
    // 注入变量**名**而非值：值可能含凭据，而提示词会发往模型提供商。
    let shell_meta = craft_core::shellenv::current_meta();
    let mut snapshot = EnvContextSnapshot {
        today,
        platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        // 未配置时执行器是平台默认（Unix /bin/sh、Windows cmd，D25）——这里必须报它，
        // 不能报 $SHELL：那只是「用户偏好哪个登录 shell」，与实际解释命令的程序无关，
        // 报错会让模型按 zsh/pwsh 语法写命令，实际却被 sh/cmd 解释。
        shell: match &shell_meta {
            Some(meta) => format!("{} ({})", meta.shell_path, meta.kind.as_str()),
            None => default_executor_name(),
        },
        shell_configs: shell_meta.as_ref().map(|m| m.configs.clone()).unwrap_or_default(),
        shell_env_names: shell_meta
            .as_ref()
            .map(|m| m.vars.iter().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default(),
        runtimes: detect_runtimes(),
        git: None,
        instructions: Vec::new(),
    };

    if !workspace.is_empty() {
        // Git 摘要：不在 Git 仓库或采集失败（含 3s 超时）时整体省略（git=None）；
        // 子项失败各自空串——三段降级行为与迁移前逐条对齐。
        let workspace_owned = workspace.to_string();
        let git_result = craft_git::with_timeout(
            std::time::Duration::from_millis(GIT_TIMEOUT_MS),
            "Git 摘要采集",
            move |flag| {
                let repo = craft_git::Repository::discover(&workspace_owned)?;
                // 分支：unborn 也返回分支名；分离 HEAD 为 None → 空串。
                let branch = git_branch_name(&repo);
                // status：无分支头行（分支单列），输出与 `git status --short` 同构。
                let status = craft_git::status::status_text(&repo, false, flag)
                    .map(|text| text.trim().to_string())
                    .unwrap_or_default();
                // log：无装饰 oneline 最近 5 条，输出与 `git log --oneline -n 5` 同构
                // （unborn HEAD 在这里报错 → 空串，与迁移前一致）。
                let commits = craft_git::log::log_text(
                    &repo,
                    craft_git::log::LogOptions { limit: 5, path: None, decorate: false },
                    flag,
                )
                .map(|text| text.trim().to_string())
                .unwrap_or_default();
                Ok((branch, status, commits))
            },
        );
        if let Ok((branch, status, commits)) = git_result {
            snapshot.git = Some(GitSnapshot { branch, status, commits });
        }

        // 指令文件两级：全局 → 项目
        let global_path = std::path::Path::new(data_dir).join("AGENTS.md");
        let project_path = std::path::Path::new(workspace).join("AGENTS.md");
        if let Some(inst) = read_instruction(&global_path, "global") {
            snapshot.instructions.push(inst);
        }
        if let Some(inst) = read_instruction(&project_path, "project") {
            snapshot.instructions.push(inst);
        }
    }

    snapshot
}

/// 读取单个指令文件：缺失/空文件/目录返回 None；超行数或超字数时截断并标记 truncated。
fn read_instruction(path: &std::path::Path, scope: &'static str) -> Option<EnvInstruction> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }
    let body = std::fs::read_to_string(path).ok()?;
    if body.trim().is_empty() {
        return None;
    }
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.len() > MAX_INSTRUCTION_LINES || body.len() > MAX_INSTRUCTION_CHARS {
        let truncated_body: String = lines
            .iter()
            .take(MAX_INSTRUCTION_LINES)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(MAX_INSTRUCTION_CHARS)
            .collect();
        return Some(EnvInstruction {
            path: path.to_string_lossy().to_string(),
            scope,
            body: truncated_body,
            truncated: true,
        });
    }
    Some(EnvInstruction {
        path: path.to_string_lossy().to_string(),
        scope,
        body,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_file_read_and_truncation() {
        let dir = craft_core::testutil::unique_temp_dir("env");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "项目指令内容").unwrap();
        let inst = read_instruction(&dir.join("AGENTS.md"), "project").unwrap();
        assert_eq!(inst.body, "项目指令内容");
        assert!(!inst.truncated);
        // 超长截断
        let big = "x".repeat(MAX_INSTRUCTION_CHARS + 10);
        std::fs::write(dir.join("AGENTS.md"), &big).unwrap();
        let inst = read_instruction(&dir.join("AGENTS.md"), "project").unwrap();
        assert!(inst.truncated);
        assert!(inst.body.chars().count() <= MAX_INSTRUCTION_CHARS);
        // 缺失降级
        assert!(read_instruction(&dir.join("nope.md"), "project").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn environment_snapshot_non_git_dir_fallback() {
        let dir = craft_core::testutil::unique_temp_dir("env2");
        std::fs::create_dir_all(&dir).unwrap();
        let snap = collect_env_context("/nonexistent", &dir.to_string_lossy());
        assert!(snap.git.is_none());
        assert!(!snap.platform.is_empty());
        assert!(!snap.today.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_runtime_missing_program_returns_none() {
        assert!(detect_runtime("craftwork-nonexistent-runtime-xyz", "--version").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn detect_runtime_reports_existing_interpreter() {
        // /bin/echo 稳定存在且成功退出，用它的输出验证「成功即探测到」的路径
        let desc = detect_runtime("/bin/echo", "probe-ok").expect("echo 应探测成功");
        assert!(desc.contains("probe-ok"));
    }

    // Windows 等价（无真机，CI 的 windows runner 是唯一验证场）：
    // cmd.exe 必在 PATH，`cmd /C echo probe-ok` 的输出与退出码语义同 /bin/echo。
    #[cfg(windows)]
    #[test]
    fn detect_runtime_reports_existing_interpreter_windows() {
        let desc = detect_runtime("cmd", "/C echo probe-ok").expect("cmd 应探测成功");
        assert!(desc.contains("probe-ok"));
    }

    /// 切换 shell 后，下一次 run 采集到的环境快照（提示词输入）必须已经是新 shell。
    /// 提示词每个 run 重建，快照读的是进程缓存；设置保存时 `configure` 同步更新缓存，
    /// 所以「保存返回 → 下一条消息」之间不存在迟到。
    #[cfg(unix)]
    #[test]
    fn env_context_reflects_newly_configured_shell() {
        use std::io::Write;
        let dir = craft_core::testutil::unique_temp_dir("envcontext-shell");
        std::fs::create_dir_all(&dir).unwrap();
        let shell = dir.join("fake-shell");
        let home = dir.to_string_lossy().to_string();
        let mut f = std::fs::File::create(&shell).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "printf '%s\\n' __CRAFTWORK_ENV_BEGIN__; env -0; printf '%s\\n' __CRAFTWORK_ENV_END__").unwrap();
        drop(f);
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        craft_core::shellenv::configure(&shell.to_string_lossy(), &home).unwrap();
        let snap = collect_env_context(&home, "");
        craft_core::shellenv::clear();

        assert_eq!(snap.shell, format!("{} (other)", shell.to_string_lossy()), "快照应指向新选中的 shell");
        assert!(snap.shell_env_names.iter().any(|n| n == "PATH"), "应带上变量名清单");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
