//! worktree 能力测试（票 #6）：craft-git 的 add / remove / 检测 / prune 与系统 git 的
//! **双向互操作**（spec 第 6 节验收）——gix 创建的工作树被系统 git `worktree list` 识别、
//! `git -C <主库>` 常规操作不受影响；反向（系统 git 创建、craft-git 打开与清理）成立。
//!
//! 系统 git 只出现在测试代码（黄金对比基建的一部分，与 support 模块同一纪律）；
//! 布景与验证全部走隔离环境（GIT_CONFIG_GLOBAL/SYSTEM 指向 /dev/null）。

mod support;

use support::{capture_git, create, git_fixture, FixtureKind};

/// 在临时根下生成一个目标目录路径（worktree 落点，与主库同一临时根、随根清理）。
fn wt_dir(repo: &support::FixtureRepo, tag: &str) -> std::path::PathBuf {
    repo_path_root(repo).join(tag)
}

/// fixture 临时根（worktree 目录与主库同根，测试结束时一起清理）。
fn repo_path_root(repo: &support::FixtureRepo) -> std::path::PathBuf {
    // FixtureRepo 未暴露根目录：从工作仓库路径的父目录取（布局固定 <root>/work）。
    std::path::Path::new(repo.path()).parent().map(|p| p.to_path_buf()).expect("fixture 布局应含根目录")
}

#[test]
fn gix_worktree_is_listed_and_usable_by_system_git() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let main = repo.path();
    let dest = wt_dir(&repo, "linked");
    let branch = "craftwork/abc12345";

    // gix 创建：分支创建 + 检出（`git worktree add -b` 等价）
    craft_git::worktree::add_worktree(main, &dest, branch).expect("gix 创建工作树失败");

    // 系统 git `worktree list --porcelain` 识别（互操作验收硬门槛）。
    // porcelain 输出统一正斜杠；Windows 路径是反斜杠，断言前归一化（实测 contains 失配）。
    let listing = capture_git(main, &["worktree", "list", "--porcelain"]).expect("worktree list 失败");
    let listing = String::from_utf8_lossy(&listing);
    let dest_norm = dest.to_string_lossy().replace('\\', "/");
    assert!(listing.contains(dest_norm.as_str()), "gix 创建的工作树应被系统 git 识别：{listing}");
    assert!(listing.contains(branch), "分支登记应一致：{listing}");

    // `git -C <主库>` 常规操作不受影响
    let status = capture_git(main, &["status", "--short", "--branch"]).expect("主库 status 失败");
    assert!(String::from_utf8_lossy(&status).contains("## master"), "主库 status 正常：{}", String::from_utf8_lossy(&status));
    let log = capture_git(main, &["log", "--oneline", "-1"]).expect("主库 log 失败");
    assert!(!String::from_utf8_lossy(&log).is_empty());

    // 工作树内的常规 git 操作（检出产物完整：文件与 index 就位）
    let checked = capture_git(&dest, &["status", "--short", "--branch"]).expect("工作树 status 失败");
    let checked = String::from_utf8_lossy(&checked);
    assert!(checked.contains(&format!("## {branch}")), "工作树检出在目标分支：{checked}");
    git_fixture(&dest, &["config", "user.email", "fixture@craftwork.invalid"]).unwrap();
    git_fixture(&dest, &["config", "user.name", "craft-fixture"]).unwrap();
    std::fs::write(dest.join("w.txt"), "worktree\n").expect("写工作树文件失败");
    git_fixture(&dest, &["add", "."]).unwrap();
    git_fixture(&dest, &["commit", "-qm", "in-worktree"]).unwrap();

    // craft-git 读取：未提交改动（已全部提交 → 干净）与未合并提交（主库 master 未含 → true）
    assert!(
        !craft_git::worktree::has_uncommitted_changes(&dest).expect("未提交检测失败"),
        "提交后工作树应干净"
    );
    assert!(
        craft_git::worktree::has_unmerged_commits(main, &dest).expect("未合并检测失败"),
        "工作树分支的提交未被主库 HEAD 包含 → 未合并"
    );

    // 主库合并后（merge 工作树分支），未合并检测翻转
    git_fixture(main, &["merge", "-q", "--no-ff", "-m", "merge wt", branch]).unwrap();
    assert!(
        !craft_git::worktree::has_unmerged_commits(main, &dest).expect("未合并检测失败"),
        "合并后分支提交已被主库 HEAD 包含"
    );

    // 分支占用检查：同分支再 add 第二个工作树 → 冲突拒绝（等价 git fatal）
    let second = wt_dir(&repo, "linked-second");
    let err = craft_git::worktree::add_worktree(main, &second, branch).expect_err("占用分支应被拒");
    assert!(err.to_string().contains("创建 git 工作树失败"), "实际：{err}");
    assert!(!second.exists(), "失败不留目标目录");

    // unborn HEAD（无提交仓库）拒绝创建（对齐 git 的 fatal）
    let unborn = create(FixtureKind::Unborn).expect("生成 unborn fixture 失败");
    let unborn_dest = wt_dir(&unborn, "linked");
    let err = craft_git::worktree::add_worktree(unborn.path(), &unborn_dest, "craftwork/xyz").expect_err("unborn 应被拒");
    assert!(err.to_string().contains("没有任何提交"), "实际：{err}");
}

