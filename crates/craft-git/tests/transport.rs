//! transport 模块集成测试：本地 fixture 全链路（clone / fetch / 浅克隆 / 超时 / 错误归一化）。
//!
//! 自动化测试不碰外网（票面硬约束）：克隆源一律是本地 fixture 仓库（裸路径或 file://）；
//! 真实网络（GitHub 克隆、加速前缀、代理转发）的验收写成 `#[ignore]` 用例，由人工在
//! 具备外网 / 代理的环境下执行（见文件尾部与两票「进展」记录）。
//!
//! 关于回退用例的测试策略（票面折衷方案）：加速改写只对 github.com host 生效，而本地
//! 无法让 github.com URL 克隆成功，「改写失败 → 原始 URL 成功」的完整闭环只能在真实网络
//! 验证。因此调用序列与回退标注由 transport.rs 内部单元测试（可注入的尝试层）断言；
//! 本文件用「非 github host + 加速前缀」验证不改写路径，真实回退走 `#[ignore]` 用例。

mod support;

use std::time::{Duration, Instant};

use support::{FixtureKind, capture_git, commit_all, create};

use craft_git::transport::{CloneRequest, FetchRequest, clone_repo, fetch_repo};

use craft_git::Repository;

/// fixture 仓库的 HEAD 完整哈希（系统 git 读取，作为期望值基准）。
fn fixture_head_sha(fixture_path: &std::path::Path) -> String {
    let bytes = capture_git(fixture_path, &["rev-parse", "HEAD"]).expect("读取 fixture HEAD 失败");
    String::from_utf8(bytes)
        .expect("HEAD 输出应为 UTF-8")
        .trim()
        .to_string()
}

/// 期望的短哈希（transport 固定取 7 位，与 `git rev-parse --short` 默认长度一致）。
fn short_sha(full: &str) -> String {
    full.chars().take(7).collect()
}

// ---------- clone 基本链路 ----------

#[test]
fn clone_from_local_path_succeeds_and_resolves_head_sha() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let dest = dest_root.path().join("cloned");

    let req = CloneRequest::new(fixture.path().to_string_lossy().into_owned(), dest.clone());
    let outcome = clone_repo(req).expect("本地路径克隆应成功");

    let full = fixture_head_sha(fixture.path());
    assert_eq!(outcome.head_sha, short_sha(&full), "HEAD 短哈希应与 fixture 一致");
    assert!(!outcome.fell_back, "无加速改写时不应标注回退");
    assert_eq!(
        outcome.url_used,
        fixture.path().to_string_lossy().into_owned(),
        "url_used 应为原始路径"
    );
    // 克隆产物是带工作树的完整仓库，文件已检出
    assert!(dest.join("README.md").is_file(), "工作树文件应被检出");
    let opened = Repository::open(&dest).expect("克隆产物应可被 gix 打开");
    assert_eq!(opened.kind(), craft_git::RepositoryKind::Worktree);
}

#[test]
fn clone_from_file_url_succeeds() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let url = format!("file://{}", fixture.path().display());

    let outcome = clone_repo(CloneRequest::new(url, dest_root.path().join("cloned")))
        .expect("file:// URL 克隆应成功");
    let full = fixture_head_sha(fixture.path());
    assert_eq!(outcome.head_sha, short_sha(&full));
}

// ---------- 浅克隆 ----------

#[test]
fn shallow_clone_limits_history_to_requested_depth() {
    // ShortHistory 有 3 个提交；depth=1 后本地只有 HEAD 一个提交（对齐 git clone --depth 1）
    let fixture = create(FixtureKind::ShortHistory).expect("生成 fixture 失败");
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let dest = dest_root.path().join("shallow");

    let mut req = CloneRequest::new(fixture.path().to_string_lossy().into_owned(), dest.clone());
    req.shallow_depth = Some(1);
    let outcome = clone_repo(req).expect("浅克隆应成功");

    let full = fixture_head_sha(fixture.path());
    assert_eq!(outcome.head_sha, short_sha(&full), "浅克隆 HEAD 应指向同一提交");

    // 互操作校验（用系统 git 读取 gix 克隆的产物）：提交数与 shallow 边界文件
    let count = capture_git(&dest, &["rev-list", "--count", "HEAD"]).expect("统计提交数失败");
    assert_eq!(
        String::from_utf8(count).expect("UTF-8").trim(),
        "1",
        "depth=1 浅克隆本地应只有 1 个提交"
    );
    assert!(dest.join(".git").join("shallow").is_file(), "浅克隆应留下 shallow 边界文件");
}

