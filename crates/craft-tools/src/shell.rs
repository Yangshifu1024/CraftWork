//! shell_exec / git_* 内置工具。
//!
//! shell_exec 是 Agent 执行命令的唯一通道，不是交互式终端（终端已随决策 D1 移除，
//! 禁止重新引入 PTY）：子进程由 craft_core::platform::run 统一保证独立进程组、
//! 环境白名单清洗（防模型密钥泄漏给子进程）、超时杀整组与输出有界缓冲。
//! git_* 家族只作用于主要目录仓库，且默认不推送。
//!
//! 输出预算（D29）：`platform::run` 只报「是否截断 + 真实丢弃字节」，不往正文写标记；
//! 标记文案在本模块写唯一一份（`shell_exec` 的退出码行之后），并且把截断事实申报到
//! `ToolResult::truncation`，执行出口据此不再二次截断。`bash_output` 的 `tail_bytes`
//! 上限与它的输出预算用同一个常量（`BASH_OUTPUT_MAX_TAIL_BYTES`）——两处不同步就会
//! 出现「要 64 KB 只拿到 32 KB」。
//!
//! 执行器（D25）：前台与后台都用 `shellenv::command_invocation` 解析——用户选中 shell
//! 就用它解释命令，未选择时回落平台默认（Unix `/bin/sh -c`、Windows `cmd /C`）。
//! 环境与执行器必须指向同一个 shell，否则「设置里选了 pwsh」只会改环境变量。
//! 各家族的 argv 差异（zsh 的 `-f -o noequals -o nonomatch`、csh 的 `set nonomatch`
//! 前缀）全在 `command_invocation` 里单点决定，本模块只负责原样使用，不做二次改写。
//!
//! git_* 家族的引擎现状（D13 迁移）：`git_status` /
//! `git_log` / `git_diff` / `git_commit` 四工具已全部走 craft-git（gix 进程内
//! 执行，零进程调用；身份、提交对象、ref 与 reflog 均由 gix 完成）。

use serde_json::{json, Value};

use craft_core::platform::run;
use craft_core::platform::{kill_process_group, spawn_detached, RunSpec};
use craft_policy::PolicyRisk;

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec, MAX_OUTPUT_BYTES};
use crate::shell_tasks::{is_sleep_command, new_task_id, now_millis, ShellTask, TaskState};

/// 装配子进程环境前的取用点刷新（详见 `shellenv::ensure_fresh`）。
///
/// 过期才真跑 shell 启动文件；捕获失败沿用上一次的值——命令宁可拿陈旧环境，也不能
/// 因环境为空而找不到用户装的 pnpm / cargo（静默命中系统旧版本比直接失败更难查）。
/// 并发由 `ensure_fresh` 内部的双检串行化，这里不必加锁。
fn refresh_shell_env() {
    let Some(shell) = craft_core::shellenv::configured_shell() else { return };
    let home = craft_core::platform::home_dir_string();
    let _ = craft_core::shellenv::ensure_fresh(&shell, &home);
}

fn err_result(message: impl Into<String>) -> ToolResult {
    ToolResult::err(message)
}

/// 解析 stdout/stderr 合并的输出文件尾部（默认 8 KiB）。
///
/// 后台任务的输出可能很大（编译日志动辄几十 MB），一律尾部读取：看进度时关心的
/// 永远是最新几行。读不到（文件被删、任务号错）返回可读错误而不是空串——空串会让
/// 模型以为「命令没输出」，实际是找错了对象。
fn read_output_tail(path: &str, max_bytes: usize) -> Result<(String, usize), String> {
    let data = std::fs::read(path).map_err(|e| format!("读取任务输出失败：{e}"))?;
    let total = data.len();
    let start = total.saturating_sub(max_bytes);
    // 从 UTF-8 字符边界起读，避免多字节字符被腰斩成替换符
    let mut start = start;
    while start < total && !std::str::from_utf8(&data[start..]).is_ok() {
        start += 1;
    }
    let text = String::from_utf8_lossy(&data[start..]).to_string();
    Ok((text, total))
}

/// 后台任务启动后的工具结果正文：任务号 + 输出文件 + 「完成会通知你」。
///
/// 文案按 provider 可见的后台任务告知口径写（"Command running in background with ID: …"
/// 一段），但输出路径与读取方式用本仓库的工具名（`bash_output`），不引用不存在的工具。
fn background_ack(task: &ShellTask, total_bytes: usize) -> String {
    let reason = if task.from_timeout {
        "命令未在前台等待预算内完成，已转入后台继续运行".to_string()
    } else {
        "命令已在后台运行".to_string()
    };
    format!(
        "{}（任务号 {}，命令：{}）。\n\
         输出写入：{}\n\
         它结束后会自动通知你，不需要轮询等待，不要用 wait / agent_wait 等（wait 只等时间，agent_wait 只等子代理）。\n\
         需要看具体输出时用 bash_output（task_id = {}）读一次，不要反复查看；要停止用 kill_shell。",
        reason, task.id, task.command, task.output_path, task.id
    ) + if total_bytes > 0 {
        ""
    } else {
        "\n（目前还没有输出）"
    }
}

