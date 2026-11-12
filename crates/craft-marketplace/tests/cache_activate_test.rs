//! 票 #4 验收集成测试：版本化缓存布局 + staging 原子激活 + 版本裁剪 + 启动清理。
//!
//! 覆盖场景（票验收清单）：
//! 1. 激活成功路径（拷贝保留来源、行与目录一致、0700 权限）；
//! 2. staging 失败回滚（来源缺失 / 符号链接拒绝，事务不被触碰）；
//! 3. 指针事务失败后目录不生效（指针不变、staging 清理）；
//! 4. 裁剪保留最近 3 版；同版本原地重装；
//! 5. 启动清理残留（staging / 孤儿版本 / 无主插件目录 / 散落文件）；
//! 6. 「rename 前崩溃」与「事务后 rename 前杀进程」两种时序模拟（测试内直接构造
//!    状态，断言读侧以 SQLite 指针为权威、清理自愈不误删）。
//!
//! 下载编排（clone）不在本票范围：所有激活都直接喂「已就位的来源目录」。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rusqlite::Connection;

use craft_marketplace::cache::{self, ActivateError, MarketplaceLayout};
use craft_marketplace::store;
use craft_store::db::{StoreError, StoreResult};
use craft_store::Db;

// ---------- 测试辅助 ----------

/// 唯一临时目录（隔离：每个测试独立的 skills-marketplace 根与来源根）。
fn tmp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cw-cache-test-{tag}-{}-{n}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录失败");
    dir
}

/// 在目录内写一批相对路径文本文件（含中间目录创建）。
fn write_tree(root: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("创建 fixture 目录失败");
        }
        std::fs::write(&path, content).expect("写入 fixture 文件失败");
    }
}

/// 读文本文件（断言版本目录内容用）。
fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path).expect("读取文件失败")
}

/// 目录下的全部条目名（排序；read_dir 天然含隐藏条目，断言 staging 无残留也用它）。
fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("读取目录失败")
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// 添加市场源行（skill_plugins 外键依赖市场行存在）。
fn add_market(db: &Db, id: &str) {
    let conn = db.lock_conn();
    store::add_marketplace(
        &conn,
        store::NewMarketplace { id, source: id, repo_url: "https://github.com/example/repo" },
    )
    .expect("添加市场源失败");
}

/// 写行闭包统一类型（分支选择安装 / 升级闭包时需要同一具体类型，装箱消歧）。
type RowWriter = Box<dyn FnOnce(&Connection, &Path) -> StoreResult<Vec<String>>>;

/// 新装写行闭包（编排层的 install 分支）：insert_plugin 行 + 返回保留列表 [version]。
/// 全 owned 捕获，避免生命周期噪音。
fn install_row(plugin_id: String, marketplace_id: String, name: String, version: String) -> RowWriter {
    Box::new(move |conn, install_path| {
        store::insert_plugin(
            conn,
            store::NewPlugin {
                id: &plugin_id,
                marketplace_id: &marketplace_id,
                name: &name,
                current_version: &version,
                versions: std::slice::from_ref(&version),
                install_path: &install_path.to_string_lossy(),
                source_kind: store::source_kind::ENTRY_RELATIVE,
                source_ref: "./",
                source_sha: Some("abc1234"),
            },
        )?;
        Ok(vec![version.clone()])
    })
}

/// 升级写行闭包（编排层的 update 分支）：append_plugin_version 追加 + 切指针 + 裁剪，
/// 返回磁盘保留列表。同版本重复安装也走它（append 幂等）。
fn upgrade_row(plugin_id: String, version: String, keep_last: usize) -> RowWriter {
    Box::new(move |conn, _install_path| store::append_plugin_version(conn, &plugin_id, &version, keep_last))
}

/// 恒失败写行闭包（模拟事务内失败：指针不落、行不落）。
fn failing_row() -> RowWriter {
    Box::new(|_conn, _install_path| Err(StoreError::Other("模拟事务失败".to_string())))
}

/// 读当前版本指针。
fn current_version(db: &Db, plugin_id: &str) -> String {
    let conn = db.lock_conn();
    store::get_plugin(&conn, plugin_id).expect("读取插件行失败").current_version
}

/// 读保留版本列表。
fn kept_versions(db: &Db, plugin_id: &str) -> Vec<String> {
    let conn = db.lock_conn();
    store::get_plugin(&conn, plugin_id).expect("读取插件行失败").versions
}

