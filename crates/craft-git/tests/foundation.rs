//! 票 #1 基建自测：fixture 矩阵生成的稳定性（确定性、可并行）与黄金对比 helper 自身可用性。
//!
//! 本票尚未实现 status / log 的 craft-git 渲染（M3、M4 票接入），因此黄金 helper 的
//! 自身验证采用「系统 git 双跑」策略：基准与占位实现都来自系统 git，验证
//! 「捕获 → 断言 → 用例执行」链路端到端可用；M3 起把占位实现换成 craft-git 渲染。

mod support;

use std::path::Path;
use std::thread;

use support::{
    assert_golden_bytes_equal, capture_git, create, git_fixture, run_golden_case, FixtureKind, GoldenCase,
};

// ---------- fixture 矩阵生成器 ----------

#[test]
fn fixture_matrix_all_kinds_are_created_and_discovered() {
    let kinds = [
        FixtureKind::Unborn,
        FixtureKind::Clean,
        FixtureKind::DirtyMixed,
        FixtureKind::ShortHistory,
        FixtureKind::UpstreamAhead,
        FixtureKind::Utf8Names,
        FixtureKind::DiffStagedMixed,
        FixtureKind::DiffWorktreeMixed,
        FixtureKind::DiffUtf8Mixed,
    ];
    for kind in kinds {
        let repo = create(kind).unwrap_or_else(|e| panic!("生成 {kind:?} fixture 失败：{e}"));

        // gix 能发现并打开 git 生成的仓库（互操作的第一道门槛）。
        let opened = craft_git::Repository::discover(repo.path())
            .unwrap_or_else(|e| panic!("gix 打开 {kind:?} fixture 失败：{e}"));
        assert_eq!(
            opened.kind(),
            craft_git::RepositoryKind::Worktree,
            "{kind:?} 应识别为普通工作树仓库"
        );

        // 元数据与场景设计一致。
        match kind {
            FixtureKind::Unborn => {
                assert_eq!(repo.head_commits(), 0, "unborn 仓库无提交");
            }
            FixtureKind::Clean => {
                assert_eq!(repo.head_commits(), 1);
                assert!(!repo.has_upstream());
            }
            FixtureKind::DirtyMixed => assert_eq!(repo.head_commits(), 1),
            FixtureKind::ShortHistory => assert_eq!(repo.head_commits(), 3),
            FixtureKind::UpstreamAhead => {
                assert_eq!(repo.head_commits(), 2);
                assert!(repo.has_upstream(), "upstream 场景必须配置 origin");
                assert!(repo.remote_path().is_some());
            }
            FixtureKind::Utf8Names => assert_eq!(repo.head_commits(), 2),
            FixtureKind::DiffStagedMixed => assert_eq!(repo.head_commits(), 1),
            FixtureKind::DiffWorktreeMixed => assert_eq!(repo.head_commits(), 1),
            FixtureKind::DiffUtf8Mixed => assert_eq!(repo.head_commits(), 1),
        }
    }
}

#[test]
fn fixture_generation_is_deterministic_across_runs() {
    // 固定身份 + 固定日期 ⇒ 同一场景两次生成的 HEAD 哈希字节级一致。
    for kind in [
        FixtureKind::Clean,
        FixtureKind::DirtyMixed,
        FixtureKind::ShortHistory,
        FixtureKind::UpstreamAhead,
        FixtureKind::Utf8Names,
        FixtureKind::DiffStagedMixed,
        FixtureKind::DiffWorktreeMixed,
        FixtureKind::DiffUtf8Mixed,
    ] {
        let first = create(kind).expect("第一次生成失败");
        let second = create(kind).expect("第二次生成失败");

        let head = |dir: &Path| {
            capture_git(dir, &["rev-parse", "HEAD"]).expect("读取 HEAD 失败")
        };
        assert_eq!(
            head(first.path()),
            head(second.path()),
            "{kind:?} 两次生成的 HEAD 哈希不一致，布景存在不确定性"
        );
    }
}