/// 在工作区内启动后台命令（`run_in_background=true` 与前台超时转后台共用）。
fn start_background(ctx: &ToolCtx, command: &str, cwd: &str, from_timeout: bool) -> Result<ShellTask, String> {
    let task_id = new_task_id();
    // 输出目录按会话切分：删除会话时连同任务日志一起清理，互不干扰
    let dir = std::path::Path::new(&ctx.env.data_dir)
        .join("shell-tasks")
        .join(task_scope(&ctx.thread_scope_id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建任务目录失败：{e}"))?;
    let output_path = dir.join(format!("{task_id}.log"));

    // 执行器跟随用户选中的 shell；未选择时回落平台默认（D25）
    let (shell_file, shell_args) = craft_core::shellenv::command_invocation(command);
    let spec = RunSpec {
        file: shell_file,
        args: shell_args,
        dir: Some(cwd.to_string()),
        // 后台任务没有前台 deadline：生命周期由任务注册表与 kill_shell 决定
        timeout_ms: None,
        output_limit: None,
        // 用户选中的 shell 环境（D18）；未配置时为 None，回落进程环境清洗结果
        env: {
            refresh_shell_env();
            craft_core::shellenv::current_vars()
        },
        new_session: false,
    };
    let process = spawn_detached(&spec, &output_path)?;
    let task = ShellTask {
        id: task_id,
        thread_id: ctx.thread_scope_id(),
        pid: process.pid,
        output_path: process.output_path,
        command: command.to_string(),
        state: TaskState::Running,
        exit_code: None,
        started_at: now_millis(),
        finished_at: 0,
        from_timeout,
        notified: false,
    };
    ctx.env.shell_tasks.register(task.clone());
    // 交棒给等待线程：进程退出即结算 + 投递通知，工具调用本身已经返回
    ctx.env.shell_tasks.watch(process.child, task.clone());
    Ok(task)
}

/// 任务目录的组织键：优先会话 id，缺失时退化为工作区名，避免落到 data_dir 根。
fn task_scope(thread_id: &str) -> String {
    if thread_id.is_empty() {
        "default".to_string()
    } else {
        thread_id.to_string()
    }
}

// ---------- shell_exec ----------

/// 在工作区内执行 shell 命令（R2 执行类；cwd 缺省为工作区根）。
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
        // sleep 命令前置拒绝（D21）：短暂等待用 wait 工具（可被事件与用户打断），等外部
        // 任务用 run_in_background=true。此前只拦超时路径，预算内的短 sleep 照常执行，
        // 模型仍能用它空转；现在把缺口堵上。首词判定，`sh -c 'sleep 10'` 的绕过边界不变。
        if is_sleep_command(&command) {
            return err_result(
                "不要用 shell 跑 sleep：短暂等待（限流退避、等服务就绪、重试间隔）用 wait 工具；\
                 等外部任务用 run_in_background=true，完成会自动通知。",
            );
        }
        let cwd = if ctx.target.is_empty() { ctx.workspace.clone() } else { ctx.target.clone() };
        // 显式后台：启动即返回，不占前台预算
        if ctx.args.get("run_in_background").and_then(Value::as_bool).unwrap_or(false) {
            return match start_background(ctx, &command, &cwd, false) {
                Ok(task) => ToolResult {
                    output: background_ack(&task, 0),
                    is_error: false,
                    code: String::new(),
                    meta: Some(json!({
                        "backgroundTaskId": task.id,
                        "outputPath": task.output_path,
                        "pid": task.pid,
                    })),
                    media: Vec::new(),
                    truncation: None,
                },
                Err(err) => err_result(err),
            };
        }

        let timeout_sec = ctx.int_arg("timeout", 120).clamp(1, 600);
        let started = std::time::Instant::now();
        // 执行器跟随用户选中的 shell；未选择时回落平台默认（Unix /bin/sh -c、
        // Windows cmd /C，D25）
        let (shell_file, shell_args) = craft_core::shellenv::command_invocation(&command);
        let result = run(RunSpec {
            file: shell_file,
            args: shell_args,
            dir: Some(cwd.clone()),
            timeout_ms: Some(timeout_sec as u64 * 1000),
            // 显式给上限（D29）：与工具出口同一个常量，内层不再自行追加标记文本，
            // 截断事实经 `output_truncated` 上报，标记文案只在工具层写一份。
            output_limit: Some(MAX_OUTPUT_BYTES),
            // 用户选中的 shell 环境（D18）：PATH 里的 node/cargo/pnpm 等由它提供
            env: {
                refresh_shell_env();
                craft_core::shellenv::current_vars()
            },
            new_session: false,
        });
        let duration = started.elapsed();
        // 超时处理分两种（判据见 is_sleep_command 与「前台超时转后台」的取舍）：
        // - 首词是 sleep：拿 sleep 等待外部任务的典型形态，转后台只会把空转从可见变不可见，
        //   直接按超时失败（原本的语义），让模型改用 run_in_background 或 bash_output；
        //   sleep 前置拒绝（D21）后这里对首词 sleep 已不可达，保留作两处谓词漂移的兜底；
        // - 其余命令：不杀进程，转后台继续跑。`pnpm build` / `cargo test` / `gh run watch`
        //   这类长命令本来就会超预算，杀掉等于白跑，转后台才能拿回结果。
        if result.timed_out {
            if is_sleep_command(&command) {
                return ToolResult {
                    output: format!(
                        "命令超时（{}s）：{}。\n\
                         用 sleep 等待外部任务不会让任务变快，只会白占时间。\
                         改用 shell_exec 的 run_in_background=true（命令本身就等到出结果），\
                         或用 bash_output 查看已有后台任务的进度。",
                        duration.as_secs(),
                        result.output
                    ),
                    is_error: true,
                    code: "tool_error".into(),
                    meta: Some(json!({ "exitCode": -1, "timedOut": true })),
                    media: Vec::new(),
                    truncation: None,
                };
            }
            return match start_background(ctx, &command, &cwd, true) {
                Ok(task) => ToolResult {
                    output: background_ack(&task, 0),
                    is_error: false,
                    code: String::new(),
                    meta: Some(json!({
                        "backgroundTaskId": task.id,
                        "outputPath": task.output_path,
                        "pid": task.pid,
                        "fromTimeout": true,
                        "partialOutput": result.output,
                    })),
                    media: Vec::new(),
                    truncation: None,
                },
                Err(err) => err_result(format!(
                    "命令超时（{}s），且转入后台失败：{err}。已捕获的部分输出：{}",
                    duration.as_secs(),
                    result.output
                )),
            };
        }
        // run 的约定：exit_code < 0 仅表示进程未能启动（spawn 失败），error 字段才有值
        let spawn_error = (result.exit_code < 0).then(|| result.error.clone()).flatten();
        if let Some(error) = spawn_error {
            return err_result(format!("执行命令失败：{error}"));
        }
        let output = result.output.trim_end_matches(['\n']).to_string();
        let output = if output.is_empty() { "(无输出)".to_string() } else { output };
        // 截断事实由本工具申报（D29）：`platform::run` 只报事实与真实丢弃量，不追加
        // 标记文本；标记文案在这里唯一一份。
        let truncation = result.output_truncated.then(|| crate::registry::TruncationInfo {
            dropped_bytes: result.dropped_bytes,
            note: format!("输出超过 {}KB 上限已截断", MAX_OUTPUT_BYTES / 1024),
        });
        let mut body = format!("$ {command}\n{output}\n[退出码 {} · {}s]", result.exit_code, duration.as_secs());
        if result.output_truncated {
            body.push_str(&format!(
                "\n…（输出超过 {}KB 上限，已丢弃 {} 字节；需要后续内容请把输出重定向到文件后分页读取）",
                MAX_OUTPUT_BYTES / 1024,
                result.dropped_bytes
            ));
        }
        ToolResult {
            output: body,
            is_error: result.exit_code != 0,
            code: if result.exit_code != 0 { "tool_error".into() } else { String::new() },
            meta: Some(json!({ "exitCode": result.exit_code, "truncated": result.output_truncated })),
            media: Vec::new(),
            truncation,
        }
    }
}

impl ShellExec {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "shell_exec".into(),
        description: "在工作区内执行 shell 命令：用户选定了 shell 时用该 shell 解释，未选择时用平台默认（Unix /bin/sh -c、Windows cmd /C）。输出上限 32KB，前台超时上限 600 秒（默认 120）。命令在清洗后的环境中运行（Python 保证 UTF-8 输出）。等待外部任务（CI、gh run、构建、长测试）用 run_in_background=true，不要用 sleep 轮询，不要用 wait / agent_wait 等待（wait 只等时间，agent_wait 只等子代理）；前台命令超时后除 sleep 外会自动转入后台继续运行。跨多文件的批量修改或大文件处理，建议写 Python 脚本执行；处理不可信文件时给 python 加 -I。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "要执行的命令" },
                "cwd": { "type": "string", "description": "相对工作区的运行目录，默认工作区根" },
                "timeout": { "type": "integer", "minimum": 1, "maximum": 600, "description": "前台等待预算（秒），默认 120；转入后台后该预算失效" },
                "run_in_background": {
                    "type": "boolean",
                    "description": "true=后台运行，立即返回任务号与输出文件路径，结束时自动通知；省略或 false=前台等待结果",
                },
            },
            "required": ["command"],
        }),
        risk: PolicyRisk::R2,
        path_arg: Some("cwd".into()),
        command_arg: Some("command".into()),
        mutates: true,
        work: false,
        title: "执行命令".into(),
        summary_arg: Some("command".into()),
        summary_text: None,
        }
    }
}

