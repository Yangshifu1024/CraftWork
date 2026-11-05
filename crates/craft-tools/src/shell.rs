//! shell_exec / git_* 内置工具。

use serde_json::{json, Value};

use craft_core::platform::run;
use craft_core::platform::RunSpec;
use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

fn err_result(message: impl Into<String>) -> ToolResult {
    ToolResult::err(message)
}

// ---------- shell_exec ----------

pub struct ShellExec;

impl Tool for ShellExec {
    fn spec(&self) -> ToolSpec {
        ShellExec::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let command = ctx.str_arg("command").trim().to_string();
        if command.is_empty() {
            return err_result("command 不能为空");
        }
        let timeout_sec = ctx.int_arg("timeout", 120).clamp(1, 600);
        let started = std::time::Instant::now();
        // Windows 走 cmd /C；Unix 走 /bin/sh -c（Windows 实机验证属 D7 排期）
        #[cfg(unix)]
        let (shell_file, shell_args): (&str, Vec<String>) = ("/bin/sh", vec!["-c".into(), command.clone()]);
        #[cfg(windows)]
        let (shell_file, shell_args): (&str, Vec<String>) = ("cmd", vec!["/C".into(), command.clone()]);
        let result = run(RunSpec {
            file: shell_file.into(),
            args: shell_args,
            dir: if ctx.target.is_empty() { Some(ctx.workspace.clone()) } else { Some(ctx.target.clone()) },
            timeout_ms: Some(timeout_sec as u64 * 1000),
            output_limit: None,
        });
        let duration = started.elapsed();
        if result.timed_out {
            return ToolResult {
                output: format!("命令超时（{}s）：{}", duration.as_secs(), result.output),
                is_error: true,
                code: "tool_error".into(),
                meta: Some(json!({ "exitCode": -1, "timedOut": true })),
            };
        }
        let spawn_error = (result.exit_code < 0).then(|| result.error.clone()).flatten();
        if let Some(error) = spawn_error {
            return err_result(format!("执行命令失败：{error}"));
        }
        let output = result.output.trim_end_matches(['\n']).to_string();
        let output = if output.is_empty() { "(无输出)".to_string() } else { output };
        ToolResult {
            output: format!("$ {command}\n{output}\n[退出码 {} · {}s]", result.exit_code, duration.as_secs()),
            is_error: result.exit_code != 0,
            code: if result.exit_code != 0 { "tool_error".into() } else { String::new() },
            meta: Some(json!({ "exitCode": result.exit_code })),
        }
    }
}

impl ShellExec {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "shell_exec".into(),
        description: "在工作区内执行 shell 命令（/bin/sh -c）。输出上限 32KB，超时上限 600 秒（默认 120）。命令在清洗后的环境中运行。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "要执行的命令" },
                "cwd": { "type": "string", "description": "相对工作区的运行目录，默认工作区根" },
                "timeout": { "type": "integer", "minimum": 1, "maximum": 600, "description": "超时秒数，默认 120" },
            },
            "required": ["command"],
        }),
        risk: PolicyRisk::R2,
        path_arg: Some("cwd".into()),
        command_arg: Some("command".into()),
        work: false,
        title: "执行命令".into(),
        summary_arg: Some("command".into()),
        summary_text: None,
        }
    }
}

// ---------- git_* ----------

fn git(ctx: &ToolCtx, args: &[&str], timeout_sec: u64) -> (craft_core::platform::RunResult, String) {
    let result = run(RunSpec {
        file: "git".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        dir: Some(ctx.workspace.clone()),
        timeout_ms: Some(timeout_sec * 1000),
        output_limit: None,
    });
    let output = result.output.clone();
    (result, output)
}

pub struct GitStatus;