#[test]
fn fixture_generation_supports_parallel_creation() {
    // 多线程同时生成（含同场景并发），验证无共享状态、可并行。
    thread::scope(|scope| {
        let handles: Vec<_> = [
            FixtureKind::Clean,
            FixtureKind::Clean,
            FixtureKind::DirtyMixed,
            FixtureKind::Utf8Names,
        ]
        .map(|kind| scope.spawn(move || create(kind).expect("并行生成失败")))
        .into_iter()
        .collect();
        for handle in handles {
            handle.join().expect("并行生成线程崩溃");
        }
    });
}

#[test]
fn unborn_fixture_has_no_head_revision() {
    let repo = create(FixtureKind::Unborn).expect("生成 unborn fixture 失败");
    let err = capture_git(repo.path(), &["rev-parse", "HEAD"])
        .expect_err("unborn 仓库读取 HEAD 应失败");
    assert!(err.contains("HEAD"), "错误信息应指向 HEAD 缺失：{err}");
}

// ---------- 黄金对比 helper 自身 ----------

#[test]
fn golden_assert_passes_on_identical_bytes() {
    assert_golden_bytes_equal("完全一致", b"## master\n", b"## master\n")
        .expect("字节一致时应通过");
    assert_golden_bytes_equal("空输出", b"", b"").expect("空输出一致应通过");
}

#[test]
fn golden_assert_fails_with_label_offset_and_previews() {
    let golden = b"## master...origin/master [ahead 1]\n";
    let actual = b"## master...origin/master [ahead 2]\n";
    let err = assert_golden_bytes_equal("ahead 计数", golden, actual)
        .expect_err("字节不一致时应失败");
    assert!(err.contains("ahead 计数"), "失败消息应含用例名：{err}");
    // 偏差出现在数字 `1`/`2`（偏移 33：`## master...origin/master [ahead ` 共 33 字节）。
    assert!(err.contains("偏移 33"), "失败消息应含首个偏差偏移：{err}");
    assert!(err.contains("[ahead 1]"), "失败消息应含基准预览：{err}");
    assert!(err.contains("[ahead 2]"), "失败消息应含实现预览：{err}");
}

#[test]
fn golden_capture_is_byte_stable_across_runs() {
    let repo = create(FixtureKind::DirtyMixed).expect("生成 fixture 失败");
    let first = capture_git(repo.path(), &["status", "--short", "--branch"]).expect("捕获失败");
    let second = capture_git(repo.path(), &["status", "--short", "--branch"]).expect("捕获失败");
    assert_eq!(first, second, "同一 fixture 上重复捕获 status 输出应字节一致");
}

/// 占位实现：craft-git status 渲染未接入（M3 票），暂以系统 git 同命令代替，
/// 验证 GoldenCase 链路（捕获基准 → 实现 → 逐字节断言）端到端可用。
fn status_short_branch_via_git(dir: &Path) -> Result<Vec<u8>, String> {
    capture_git(dir, &["status", "--short", "--branch"])
}

/// 占位实现：同上，log 版（M3 票替换为 craft-git 渲染）。
fn log_oneline_decorate_via_git(dir: &Path) -> Result<Vec<u8>, String> {
    capture_git(dir, &["log", "--oneline", "--decorate", "-n", "3"])
}

#[test]
fn golden_run_case_passes_when_placeholder_matches_git() {
    let repo = create(FixtureKind::DirtyMixed).expect("生成 fixture 失败");
    run_golden_case(
        &repo,
        &GoldenCase {
            label: "status --short --branch（占位实现）",
            git_args: &["status", "--short", "--branch"],
            actual: status_short_branch_via_git,
        },
    )
    .expect("占位实现与基准应逐字节一致");

    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    run_golden_case(
        &repo,
        &GoldenCase {
            label: "log --oneline --decorate（占位实现）",
            git_args: &["log", "--oneline", "--decorate", "-n", "3"],
            actual: log_oneline_decorate_via_git,
        },
    )
    .expect("占位实现与基准应逐字节一致");
}

