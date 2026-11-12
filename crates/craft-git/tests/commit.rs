//! 票 #5：两模式提交管线的单元测试与双向互操作测试。
//!
//! fixture 一律由系统 git 生成（support 基建）；craft-git 提交后的仓库状态用
//! **隔离环境下的系统 git** 读验证（`capture_git`）——这同时就是互操作验收的
//! 「gix 写、git 读」方向；「git 写、gix 读」方向由系统 git 提交布景 +
//! craft-git status/log 读验证覆盖。
//!
//! 提交身份经 fixture 仓库的 local config 注入（`set_local_config`），隔离打开的
//! gix 与隔离环境的系统 git 读到同一来源；测试进程宿主机的全局配置不参与。

mod support;

use std::path::Path;

use support::{
    capture_git, create, discover_isolated, interrupt_flag, remove_local_config, set_local_config,
    FixtureKind, FIXTURE_USER_EMAIL, FIXTURE_USER_NAME,
};

use craft_git::commit::{commit, CommitOptions};

// ---------- 测试辅助 ----------

/// 组装提交参数（路径限定模式传 `paths`，`-am` 模式传空）。
fn commit_options(message: &str, paths: &[&str]) -> CommitOptions {
    CommitOptions {
        message: message.to_string(),
        paths: paths.iter().map(|p| p.as_bytes().to_vec()).collect(),
        should_interrupt: interrupt_flag(),
    }
}

/// 给 fixture 仓库写 local 身份并隔离打开（提交需要 user.name / user.email；
/// 布景只经环境变量传身份，local config 默认无身份）。
fn open_committer(path: &Path) -> Result<craft_git::Repository, String> {
    set_local_config(path, "user.name", FIXTURE_USER_NAME)?;
    set_local_config(path, "user.email", FIXTURE_USER_EMAIL)?;
    discover_isolated(path)
}

/// 执行提交并展开错误（panic 友好）。
fn run_commit(
    repo: &craft_git::Repository,
    options: CommitOptions,
) -> Result<craft_git::commit::CommitOutcome, String> {
    commit(repo, options).map_err(|e| format!("提交失败：{e}"))
}

/// 隔离系统 git 的 stdout 文本（trim 尾部换行）。
fn git_text(path: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&capture_git(path, args).expect("系统 git 读取失败"))
        .into_owned()
        .trim()
        .to_string()
}