// ---------- fetch（已有仓库刷新） ----------

#[test]
fn fetch_without_remote_changes_reports_no_update() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let dest = dest_root.path().join("cloned");
    let url = fixture.path().to_string_lossy().into_owned();

    clone_repo(CloneRequest::new(url.clone(), dest.clone())).expect("克隆应成功");

    let outcome = fetch_repo(FetchRequest::new(url, dest.clone())).expect("重复 fetch 应成功");
    assert!(!outcome.updated, "远程无新提交时不应报更新");
    assert!(!outcome.fell_back);
    let full = fixture_head_sha(fixture.path());
    assert_eq!(outcome.head_sha, short_sha(&full), "HEAD 不应变化");
}

#[test]
fn fetch_picks_up_new_commits_and_reports_update() {
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let dest = dest_root.path().join("cloned");
    let url = fixture.path().to_string_lossy().into_owned();

    clone_repo(CloneRequest::new(url.clone(), dest.clone())).expect("克隆应成功");

    // 远程新增一个提交后再刷新
    std::fs::write(fixture.path().join("second.txt"), "second round of content\n")
        .expect("写入新文件失败");
    commit_all(fixture.path(), "second commit for fetch", 10).expect("追加提交失败");

    let outcome = fetch_repo(FetchRequest::new(url, dest.clone())).expect("增量 fetch 应成功");
    assert!(outcome.updated, "远程有新提交时应报更新");
    let full = fixture_head_sha(fixture.path());
    assert_eq!(outcome.head_sha, short_sha(&full), "刷新后 HEAD 应推进到新提交");

    // 互操作校验：克隆仓库的 origin/master 与远程一致
    let remote_head =
        capture_git(&dest, &["rev-parse", "origin/master"]).expect("读取跟踪分支失败");
    assert_eq!(
        String::from_utf8(remote_head).expect("UTF-8").trim(),
        full,
        "origin/master 跟踪分支应随 fetch 更新"
    );
}

// ---------- 超时与错误归一化 ----------

#[test]
fn clone_unreachable_host_fails_as_network_error_within_deadline() {
    // 10.255.255.1 是 RFC1918 内的保留地址：SYN 无响应的「黑洞」，连接只能靠超时收尾。
    // 票面要求「不可达地址 + 短超时模拟超时触发」：timeout=2s 经 reqwest 的
    // connect_timeout 生效（见 transport.rs 超时机制注释）。协作式中断与定时线程本身
    // 已由 transport.rs 单元测试覆盖；此处验证端到端不永久挂死且错误归一化为 network。
    // 注：极少数网络环境可能对该地址立即返回 unreachable（connection refused 类），
    // 两条路径都归一化为 network 错误，断言不区分具体文案。
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let mut req = CloneRequest::new(
        "https://10.255.255.1:81/owner/repo.git".to_string(),
        dest_root.path().join("cloned"),
    );
    req.timeout = Duration::from_secs(2);
    req.retries = 0;

    let started = Instant::now();
    let err = clone_repo(req)
        .expect_err("黑洞地址克隆应失败")
        ;
    let elapsed = started.elapsed();

    assert_eq!(err.code(), "network", "不可达主机应归一化为 network 错误");
    assert!(
        err.to_string().contains("10.255.255.1"),
        "聚合错误应含尝试的 URL：{err}"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "短超时下不应挂死（实际耗时 {elapsed:?}）"
    );
}

#[test]
fn clone_missing_local_source_reports_not_found_without_retry() {
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let missing = dest_root.path().join("no-such-source");

    let mut req = CloneRequest::new(missing.to_string_lossy().into_owned(), dest_root.path().join("cloned"));
    // 本地路径失败是非网络类错误：即便给了重试预算也不应消耗（fail fast 由编排保证）
    req.retries = 3;

    let started = Instant::now();
    let err = clone_repo(req).expect_err("源不存在的克隆应失败");
    let elapsed = started.elapsed();

    assert_eq!(
        err.code(),
        "repo_not_found",
        "本地源不存在应归一化为仓库不存在：{err}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "非网络类失败不应重试（实际耗时 {elapsed:?}）"
    );
}

// ---------- 加速前缀（非 github host 不改写） ----------