#[test]
fn golden_run_case_reports_mismatch_between_different_commands() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    // 故意让「基准」与「占位实现」跑不同命令，验证用例失败路径。
    let err = run_golden_case(
        &repo,
        &GoldenCase {
            label: "不同命令应不一致",
            git_args: &["status", "--short", "--branch"],
            actual: log_oneline_decorate_via_git,
        },
    )
    .expect_err("不同命令的输出不可能逐字节一致");
    assert!(err.contains("不同命令应不一致"), "失败消息应含用例名：{err}");
}

// ---------- 黄金基准的形态锚点（供 M3~M5 对齐）----------

#[test]
fn golden_status_short_branch_shape_on_dirty_fixture() {
    let repo = create(FixtureKind::DirtyMixed).expect("生成 fixture 失败");
    let out = String::from_utf8(
        capture_git(repo.path(), &["status", "--short", "--branch"]).expect("捕获失败"),
    )
    .expect("status 输出应为 UTF-8");

    // 逐行精确锚定（fixture 布景确定 ⇒ 输出行序确定）。注意实测的 git 排序：
    // staged 段在前（README.md 的 staged 修改居首），其余按路径序；未跟踪段最后。
    // 以系统 git 实际输出为准（这正是黄金对比的含义——预期来自 git 真实行为）。
    let expected = [
        "## master",
        "M  README.md",                    // staged 修改（实测中排在所有变更行之前）
        " M assets/logo.png",              // 二进制修改，unstaged
        "R  docs/a.md -> docs/b.md",       // staged 改名
        " D extra/stale.txt",              // unstaged 删除
        "D  nested/deep/file.txt",         // staged 删除
        " M src/lib.rs",                   // unstaged 修改
        "?? more/",                        // 未跟踪嵌套目录折叠
        "?? scratch.txt",                  // 未跟踪根级文件
    ];
    let actual_lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        actual_lines, expected,
        "DirtyMixed 的 status --short --branch 形态与预期不符；\n实际输出：\n{out}"
    );
}

#[test]
fn golden_status_branch_line_reflects_upstream_state() {
    // 无 upstream：分支行只有分支名（票面「无 upstream」场景的输出锚点）。
    let clean = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let out = String::from_utf8(
        capture_git(clean.path(), &["status", "--short", "--branch"]).expect("捕获失败"),
    )
    .expect("输出应为 UTF-8");
    assert_eq!(out.lines().next(), Some("## master"), "无 upstream 时分支行应为「## master」");

    // 有 upstream 且领先 1：`...origin/master [ahead 1]`（spec 第 5 节 ahead/behind 渲染的对照锚点）。
    let ahead = create(FixtureKind::UpstreamAhead).expect("生成 fixture 失败");
    let out = String::from_utf8(
        capture_git(ahead.path(), &["status", "--short", "--branch"]).expect("捕获失败"),
    )
    .expect("输出应为 UTF-8");
    let branch_line = out.lines().next().expect("输出非空");
    assert_eq!(
        branch_line, "## master...origin/master [ahead 1]",
        "upstream 领先场景的分支行形态与预期不符"
    );
}