#[test]
fn gix_add_worktree_reuses_existing_branch_after_directory_loss() {
    // 目录丢失重建场景：分支已存在（工作树被手动删除、admin 目录被 prune 后）复用原分支
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let main = repo.path();
    let dest = wt_dir(&repo, "linked");
    let branch = "craftwork/rebuild";

    craft_git::worktree::add_worktree(main, &dest, branch).expect("首次创建失败");
    git_fixture(&dest, &["config", "user.email", "fixture@craftwork.invalid"]).unwrap();
    git_fixture(&dest, &["config", "user.name", "craft-fixture"]).unwrap();
    std::fs::write(dest.join("w.txt"), "history\n").unwrap();
    git_fixture(&dest, &["add", "."]).unwrap();
    git_fixture(&dest, &["commit", "-qm", "kept"]).unwrap();

    // 手动删目录（模拟用户删除）+ prune 清失效登记
    std::fs::remove_dir_all(&dest).unwrap();
    let pruned = craft_git::worktree::prune(main).expect("prune 失败");
    assert_eq!(pruned, vec!["linked".to_string()], "失效管理目录被清除");

    // 重建：分支已存在 → 复用（历史提交不丢）
    craft_git::worktree::add_worktree(main, &dest, branch).expect("重建失败");
    let log = capture_git(&dest, &["log", "--oneline", "-2"]).expect("重建后 log 失败");
    assert!(String::from_utf8_lossy(&log).contains("kept"), "分支历史提交保留：{}", String::from_utf8_lossy(&log));
}

