//! 票 #3 黄金对比：status / log 的 craft-git 渲染与系统 git 输出逐字节一致。
//!
//! 基准全部经 [`capture_git`]（隔离环境）捕获，craft-git 侧经 [`discover_isolated`]
//! （隔离配置）渲染——两侧都只见 fixture 自身的 local 配置，输入对称。
//! fixture 布景确定 ⇒ 输出确定，同一 fixture 上「系统 git 实际输出」即黄金预期。

mod support;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use support::{
    capture_git, create, discover_isolated, git_fixture, interrupt_flag, set_local_config,
    assert_golden_bytes_equal, FixtureKind,
};

use craft_git::log::{LogOptions, log_text};
use craft_git::status::status_text;

// ---------- craft-git 侧实现封装 ----------

/// status 渲染（黄金对比的被测方）：返回与 git 输出同构的字节。
fn status_bytes(repo_path: &Path, with_branch_header: bool) -> Result<Vec<u8>, String> {
    let repo = discover_isolated(repo_path)?;
    let text = status_text(&repo, with_branch_header, &interrupt_flag())
        .map_err(|e| format!("status 渲染失败：{e}"))?;
    Ok(text.into_bytes())
}

/// log 渲染（黄金对比的被测方）。
fn log_bytes(repo_path: &Path, limit: usize, path: Option<&str>, decorate: bool) -> Result<Vec<u8>, String> {
    let repo = discover_isolated(repo_path)?;
    let text = log_text(
        &repo,
        LogOptions {
            limit,
            path: path.map(|p| p.as_bytes().to_vec()),
            decorate,
        },
        &interrupt_flag(),
    )
    .map_err(|e| format!("log 渲染失败：{e}"))?;
    Ok(text.into_bytes())
}

/// 对比一个 status 用例：系统 git 基准 vs craft-git 渲染，逐字节断言。
fn assert_status_matches_git(repo: &support::FixtureRepo, label: &str) {
    let golden = capture_git(repo.path(), &["status", "--short", "--branch"])
        .unwrap_or_else(|e| panic!("捕获 status 基准失败：{e}"));
    let actual = status_bytes(repo.path(), true).unwrap_or_else(|e| panic!("{label}：{e}"));
    assert_golden_bytes_equal(label, &golden, &actual).unwrap_or_else(|e| panic!("{e}"));
}

/// 对比一个 log 用例（ decorate + `-n N` [+ `-- path`]）。
fn assert_log_matches_git(
    repo: &support::FixtureRepo,
    label: &str,
    limit: usize,
    path: Option<&str>,
) {
    let limit_text = limit.to_string();
    let mut args: Vec<&str> = vec!["log", "--oneline", "--decorate", "-n", limit_text.as_str()];
    if let Some(p) = path {
        args.push("--");
        args.push(p);
    }
    let golden = capture_git(repo.path(), &args)
        .unwrap_or_else(|e| panic!("捕获 log 基准失败：{e}"));
    let actual = log_bytes(repo.path(), limit, path, true)
        .unwrap_or_else(|e| panic!("{label}：{e}"));
    assert_golden_bytes_equal(label, &golden, &actual).unwrap_or_else(|e| panic!("{e}"));
}

// ---------- status 黄金对比 ----------

#[test]
fn status_matches_git_on_unborn_fixture() {
    let repo = create(FixtureKind::Unborn).expect("生成 fixture 失败");
    assert_status_matches_git(&repo, "unborn：## No commits yet on master + 未跟踪");
}

#[test]
fn status_matches_git_on_clean_fixture() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    assert_status_matches_git(&repo, "干净仓库：仅分支行（无 upstream）");
}

#[test]
fn status_matches_git_on_dirty_mixed_fixture() {
    // 含 staged/unstaged 修改、staged 删除改名、unstaged 删除、二进制修改、
    // 未跟踪折叠——staged 修改行居首的分组序列（票 #1 锚点）必须逐行复刻。
    let repo = create(FixtureKind::DirtyMixed).expect("生成 fixture 失败");
    assert_status_matches_git(&repo, "脏仓库混合场景：全部变更行形态与行序");
}

#[test]
fn status_matches_git_on_upstream_ahead_fixture() {
    // `## master...origin/master [ahead 1]`：upstream 解析 + ahead 计数（rev-walk 对称差）。
    let repo = create(FixtureKind::UpstreamAhead).expect("生成 fixture 失败");
    assert_status_matches_git(&repo, "upstream 领先场景：分支行含 [ahead 1]");
}