#[cfg(unix)]
fn dir_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// ---------- 1. 激活成功路径 ----------

#[test]
fn activate_success_copies_source_writes_row_and_keeps_source_intact() {
    let root = tmp_dir("ok");
    let source_root = tmp_dir("ok-source");
    write_tree(
        &source_root,
        &[
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文 1.0.0"),
            (".claude-plugin/plugin.json", r#"{"name":"demo","version":"1.0.0"}"#),
        ],
    );
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    let outcome = cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("激活应成功");

    // 缓存路径段 = 版本：目录在版本目录名下，内容完整拷贝
    let version_dir = layout.version_dir("skills", "demo", "1.0.0");
    assert_eq!(outcome.install_path, version_dir);
    assert_eq!(read_text(&version_dir.join("skills/demo/SKILL.md")), "---\nname: demo\n---\n正文 1.0.0");
    assert!(version_dir.join(".claude-plugin/plugin.json").is_file(), "插件清单副本随拷贝保留");

    // 激活是拷贝不是 rename：来源原样保留
    assert!(source_root.join("skills/demo/SKILL.md").is_file(), "来源目录必须被保留");

    // staging 无残留：插件目录下只有正式版本目录
    assert_eq!(entry_names(&layout.plugin_cache_dir("skills", "demo")), vec!["1.0.0".to_string()]);

    // kept / pruned 语义
    assert_eq!(outcome.kept_versions, vec!["1.0.0".to_string()]);
    assert!(outcome.pruned.is_empty());
    assert!(outcome.prune_failures.is_empty());

    // SQLite 行与磁盘一致（install_path 指向新目录，指针指向新版本）
    let row = {
        let conn = db.lock_conn();
        store::get_plugin(&conn, &pid).expect("安装行应存在")
    };
    assert_eq!(row.current_version, "1.0.0");
    assert_eq!(row.versions, vec!["1.0.0".to_string()]);
    assert_eq!(row.install_path, version_dir.to_string_lossy());
    assert!(row.enabled, "新安装默认启用");

    // 目录权限 0700（对齐数据目录惯例）
    #[cfg(unix)]
    {
        assert_eq!(dir_mode(&layout.cache_root()), 0o700, "cache 根 0700");
        assert_eq!(dir_mode(&layout.plugin_cache_dir("skills", "demo")), 0o700, "插件缓存目录 0700");
        assert_eq!(dir_mode(&version_dir), 0o700, "版本目录 0700");
    }

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 2. 路径段安全：非法版本 / 市场 id / 插件名 ----------

#[test]
fn activate_rejects_unsafe_path_segments_without_disk_or_db_side_effects() {
    let root = tmp_dir("seg");
    let source_root = tmp_dir("seg-source");
    write_tree(&source_root, &[("skills/demo/SKILL.md", "正文")]);
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");

    // 直接校验：穿越、分隔符、点开头（与 staging 命名冲突）、空白、空串
    for bad in ["../evil", "a/b", "a\\b", ".hidden", ".1.0.0.stage-x", "", "  ", "a\tb", "a\nb", "."] {
        let err = cache::validate_version_segment(bad).expect_err(&format!("版本 {bad:?} 应被拒"));
        assert!(
            matches!(err, ActivateError::SegmentInvalid { .. }),
            "版本 {bad:?} 应报 SegmentInvalid，实际 {err:?}"
        );
        assert!(!err.to_string().is_empty(), "错误消息可读");
    }
    for good in ["1.0.0", "9d3f2a1", "5.3.2-beta.1+build", "中文版本", "release_2026"] {
        cache::validate_version_segment(good).unwrap_or_else(|e| panic!("版本 {good:?} 应通过：{e}"));
    }

    // 经激活入口拒绝（版本非法）：磁盘无插件目录、数据库无行
    let pid = store::plugin_id("demo", "skills");
    for bad in ["../evil", "a/b", ".hidden"] {
        let result = cache::activate_plugin_version(
            &layout,
            &db,
            "skills",
            "demo",
            bad,
            &source_root,
            install_row(pid.clone(), "skills".into(), "demo".into(), bad.to_string()),
        );
        assert!(matches!(result, Err(ActivateError::SegmentInvalid { .. })), "版本 {bad:?} 应被拒");
    }
    assert!(
        !layout.plugin_cache_dir("skills", "demo").exists(),
        "校验失败不应创建插件缓存目录"
    );
    {
        let conn = db.lock_conn();
        assert!(store::list_plugins(&conn).unwrap().is_empty(), "校验失败不应写安装行");
    }

    // 市场 id / 插件名同样拒绝点开头（启动清理会把点开头条目当残留，激活入口必须拦截）
    for (market, name) in [(".evil", "demo"), ("skills", ".evil")] {
        let result = cache::activate_plugin_version(
            &layout,
            &db,
            market,
            name,
            "1.0.0",
            &source_root,
            install_row(store::plugin_id(name, market), market.to_string(), name.to_string(), "1.0.0".into()),
        );
        assert!(matches!(result, Err(ActivateError::SegmentInvalid { .. })), "{market}/{name} 应被拒");
    }

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 3. staging 失败回滚（来源缺失 / 符号链接） ----------

#[test]
fn activate_missing_source_fails_without_side_effects() {
    let root = tmp_dir("missing-src");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let missing = root.join("no-such-source");

    let mut write_called = false;
    let result = cache::activate_plugin_version(&layout, &db, "skills", "demo", "1.0.0", &missing, |_conn, _p| {
        write_called = true;
        Ok(vec![])
    });
    assert!(
        matches!(&result, Err(ActivateError::SourceMissing { path }) if path == &missing),
        "应报 SourceMissing 且携带来源路径，实际 {result:?}"
    );
    assert!(!write_called, "来源校验失败不应进入事务写行");
    assert!(!layout.cache_root().exists(), "未进入拷贝阶段不应创建缓存树");
    {
        let conn = db.lock_conn();
        assert!(store::list_plugins(&conn).unwrap().is_empty());
    }

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn activate_symlink_source_rejects_and_cleans_staging_before_transaction() {
    let root = tmp_dir("symlink");
    let source_root = tmp_dir("symlink-source");
    write_tree(
        &source_root,
        &[
            ("skills/demo/SKILL.md", "正文"),
            ("outside/secret.txt", "不该被拷出的内容"),
        ],
    );
    // 来源内含符号链接（指向来源外）：拷贝层必须拒绝
    std::os::unix::fs::symlink(source_root.join("outside/secret.txt"), source_root.join("skills/linked"))
        .expect("创建符号链接失败");

    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");

    let mut write_called = false;
    let result = cache::activate_plugin_version(&layout, &db, "skills", "demo", "1.0.0", &source_root, |_conn, _p| {
        write_called = true;
        Ok(vec![])
    });
    match &result {
        Err(ActivateError::SymlinkRejected { path }) => {
            assert!(path.ends_with("skills/linked"), "应指向符号链接本身：{}", path.display())
        }
        other => panic!("应报 SymlinkRejected，实际 {other:?}"),
    }
    assert!(!write_called, "拷贝阶段失败不应进入事务写行");

    // staging 已清理、无正式版本目录、无安装行
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    assert_eq!(entry_names(&plugin_dir), Vec::<String>::new(), "staging 应被清空回滚");
    {
        let conn = db.lock_conn();
        assert!(store::list_plugins(&conn).unwrap().is_empty(), "拷贝失败不应写安装行");
    }

    // 来源不被破坏：符号链接本体与目标文件原样保留
    assert!(
        source_root.join("skills/linked").symlink_metadata().unwrap().file_type().is_symlink(),
        "来源中的符号链接应原样保留"
    );
    assert_eq!(read_text(&source_root.join("outside/secret.txt")), "不该被拷出的内容");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

/// Windows 等价（无真机，CI 的 windows runner 是唯一验证场）。
///
/// 用真正的 NTFS 文件 symlink（需要管理员或开发者模式特权，GitHub runner 具备）；
/// junction 不适用——std 的 `FileType::is_symlink` 对 junction 返回 false，
/// 拷贝层的 symlink 拒绝无法用同一语义识别它。无特权环境打印原因后跳过。
#[cfg(windows)]
#[test]
fn activate_symlink_source_rejects_and_cleans_staging_before_transaction_windows() {
    let root = tmp_dir("symlink");
    let source_root = tmp_dir("symlink-source");
    write_tree(
        &source_root,
        &[
            ("skills/demo/SKILL.md", "正文"),
            ("outside/secret.txt", "不该被拷出的内容"),
        ],
    );
    if let Err(err) = std::os::windows::fs::symlink_file(
        source_root.join("outside/secret.txt"),
        source_root.join("skills/linked"),
    ) {
        eprintln!("跳过：创建 symlink 失败（需要特权）：{err}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&source_root);
        return;
    }

    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");

    let mut write_called = false;
    let result = cache::activate_plugin_version(&layout, &db, "skills", "demo", "1.0.0", &source_root, |_conn, _p| {
        write_called = true;
        Ok(vec![])
    });
    match &result {
        Err(ActivateError::SymlinkRejected { path }) => {
            assert!(path.ends_with("skills\\linked") || path.ends_with("skills/linked"), "应指向符号链接本身：{}", path.display())
        }
        other => panic!("应报 SymlinkRejected，实际 {other:?}"),
    }
    assert!(!write_called, "拷贝阶段失败不应进入事务写行");

    // staging 已清理、无正式版本目录、无安装行
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    assert_eq!(entry_names(&plugin_dir), Vec::<String>::new(), "staging 应被清空回滚");
    {
        let conn = db.lock_conn();
        assert!(store::list_plugins(&conn).unwrap().is_empty(), "拷贝失败不应写安装行");
    }

    // 来源不被破坏：符号链接本体与目标文件原样保留
    assert!(
        source_root.join("skills/linked").symlink_metadata().unwrap().file_type().is_symlink(),
        "来源中的符号链接应原样保留"
    );
    assert_eq!(read_text(&source_root.join("outside/secret.txt")), "不该被拷出的内容");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 4. 事务失败：指针不变、目录不生效 ----------

#[test]
fn activate_transaction_failure_rolls_back_pointer_and_clears_staging() {
    let root = tmp_dir("tx-fail");
    let source_root = tmp_dir("tx-fail-source");
    write_tree(&source_root, &[("skills/demo/SKILL.md", "正文")]);
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    // 先成功安装 1.0.0
    cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("首次安装应成功");

    // 升级 2.0.0：事务内写行失败 → staging 清理、指针不变、2.0.0 目录不存在
    let result = cache::activate_plugin_version(&layout, &db, "skills", "demo", "2.0.0", &source_root, failing_row());
    match &result {
        Err(ActivateError::TransactionFailed { reason }) => {
            assert!(reason.contains("模拟事务失败"), "错误应携带事务失败原因：{reason}")
        }
        other => panic!("应报 TransactionFailed，实际 {other:?}"),
    }
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    assert_eq!(entry_names(&plugin_dir), vec!["1.0.0".to_string()], "只应剩旧版本目录、staging 无残留");
    assert!(!plugin_dir.join("2.0.0").exists(), "事务失败不应产生正式版本目录");
    assert_eq!(current_version(&db, &pid), "1.0.0", "事务失败指针不变");
    assert_eq!(kept_versions(&db, &pid), vec!["1.0.0".to_string()]);

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 5. 裁剪保留最近 3 版 ----------

#[test]
fn activate_prunes_oldest_versions_beyond_keep_last_three() {
    let root = tmp_dir("prune");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    let mut last_pruned = Vec::new();
    for version in ["1.0.0", "2.0.0", "3.0.0", "4.0.0"] {
        let source_root = tmp_dir(&format!("prune-src-{version}"));
        let content = format!("正文 {version}");
        write_tree(&source_root, &[("skills/demo/SKILL.md", content.as_str())]);
        let row = if version == "1.0.0" {
            install_row(pid.clone(), "skills".into(), "demo".into(), version.to_string())
        } else {
            upgrade_row(pid.clone(), version.to_string(), 3)
        };
        let outcome = cache::activate_plugin_version(&layout, &db, "skills", "demo", version, &source_root, row)
            .unwrap_or_else(|e| panic!("升级到 {version} 应成功：{e}"));
        last_pruned = outcome.pruned;
        // 每个版本目录内容可区分
        assert_eq!(
            read_text(&plugin_dir.join(version).join("skills/demo/SKILL.md")),
            format!("正文 {version}"),
            "版本 {version} 目录内容正确"
        );
        let _ = std::fs::remove_dir_all(&source_root);
    }

    // 磁盘只保留最近 3 版（当前 + 2 历史），最旧版本被裁剪
    assert_eq!(entry_names(&plugin_dir), vec!["2.0.0".to_string(), "3.0.0".to_string(), "4.0.0".to_string()]);
    assert_eq!(last_pruned, vec!["1.0.0".to_string()], "最后一步裁掉的应是最旧的 1.0.0");
    assert_eq!(kept_versions(&db, &pid), vec!["2.0.0".to_string(), "3.0.0".to_string(), "4.0.0".to_string()]);
    assert_eq!(current_version(&db, &pid), "4.0.0");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- 6. 同版本原地重装 ----------

#[test]
fn activate_same_version_reinstall_replaces_dir_in_place() {
    let root = tmp_dir("reinstall");
    let source_root = tmp_dir("reinstall-source");
    write_tree(&source_root, &[("skills/demo/SKILL.md", "旧内容")]);
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    let version_dir = plugin_dir.join("1.0.0");

    // 首装 + 手动放置一个保留列表外的历史版本目录（制造重装前的多版本状态）
    cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("首装应成功");
    let stale = plugin_dir.join("0.9.0");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("marker"), "stale").unwrap();

    // 同版本重装：先改来源内容，再走 append 幂等写行
    std::fs::write(source_root.join("skills/demo/SKILL.md"), "新内容").unwrap();
    let outcome = cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        upgrade_row(pid.clone(), "1.0.0".into(), 3),
    )
    .expect("同版本重装应成功");

    // 目录被替换为新内容（先 staging 再替换，不是 no-op）
    assert_eq!(read_text(&version_dir.join("skills/demo/SKILL.md")), "新内容", "重装应替换目录内容");
    // append 幂等：versions 不产生重复项；保留列表外的 0.9.0 被裁剪
    assert_eq!(outcome.kept_versions, vec!["1.0.0".to_string()]);
    assert_eq!(kept_versions(&db, &pid), vec!["1.0.0".to_string()]);
    assert_eq!(current_version(&db, &pid), "1.0.0");
    assert_eq!(outcome.pruned, vec!["0.9.0".to_string()], "重装同时裁掉保留列表外的旧版本目录");
    assert_eq!(entry_names(&plugin_dir), vec!["1.0.0".to_string()], "重装不留 staging 与多余目录");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 7. 启动清理：残留自愈 ----------

#[test]
fn startup_cleanup_removes_stage_residue_orphan_versions_and_stray_files() {
    let root = tmp_dir("cleanup");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    let source_root = tmp_dir("cleanup-source");
    write_tree(&source_root, &[("skills/demo/SKILL.md", "正文")]);
    cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("首装应成功");

    // 直接构造崩溃残留状态：staging 残留、孤儿版本目录、散落文件
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    let stage = plugin_dir.join(".0.9.0.stage-deadbeef");
    std::fs::create_dir_all(stage.join("skills/demo")).unwrap();
    std::fs::write(stage.join("skills/demo/SKILL.md"), "半成品").unwrap();
    let orphan = plugin_dir.join("0.5.0");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("marker"), "orphan").unwrap();
    let stray = plugin_dir.join("stray.txt");
    std::fs::write(&stray, "散落文件").unwrap();

    let report = cache::cleanup_cache(&layout, &db);
    assert!(
        report.removed_stages.contains(&stage),
        "staging 残留应被清理：{:?}",
        report.removed_stages
    );
    assert!(
        report.removed_orphans.contains(&orphan) && report.removed_orphans.contains(&stray),
        "孤儿版本与散落文件应被清理：{:?}",
        report.removed_orphans
    );
    assert!(report.failures.is_empty(), "清理不应有失败：{:?}", report.failures);

    // 保留列表内的版本目录不动；指针不变
    assert!(plugin_dir.join("1.0.0").is_dir(), "保留版本不应被误删");
    assert_eq!(entry_names(&plugin_dir), vec!["1.0.0".to_string()]);
    assert_eq!(current_version(&db, &pid), "1.0.0");

    // 再次清理：幂等（无任何动作）
    let second = cache::cleanup_cache(&layout, &db);
    assert!(second.is_empty(), "清理应幂等：{second:?}");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 8. 崩溃时序模拟（读侧以指针为权威） ----------

/// 时序 A「rename 前崩溃」（事务未提交）：staging 拷贝完成但指针仍指旧版本、versions
/// 不含新版本时进程即死。启动清理应删 staging；旧指针与旧版本目录完好无损。
#[test]
fn crash_before_rename_uncommitted_transaction_keeps_old_pointer_and_clears_stage() {
    let root = tmp_dir("crash-before");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    let source_root = tmp_dir("crash-before-src");
    write_tree(&source_root, &[("skills/demo/SKILL.md", "正文")]);
    cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("首装应成功");

    // 直接构造「rename 前崩溃」状态：staging（内容完整）在、指针仍在 1.0.0
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    let stage = plugin_dir.join(".2.0.0.stage-cafe1234");
    std::fs::create_dir_all(stage.join("skills/demo")).unwrap();
    std::fs::write(stage.join("skills/demo/SKILL.md"), "新版本内容").unwrap();
    assert_eq!(current_version(&db, &pid), "1.0.0", "事务未提交，指针应仍是旧版本");

    let report = cache::cleanup_cache(&layout, &db);
    assert!(report.removed_stages.contains(&stage), "崩溃残留 staging 应被清理：{:?}", report.removed_stages);
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    // 读侧以指针为权威：指针与保留列表不变，旧版本目录完好，新版本目录从未出现
    assert_eq!(current_version(&db, &pid), "1.0.0");
    assert_eq!(kept_versions(&db, &pid), vec!["1.0.0".to_string()]);
    assert_eq!(entry_names(&plugin_dir), vec!["1.0.0".to_string()]);
    assert!(!plugin_dir.join("2.0.0").exists());

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

/// 时序 B「事务后 rename 前杀进程」：SQLite 指针已提交到新版本（versions 已含新版本）
/// 但 staging 尚未 rename 正式版本目录。启动清理应删 staging、不误删任何保留版本；
/// 读侧以指针为权威——指针保持新版本（该版本目录缺失由重装恢复，清理层不做假修复）。
#[test]
fn crash_after_commit_before_rename_keeps_pointer_authoritative_and_clears_stage() {
    let root = tmp_dir("crash-after");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    let source_root = tmp_dir("crash-after-src");
    write_tree(&source_root, &[("skills/demo/SKILL.md", "正文")]);
    cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("首装应成功");

    // 直接构造「事务已提交、rename 前杀进程」状态：指针已到 2.0.0（versions 含它），
    // 磁盘只有 staging 与旧版本目录
    {
        let conn = db.lock_conn();
        store::append_plugin_version(&conn, &pid, "2.0.0", 3).expect("提交事务失败");
    }
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    let stage = plugin_dir.join(".2.0.0.stage-beef5678");
    std::fs::create_dir_all(stage.join("skills/demo")).unwrap();
    std::fs::write(stage.join("skills/demo/SKILL.md"), "新版本内容").unwrap();
    assert_eq!(current_version(&db, &pid), "2.0.0");
    assert!(!plugin_dir.join("2.0.0").exists(), "构造状态：正式版本目录尚未产生");

    let report = cache::cleanup_cache(&layout, &db);
    assert!(report.removed_stages.contains(&stage), "staging 应被清理：{:?}", report.removed_stages);
    assert!(
        report.removed_orphans.is_empty(),
        "保留列表内的版本不应被当孤儿误删：{:?}",
        report.removed_orphans
    );
    assert!(report.failures.is_empty(), "{:?}", report.failures);

    // 读侧以指针为权威：指针保持 2.0.0（不回退、不假修复），1.0.0 目录保留
    assert_eq!(current_version(&db, &pid), "2.0.0", "指针已提交即权威，清理不回滚指针");
    assert_eq!(kept_versions(&db, &pid), vec!["1.0.0".to_string(), "2.0.0".to_string()]);
    assert!(plugin_dir.join("1.0.0").is_dir(), "历史版本目录应保留");
    assert_eq!(entry_names(&plugin_dir), vec!["1.0.0".to_string()]);

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

// ---------- 9. 清理边界 ----------

#[test]
fn startup_cleanup_removes_rowless_plugin_dirs_but_never_touches_kept_version_interior() {
    let root = tmp_dir("cleanup-boundary");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");
    add_market(&db, "skills");
    let pid = store::plugin_id("demo", "skills");

    // 有安装行的插件：版本目录内部含点开头内容（如 .claude-plugin 清单目录）
    let source_root = tmp_dir("cleanup-boundary-src");
    write_tree(
        &source_root,
        &[
            ("skills/demo/SKILL.md", "正文"),
            (".claude-plugin/plugin.json", r#"{"name":"demo"}"#),
            ("skills/demo/.hidden-asset.txt", "技能本体资源"),
        ],
    );
    cache::activate_plugin_version(
        &layout,
        &db,
        "skills",
        "demo",
        "1.0.0",
        &source_root,
        install_row(pid.clone(), "skills".into(), "demo".into(), "1.0.0".into()),
    )
    .expect("首装应成功");

    // 无安装行的插件目录（卸载 / 级联删除后崩溃残留）
    let rowless = layout.plugin_cache_dir("skills", "ghost");
    std::fs::create_dir_all(rowless.join("1.0.0")).unwrap();
    std::fs::write(rowless.join("1.0.0/marker"), "残留").unwrap();

    let report = cache::cleanup_cache(&layout, &db);
    assert!(report.removed_orphans.contains(&rowless), "无主插件目录应整删：{:?}", report.removed_orphans);
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert!(!rowless.exists());

    // 版本目录内部属于插件本体：点开头内容（.claude-plugin/、技能内隐藏资源）绝不被清理误删
    let version_dir = layout.version_dir("skills", "demo", "1.0.0");
    assert!(version_dir.join(".claude-plugin/plugin.json").is_file(), "插件清单副本不应被启动清理误删");
    assert!(version_dir.join("skills/demo/.hidden-asset.txt").is_file(), "版本目录内部隐藏文件不应被误删");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&source_root);
}

/// 数据库读失败时清理只清 staging、跳过孤儿判定（宁漏勿误）。
/// 用「versions 列损坏」（store 层读行显式报错的真实路径）模拟数据库读失败。
#[test]
fn cleanup_with_unreadable_db_only_clears_stages_and_skips_orphan_judgement() {
    let root = tmp_dir("cleanup-db-fail");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db_path = root.join("craftwork.db");

    // 建表 → 关连接 → 外部把 versions 列写坏（not-json，store 层读行必报错）→ 重开
    {
        let db = Db::open(&db_path).expect("打开测试库失败");
        let conn = db.lock_conn();
        store::add_marketplace(
            &conn,
            store::NewMarketplace { id: "skills", source: "skills", repo_url: "https://github.com/example/repo" },
        )
        .unwrap();
        drop(conn);
        drop(db);
    }
    {
        let raw = Connection::open(&db_path).expect("打开原始连接失败");
        raw.execute(
            "INSERT INTO skill_plugins (id, marketplace_id, name, current_version, versions, install_path, source_kind, source_ref, enabled, installed_at, updated_at)
             VALUES ('demo@skills', 'skills', 'demo', '1.0.0', 'not-json', '/x', 'entry-relative', './', 1, 1, 1)",
            [],
        )
        .expect("写入坏行失败");
    }
    let db = Db::open(&db_path).expect("重新打开测试库失败");

    // 手工构造残留：staging + 孤儿版本
    let plugin_dir = layout.plugin_cache_dir("skills", "demo");
    let stage = plugin_dir.join(".1.0.0.stage-abc00001");
    std::fs::create_dir_all(&stage).unwrap();
    let orphan = plugin_dir.join("1.0.0");
    std::fs::create_dir_all(&orphan).unwrap();

    let report = cache::cleanup_cache(&layout, &db);
    assert!(report.removed_stages.contains(&stage), "staging 清理不依赖数据库：{:?}", report.removed_stages);
    assert!(report.removed_orphans.is_empty(), "数据库读失败时不应做孤儿判定：{:?}", report.removed_orphans);
    assert!(!report.failures.is_empty(), "读失败应写进报告 failures：{:?}", report.failures);
    assert!(orphan.exists(), "孤儿目录在无数据库依据时应保留（宁漏勿误）");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
}

/// cache 目录不存在时清理为 no-op（首次启动等场景）。
#[test]
fn cleanup_without_cache_directory_reports_empty() {
    let root = tmp_dir("cleanup-empty");
    let layout = MarketplaceLayout::new(root.join("skills-marketplace"));
    let db = Db::open(&root.join("craftwork.db")).expect("打开测试库失败");

    let report = cache::cleanup_cache(&layout, &db);
    assert!(report.is_empty(), "无缓存目录应空报告：{report:?}");

    drop(db);
    let _ = std::fs::remove_dir_all(&root);
}
