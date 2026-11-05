//! 环境上下文快照：日期 / 平台 / Shell + Git 摘要 + 指令文件（AGENTS.md 两级）。
//! 任何一步失败都降级省略，不阻断运行——环境公告是增强信息。
//! 同步采集，调用方放阻塞线程。

use craft_core::platform::{run, RunSpec};

const GIT_TIMEOUT_MS: u64 = 3000;
const GIT_OUTPUT_LIMIT: usize = 64 * 1024;
const MAX_INSTRUCTION_LINES: usize = 200;
const MAX_INSTRUCTION_CHARS: usize = 25000;

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
    pub shell: String,
    /// Git 摘要；不在 Git 工作区内为 None
    pub git: Option<GitSnapshot>,
    pub instructions: Vec<EnvInstruction>,
}

fn git_run(workspace: &str, args: &[&str]) -> craft_core::platform::RunResult {
    run(RunSpec {
        file: "git".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        dir: Some(workspace.to_string()),
        timeout_ms: Some(GIT_TIMEOUT_MS),
        output_limit: Some(GIT_OUTPUT_LIMIT),
    })
}

/// 采集环境快照（Git 失败或超时返回空串并跳过；任何失败都降级省略）。
pub fn collect_env_context(data_dir: &str, workspace: &str) -> EnvContextSnapshot {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut snapshot = EnvContextSnapshot {
        today,
        platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        shell: std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_string))
            .unwrap_or_default(),
        git: None,
        instructions: Vec::new(),
    };

    if !workspace.is_empty() {
        let inside = git_run(workspace, &["rev-parse", "--is-inside-work-tree"]);
        if inside.exit_code == 0 && inside.output.trim() == "true" {
            let branch = git_run(workspace, &["symbolic-ref", "--short", "-q", "HEAD"]);
            let status = git_run(workspace, &["status", "--short"]);
            let log = git_run(workspace, &["log", "--oneline", "-n", "5"]);
            snapshot.git = Some(GitSnapshot {
                branch: if branch.exit_code == 0 { branch.output.trim().to_string() } else { String::new() },
                status: if status.exit_code == 0 { status.output.trim().to_string() } else { String::new() },
                commits: if log.exit_code == 0 { log.output.trim().to_string() } else { String::new() },
            });
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
        let dir = std::env::temp_dir().join(format!(
            "cw-env-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
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
        let dir = std::env::temp_dir().join(format!(
            "cw-env2-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let snap = collect_env_context("/nonexistent", &dir.to_string_lossy());
        assert!(snap.git.is_none());
        assert!(!snap.platform.is_empty());
        assert!(!snap.today.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