// ---------- bash_output / kill_shell（后台命令任务） ----------

/// `bash_output` 的 `tail_bytes` 参数上限（字节）：读取量与工具出口预算联动，
/// 两处必须用同一个常量，否则「要 64KB」会被出口按 32KB 砍掉（D29）。
pub const BASH_OUTPUT_MAX_TAIL_BYTES: usize = 65536;

/// 读取后台命令任务的输出尾部（R0 只读）。
///
/// 只按会话归属查找任务：跨会话读别人的输出是串号，一律拒绝。默认读尾部 8 KiB——
/// 看进度关心的永远是最新几行；要更多用 `offset` 分页（从文件头部起算的字节数）。
pub struct BashOutput;

impl Tool for BashOutput {
    fn spec(&self) -> ToolSpec {
        BashOutput::spec()
    }

    /// 它的 `tail_bytes` 参数上限是 64 KiB（见 `execute` 的 clamp），出口预算必须能容纳
    /// 请求量本身，否则「要 64 KB」永远只能拿到 32 KB（D29）。留 512 字节给头部信息行。
    fn output_ceiling(&self) -> usize {
        BASH_OUTPUT_MAX_TAIL_BYTES + 512
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let task_id = ctx.str_arg("task_id");
        if task_id.trim().is_empty() {
            return err_result("task_id 不能为空（用后台任务的 bash_xxx 编号）");
        }
        let Some(task) = ctx.env.shell_tasks.get(&ctx.thread_scope_id(), &task_id) else {
            return err_result(format!("找不到后台任务 {task_id}（可能已清理，或不属于当前会话）"));
        };
        let max_bytes = (ctx.int_arg("tail_bytes", 8192).clamp(1024, BASH_OUTPUT_MAX_TAIL_BYTES as i64)) as usize;
        let (tail, total) = match read_output_tail(&task.output_path, max_bytes) {
            Ok(pair) => pair,
            Err(err) => return err_result(err),
        };
        let header = format!(
            "任务 {}｜{}｜已运行 {}s{}｜输出 {} 字节｜文件 {}",
            task.id,
            task.state.as_str(),
            task.duration_ms() / 1000,
            task.exit_code.map(|c| format!("｜退出码 {c}")).unwrap_or_default(),
            total,
            task.output_path
        );
        let body = if tail.trim().is_empty() {
            "（还没有输出）".to_string()
        } else if total > tail.len() {
            format!("…（仅显示最后 {max_bytes} 字节）\n{tail}")
        } else {
            tail
        };
        ToolResult::ok_with_meta(
            format!("{header}\n{body}"),
            json!({
                // backgroundTaskId 与 shell_exec 启动回执同名：渲染层据此把同一任务的
                // 多次查看合并成一条等待行（`crates/craft-agent/src/shell_tasks.rs` 模块头）。
                "backgroundTaskId": task.id,
                "taskId": task.id,
                "state": task.state.as_str(),
                "exitCode": task.exit_code,
                "totalBytes": total,
                "outputPath": task.output_path,
            }),
        )
    }
}

impl BashOutput {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: "bash_output".into(),
            description: "查看后台命令任务当前的输出尾部（默认最后 8KB）。只在确实需要看具体输出时调用；任务结束会自动通知，不要反复轮询等待完成。".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "后台任务编号（bash_ 开头，shell_exec 返回）" },
                    "tail_bytes": { "type": "integer", "minimum": 1024, "maximum": 65536, "description": "读取尾部字节数，默认 8192" },
                },
                "required": ["task_id"],
            }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: false,
            title: "查看命令输出".into(),
            summary_arg: Some("task_id".into()),
            summary_text: None,
        }
    }
}

/// 终止后台命令任务（R2）：整组杀，孙进程一并回收。
pub struct KillShell;

impl Tool for KillShell {
    fn spec(&self) -> ToolSpec {
        KillShell::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let task_id = ctx.str_arg("task_id");
        if task_id.trim().is_empty() {
            return err_result("task_id 不能为空（用后台任务的 bash_xxx 编号）");
        }
        let scope = ctx.thread_scope_id();
        let Some(task) = ctx.env.shell_tasks.get(&scope, &task_id) else {
            return err_result(format!("找不到后台任务 {task_id}（可能已清理，或不属于当前会话）"));
        };
        if task.state.is_terminal() {
            return ToolResult::ok(format!(
                "任务 {} 已结束（{}），无需终止。输出仍在：{}",
                task.id,
                task.state.as_str(),
                task.output_path
            ));
        }
        // 先结算为 Killed 再杀：与等待线程竞争时保证「显式终止」的终态不被覆盖——
        // Windows 上 taskkill 返回时目标进程正在终止，等待线程可能先一步 settle 成
        // Completed（CI 实测踩中：用户点了终止却收到「已完成」通知）。先占住 Killed
        // 终态，等待线程后到时 settle 幂等挡住，不再重复通知。
        ctx.env.shell_tasks.settle(&task.id, TaskState::Killed, None);
        kill_process_group(task.pid);
        ToolResult::ok_with_meta(
            format!("已终止任务 {}（整组杀，含其子进程）。输出文件保留：{}", task.id, task.output_path),
            json!({ "taskId": task.id, "state": TaskState::Killed.as_str() }),
        )
    }
}

impl KillShell {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: "kill_shell".into(),
            description: "终止一个仍在运行的后台命令任务（整组杀，含子进程）。".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "后台任务编号（bash_ 开头）" },
                },
                "required": ["task_id"],
            }),
            risk: PolicyRisk::R2,
            path_arg: None,
            command_arg: None,
            mutates: true,
            work: false,
            title: "终止后台命令".into(),
            summary_arg: Some("task_id".into()),
            summary_text: None,
        }
    }
}

// ---------- git_* ----------

/// 查看主要目录 Git 状态（R0 只读）。
///
/// 状态采集走 craft-git（gix 进程内渲染，输出与 `git status --short --branch` 逐字节
/// 同构，见 `crates/craft-git/src/status.rs`）。超时沿用迁移前 20s
/// （with_timeout 的协作式中断等价物）。
pub struct GitStatus;

impl Tool for GitStatus {
    fn spec(&self) -> ToolSpec {
        GitStatus::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let workspace = ctx.workspace.clone();
        let result = craft_git::with_timeout(std::time::Duration::from_secs(20), "git status", move |flag| {
            let repo = craft_git::Repository::discover(&workspace)?;
            craft_git::status::status_text(&repo, true, flag)
        });
        let out = match result {
            Ok(text) => text.trim().to_string(),
            Err(err) => return err_result(format!("git status 失败：{err}")),
        };
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
        mutates: false,
        work: false,
        title: "查看 Git 状态".into(),
        summary_arg: None,
        summary_text: Some("git status".into()),
        }
    }
}