/// 断言提交后状态三方一致：系统 git status 只剩预期未跟踪项、craft-git status
/// 与之同构、index 与 HEAD tree 一致（write-tree hash 相等）。
fn assert_commit_settled(path: &Path, expected_untracked: &[&str]) {
    // 系统 git status 的完整预期输出（未跟踪项按传入顺序；porcelain 无额外行）。
    let expected_status = expected_untracked
        .iter()
        .map(|u| format!("?? {u}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        git_text(path, &["status", "--porcelain"]),
        expected_status,
        "提交后 status 应只剩预期未跟踪项"
    );

    // index 与 HEAD tree 一致（`git write-tree` 从 index 构建，无差异时 hash 相同）。
    let index_tree = git_text(path, &["write-tree"]);
    let head_tree = git_text(path, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(index_tree, head_tree, "index 与 HEAD tree 应一致");

    // craft-git 侧重新打开读 status（模拟真实消费方的新会话），变更行应与系统
    // git 同构（`##` 分支头行不参与对比）。
    let repo = discover_isolated(path).expect("重新打开仓库失败");
    let text = craft_git::status::status_text(&repo, true, &interrupt_flag())
        .expect("craft-git status 失败");
    let craft_lines: Vec<String> = text
        .lines()
        .filter(|line| !line.starts_with("## "))
        .map(str::to_string)
        .collect();
    assert_eq!(craft_lines.len(), expected_untracked.len(), "craft-git status 行数不符：{text:?}");
    for (line, untracked) in craft_lines.iter().zip(expected_untracked.iter()) {
        assert_eq!(line, &format!("?? {untracked}"), "craft-git status 未跟踪行不符：{text:?}");
    }
}

/// 断言 HEAD 提交触及的文件集合（`git show --name-status` 的路径 + 状态行）。
fn assert_head_touches(path: &Path, expected: &[(&str, &str)]) {
    let raw = git_text(path, &["show", "--name-status", "--format=", "HEAD"]);
    let mut lines: Vec<String> = raw
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    lines.sort();
    let mut expected_sorted: Vec<String> = expected
        .iter()
        .map(|(status, name)| format!("{status}\t{name}"))
        .collect();
    expected_sorted.sort();
    assert_eq!(
        lines, expected_sorted,
        "HEAD 提交触及的文件集合不符（实际 {raw:?}）"
    );
}

// ---------- -am 模式 ----------

#[test]
fn commit_am_includes_tracked_changes_and_excludes_untracked() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    // 布景：修改 + 删除 + 未跟踪新文件。
    std::fs::write(path.join("README.md"), "am modified content\n").expect("写布景失败");
    std::fs::remove_file(path.join("src/lib.rs")).expect("删除布景失败");
    std::fs::write(path.join("untracked.txt"), "must stay untracked\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    let outcome = run_commit(&repo, commit_options("am: tracked changes", &[]))
        .expect("提交应成功");

    assert_head_touches(
        path,
        &[("M", "README.md"), ("D", "src/lib.rs")],
    );
    assert_commit_settled(path, &["untracked.txt"]);
    // 短 hash 与 `git rev-parse --short HEAD` 一致（工具层 meta 的 hash 来源语义）。
    assert_eq!(outcome.short_hash, git_text(path, &["rev-parse", "--short", "HEAD"]));
}

#[test]
fn commit_am_includes_staged_changes_and_worktree_overrides() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    // 布景：README.md 先 stage 版本 A，工作树再改成版本 B → -am 应提交 B；
    // 另一个全新文件 stage 后不碰 → -am 应包含（提交整个 index 的 -a 语义）。
    std::fs::write(path.join("README.md"), "staged version a\n").expect("写布景失败");
    support::git_fixture(path, &["add", "README.md"]).expect("add 布景失败");
    std::fs::write(path.join("staged_new.txt"), "staged new file\n").expect("写布景失败");
    support::git_fixture(path, &["add", "staged_new.txt"]).expect("add 布景失败");
    std::fs::write(path.join("README.md"), "worktree version b\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    run_commit(&repo, commit_options("am: staged + worktree", &[])).expect("提交应成功");

    // 内容断言：工作树版本覆盖 staged 版本；staged 新文件进入提交。
    assert_eq!(
        git_text(path, &["show", "HEAD:README.md"]),
        "worktree version b",
        "同路径暂存 + 工作树变更时工作树内容应胜出"
    );
    assert_eq!(git_text(path, &["show", "HEAD:staged_new.txt"]), "staged new file");
    assert_commit_settled(path, &[]);
}

#[test]
fn commit_am_reports_nothing_to_commit_on_clean_repo() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    let repo = open_committer(path).expect("打开仓库失败");

    let err = commit(&repo, commit_options("no changes", &[])).expect_err("干净仓库应拒绝提交");
    assert!(
        matches!(err, craft_git::Error::NothingToCommit { .. }),
        "应返回 NothingToCommit，实际：{err:?}"
    );
    // 文案与系统 git 空提交拒绝的核心输出逐字一致（实测锚定）。
    assert_eq!(
        err.to_string(),
        "On branch master\nnothing to commit, working tree clean"
    );
}

#[test]
fn commit_am_on_unborn_repo_reports_nothing_to_commit() {
    // unborn 布景留了一个未跟踪文件：`-am` 只认已跟踪改动，因此同样拒绝。
    let fixture = create(FixtureKind::Unborn).expect("生成 fixture 失败");
    let path = fixture.path();
    let repo = open_committer(path).expect("打开仓库失败");

    let err = commit(&repo, commit_options("unborn", &[])).expect_err("unborn 仓库应拒绝 -am 提交");
    assert_eq!(
        err.to_string(),
        "On branch master\n\nNo commits yet\n\nnothing to commit (create/copy files and use \"git add\" to track)"
    );
}

#[test]
fn commit_am_with_dot_pathspec_commits_untracked_like_git_add() {
    // `git add .` 语义：`.` 规范化为「匹配一切」，未跟踪文件可提交。
    let fixture = create(FixtureKind::Unborn).expect("生成 fixture 失败");
    let path = fixture.path();
    let repo = open_committer(path).expect("打开仓库失败");

    run_commit(&repo, commit_options("dot pathspec", &["."])).expect("提交应成功");
    assert_head_touches(path, &[("A", "src/main.rs")]);
    assert_commit_settled(path, &[]);
}

// ---------- 路径限定模式 ----------

#[test]
fn commit_with_paths_commits_only_matching_paths() {
    // DirtyMixed 布景同时含 staged / unstaged 修改、删除、改名、二进制与未跟踪，
    // 是验证「路径外一律不动」的最佳布景。
    let fixture = create(FixtureKind::DirtyMixed).expect("生成 fixture 失败");
    let path = fixture.path();
    let repo = open_committer(path).expect("打开仓库失败");

    run_commit(&repo, commit_options("path limited: scratch", &["scratch.txt"]))
        .expect("提交应成功");

    // 提交只含指定文件。
    assert_head_touches(path, &[("A", "scratch.txt")]);
    // 路径外的暂存状态原样保留：staged 修改（README.md）与 staged 删除
    // （nested/deep/file.txt）、staged 改名（docs）都还在 index。
    let cached = git_text(path, &["diff", "--cached", "--name-only"]);
    assert!(cached.contains("README.md"), "路径外 staged 修改应保留：{cached:?}");
    assert!(cached.contains("nested/deep/file.txt"), "路径外 staged 删除应保留：{cached:?}");
    // 提交后状态：scratch.txt 已干净；布景其余变更与未跟踪 more/ 目录保持。
    let status = git_text(path, &["status", "--porcelain"]);
    assert!(!status.contains("scratch.txt"), "scratch.txt 已提交不应再出现：{status:?}");
    assert!(status.contains("?? more/"), "路径外未跟踪目录应保持：{status:?}");
    assert!(status.contains(" M src/lib.rs") || status.contains("M src/lib.rs"), "路径外 unstaged 修改应保持：{status:?}");
}

#[test]
fn commit_with_paths_includes_untracked_and_keeps_outside_paths_untouched() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    // 布景：指定路径（未跟踪新文件）+ 路径外修改。
    std::fs::create_dir_all(path.join("new_dir")).expect("创建布景目录失败");
    std::fs::write(path.join("new_dir/inner.txt"), "nested untracked\n").expect("写布景失败");
    std::fs::write(path.join("README.md"), "outside change\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    run_commit(&repo, commit_options("path limited: new_dir", &["new_dir"]))
        .expect("提交应成功");

    // 只提交了指定目录下的未跟踪文件；路径外修改保持未提交。
    assert_head_touches(path, &[("A", "new_dir/inner.txt")]);
    let status = git_text(path, &["status", "--porcelain"]);
    assert!(status.contains("M README.md"), "路径外修改应保持未提交：{status:?}");
    assert!(!status.contains("new_dir"), "指定路径应已干净：{status:?}");
}

#[test]
fn commit_with_paths_records_deletion_of_tracked_file() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::remove_file(path.join("nested/deep/file.txt")).expect("删除布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    run_commit(&repo, commit_options("delete nested file", &["nested/deep/file.txt"]))
        .expect("提交应成功");

    assert_head_touches(path, &[("D", "nested/deep/file.txt")]);
    // tree 中该路径已消失。
    let listing = git_text(path, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(!listing.contains("nested"), "删除路径不应留在 tree 中：{listing:?}");
    assert_commit_settled(path, &[]);
}

#[test]
fn commit_with_paths_creates_correct_tree_for_nested_directories() {
    // unborn 布景（src/main.rs 未跟踪）+ 新增嵌套未跟踪目录：一次提交验证
    // tree 的层级结构（顶层应为两个目录条目而非扁平文件）。
    let fixture = create(FixtureKind::Unborn).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::create_dir_all(path.join("more/deep")).expect("创建布景目录失败");
    std::fs::write(path.join("more/deep/scratch.txt"), "nested content\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    let outcome = run_commit(&repo, commit_options("root: nested dirs", &["src", "more"]))
        .expect("提交应成功");

    // 完整路径列表（字节序）。
    assert_eq!(
        git_text(path, &["ls-tree", "-r", "--name-only", "HEAD"]),
        "more/deep/scratch.txt\nsrc/main.rs"
    );
    // 顶层只有目录条目（tree 结构正确，而非 `more/deep/scratch.txt` 扁平化）。
    assert_eq!(git_text(path, &["ls-tree", "--name-only", "HEAD"]), "more\nsrc");
    assert_commit_settled(path, &[]);
    // unborn 仓库首个提交：root commit（无父），短 hash 可读。
    assert_eq!(git_text(path, &["rev-list", "--count", "HEAD"]), "1");
    assert_eq!(outcome.short_hash, git_text(path, &["rev-parse", "--short", "HEAD"]));
}

#[test]
fn commit_with_paths_leaves_staged_content_out_of_commit() {
    // 路径限定提交不触及路径外 staged 变更的完整闭环：先 stage 一个路径外修改，
    // 提交指定路径后该 staged 变更必须原样留在 index（git 临时 index 语义）。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join("README.md"), "staged outside\n").expect("写布景失败");
    support::git_fixture(path, &["add", "README.md"]).expect("add 布景失败");
    std::fs::write(path.join("target.txt"), "untracked target\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    run_commit(&repo, commit_options("path limited with staged outside", &["target.txt"]))
        .expect("提交应成功");

    assert_head_touches(path, &[("A", "target.txt")]);
    // HEAD 中 README.md 仍是布景原内容（路径外 staged 修改没有混进提交）。
    assert_eq!(
        git_text(path, &["show", "HEAD:README.md"]),
        "# fixture\n\nclean baseline",
        "路径外 staged 修改不应进入提交"
    );
    // staged 修改仍留在 index（diff --cached 可见）且未混入提交。
    assert_eq!(
        git_text(path, &["diff", "--cached", "--name-only"]),
        "README.md",
        "路径外 staged 修改应保留在 index"
    );
}

#[test]
fn commit_with_paths_respects_gitignore() {
    // `git add` 的忽略语义：.gitignore 与被忽略文件同场景提交时，只有 .gitignore
    // 进提交；ignored.txt 不进提交（提交 .gitignore 生效后连 status 都不再显示它）。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join(".gitignore"), "ignored.txt\n").expect("写布景失败");
    std::fs::write(path.join("ignored.txt"), "must be ignored\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    run_commit(&repo, commit_options("gitignore", &["."])).expect("提交应成功");

    assert_head_touches(path, &[("A", ".gitignore")]);
    // 文件仍在磁盘、被 ignore 规则命中（check-ignore 退出码 0 / 输出路径）。
    assert!(path.join("ignored.txt").exists(), "被忽略文件应留在磁盘");
    assert_eq!(git_text(path, &["check-ignore", "ignored.txt"]), "ignored.txt");
    // porcelain 全空（ignored.txt 已被忽略、其余干净），index 与 tree 一致。
    assert_commit_settled(path, &[]);
}

#[test]
fn commit_with_paths_rejects_unknown_pathspec_like_git_add() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join("README.md"), "changed\n").expect("写布景失败");
    let repo = open_committer(path).expect("打开仓库失败");

    let err = commit(&repo, commit_options("bad pathspec", &["nope.txt"]))
        .expect_err("不存在的路径应失败");
    assert!(
        matches!(err, craft_git::Error::PathspecNotFound { .. }),
        "应返回 PathspecNotFound，实际：{err:?}"
    );
    // 文案与 `git add -- nope.txt` 的 fatal 输出逐字一致。
    assert_eq!(
        err.to_string(),
        "fatal: pathspec 'nope.txt' did not match any files"
    );
}

// ---------- 身份 ----------

#[test]
fn commit_fails_with_identity_missing_error() {
    // 隔离打开 + local 无身份 → 级联全部缺失。create() 现在统一写入仓库级身份
    // （worktree 系测试的 gix 需要），本用例还原「无身份」前提。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    remove_local_config(path, "user.name").expect("移除 local user.name 失败");
    remove_local_config(path, "user.email").expect("移除 local user.email 失败");
    std::fs::write(path.join("README.md"), "changed\n").expect("写布景失败");
    let repo = discover_isolated(path).expect("隔离打开仓库失败");

    let err = commit(&repo, commit_options("no identity", &[])).expect_err("缺身份应失败");
    assert!(
        matches!(err, craft_git::Error::IdentityMissing { name_missing: true, email_missing: true }),
        "应返回 IdentityMissing（两侧缺失），实际：{err:?}"
    );
    let text = err.to_string();
    assert!(text.contains("user.name"), "文案应提示配置 user.name：{text}");
    assert!(text.contains("user.email"), "文案应提示配置 user.email：{text}");
}

#[test]
fn commit_uses_local_identity_from_config_cascade() {
    // local 级身份（local config 优先级最高）经级联读取后写入提交。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join("README.md"), "identity cascade\n").expect("写布景失败");
    let repo = open_committer(path).expect("打开仓库失败");

    run_commit(&repo, commit_options("identity via local config", &[])).expect("提交应成功");
    // 系统 git（隔离环境）读出的 author 与 committer 都是 local config 里的身份。
    let author = git_text(path, &["log", "-1", "--format=%an <%ae>"]);
    assert_eq!(author, format!("{FIXTURE_USER_NAME} <{FIXTURE_USER_EMAIL}>"));
    let committer = git_text(path, &["log", "-1", "--format=%cn <%ce>"]);
    assert_eq!(committer, format!("{FIXTURE_USER_NAME} <{FIXTURE_USER_EMAIL}>"));
}

// ---------- 互操作：系统 git 写、craft-git 读 ----------

#[test]
fn system_git_commit_then_craft_git_reads_clean_status_and_log() {
    // 反向互操作：系统 git 提交后，craft-git 的 status 应读出干净、log 应读到新提交。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join("README.md"), "written by system git\n").expect("写布景失败");
    std::fs::write(path.join("from_git.txt"), "git wrote this\n").expect("写布景失败");
    support::commit_all(path, "committed by system git", 1).expect("系统 git 提交失败");

    let repo = discover_isolated(path).expect("隔离打开仓库失败");
    let text = craft_git::status::status_text(&repo, true, &interrupt_flag())
        .expect("craft-git status 失败");
    // 除分支头行（`## master`）外应无任何变更行。
    assert!(
        text.lines().filter(|line| !line.starts_with("## ")).count() == 0,
        "系统 git 提交后 craft-git status 应干净，实际：{text:?}"
    );

    // log 首行是系统 git 写入的消息。
    let log_text = craft_git::log::log_text(
        &repo,
        craft_git::log::LogOptions { limit: 5, path: None, decorate: true },
        &interrupt_flag(),
    )
    .expect("craft-git log 失败");
    assert!(
        log_text.contains("committed by system git"),
        "craft-git log 应读到系统 git 的提交：{log_text:?}"
    );
    // index 与 HEAD tree 一致（系统 git 提交后的仓库对 craft-git 同样自洽）。
    assert_eq!(git_text(path, &["write-tree"]), git_text(path, &["rev-parse", "HEAD^{tree}"]));
}

#[test]
fn gix_commit_then_system_git_show_and_log_are_consistent() {
    // 正向互操作的补充断言：gix 提交后系统 git 的 log / show --stat / porcelain
    // 全链路可读（其余用例已覆盖 show --stat 与 porcelain，这里聚焦 log 序列与
    // show 的提交对象完整性）。
    let fixture = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join("f1.txt"), "gix appended\n").expect("写布景失败");

    let repo = open_committer(path).expect("打开仓库失败");
    let outcome = run_commit(&repo, commit_options("gix commit on git history", &[]))
        .expect("提交应成功");

    // 历史 4 条：3 条布景 + 1 条 gix 提交；新提交的父是原 HEAD。
    assert_eq!(git_text(path, &["rev-list", "--count", "HEAD"]), "4");
    let parents = git_text(path, &["log", "-1", "--format=%P", "HEAD"]);
    let previous_head = git_text(path, &["rev-parse", "HEAD~1"]);
    assert_eq!(parents, previous_head, "新提交的父应是布景的原 HEAD");
    assert_eq!(outcome.commit_id.to_hex().to_string(), git_text(path, &["rev-parse", "HEAD"]));
    // show --stat 含文件名与变更摘要（对象可被系统 git 完整解析）。
    let stat = git_text(path, &["show", "--stat", "--format=", "HEAD"]);
    assert!(stat.contains("f1.txt"), "show --stat 应含提交文件：{stat:?}");
    assert!(stat.contains("1 file changed"), "show --stat 应有摘要行：{stat:?}");
}

// ---------- 中断 ----------

#[test]
fn commit_stops_when_interrupt_flag_is_set() {
    // 协作式中断：标志置位后收集阶段立即以错误返回（超时包装的等价语义）。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    std::fs::write(path.join("README.md"), "interrupted\n").expect("写布景失败");
    set_local_config(path, "user.name", FIXTURE_USER_NAME).expect("写 local 身份失败");
    set_local_config(path, "user.email", FIXTURE_USER_EMAIL).expect("写 local 身份失败");
    let repo = discover_isolated(path).expect("隔离打开仓库失败");

    let options = CommitOptions {
        message: "should not land".into(),
        paths: Vec::new(),
        should_interrupt: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };
    let result = commit(&repo, options);
    assert!(result.is_err(), "置位中断标志后提交应以错误返回");
    // 提交不应落地：HEAD 仍是布景的唯一提交。
    assert_eq!(git_text(path, &["rev-list", "--count", "HEAD"]), "1");
}

// ---------- EOL 归一（Windows autocrlf 检出必须归一入库内容） ----------

/// `core.autocrlf=true` 时工作区的 CRLF 必须归一成 LF 再入库。
///
/// 此前 blob 按原始字节写入，Windows 检出（CRLF）会把 CRLF 原样提交，历史里
/// 出现「只改一行却整篇重写」的假 diff，也与 `.gitattributes` 的 `text=auto` 冲突。
#[test]
fn commit_normalizes_crlf_to_lf_when_autocrlf_is_enabled() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    set_local_config(path, "core.autocrlf", "true").expect("设置 autocrlf 失败");
    let repo = open_committer(path).expect("打开仓库失败");

    // 工作区内容按 CRLF 写入（模拟 Windows 检出后的状态）。
    std::fs::write(path.join("crlf.txt"), b"line1\r\nline2\r\n").expect("写入失败");
    run_commit(&repo, commit_options("add crlf file", &["crlf.txt"])).expect("提交失败");

    let blob = capture_git(path, &["cat-file", "-p", "HEAD:crlf.txt"]).expect("读取 blob 失败");
    assert!(
        !blob.windows(2).any(|w| w == b"\r\n"),
        "CRLF 应归一并入库为 LF，实际 blob：{:?}",
        String::from_utf8_lossy(&blob)
    );
    assert_eq!(String::from_utf8_lossy(&blob), "line1\nline2\n");
}

/// 二进制内容不参与 EOL 归一（含 `\0` 的文件字节必须完全一致）。
#[test]
fn commit_preserves_binary_bytes_without_eol_conversion() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    set_local_config(path, "core.autocrlf", "true").expect("设置 autocrlf 失败");
    let repo = open_committer(path).expect("打开仓库失败");

    let binary: &[u8] = b"\x00\x01binary\r\nraw\x00";
    std::fs::write(path.join("data.bin"), binary).expect("写入失败");
    run_commit(&repo, commit_options("add binary", &["data.bin"])).expect("提交失败");

    let blob = capture_git(path, &["cat-file", "-p", "HEAD:data.bin"]).expect("读取 blob 失败");
    assert_eq!(blob, binary, "二进制内容必须逐字节原样入库");
}