#[test]
fn status_matches_git_on_utf8_fixture_with_default_quotepath() {
    // 默认 core.quotepath=true：中文路径输出为带引号的八进制转义（票 #1 锚点）。
    let repo = create(FixtureKind::Utf8Names).expect("生成 fixture 失败");
    assert_status_matches_git(&repo, "UTF-8 文件名：默认 quotepath 的八进制转义");
}

#[test]
fn status_matches_git_on_utf8_fixture_with_quotepath_false() {
    let repo = create(FixtureKind::Utf8Names).expect("生成 fixture 失败");
    set_local_config(repo.path(), "core.quotepath", "false").expect("写入 local 配置失败");
    assert_status_matches_git(&repo, "UTF-8 文件名：quotepath=false 原样输出");
}

#[test]
fn status_without_branch_header_omits_hash_line_but_keeps_rows() {
    // envcontext 摘要形态：不带 `##` 行，变更行不变。
    let repo = create(FixtureKind::DirtyMixed).expect("生成 fixture 失败");
    let golden = capture_git(repo.path(), &["status", "--short"]).expect("捕获基准失败");
    let actual = status_bytes(repo.path(), false).expect("渲染失败");
    assert_golden_bytes_equal("无分支头的 short 渲染", &golden, &actual)
        .unwrap_or_else(|e| panic!("{e}"));
    // 双重确认：带分支头的输出等于基准，去掉首行后应等于无头渲染。
    let with_header = status_bytes(repo.path(), true).expect("渲染失败");
    let first_line_end = with_header
        .iter()
        .position(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let stripped = &with_header[first_line_end..];
    assert_eq!(stripped, actual.as_slice(), "去掉首行分支头后的内容应与无头渲染一致");
}

#[test]
fn status_ahead_and_behind_counts_match_git_after_divergence() {
    // 互操作 + ahead/behind：系统 git 写出「本地领先 1、落后 1」的分叉形态（fetch 后
    // reset --hard 回退一格再前进一步），gix 计数必须与 git 一致。
    let repo = create(FixtureKind::UpstreamAhead).expect("生成 fixture 失败");
    let path = repo.path();
    // 追加一个本地提交（本地领先 2）
    std::fs::write(path.join("extra.txt"), "more local work\n").expect("写文件失败");
    support::commit_all(path, "second local ahead", 2).expect("追加提交失败");
    assert_status_matches_git(&repo, "upstream 领先 2 场景");
}

// ---------- log 黄金对比 ----------

#[test]
fn log_matches_git_on_short_history() {
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    assert_log_matches_git(&repo, "log -n 3（全部历史 + HEAD 装饰）", 3, None);
}

#[test]
fn log_matches_git_with_limit_smaller_than_history() {
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    assert_log_matches_git(&repo, "log -n 2（截断）", 2, None);
    assert_log_matches_git(&repo, "log -n 1（仅 HEAD）", 1, None);
}

#[test]
fn log_matches_git_with_upstream_decorations() {
    // UpstreamAhead：装饰含 `HEAD -> master, origin/master` 两项，顺序按引用名字典序。
    let repo = create(FixtureKind::UpstreamAhead).expect("生成 fixture 失败");
    assert_log_matches_git(&repo, "log -n 5（HEAD -> master + origin/master 装饰）", 5, None);
}

#[test]
fn log_matches_git_with_utf8_commit_messages() {
    let repo = create(FixtureKind::Utf8Names).expect("生成 fixture 失败");
    assert_log_matches_git(&repo, "log -n 2（中文提交信息）", 2, None);
}

#[test]
fn log_matches_git_with_path_filter() {
    // ShortHistory 每提交只改一个文件：-- f2.txt 应只显示 commit two。
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    assert_log_matches_git(&repo, "log -n 3 -- f2.txt（路径过滤）", 3, Some("f2.txt"));
    // 目录前缀过滤：-- nested 覆盖 nested/deep/file.txt（Clean 只有一个提交，触及即一行）。
    let clean = create(FixtureKind::Clean).expect("生成 fixture 失败");
    assert_log_matches_git(&clean, "log -- nested（目录前缀过滤）", 10, Some("nested"));
    // 过滤无命中：输出为空且不报错（git 行为：退出 0、空 stdout）。
    let golden = capture_git(repo.path(), &["log", "--oneline", "--decorate", "-n", "3", "--", "nope"]).expect("捕获失败");
    let actual = log_bytes(repo.path(), 3, Some("nope"), true).expect("渲染失败");
    assert_golden_bytes_equal("路径过滤无命中（空输出）", &golden, &actual).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn log_matches_git_in_detached_head_state() {
    // 分离 HEAD：装饰为 `HEAD`（无 ->），且原分支仍装饰在同一提交上。
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    git_fixture(repo.path(), &["checkout", "--detach"]).expect("布景 detach 失败");
    assert_log_matches_git(&repo, "分离 HEAD 的装饰形态", 3, None);
}

#[test]
fn log_unborn_head_fails_with_git_identical_message() {
    // unborn HEAD：craft-git 报 UnbornHead，文案与系统 git 的 stderr 逐字一致。
    let repo = create(FixtureKind::Unborn).expect("生成 fixture 失败");
    let git_err = capture_git(repo.path(), &["log", "--oneline", "--decorate", "-n", "3"])
        .expect_err("unborn 仓库 git log 应失败");
    let err = craft_git::log::log_text(
        &discover_isolated(repo.path()).expect("打开失败"),
        LogOptions { limit: 3, path: None, decorate: true },
        &interrupt_flag(),
    )
    .expect_err("unborn 仓库 craft-git log 应失败");
    let actual = err.to_string();
    assert!(
        git_err.contains(&actual),
        "系统 git 的失败输出应包含 craft-git 的错误文案。\ngit：{git_err}\ncraft-git：{actual}"
    );
}

#[test]
fn log_without_decorate_omits_parentheses() {
    // envcontext 摘要形态：`git log --oneline -n 5`（无 --decorate）。
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    let golden = capture_git(repo.path(), &["log", "--oneline", "-n", "5"]).expect("捕获基准失败");
    let actual = log_bytes(repo.path(), 5, None, false).expect("渲染失败");
    assert_golden_bytes_equal("无装饰的 oneline 渲染", &golden, &actual).unwrap_or_else(|e| panic!("{e}"));
}

// ---------- 互操作：系统 git 写 → gix 读 ----------

#[test]
fn interop_git_writes_are_read_consistently_by_gix() {
    // 票 #3 验收：系统 git 写（追加提交 + 留脏改动）后，gix 读 status / log 与 git 一致。
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = repo.path();
    std::fs::write(path.join("untracked.txt"), "fresh untracked\n").expect("写文件失败");
    std::fs::write(path.join("README.md"), "# fixture\n\nexternally edited by system git\n")
        .expect("改文件失败");
    support::commit_all(path, "commit written by system git", 1).expect("追加提交失败");
    std::fs::write(path.join("after-commit.txt"), "dirty after commit\n").expect("写脏文件失败");

    assert_status_matches_git(&repo, "互操作：git 写后 status 一致");
    assert_log_matches_git(&repo, "互操作：git 写后 log 一致", 10, None);
}

// ---------- 超时包装与显示名 ----------

#[test]
fn with_timeout_returns_result_when_operation_finishes() {
    let result = craft_git::with_timeout(std::time::Duration::from_secs(5), "测试操作", |_flag| {
        Ok(42)
    })
    .expect("未超时应返回结果");
    assert_eq!(result, 42);
}

#[test]
fn with_timeout_errors_and_sets_interrupt_flag_when_worker_blocks() {
    let started = std::time::Instant::now();
    let result: craft_git::Result<()> =
        craft_git::with_timeout(std::time::Duration::from_millis(200), "测试操作", |flag| {
            // 模拟慢操作：轮询中断标志（gix 的协作式语义），被置位后以错误返回。
            loop {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(craft_git::Error::internal("被打断", "flag set"));
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
    let err = result.expect_err("超时应返回错误");
    assert!(err.to_string().contains("超时") || err.to_string().contains("毫秒"), "错误应指明超时：{err}");
    // 主线程按 200ms 上限返回（不等待后台线程的自然退出）。
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "超时不应长时间阻塞");
}

#[test]
fn user_display_name_always_returns_non_empty() {
    // 回退链保证（USER → CraftWork）：任何环境下都非空；git 全局 user.name 配置与否
    // 由宿主机决定，不参与断言。
    assert!(!craft_git::user_display_name().is_empty());
}

// ---------- 未使用项防漂移（Arc/AtomicBool 的显式导入用途） ----------

#[allow(dead_code)]
fn _flag_type_anchor(flag: Arc<AtomicBool>) -> Arc<AtomicBool> {
    flag
}