impl Tool for GitStatus {
    fn spec(&self) -> ToolSpec {
        GitStatus::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let (result, _) = git(ctx, &["status", "--short", "--branch"], 20);
        if result.exit_code != 0 {
            return err_result(format!(
                "git status 失败：{}",
                if result.output.trim().is_empty() { "目录不是 Git 仓库" } else { result.output.trim() }
            ));
        }
        let out = result.output.trim().to_string();
        ToolResult::ok_with_meta(
            if out.is_empty() { "工作区干净（没有未提交的改动）".to_string() } else { out },
            json!({ "path": ctx.workspace }),
        )
    }
}

impl GitStatus {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "git_status".into(),
        description: "查看主要工作目录的 Git 状态（分支与未提交改动）。".into(),
        schema: json!({ "type": "object", "properties": {} }),
        risk: PolicyRisk::R0,
        path_arg: None,
        command_arg: None,
        work: false,
        title: String::new(),
        summary_arg: None,
        summary_text: Some("git status".into()),
        }
    }
}

pub struct GitDiff;

impl Tool for GitDiff {
    fn spec(&self) -> ToolSpec {
        GitDiff::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let mut args: Vec<&str> = vec!["diff", "--no-color"];
        let staged = ctx.args.get("staged").and_then(Value::as_bool).unwrap_or(false);
        if staged {
            args.push("--staged");
        }
        if !ctx.display.is_empty() {
            args.push("--");
            args.push(&ctx.display);
        }
        let (result, _) = git(ctx, &args, 30);
        if result.exit_code != 0 {
            return err_result(format!("git diff 失败：{}", result.output.trim()));
        }
        ToolResult::ok_with_meta(
            if result.output.trim().is_empty() { "没有差异".to_string() } else { result.output.trim().to_string() },
            json!({ "staged": staged }),
        )
    }
}

impl GitDiff {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "git_diff".into(),
        description: "查看文件或整个仓库的 Git 差异（默认未暂存；staged=true 查看已暂存）。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "限制差异范围的路径" },
                "staged": { "type": "boolean", "description": "查看已暂存的差异，默认 false" },
            },
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        work: false,
        title: String::new(),
        summary_arg: Some("path".into()),
        summary_text: Some("git diff".into()),
        }
    }
}

pub struct GitLog;

impl Tool for GitLog {
    fn spec(&self) -> ToolSpec {
        GitLog::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let limit = ctx.int_arg("limit", 20).clamp(1, 100);
        let mut args: Vec<String> = vec!["log".into(), "--oneline".into(), "--decorate".into(), "-n".into(), limit.to_string()];
        if !ctx.display.is_empty() {
            args.push("--".into());
            args.push(ctx.display.clone());
        }
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let (result, _) = git(ctx, &arg_refs, 30);
        if result.exit_code != 0 {
            return err_result(format!("git log 失败：{}", result.output.trim()));
        }
        ToolResult::ok_with_meta(
            if result.output.trim().is_empty() { "还没有提交记录".to_string() } else { result.output.trim().to_string() },
            json!({ "limit": limit }),
        )
    }
}

impl GitLog {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "git_log".into(),
        description: "查看提交记录（oneline + 修饰，默认最近 20 条，最多 100 条）。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "条数，默认 20" },
                "path": { "type": "string", "description": "限制历史范围的路径" },
            },
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        work: false,
        title: String::new(),
        summary_arg: Some("path".into()),
        summary_text: Some("git log".into()),
        }
    }
}

pub struct GitCommit;