#[test]
fn gix_worktree_in_long_path_is_usable_by_system_git() {
    // Windows 长路径回归：gix 的 realpath 在 Windows 上返回 verbatim 形式的
    // `\\?\C:\...`，若原样写进链接文件，系统 git 会报「gitfile does not point to a
    // valid repository」。用深层目录把路径推长，尽量触发 verbatim 分支；无论是否触发，
    // 最终产物都必须能被系统 git 正常打开。
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let main = repo.path();
    // 逐层加深：Windows 上超过 MAX_PATH(260) 时 gix 必须走 verbatim 写法
    let mut deep = wt_dir(&repo, "deep");
    for segment in ["aaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cccccccccccccccccccccccccccc"] {
        deep = deep.join(segment);
    }
    let branch = "craftwork/longpath";

    craft_git::worktree::add_worktree(main, &deep, branch).expect("长路径创建工作树失败");

    // 链接文件不得残留 verbatim 前缀（系统 git 不认这种写法）
    let dot_git = std::fs::read_to_string(deep.join(".git")).expect("读取工作树 .git 失败");
    assert!(!dot_git.contains(r"\\?\"), "工作树 .git 不应含 verbatim 前缀：{dot_git}");
    assert!(dot_git.starts_with("gitdir: "), "保持 gitfile 格式：{dot_git}");

    // 硬门槛：系统 git 能在该工作树内正常工作（修复前此处 fatal）
    let status = capture_git(&deep, &["status", "--short", "--branch"]).expect("长路径工作树内系统 git 失败");
    assert!(String::from_utf8_lossy(&status).contains(&format!("## {branch}")), "检出在目标分支：{}", String::from_utf8_lossy(&status));
    // 主库侧登记文件同样可被系统 git 反向解析
    let listing = capture_git(main, &["worktree", "list", "--porcelain"]).expect("worktree list 失败");
    assert!(String::from_utf8_lossy(&listing).contains("craftwork/longpath"), "主库应登记该工作树：{}", String::from_utf8_lossy(&listing));
}

#[test]
fn system_git_worktree_is_removable_by_craft_git() {
    // 反向互操作：系统 git 创建 → craft-git 移除
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let main = repo.path();
    let dest = wt_dir(&repo, "git-made");
    git_fixture(main, &["worktree", "add", "-b", "craftwork/git-made", dest.to_string_lossy().as_ref()])
        .expect("系统 git 创建工作树失败");

    // craft-git 能打开并检测
    assert!(
        !craft_git::worktree::has_uncommitted_changes(&dest).expect("打开失败"),
        "系统 git 创建的工作树是干净的"
    );

    // craft-git 强制移除 + 删除分支
    craft_git::worktree::remove_worktree(main, &dest, true).expect("craft-git 移除失败");
    assert!(!dest.exists(), "检出目录已删除");
    let listing = capture_git(main, &["worktree", "list", "--porcelain"]).expect("worktree list 失败");
    assert!(
        !String::from_utf8_lossy(&listing).contains("git-made"),
        "移除后系统 git 不再登记：{}",
        String::from_utf8_lossy(&listing)
    );
    // 分支仍在（remove 不删分支，级联由调用方决定）→ delete_branch 删除
    craft_git::worktree::delete_branch(main, "craftwork/git-made").expect("删除分支失败");
    let branches = capture_git(main, &["branch", "--list", "craftwork/git-made"]).expect("branch list 失败");
    assert!(String::from_utf8_lossy(&branches).trim().is_empty(), "分支已删除");

    // 重复删除分支幂等（best-effort 语义）
    craft_git::worktree::delete_branch(main, "craftwork/git-made").expect("重复删除应静默成功");
}

#[test]
fn prune_keeps_live_worktrees_and_clears_only_stale_ones() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let main = repo.path();
    let live = wt_dir(&repo, "live");
    let stale = wt_dir(&repo, "stale");

    // 两个工作树：一个保留、一个手动删目录制造失效
    craft_git::worktree::add_worktree(main, &live, "craftwork/live").expect("创建 live 失败");
    git_fixture(main, &["worktree", "add", "-b", "craftwork/stale", stale.to_string_lossy().as_ref()])
        .expect("系统 git 创建 stale 失败");
    std::fs::remove_dir_all(&stale).unwrap();

    let pruned = craft_git::worktree::prune(main).expect("prune 失败");
    assert_eq!(pruned, vec!["stale".to_string()], "只清失效登记：{pruned:?}");
    assert!(live.exists(), "活的工作树不受影响");
    let listing = capture_git(main, &["worktree", "list", "--porcelain"]).expect("worktree list 失败");
    let listing = String::from_utf8_lossy(&listing);
    assert!(listing.contains("live"), "活工作树仍被登记：{listing}");
    assert!(!listing.contains("stale"), "失效工作树已从登记清除：{listing}");

    // 无 worktrees 目录的仓库：prune 幂等返回空
    let fresh = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let pruned = craft_git::worktree::prune(fresh.path()).expect("prune 失败");
    assert!(pruned.is_empty());
}

#[test]
fn remove_worktree_without_force_refuses_dirty_and_force_discards() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let main = repo.path();
    let dest = wt_dir(&repo, "dirty");

    craft_git::worktree::add_worktree(main, &dest, "craftwork/dirty").expect("创建失败");
    std::fs::write(dest.join("dirty.txt"), "未提交改动\n").unwrap();

    // 非 force：有未提交改动被拒（等价 git worktree remove 的拒绝）
    let err = craft_git::worktree::remove_worktree(main, &dest, false).expect_err("脏工作树应被拒");
    assert!(err.to_string().contains("移除 git 工作树失败"), "实际：{err}");
    assert!(dest.exists(), "被拒后目录保留");

    // force：丢弃改动强制移除（迁移前服务层 `--force` 语义）
    craft_git::worktree::remove_worktree(main, &dest, true).expect("force 移除失败");
    assert!(!dest.exists());
}

#[test]
fn branch_name_validation_rejects_path_escape() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let dest = wt_dir(&repo, "escape");
    let err = craft_git::worktree::add_worktree(repo.path(), &dest, "../escape").expect_err("穿越分支名应被拒");
    assert!(err.to_string().contains("分支名不合法"), "实际：{err}");
    assert!(!dest.exists());
}