/// 查看 Git 差异，默认未暂存、staged=true 查看已暂存（R0 只读）。
///
/// diff 渲染走 craft-git（gix 进程内执行，输出与 `git diff --no-color [--staged]
/// [-- path]` 逐字节同构，见 `crates/craft-git/src/diff.rs`）；参数 schema、
/// 「git diff 失败：」错误前缀与「没有差异」空差异语义保持不变。
/// 超时沿用迁移前 30s（with_timeout 的协作式中断等价物）。
///
/// 空输出分两种成因分别报文案：仓库确实没有改动（「没有差异」）/ pathspec 把
/// 变更全滤掉（明确说「有 N 个文件改动但 path 没命中」，附 changedFiles /
/// shownFiles 元数据）。路径分隔符由 craft-git 归一，这里直接传展示路径。
pub struct GitDiff;

impl Tool for GitDiff {
    fn spec(&self) -> ToolSpec {
        GitDiff::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let staged = ctx.args.get("staged").and_then(Value::as_bool).unwrap_or(false);
        let path = if ctx.display.is_empty() { None } else { Some(ctx.display.clone()) };
        let workspace = ctx.workspace.clone();
        let result = craft_git::with_timeout(std::time::Duration::from_secs(30), "git diff", move |flag| {
            let repo = craft_git::Repository::discover(&workspace)?;
            craft_git::diff::diff_text(
                &repo,
                craft_git::diff::DiffOptions {
                    staged,
                    path: path.map(|p| p.into_bytes()),
                },
                flag,
            )
        });
        let diff = match result {
            Ok(diff) => diff,
            Err(err) => return err_result(format!("git diff 失败：{err}")),
        };
        let out = diff.text.trim().to_string();
        let meta = json!({
            "staged": staged,
            "changedFiles": diff.changed_files,
            "shownFiles": diff.shown_files,
        });
        if !out.is_empty() {
            return ToolResult::ok_with_meta(out, meta);
        }
        // 空输出的两种成因必须分开报：仓库真没改动 vs pathspec 把变更全滤掉了
        // （后者多半是路径参数写错——静默说「没有差异」会让这类问题无从排查）。
        if diff.changed_files > 0 && diff.shown_files == 0 {
            return ToolResult::ok_with_meta(
                format!(
                    "没有匹配路径参数的差异：仓库中有 {} 个文件改动，但 path={} 没有命中任何一个。\
                     检查路径是否写错（应为相对主要目录的路径，或省略 path 查看全部）。",
                    diff.changed_files, ctx.display
                ),
                meta,
            );
        }
        ToolResult::ok_with_meta("没有差异", meta)
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
        mutates: false,
        work: false,
        title: "查看 Git 差异".into(),
        summary_arg: Some("path".into()),
        summary_text: Some("git diff".into()),
        }
    }
}

/// 查看提交记录，oneline + 修饰（R0 只读）。
///
/// 历史遍历与渲染走 craft-git（输出与 `git log --oneline --decorate -n N [-- path]`
/// 逐字节同构，见 `crates/craft-git/src/log.rs`）；unborn HEAD 的报错文案与迁移前
/// 系统 git 的 fatal 输出逐字一致（`Error::UnbornHead`）。超时沿用迁移前 30s。
pub struct GitLog;

impl Tool for GitLog {
    fn spec(&self) -> ToolSpec {
        GitLog::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let limit = ctx.int_arg("limit", 20).clamp(1, 100);
        let path = if ctx.display.is_empty() { None } else { Some(ctx.display.clone()) };
        let workspace = ctx.workspace.clone();
        let result = craft_git::with_timeout(std::time::Duration::from_secs(30), "git log", move |flag| {
            let repo = craft_git::Repository::discover(&workspace)?;
            craft_git::log::log_text(
                &repo,
                craft_git::log::LogOptions {
                    limit: limit as usize,
                    path: path.map(|p| p.into_bytes()),
                    decorate: true,
                },
                flag,
            )
        });
        let out = match result {
            Ok(text) => text.trim().to_string(),
            Err(err) => return err_result(format!("git log 失败：{err}")),
        };
        ToolResult::ok_with_meta(
            if out.is_empty() { "还没有提交记录".to_string() } else { out },
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
        mutates: false,
        work: false,
        title: "查看提交记录".into(),
        summary_arg: Some("path".into()),
        summary_text: Some("git log".into()),
        }
    }
}

/// 提交代码（R1 写工作区）；默认不推送——推送永远由用户手动执行。
///
/// 提交管线走 craft-git（gix 进程内执行，两模式语义与身份级联见
/// `crates/craft-git/src/commit.rs`）：指定 paths = 路径限定提交（可含未跟踪
/// 文件）；不指定 = `-am`（仅已跟踪文件修改 / 删除，新文件不含）。短 hash 由
/// 管线直接返回（`rev-parse --short` 等价）。错误文案保留迁移前格式：
/// message 校验在工具层；`提交失败: ` 前缀 + craft-git 的归一化错误（无变更 /
/// 身份缺失 / pathspec 不存在分别对齐 git 的对应输出）。超时沿用迁移前 30s。
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
        // \0 是 exec 参数的硬限制；2000 字节防异常长信息
        if message.contains('\0') || message.len() > 2000 {
            return err_result("提交信息无效（为空、含空字符或超过 2000 字节）");
        }
        let paths_arg: Vec<String> = ctx
            .args
            .get("paths")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        // 指定路径经 resolve_extra 解析：仅允许主要目录（与 git_ 工具的目录约束一致）
        let mut paths: Vec<String> = Vec::new();
        for p in &paths_arg {
            match ctx.env.resolve_extra(&ctx.roots, p) {
                Ok((_, display)) => paths.push(display),
                Err(err) => return err_result(err.to_string()),
            }
        }
        let workspace = ctx.workspace.clone();
        let commit_paths: Vec<Vec<u8>> = paths.iter().map(|p| p.as_bytes().to_vec()).collect();
        let result = craft_git::with_timeout(std::time::Duration::from_secs(30), "git commit", move |flag| {
            let repo = craft_git::Repository::discover(&workspace)?;
            craft_git::commit::commit(
                &repo,
                craft_git::commit::CommitOptions {
                    message,
                    paths: commit_paths,
                    should_interrupt: std::sync::Arc::clone(flag),
                },
            )
        });
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(err) => return err_result(format!("提交失败: {err}")),
        };
        let first_line = ctx.str_arg("message").trim().split('\n').next().unwrap_or_default().to_string();
        // 路径限定提交只提交指定路径，其余变更留在工作区且索引不动（git 语义）。
        // 这层语义必须如实报出：只给 hash 会让调用方以为「整个工作区都提交了」。
        let mut body = format!(
            "已提交 {}：{first_line}\n（未推送；本次 {} 个文件",
            outcome.short_hash, outcome.committed_files
        );
        if outcome.leftover_files > 0 {
            body.push_str(&format!(
                "，另有 {} 个变更未包含——paths 只提交列出的路径，其余仍在工作区；\
                 要全部提交请省略 paths",
                outcome.leftover_files
            ));
        }
        body.push('）');
        ToolResult::ok_with_meta(
            body,
            json!({
                "hash": outcome.short_hash,
                "message": ctx.str_arg("message").trim(),
                "paths": paths,
                "committedFiles": outcome.committed_files,
                "leftoverFiles": outcome.leftover_files,
            }),
        )
    }
}