impl Tool for GitCommit {
    fn spec(&self) -> ToolSpec {
        GitCommit::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let message = ctx.str_arg("message").trim().to_string();
        if message.is_empty() {
            return err_result("message 不能为空");
        }
        if message.contains('\0') || message.len() > 2000 {
            return err_result("提交信息无效（为空、含空字符或超过 2000 字节）");
        }
        let paths_arg: Vec<String> = ctx
            .args
            .get("paths")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        let mut paths: Vec<String> = Vec::new();
        for p in &paths_arg {
            match ctx.env.resolve_extra(&ctx.roots, p) {
                Ok((_, display)) => paths.push(display),
                Err(err) => return err_result(err.to_string()),
            }
        }
        if !paths.is_empty() {
            let add_refs: Vec<&str> = std::iter::once("add")
                .chain(std::iter::once("--"))
                .chain(paths.iter().map(String::as_str))
                .collect();
            let (add, _) = git(ctx, &add_refs, 30);
            if add.exit_code != 0 {
                return err_result(format!("提交失败: {}", add.output.trim()));
            }
            let commit_refs: Vec<&str> = std::iter::once("commit")
                .chain(std::iter::once("-m"))
                .chain(std::iter::once(message.as_str()))
                .chain(std::iter::once("--"))
                .chain(paths.iter().map(String::as_str))
                .collect();
            let (commit, _) = git(ctx, &commit_refs, 30);
            if commit.exit_code != 0 {
                return err_result(format!("提交失败: {}", commit.output.trim()));
            }
        } else {
            let (commit, _) = git(ctx, &["commit", "-am", &message], 30);
            if commit.exit_code != 0 {
                return err_result(format!("提交失败: {}", commit.output.trim()));
            }
        }
        let (hash, _) = git(ctx, &["rev-parse", "--short", "HEAD"], 10);
        let hash_out = if hash.exit_code == 0 { hash.output.trim().to_string() } else { String::new() };
        let first_line = message.split('\n').next().unwrap_or(&message).to_string();
        ToolResult::ok_with_meta(
            format!("已提交 {hash_out}：{first_line}\n（未推送）"),
            json!({ "hash": hash_out, "message": message, "paths": paths }),
        )
    }
}

impl GitCommit {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "git_commit".into(),
        description: "提交代码（默认不推送）。有 paths 时先 add 再提交这些文件；否则提交全部已跟踪改动。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "提交信息" },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "要提交的文件路径（相对工作区）" },
            },
            "required": ["message"],
        }),
        risk: PolicyRisk::R1,
        path_arg: None,
        command_arg: None,
        work: false,
        title: "提交代码".into(),
        summary_arg: Some("message".into()),
        summary_text: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolEnv;
    use std::sync::Arc;

    fn make_ctx(args: Value, roots: Vec<String>) -> (tempdir::TempDir, ToolCtx) {
        let dir = tempdir::TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Arc::new(craft_store::db::Db::open(&db_path).unwrap());
        let env = Arc::new(ToolEnv { backup_dir: dir.path().join("backups").to_string_lossy().to_string(), db });
        let ctx = ToolCtx {
            name: "test".into(),
            args,
            target: String::new(),
            display: String::new(),
            workspace: roots.first().cloned().unwrap_or_default(),
            roots,
            target_root: String::new(),
            err: None,
            env,
        };
        (dir, ctx)
    }

    #[cfg(unix)]
    #[test]
    fn shell_exec_捕获输出与退出码() {
        let (_dir, ctx) = make_ctx(json!({"command": "echo hello; exit 3"}), vec!["/tmp".to_string()]);
        let result = ShellExec.execute(&ctx);
        assert!(result.is_error);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("[退出码 3"));
    }

    #[cfg(unix)]
    #[test]
    fn shell_exec_空命令拒绝() {
        let (_dir, ctx) = make_ctx(json!({"command": "  "}), vec!["/tmp".to_string()]);
        assert!(ShellExec.execute(&ctx).is_error);
    }

    #[cfg(unix)]
    #[test]
    fn shell_exec_超时杀进程() {
        let (_dir, ctx) = make_ctx(json!({"command": "sleep 30", "timeout": 1}), vec!["/tmp".to_string()]);
        let result = ShellExec.execute(&ctx);
        assert!(result.is_error);
        assert!(result.output.contains("命令超时"));
    }
}

// 最小临时目录助手（仅测试构建）
#[cfg(test)]
mod tempdir {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Result<Self, String> {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "cw-shell-test-{}-{}-{}",
                std::process::id(),
                n,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
            ));
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            Ok(TempDir(path))
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