#[test]
fn accelerator_prefix_is_ignored_for_non_github_host() {
    // 本地路径的 host 不是 github.com：即使加速前缀指向本地不可达端口也不改写、
    // 不回退，直接成功且不标注 fell_back。
    let fixture = create(FixtureKind::Clean).expect("生成 fixture 失败");
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let dest = dest_root.path().join("cloned");

    let mut req = CloneRequest::new(fixture.path().to_string_lossy().into_owned(), dest);
    req.accelerator_prefix = Some("https://127.0.0.1:1/".to_string());

    let outcome = clone_repo(req).expect("非 github host 不应受加速前缀影响");
    assert!(!outcome.fell_back, "未发生改写就不应标注回退");
}

// ---------- 真实网络手工验收（不进自动化，票面「进展」记录结论） ----------
//
// 执行方式（需外网）：
//   cargo test -p craft-git --test transport -- --ignored --nocapture
// 代理用例需显式注入代理地址：
//   CRAFT_GIT_TEST_PROXY_URL=http://127.0.0.1:7890 cargo test -p craft-git --test transport \
//       -- --ignored --nocapture

/// 手工验收 1：加速直连克隆 GitHub 公开仓库（默认 gh-proxy.com 前缀）。
#[test]
#[ignore = "手工验收：需要真实外网（加速直连 GitHub 公开仓库）"]
fn manual_accelerated_clone_from_real_github() {
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let mut req = CloneRequest::new(
        "https://github.com/octocat/Hello-World.git".to_string(),
        dest_root.path().join("hello-world"),
    );
    req.accelerator_prefix = Some("https://gh-proxy.com/".to_string());
    req.timeout = Duration::from_secs(90);

    let outcome = clone_repo(req).expect("加速直连克隆应成功");
    println!("[手工验收] 加速直连克隆成功：head_sha={} fell_back={}", outcome.head_sha, outcome.fell_back);
    // 直连成功 = 改写 URL 一次成功，未发生回退
    assert!(!outcome.fell_back);
    assert_eq!(
        outcome.url_used,
        "https://gh-proxy.com/https://github.com/octocat/Hello-World.git",
        "加速直连成功时最终 URL 应为改写地址"
    );
}

/// 手工验收 2：加速前缀指向不可达地址 → 自动回退原始 URL 成功且 fell_back = true。
#[test]
#[ignore = "手工验收：需要真实外网（验证加速失败回退 GitHub 原始地址）"]
fn manual_accelerator_fallback_falls_back_to_original_url() {
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let mut req = CloneRequest::new(
        "https://github.com/octocat/Hello-World.git".to_string(),
        dest_root.path().join("hello-world"),
    );
    // 本地不可达端口：改写请求必然快速失败，随后回退原始 URL（需外网可达）
    req.accelerator_prefix = Some("https://127.0.0.1:9/".to_string());
    req.timeout = Duration::from_secs(90);
    req.retries = 1;

    let outcome = clone_repo(req).expect("加速失败应回退原始 URL 并成功");
    println!("[手工验收] 回退克隆成功：head_sha={} fell_back={}", outcome.head_sha, outcome.fell_back);
    assert!(outcome.fell_back, "经回退成功必须标注 fell_back=true");
    assert_eq!(outcome.url_used, "https://github.com/octocat/Hello-World.git");
}

/// 手工验收 3：代理转发克隆（gix_proxy_options 的 custom 模式产物，含 socks5/http）。
#[test]
#[ignore = "手工验收：需要真实外网与本地代理（经 CRAFT_GIT_TEST_PROXY_URL 注入）"]
fn manual_clone_via_proxy() {
    let Ok(proxy_url) = std::env::var("CRAFT_GIT_TEST_PROXY_URL") else {
        panic!("请设置 CRAFT_GIT_TEST_PROXY_URL（如 http://127.0.0.1:7890 或 socks5://127.0.0.1:1080）");
    };
    let dest_root = tempfile::tempdir().expect("创建目标目录失败");
    let mut req = CloneRequest::new(
        "https://github.com/octocat/Hello-World.git".to_string(),
        dest_root.path().join("hello-world"),
    );
    // 对齐 craft-core 的 custom 模式产物：代理 URL + 内置本地排除
    req.proxy = Some(craft_core::network::GixProxyOptions {
        proxy: Some(proxy_url),
        no_proxy: Some(craft_core::network::BUILTIN_NO_PROXY.to_string()),
    });
    req.timeout = Duration::from_secs(90);

    let outcome = clone_repo(req).expect("经代理克隆应成功");
    println!("[手工验收] 代理克隆成功：head_sha={}", outcome.head_sha);
    assert!(!outcome.fell_back);
}