#[test]
fn golden_log_oneline_decorate_shape_on_short_history() {
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    let out = String::from_utf8(
        capture_git(repo.path(), &["log", "--oneline", "--decorate", "-n", "3"]).expect("捕获失败"),
    )
    .expect("log 输出应为 UTF-8");

    // 顶部装饰只出现在 HEAD 提交行；行序为最新在前。
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "-n 3 应输出 3 行：{out}");
    assert!(
        lines[0].contains("(HEAD -> master)"),
        "首行应含 HEAD -> master 装饰：{out}"
    );
    assert!(!lines[1].contains("HEAD"), "非 HEAD 行不应含装饰：{out}");
    assert!(lines.iter().any(|l| l.contains("commit one")));
    assert!(lines.iter().any(|l| l.contains("commit three")));
    // 短哈希长度：git 默认 --oneline 用 7 位短哈希（后跟一个空格分隔消息）。
    let head_hash = lines[0].split(' ').next().expect("行首为短哈希");
    assert_eq!(head_hash.len(), 7, "--oneline 短哈希应为 7 位：{}", lines[0]);
    // 与 rev-parse 的短哈希一致（M3 实现短 hash 返回的锚点）。
    let full = String::from_utf8(capture_git(repo.path(), &["rev-parse", "HEAD"]).expect("读取失败"))
        .expect("rev-parse 输出应为 UTF-8");
    assert!(
        full.trim_start().starts_with(head_hash),
        "rev-parse HEAD 应以 --oneline 显示的短哈希开头"
    );
}

#[test]
fn golden_utf8_output_escaping_and_message_encoding() {
    let repo = create(FixtureKind::Utf8Names).expect("生成 fixture 失败");

    // 默认 core.quotepath=true：未跟踪中文路径输出为带引号的八进制转义（M3 渲染需复刻）。
    let status = capture_git(repo.path(), &["status", "--short", "--branch"]).expect("捕获失败");
    let status = String::from_utf8_lossy(&status);
    assert!(
        status.contains("\\350\\215\\211"),
        "默认 quotepath 下中文路径应转义为八进制（\\350\\215\\211 是「草」的 UTF-8 字节序列）：\n{status}"
    );
    // 转义路径以双引号包裹（git 对非 ASCII 路径的引用习惯）。
    assert!(
        status.contains('"'),
        "转义路径应包裹在双引号内：\n{status}"
    );

    // quotepath=false 时中文路径原样输出（craft-git 若按 UTF-8 直出，以此为对照）。
    let raw = capture_git(
        repo.path(),
        &["-c", "core.quotepath=false", "status", "--short", "--branch"],
    )
    .expect("捕获失败");
    let raw = String::from_utf8_lossy(&raw);
    assert!(
        raw.contains("?? 草稿.md"),
        "quotepath=false 下中文路径应原样输出：\n{raw}"
    );

    // 提交信息不受 quotepath 影响：--oneline 显示 UTF-8 中文原文。
    let log = capture_git(repo.path(), &["log", "--oneline", "--decorate", "-n", "2"]).expect("捕获失败");
    let log = String::from_utf8_lossy(&log);
    assert!(
        log.contains("初始提交：项目骨架说明"),
        "中文提交信息应原样输出：\n{log}"
    );
}

#[test]
fn golden_clean_fixture_status_is_only_branch_line() {
    // 干净仓库：status --short --branch 只有分支行（无任何变更行）。
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let out = capture_git(repo.path(), &["status", "--short", "--branch"]).expect("捕获失败");
    assert_eq!(out, b"## master\n", "干净仓库应只输出分支行");
}

#[test]
fn log_limit_smaller_than_history_truncates() {
    // -n 2 在 3 提交历史上只输出 2 行（M3 log 分页锚点）。
    let repo = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    let out = capture_git(repo.path(), &["log", "--oneline", "--decorate", "-n", "2"]).expect("捕获失败");
    assert_eq!(
        String::from_utf8_lossy(&out).lines().count(),
        2,
        "-n 2 应截断为 2 行"
    );
}

#[test]
fn fixture_branch_name_is_pinned_by_init() {
    // 隔离环境下 init.defaultBranch 不可达，布景显式 -b master；验证分支名与预期一致。
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let branch = git_fixture(repo.path(), &["rev-parse", "--abbrev-ref", "HEAD"]).expect("读取分支失败");
    assert_eq!(branch.trim(), repo.branch(), "分支名应固定为 init -b 指定的名称");
}