impl GitCommit {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "git_commit".into(),
        description: "提交代码（默认不推送）。有 paths 时**只提交这些路径**（等价 git add -- <paths> + git commit -- <paths>）：\
其余变更不进本次提交、索引也不动，结果里会报出剩余数量；省略 paths 时提交全部已跟踪改动（-am 语义，不含未跟踪文件）。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "提交信息" },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "只提交这些文件路径（相对工作区）；省略则提交全部已跟踪改动" },
            },
            "required": ["message"],
        }),
        risk: PolicyRisk::R1,
        path_arg: None,
        command_arg: None,
        mutates: true,
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
        let env = Arc::new(ToolEnv { backup_dir: dir.path().join("backups").to_string_lossy().to_string(), data_dir: String::new(), db, shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() });
        let ctx = ToolCtx {
            name: "test".into(),
            args,
            target: String::new(),
            display: String::new(),
            workspace: roots.first().cloned().unwrap_or_default(),
            roots,
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env,
        };
        (dir, ctx)
    }

    #[cfg(unix)]
    #[test]
    fn shell_exec_captures_output_and_exit_code() {
        let (_dir, ctx) = make_ctx(json!({"command": "echo hello; exit 3"}), vec!["/tmp".to_string()]);
        let result = ShellExec.execute(&ctx);
        assert!(result.is_error);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("[退出码 3"));
    }

    // 空命令拒绝是平台无关的策略检查，单一版本即可
    #[test]
    fn shell_exec_rejects_empty_command() {
        let (_dir, ctx) = make_ctx(json!({"command": "  "}), vec!["/tmp".to_string()]);
        assert!(ShellExec.execute(&ctx).is_error);
    }

    // shell_exec_timeout_kills_process 已随 D21 删除：sleep 命令前置拒绝后，前台超时只剩
    // 「非 sleep 命令转后台」一条路（foreground_timeout_hands_command_to_background…覆盖），
    // 「超时硬失败」的场景在 Unix 也不可达了（原依赖 sleep 例外）。Windows 注释同此。

    // ---------- Windows 等价测试（无真机，CI 的 windows runner 是唯一验证场） ----------
    // 命令等价：`echo x; exit n` → `echo x & exit /b n`；`sleep N` → `ping -n N+1
    // 127.0.0.1 > nul`（cmd 的 `timeout /t` 在无 stdin 环境报「输入重定向不受支持」，
    // 不可用）。命令串由工具层写死 cmd /C 执行，测试只需给出 cmd 语义的命令文本。

    #[cfg(windows)]
    #[test]
    fn shell_exec_captures_output_and_exit_code_windows() {
        let (_dir, ctx) = make_ctx(json!({"command": "echo hello & exit /b 3"}), vec!["C:\\".to_string()]);
        let result = ShellExec.execute(&ctx);
        assert!(result.is_error);
        assert!(result.output.contains("hello"));
        assert!(result.output.contains("[退出码 3"), "{}", result.output);
    }

    // shell_exec_timeout_kills_process 没有 Windows 版本，两个原因：
    // 1. cmd 没有 sleep 命令，`cmd /C sleep 30` 立即报错退出，「首词 sleep 挂住
    //    进程到超时」这个 sleep 例外分支的触发前提在 Windows 不存在；
    // 2. Windows 的等待型命令（ping 等）超时后走「转后台」分支，返回 is_error=false
    //    ——这是设计行为（首词非 sleep 一律转后台），已由
    //    foreground_timeout_hands_command_to_background_instead_of_killing_it_windows 覆盖。
    //    「超时硬失败」的场景在 Windows 前台路径不可达。

    // ---------- 执行器跟随 shell 选择（D25） ----------

    /// shell 环境缓存是进程级全局量：这三个用例会 `configure` / `clear` 它，
    /// 与同二进制里的其它命令用例并行时会互相干扰，故串行。poison 不致命。
    ///
    /// 必须与三个调用点同带 `#[cfg(windows)]`：它们都是 Windows 专属用例，不这样写
    /// 会在非 Windows 上变成死代码，而 `cargo clippy --all-targets -- -D warnings`
    /// 会把 `dead_code` 当错误（CI 三平台因此红了三个提交）。
    #[cfg(windows)]
    static EXECUTOR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(windows)]
    fn executor_test_serial() -> std::sync::MutexGuard<'static, ()> {
        EXECUTOR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 未选择 shell 时执行器必须是平台默认：Windows 上是 cmd。这条用例同时是
    /// 「未选择不改行为」的回归守卫——本改动不得影响既有默认语义。
    #[cfg(windows)]
    #[test]
    fn shell_exec_uses_cmd_when_shell_not_configured() {
        let _serial = executor_test_serial();
        craft_core::shellenv::clear();
        let (_dir, ctx) = make_ctx(json!({"command": "echo %COMSPEC%"}), vec!["C:\\".to_string()]);
        let result = ShellExec.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        // cmd 会展开 %COMSPEC%；PowerShell 不认这种写法
        assert!(
            result.output.to_lowercase().contains("cmd.exe"),
            "未配置时应由 cmd 解释：{}",
            result.output
        );
    }

    /// 选中 PowerShell 后，同一命令由 PS 解释：`$PSVersionTable` 可用。
    /// 这是本次改动的核心验收点（此前设置里选 pwsh 只改环境，命令仍被 cmd 解释）。
    #[cfg(windows)]
    #[test]
    fn shell_exec_runs_through_configured_powershell() {
        let _serial = executor_test_serial();
        let Some(shell) = craft_core::shellenv::detect_shells()
            .into_iter()
            .find(|s| s.kind == craft_core::shellenv::ShellKind::Powershell)
        else {
            eprintln!("无 PowerShell，跳过：不能因此判失败");
            return;
        };
        let home = craft_core::platform::home_dir_string();
        if craft_core::shellenv::configure(&shell.path, &home).is_err() {
            eprintln!("PowerShell 环境捕获失败，跳过");
            return;
        }

        let (_dir, ctx) =
            make_ctx(json!({"command": "Write-Output $PSVersionTable.PSEdition"}), vec!["C:\\".to_string()]);
        let result = ShellExec.execute(&ctx);
        craft_core::shellenv::clear();

        assert!(!result.is_error, "{}", result.output);
        // cmd 会把整串原样回显（含 $ 与 .），PowerShell 才给出 Core / Desktop
        assert!(
            result.output.contains("Core") || result.output.contains("Desktop"),
            "应由 PowerShell 解释：{}",
            result.output
        );
        // 输出首行是 `$ <命令>` 回显，命令文本必然出现，不能用它判断执行器；
        // cmd 解释 PowerShell 语法时报的是「不是内部或外部命令」，那才是真正的失败信号。
        assert!(
            !result.output.contains("不是内部或外部命令") && !result.output.contains("is not recognized"),
            "不应是 cmd 的报错：{}",
            result.output
        );
    }

    /// 退出码在 PowerShell 下也要正确传播：`-Command` 不传播原生命令退出码，
    /// 靠脚本尾的 `$LASTEXITCODE` 补——没有它 `cmd /c exit 7` 会上报 0。
    #[cfg(windows)]
    #[test]
    fn shell_exec_propagates_exit_code_through_powershell() {
        let _serial = executor_test_serial();
        let Some(shell) = craft_core::shellenv::detect_shells()
            .into_iter()
            .find(|s| s.kind == craft_core::shellenv::ShellKind::Powershell)
        else {
            eprintln!("无 PowerShell，跳过");
            return;
        };
        let home = craft_core::platform::home_dir_string();
        if craft_core::shellenv::configure(&shell.path, &home).is_err() {
            eprintln!("PowerShell 环境捕获失败，跳过");
            return;
        }

        let (_dir, ctx) = make_ctx(json!({"command": "cmd /c exit 7"}), vec!["C:\\".to_string()]);
        let result = ShellExec.execute(&ctx);
        craft_core::shellenv::clear();

        assert!(result.is_error, "非零退出码应标记为错误：{}", result.output);
        assert!(result.output.contains("[退出码 7"), "退出码应原样传播：{}", result.output);
    }

    // ---------- 后台命令任务（`crates/craft-agent/src/shell_tasks.rs` 模块头） ----------

    /// 带可写数据目录的 ctx：后台任务要落输出文件，data_dir 不能为空。
    /// Unix 与 Windows 的后台任务测试共用（命令文本各自给 cmd/sh 语义）。
    fn make_bg_ctx(args: Value, data_dir: &std::path::Path) -> (tempdir::TempDir, ToolCtx) {
        let dir = tempdir::TempDir::new().unwrap();
        let db = Arc::new(craft_store::db::Db::open(&dir.path().join("test.db")).unwrap());
        let env = Arc::new(ToolEnv {
            backup_dir: dir.path().join("backups").to_string_lossy().to_string(),
            data_dir: data_dir.to_string_lossy().to_string(),
            db,
            shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()),
            proxy_config: crate::registry::direct_proxy_config(),
        });
        let ctx = ToolCtx {
            name: "shell_exec".into(),
            args,
            target: String::new(),
            display: String::new(),
            workspace: "/tmp".into(),
            roots: vec!["/tmp".into()],
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: "th_test".into(),
            env,
        };
        (dir, ctx)
    }

    #[cfg(unix)]
    #[test]
    fn background_shell_returns_immediately_with_task_id_and_output_path() {
        // 首词不能是 sleep：D21 起 sleep 命令前置拒绝（is_sleep_command 只看首词）
        let (_dir, ctx) = make_bg_ctx(json!({"command": "echo start >/dev/null; sleep 30", "run_in_background": true}), std::path::Path::new("/tmp"));
        let started = std::time::Instant::now();
        let result = ShellExec.execute(&ctx);
        // 立即返回：命令本身要跑 30 秒，这里必须远小于它
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "后台启动不应阻塞");
        assert!(!result.is_error, "启动成功不是错误：{}", result.output);
        let meta = result.meta.unwrap();
        let task_id = meta["backgroundTaskId"].as_str().expect("应返回任务号").to_string();
        assert!(task_id.starts_with("bash_"), "任务号前缀：{task_id}");
        let output_path = meta["outputPath"].as_str().unwrap().to_string();
        assert!(std::path::Path::new(&output_path).exists(), "启动前就要建好输出文件");
        assert!(result.output.contains("后台"), "回执应说明在后台跑：{}", result.output);
        assert!(
            result.output.contains("只等子代理"),
            "回执应写明 wait / agent_wait 与后台任务的边界：{}",
            result.output
        );

        // 任务登记在注册表里且归属本会话
        let task = ctx.env.shell_tasks.get("th_test", &task_id).expect("任务应已登记");
        assert_eq!(task.state, crate::shell_tasks::TaskState::Running);
        // 跨会话取不到
        assert!(ctx.env.shell_tasks.get("th_other", &task_id).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn bash_output_reads_tail_of_running_task() {
        let (_dir, ctx) = make_bg_ctx(json!({"command": "echo hello-from-bg; sleep 30", "run_in_background": true}), std::path::Path::new("/tmp"));
        let start = ShellExec.execute(&ctx);
        let task_id = start.meta.unwrap()["backgroundTaskId"].as_str().unwrap().to_string();
        // 等命令写出首行（后台线程与子进程都要醒来）
        let mut tail = String::new();
        for _ in 0..40 {
            let out_ctx = ToolCtx { name: "bash_output".into(), args: json!({ "task_id": task_id }), ..ctx.clone() };
            let result = BashOutput.execute(&out_ctx);
            assert!(!result.is_error, "读取失败：{}", result.output);
            // 事件载荷据此按任务号合并等待行；state 驱动运行中/终态的扫光切换
            let meta = result.meta.as_ref().expect("后台查看应带 meta");
            assert_eq!(meta["backgroundTaskId"].as_str(), Some(task_id.as_str()));
            assert_eq!(meta["state"].as_str(), Some("running"));
            tail = result.output;
            if tail.contains("hello-from-bg") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(tail.contains("hello-from-bg"), "应读到后台输出：{tail}");
        assert!(tail.contains("running"), "头部应报告任务状态：{tail}");
    }

    #[cfg(unix)]
    #[test]
    fn kill_shell_terminates_task_group_and_settles_as_killed() {
        // 首词不能是 sleep：D21 起 sleep 命令前置拒绝（is_sleep_command 只看首词）
        let (_dir, ctx) = make_bg_ctx(json!({"command": "echo start >/dev/null; sleep 60", "run_in_background": true}), std::path::Path::new("/tmp"));
        let start = ShellExec.execute(&ctx);
        let task_id = start.meta.unwrap()["backgroundTaskId"].as_str().unwrap().to_string();

        let kill_ctx = ToolCtx { name: "kill_shell".into(), args: json!({ "task_id": task_id }), ..ctx.clone() };
        let killed = KillShell.execute(&kill_ctx);
        assert!(!killed.is_error, "终止失败：{}", killed.output);
        assert!(killed.output.contains("已终止"), "{}", killed.output);

        // 结算为 Killed，且不因等待线程的结算而被改写
        std::thread::sleep(std::time::Duration::from_millis(300));
        let task = ctx.env.shell_tasks.get("th_test", &task_id).unwrap();
        assert_eq!(task.state, crate::shell_tasks::TaskState::Killed);
    }

    #[cfg(unix)]
    #[test]
    fn foreground_timeout_hands_command_to_background_instead_of_killing_it() {
        // 非 sleep 首词的长命令：超时不杀，转后台继续跑（pnpm build / cargo test 的典型场景）。
        // 首词刻意的不是 sleep——首词判定见 is_sleep_command 的已知边界。
        let (_dir, ctx) = make_bg_ctx(json!({"command": "echo start; sleep 5; echo finished", "timeout": 1}), std::path::Path::new("/tmp"));
        let result = ShellExec.execute(&ctx);
        assert!(!result.is_error, "转后台不是错误：{}", result.output);
        let meta = result.meta.unwrap();
        assert_eq!(meta["fromTimeout"], true, "meta 应标注转后台原因");
        assert!(meta["backgroundTaskId"].as_str().unwrap().starts_with("bash_"));
        assert!(result.output.contains("转入后台"), "应说明超时转后台：{}", result.output);
    }

    #[cfg(unix)]
    #[test]
    fn sleep_rejected_upfront_points_at_wait_tool() {
        // D21：sleep 命令前置拒绝——预算内的短 sleep 也不再执行，把模型引向 wait / 后台执行
        let (_dir, ctx) = make_bg_ctx(json!({"command": "sleep 30"}), std::path::Path::new("/tmp"));
        let result = ShellExec.execute(&ctx);
        assert!(result.is_error, "{}", result.output);
        assert!(result.output.contains("wait 工具"), "应引导用 wait 工具：{}", result.output);
        assert!(result.output.contains("run_in_background"), "应引导改用后台执行：{}", result.output);
        // 首词判定边界：`sh -c 'sleep 5'` 不在拒绝范围（与既有 is_sleep_command 边界一致）
        let (_dir, ctx) = make_bg_ctx(json!({"command": "sh -c 'echo hi; sleep 5'"}), std::path::Path::new("/tmp"));
        let result = ShellExec.execute(&ctx);
        assert!(!result.is_error, "非 sleep 首词不受前置拒绝：{}", result.output);
    }

    #[cfg(windows)]
    #[test]
    fn background_shell_returns_immediately_with_task_id_and_output_path_windows() {
        let (_dir, ctx) = make_bg_ctx(json!({"command": "ping -n 31 127.0.0.1 > nul", "run_in_background": true}), std::path::Path::new("C:\\"));
        let started = std::time::Instant::now();
        let result = ShellExec.execute(&ctx);
        // 立即返回：命令本身要跑 30 秒，这里必须远小于它
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "后台启动不应阻塞");
        assert!(!result.is_error, "启动成功不是错误：{}", result.output);
        let meta = result.meta.unwrap();
        let task_id = meta["backgroundTaskId"].as_str().expect("应返回任务号").to_string();
        assert!(task_id.starts_with("bash_"), "任务号前缀：{task_id}");
        let output_path = meta["outputPath"].as_str().unwrap().to_string();
        assert!(std::path::Path::new(&output_path).exists(), "启动前就要建好输出文件");
        assert!(result.output.contains("后台"), "回执应说明在后台跑：{}", result.output);

        let task = ctx.env.shell_tasks.get("th_test", &task_id).expect("任务应已登记");
        assert_eq!(task.state, crate::shell_tasks::TaskState::Running);
        assert!(ctx.env.shell_tasks.get("th_other", &task_id).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn bash_output_reads_tail_of_running_task_windows() {
        let (_dir, ctx) = make_bg_ctx(
            json!({"command": "echo hello-from-bg & ping -n 31 127.0.0.1 > nul", "run_in_background": true}),
            std::path::Path::new("C:\\"),
        );
        let start = ShellExec.execute(&ctx);
        let task_id = start.meta.unwrap()["backgroundTaskId"].as_str().unwrap().to_string();
        // 等命令写出首行（后台线程与子进程都要醒来）
        let mut tail = String::new();
        for _ in 0..40 {
            let out_ctx = ToolCtx { name: "bash_output".into(), args: json!({ "task_id": task_id }), ..ctx.clone() };
            let result = BashOutput.execute(&out_ctx);
            assert!(!result.is_error, "读取失败：{}", result.output);
            // 事件载荷据此按任务号合并等待行；state 驱动运行中/终态的扫光切换
            let meta = result.meta.as_ref().expect("后台查看应带 meta");
            assert_eq!(meta["backgroundTaskId"].as_str(), Some(task_id.as_str()));
            assert_eq!(meta["state"].as_str(), Some("running"));
            tail = result.output;
            if tail.contains("hello-from-bg") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(tail.contains("hello-from-bg"), "应读到后台输出：{tail}");
        assert!(tail.contains("running"), "头部应报告任务状态：{tail}");
    }

    #[cfg(windows)]
    #[test]
    fn kill_shell_terminates_task_group_and_settles_as_killed_windows() {
        let (_dir, ctx) = make_bg_ctx(json!({"command": "ping -n 61 127.0.0.1 > nul", "run_in_background": true}), std::path::Path::new("C:\\"));
        let start = ShellExec.execute(&ctx);
        let task_id = start.meta.unwrap()["backgroundTaskId"].as_str().unwrap().to_string();

        let kill_ctx = ToolCtx { name: "kill_shell".into(), args: json!({ "task_id": task_id }), ..ctx.clone() };
        let killed = KillShell.execute(&kill_ctx);
        assert!(!killed.is_error, "终止失败：{}", killed.output);
        assert!(killed.output.contains("已终止"), "{}", killed.output);

        // 结算为 Killed，且不因等待线程的结算而被改写
        std::thread::sleep(std::time::Duration::from_millis(300));
        let task = ctx.env.shell_tasks.get("th_test", &task_id).unwrap();
        assert_eq!(task.state, crate::shell_tasks::TaskState::Killed);
    }

    #[cfg(windows)]
    #[test]
    fn foreground_timeout_hands_command_to_background_instead_of_killing_it_windows() {
        // 非 sleep 首词的长命令：超时不杀，转后台继续跑。首词刻意的不是 sleep——
        // 首词判定见 is_sleep_command 的已知边界。
        let (_dir, ctx) = make_bg_ctx(
            json!({"command": "echo start & ping -n 6 127.0.0.1 > nul & echo finished", "timeout": 1}),
            std::path::Path::new("C:\\"),
        );
        let result = ShellExec.execute(&ctx);
        assert!(!result.is_error, "转后台不是错误：{}", result.output);
        let meta = result.meta.unwrap();
        assert_eq!(meta["fromTimeout"], true, "meta 应标注转后台原因");
        assert!(meta["backgroundTaskId"].as_str().unwrap().starts_with("bash_"));
        assert!(result.output.contains("转入后台"), "应说明超时转后台：{}", result.output);
    }

    // sleep_timeout_is_rejected_and_points_at_run_in_background 没有 Windows 版本：
    // cmd 没有 sleep 命令，`cmd /C sleep 30` 会立即报错退出，根本到不了超时——
    // 「首词 sleep 挂住进程」这个被策略保护的场景在 Windows 不存在；Windows 的
    // 等待型命令（ping 等）走超时转后台路径，已由上一条测试覆盖。

    /// `git_diff` 的展示路径是 `craft-policy::relative` 的产物（Windows 上是反斜杠），
    /// 工具层直接透传给 craft-git——必须能命中子目录文件。曾因分隔符不匹配，
    /// 所有子目录文件、以及无 path 的整仓 diff 都静默渲染成「没有差异」。
    #[test]
    fn git_diff_reports_subdirectory_changes_and_whole_repo_diff() {
        if !std::process::Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
            return;
        }
        let (dir, mut ctx) = make_ctx(json!({}), vec!["/tmp".to_string()]);
        // 独立仓库：一个根目录文件 + 一个子目录文件，都提交后各加一行。
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(repo.join("docs").join("a.md"), "line1\n").unwrap();
        std::fs::write(repo.join("root.md"), "line1\n").unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
            assert!(out.status.success(), "git {args:?} 失败：{}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init"]);
        run(&["add", "."]);
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "init"]);
        std::fs::write(repo.join("docs").join("a.md"), "line1\nline2\n").unwrap();
        std::fs::write(repo.join("root.md"), "line1\nline2\n").unwrap();

        let root = repo.to_string_lossy().to_string();
        ctx.workspace = root.clone();
        ctx.roots = vec![root];
        ctx.target = repo.join("docs").join("a.md").to_string_lossy().to_string();
        // 与 registry 一致：display 是相对主要目录的展示路径（Windows 上带反斜杠）。
        ctx.display = "docs\\a.md".to_string();
        let result = GitDiff.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("docs/a.md"), "子目录 diff 必须报出：{}", result.output);
        assert!(!result.output.contains("root.md"), "路径过滤不应带出其他文件：{}", result.output);

        // 无 path（registry 把空 path_arg 填成 "."）：整仓 diff 必须同时报两个文件。
        ctx.display = ".".to_string();
        let result = GitDiff.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("docs/a.md"), "整仓 diff 应含子目录文件：{}", result.output);
        assert!(result.output.contains("root.md"), "整仓 diff 应含根目录文件：{}", result.output);
        let meta = result.meta.unwrap();
        assert_eq!(meta["changedFiles"], 2);
        assert_eq!(meta["shownFiles"], 2);
    }

    /// 路径参数一个都不命中时，空输出要说明「有改动但路径没命中」，而不是
    /// 混同为「没有差异」——后者会让路径写错的问题无从察觉。
    #[test]
    fn git_diff_distinguishes_unmatched_path_from_no_changes() {
        if !std::process::Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
            return;
        }
        let (dir, mut ctx) = make_ctx(json!({}), vec!["/tmp".to_string()]);
        let repo = dir.path().join("repo2");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("a.txt"), "line1\n").unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
            assert!(out.status.success(), "git {args:?} 失败：{}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init"]);
        run(&["add", "."]);
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "init"]);
        std::fs::write(repo.join("a.txt"), "line1\nline2\n").unwrap();

        let root = repo.to_string_lossy().to_string();
        ctx.workspace = root.clone();
        ctx.roots = vec![root];

        ctx.display = "nope.txt".to_string();
        let result = GitDiff.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("没有匹配路径参数"), "应区分路径未命中：{}", result.output);
        let meta = result.meta.unwrap();
        assert_eq!(meta["changedFiles"], 1);
        assert_eq!(meta["shownFiles"], 0);

        // 干净仓库才是真正的「没有差异」。
        run(&["add", "."]);
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "second"]);
        ctx.display = "a.txt".to_string();
        assert_eq!(GitDiff.execute(&ctx).output, "没有差异");
    }

    /// 根目录带符号链接别名时，git paths 的展示路径必须是相对路径而非绝对路径。
    ///
    /// 背景：macOS 的临时目录在 `/var/folders`（符号链接 → `/private/var/folders`），
    /// `resolve_at` 返回解析后的真实路径；若展示路径拿词法根重算 `relative`，前缀
    /// 错位会回落成绝对路径，`git_commit` 的路径限定提交因此全部失配（报
    /// 「nothing to commit」）。Linux 上 `/tmp` 不是符号链接，该缺陷不暴露。
    #[cfg(unix)]
    #[test]
    fn git_paths_display_stays_relative_under_symlinked_root() {
        let real = tempdir::TempDir::new().unwrap();
        let alias = tempdir::TempDir::new().unwrap();
        let repo = real.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        let link = alias.path().join("repo-link");
        std::os::unix::fs::symlink(&repo, &link).unwrap();

        let root = link.to_string_lossy().to_string();
        let (_dir, ctx) = make_ctx(json!({}), vec![root]);
        let (abs, display) = ctx.env.resolve_extra(&ctx.roots, "a.txt").unwrap();
        assert_eq!(display, "a.txt", "符号链接别名根下展示路径必须是相对路径：(abs={abs}, display={display})");
    }

    /// `git_commit` 传 `paths` 时只提交这些路径，且结果必须如实报出遗留数量。
    ///
    /// 只给 hash 会让调用方以为「整个工作区都提交了」，实际上其余变更原封不动
    /// 留在工作区、索引也不动（git 的路径限定提交语义）。
    #[test]
    fn git_commit_with_paths_reports_leftover_in_output_and_meta() {
        if !std::process::Command::new("git").arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
            return;
        }
        let (dir, mut ctx) = make_ctx(json!({}), vec!["/tmp".to_string()]);
        let repo = dir.path().join("repo_commit");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
            assert!(out.status.success(), "git {args:?} 失败：{}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["init"]);
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "init", "--allow-empty"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();

        let root = repo.to_string_lossy().to_string();
        ctx.workspace = root.clone();
        ctx.roots = vec![root];
        ctx.args = json!({ "message": "only a", "paths": ["a.txt"] });
        let result = GitCommit.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("本次 1 个文件"), "应报出提交文件数：{}", result.output);
        assert!(result.output.contains("另有 1 个变更未包含"), "应报出遗留数量：{}", result.output);
        let meta = result.meta.unwrap();
        assert_eq!(meta["committedFiles"], 1);
        assert_eq!(meta["leftoverFiles"], 1);

        // b.txt 确实还在工作区未跟踪。
        let status = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&status.stdout).contains("b.txt"));

        // 无 paths：提交全部已跟踪改动。两个新文件先经一次 paths 提交进入跟踪状态，
        // 再各自改动——`-am` 语义不含未跟踪文件（与 git 一致）。
        run(&["add", "."]);
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "track"]);
        std::fs::write(repo.join("a.txt"), "a2\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b2\n").unwrap();
        ctx.args = json!({ "message": "rest" });
        let result = GitCommit.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("本次 2 个文件"), "应报出两个文件：{}", result.output);
        assert!(!result.output.contains("未包含"), "无 paths 不应出现遗留提示：{}", result.output);
    }

    #[test]
    fn git_commit_rejects_invalid_message_before_any_git_work() {
        // message 校验留在工具层（迁移前行为）：空 / 含 \0 / 超 2000 字节都拒绝。
        let (_dir, ctx) = make_ctx(json!({"message": "   "}), vec!["/tmp".to_string()]);
        assert!(GitCommit.execute(&ctx).is_error);

        let (_dir, ctx) = make_ctx(json!({"message": "a\u{0000}b"}), vec!["/tmp".to_string()]);
        let result = GitCommit.execute(&ctx);
        assert!(result.is_error);
        assert!(result.output.contains("提交信息无效"));

        let long = "好".repeat(1200); // 3600 字节 > 2000
        let (_dir, ctx) = make_ctx(json!({"message": long}), vec!["/tmp".to_string()]);
        assert!(GitCommit.execute(&ctx).is_error);
    }

    #[test]
    fn git_commit_rejects_paths_outside_main_root() {
        // paths 经 resolve_extra 仅允许主要目录：附加目录路径在评估期即拒绝。
        let (_dir, ctx) = make_ctx(
            json!({"message": "x", "paths": ["../outside.txt"]}),
            vec!["/tmp".to_string()],
        );
        let result = GitCommit.execute(&ctx);
        assert!(result.is_error);
        assert!(!result.output.contains("已提交"));
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
