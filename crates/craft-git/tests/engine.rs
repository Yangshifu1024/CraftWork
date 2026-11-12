//! engine 模块集成测试：仓库发现 / 打开、形态区分、config 快照与身份读取。
//!
//! fixture 由 support 模块用系统 git 生成（测试代码允许进程调用）。

mod support;


use support::{capture_git, capture_git_inherited, create, git_fixture, FixtureKind};

use craft_git::{Error, Repository, RepositoryKind};

// ---------- 发现与打开 ----------

#[test]
fn discover_finds_repository_from_nested_subdirectory() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let deep = repo.path().join("nested/deep");
    let opened = Repository::discover(&deep).expect("从嵌套子目录发现仓库应成功");

    // 向上找到的是 work/.git；工作树根是 work。
    let work_canonical = std::fs::canonicalize(repo.path()).expect("规范化 work 路径失败");
    assert_eq!(
        std::fs::canonicalize(opened.git_dir()).expect("规范化 git_dir 失败"),
        work_canonical.join(".git"),
        "git_dir 应指向仓库根的 .git"
    );
    assert_eq!(
        std::fs::canonicalize(opened.workdir().expect("工作树仓库必有 workdir"))
            .expect("规范化 workdir 失败"),
        work_canonical,
        "workdir 应解析到工作树根"
    );
}

#[test]
fn discover_on_plain_directory_reports_not_a_repository() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    // 建一个空子目录，从它向上找（临时目录本身不是任何仓库的子目录）。
    let plain = dir.path().join("plain");
    std::fs::create_dir(&plain).expect("创建空目录失败");
    match Repository::discover(&plain) {
        Err(Error::NotARepository { path, .. }) => {
            assert_eq!(path, plain, "错误应携带发起查找的路径");
        }
        other => panic!("普通目录应报 NotARepository，实际：{other:?}"),
    }
}

#[test]
fn discover_on_missing_path_reports_not_found() {
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let missing = dir.path().join("no/such/dir");
    match Repository::discover(&missing) {
        Err(Error::NotFound { path }) => {
            assert_eq!(path, missing, "错误应携带不存在的路径");
        }
        other => panic!("不存在路径应报 NotFound，实际：{other:?}"),
    }
}

#[test]
fn open_is_strict_and_does_not_search_upwards() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let nested = repo.path().join("nested");

    // open 严格模式：子目录不允许（与 discover 的区别）。
    match Repository::open(&nested) {
        Err(Error::NotARepository { path, .. }) => {
            assert_eq!(path, nested);
        }
        other => panic!("open 在子目录应报 NotARepository，实际：{other:?}"),
    }
    // 但在仓库根成功。
    Repository::open(repo.path()).expect("open 在仓库根应成功");
}

// ---------- 形态区分 ----------

#[test]
fn kind_distinguishes_worktree_bare_and_linked_worktree() {
    let root = tempfile::tempdir().expect("创建临时目录失败");

    // 普通工作树仓库。
    let work = root.path().join("work");
    std::fs::create_dir(&work).expect("创建 work 失败");
    git_fixture(&work, &["init", "-b", "master"]).expect("init 失败");
    assert_eq!(
        Repository::discover(&work).expect("打开失败").kind(),
        RepositoryKind::Worktree
    );

    // bare 仓库（快照形态）。
    let bare = root.path().join("bare.git");
    git_fixture(root.path(), &["init", "--bare", "-b", "master", "bare.git"]).expect("init bare 失败");
    let bare_repo = Repository::discover(&bare).expect("打开 bare 失败");
    assert_eq!(bare_repo.kind(), RepositoryKind::Bare, "bare 仓库应识别为 Bare");
    assert!(bare_repo.workdir().is_none(), "bare 仓库无工作树");

    // linked worktree：在普通仓库上 git worktree add。
    git_fixture(&work, &["config", "user.email", "wt@example.com"]).expect("配置失败");
    git_fixture(&work, &["config", "user.name", "wt tester"]).expect("配置失败");
    git_fixture(&work, &["worktree", "add", "../wt-dir", "-b", "topic"]).expect("worktree add 失败");
    let linked = Repository::discover(root.path().join("wt-dir")).expect("打开 linked worktree 失败");
    assert_eq!(
        linked.kind(),
        RepositoryKind::LinkedWorkTree,
        "git worktree add 创建的目录应识别为 LinkedWorkTree"
    );
}

// ---------- config 快照与身份 ----------

#[test]
fn identity_reads_local_config_values() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    git_fixture(repo.path(), &["config", "user.name", "张三"]).expect("写入 user.name 失败");
    git_fixture(repo.path(), &["config", "user.email", "zhang@example.com"]).expect("写入 user.email 失败");

    let opened = Repository::discover(repo.path()).expect("打开失败");
    // local 级配置是级联最高优先级（除命令行覆盖外），结果不受宿主机全局配置影响。
    let identity = opened.identity().expect("读取身份应成功");
    assert_eq!(identity.name, "张三");
    assert_eq!(identity.email, "zhang@example.com");
}

#[test]
fn config_string_prefers_latest_level_and_matches_git_output() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    git_fixture(repo.path(), &["config", "craftwork.testkey", "local-value"]).expect("写入失败");

    let opened = Repository::discover(repo.path()).expect("打开失败");
    assert_eq!(
        opened.config_string("craftwork.testkey").as_deref(),
        Some("local-value"),
        "config 快照应读出 local 级自造键"
    );
    // 黄金对比思路：同一环境两侧读值一致（git 命令输出 == gix 快照读出）。
    let via_git = String::from_utf8(capture_git(repo.path(), &["config", "craftwork.testkey"]).expect("读取失败"))
        .expect("输出应为 UTF-8");
    assert_eq!(
        opened.config_string("craftwork.testkey").as_deref(),
        Some(via_git.trim_end_matches('\n')),
        "gix 快照读取应与 git config 输出一致"
    );
    assert_eq!(opened.config_string("craftwork.unset"), None, "未配置键应返回 None");
}

#[test]
fn identity_reports_missing_parts_with_field_flags() {
    let repo = create(FixtureKind::Clean).expect("生成 fixture 失败");
    // local 只配 email：name 是否缺失取决于宿主机全局配置（级联来源）。
    git_fixture(repo.path(), &["config", "user.email", "only@example.com"]).expect("写入失败");

    let opened = Repository::discover(repo.path()).expect("打开失败");
    // 与 gix 同环境的对照：继承测试进程环境跑 git（gix 快照读的正是测试进程的真实环境），
    // 先探明「级联视角下全局是否存在 user.name」。
    let global_name = capture_git_inherited(repo.path(), &["config", "user.name"])
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim_end_matches('\n').to_string());

    match opened.identity() {
        Err(Error::IdentityMissing {
            name_missing,
            email_missing,
        }) => {
            // email 已在 local 配置，级联必命中，不应缺失。
            assert!(!email_missing, "user.email 已在 local 配置，不应缺失");
            // name 缺失 ⇔ 全局视角下也没有 user.name（两侧同环境，结论一致）。
            assert!(name_missing, "user.name 未配置时应置缺失位");
            assert!(global_name.is_none(), "git 能读到全局 user.name 而报缺失，两侧环境不一致");
        }
        Ok(identity) => {
            let global_name = global_name.expect("报缺失时应与 git 的「无全局 user.name」一致");
            assert_eq!(
                identity.name, global_name,
                "级联读到的 name 应与同环境 git config 输出一致"
            );
        }
        other => panic!("应返回 IdentityMissing 或级联身份，实际：{other:?}"),
    }
}