/// 显式 `-text` 属性（`.gitattributes`）关闭归一，CRLF 原样入库。
#[test]
fn commit_keeps_crlf_when_text_attribute_is_unset() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    set_local_config(path, "core.autocrlf", "true").expect("设置 autocrlf 失败");
    let repo = open_committer(path).expect("打开仓库失败");

    std::fs::write(path.join(".gitattributes"), b"keep.txt -text\n").expect("写入失败");
    std::fs::write(path.join("keep.txt"), b"a\r\nb\r\n").expect("写入失败");
    run_commit(&repo, commit_options("add keep", &[".gitattributes", "keep.txt"])).expect("提交失败");

    let blob = capture_git(path, &["cat-file", "-p", "HEAD:keep.txt"]).expect("读取 blob 失败");
    assert_eq!(
        String::from_utf8_lossy(&blob),
        "a\r\nb\r\n",
        "`-text` 属性下不归一（与 git 一致）"
    );
}

// ---------- 提交范围可见性 ----------

/// 路径限定提交只包含指定路径，并把「另外还有多少变更没进去」如实报出来。
#[test]
fn commit_with_paths_reports_leftover_changes() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    let repo = open_committer(path).expect("打开仓库失败");

    std::fs::write(path.join("a.txt"), b"a\n").expect("写入失败");
    std::fs::write(path.join("b.txt"), b"b\n").expect("写入失败");
    let outcome = run_commit(&repo, commit_options("only a", &["a.txt"])).expect("提交失败");

    assert_eq!(outcome.committed_files, 1, "只提交 a.txt");
    assert_eq!(outcome.leftover_files, 1, "b.txt 被 paths 排除，应计入 leftover");
    // b.txt 确实没进提交：仍在工作区未跟踪。
    let status = git_text(path, &["status", "--porcelain"]);
    assert!(status.contains("b.txt"), "b.txt 应仍留在工作区：{status}");
    let committed = git_text(path, &["ls-tree", "--name-only", "HEAD"]);
    assert!(committed.contains("a.txt"));
    assert!(!committed.contains("b.txt"), "b.txt 不应进本次提交");
}

/// 无 `paths`（`-am` 模式）提交全部已跟踪改动，leftover 恒为 0。
#[test]
fn commit_without_paths_reports_no_leftover() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let path = fixture.path();
    let repo = open_committer(path).expect("打开仓库失败");

    std::fs::write(path.join("b.txt"), b"b\n").expect("写入失败");
    std::fs::write(path.join("c.txt"), b"c\n").expect("写入失败");
    // 先提交一次，让 b/c 成为已跟踪文件。
    run_commit(&repo, commit_options("seed", &["b.txt", "c.txt"])).expect("提交失败");
    std::fs::write(path.join("b.txt"), b"b2\n").expect("写入失败");
    std::fs::write(path.join("c.txt"), b"c2\n").expect("写入失败");

    let outcome = run_commit(&repo, commit_options("amend all", &[])).expect("提交失败");
    assert_eq!(outcome.committed_files, 2, "b.txt 与 c.txt 两个改动");
    assert_eq!(outcome.leftover_files, 0, "-am 模式没有 leftover");
    assert_eq!(git_text(path, &["status", "--porcelain"]), "", "工作区应干净");
}
