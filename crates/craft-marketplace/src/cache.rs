//! 版本化缓存布局与 staging 原子激活（spec 第 3 节，票 #4）。
//!
//! # 目录布局（数据目录根由调用方传入，crate 内不硬编码 `~/.craftwork`；
//! `CRAFTWORK_HOME` 隔离由调用方保证）
//!
//! ```text
//! <root>/                          # MarketplaceLayout（<数据目录>/skills-marketplace）
//! ├── marketplaces/<市场id>/       # 市场仓库快照（清单 + 内嵌插件的拷贝来源）
//! └── cache/<市场id>/<插件名>/<版本>/  # 版本化缓存：不可变、多版本并存
//! ```
//!
//! 本模块只做布局、拷贝激活、裁剪与启动清理，**零网络零 git 调用**：下载编排
//! （市场快照 clone、外部插件仓库临时 clone，经 `craft_git::transport`）是后续
//! 票 #5 的事——激活入口 [`activate_plugin_version`] 接收**已就位的来源目录路径**
//! （市场快照内的插件目录或临时 clone 落地目录），来源在激活后被原样保留。
//!
//! # 原子激活（Tauri 单进程简化版）
//!
//! 1. staging 目录（`cache/<市场id>/<插件名>/.<版本>.stage-XXXXXXXX/`，随机后缀）
//!    递归**拷贝**来源完整（激活是拷贝不是 rename：来源可能是市场快照或临时 clone，
//!    必须保留来源）；
//! 2. SQLite 事务内写入 / 更新 `skill_plugins` 行（版本指针指向新版本；写行逻辑由
//!    调用方闭包注入，内部消费 [`crate::store`] 的 `insert_plugin` /
//!    `append_plugin_version`，并返回磁盘保留版本列表）；
//! 3. staging rename 为正式版本目录（目录名 = 版本解析结果——版本解析链是编排层
//!    职责，本模块保证传入的 version 即目录段，缓存路径段 = 已安装清单 = UI 展示
//!    三者一致）；
//! 4. 按事务返回的保留列表裁剪最旧历史版本目录（保留上限 3 由调用方在闭包里传给
//!    `append_plugin_version`）。
//!
//! 任一步失败：staging 清理；事务未提交则指针不变（缓存目录不生效）。裁剪失败
//! **不**使激活失败（指针与新版本目录均已生效，旧目录残留交给启动清理兜底），
//! 失败摘要记入 [`ActivateOutcome::prune_failures`] 供调用方写日志。
//!
//! # 崩溃一致性以 SQLite 指针为权威
//!
//! 单进程无跨进程 transaction marker / owner 存活探测（spec 第 3 节）；「rename 前
//! 崩溃」（事务未提交或已提交但 staging 未提升）留下的 `.stage-*` 与「事务已提交但
//! 提升失败」留下的孤儿版本目录，由启动清理 [`cleanup_cache`] 静默删除：不在插件
//! `versions` 保留列表内的目录、无安装行的插件目录、三层层级的直接隐藏（`.` 开头）
//! 条目。数据库读失败时**只清 staging、跳过孤儿判定**（宁漏勿误）。
//!
//! 同版本重复安装 / 升级为同版本时原地重装：先 staging 拷好，再替换正式目录。
//! 所有新建目录权限 0700（对齐数据目录惯例；非 unix 平台跳过）。

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;

use craft_core::platform::sha256_hex;
use craft_store::db::StoreResult;

use crate::store;

/// 布局路径段（市场 id / 插件名 / 版本目录名）长度上限
///（防超长文件名；semver 与 sha 短码都远小于此值）。
const MAX_SEGMENT_LEN: usize = 128;
/// staging 随机后缀长度（hex 字符数；碰撞概率可忽略，且含进程内单调计数兜底）。
const STAGE_SUFFIX_LEN: usize = 8;
/// staging 独占创建的重试上限（随机段撞车或并发竞争时换名重试）。
const STAGE_NAME_RETRIES: usize = 8;

// ---------- 布局 ----------

/// 商店缓存布局句柄：`skills-marketplace` 根目录（`<数据目录>/skills-marketplace`）
/// 下的两棵子树的路径计算。纯路径计算不触盘；目录创建发生在激活时按需进行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketplaceLayout {
    root: PathBuf,
}

impl MarketplaceLayout {
    /// 以 `skills-marketplace` 根目录构造布局（如 `<CRAFTWORK_HOME>/skills-marketplace`）。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 布局根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 市场快照树根：`<root>/marketplaces`。
    pub fn marketplaces_root(&self) -> PathBuf {
        self.root.join("marketplaces")
    }

    /// 版本化缓存树根：`<root>/cache`。
    pub fn cache_root(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// 市场仓库快照目录：`<root>/marketplaces/<市场id>/`（清单 + 内嵌插件的拷贝来源）。
    /// 目录的创建与内容填充（clone 后落位）由下载编排（票 #5）负责。
    pub fn marketplace_snapshot_dir(&self, marketplace_id: &str) -> PathBuf {
        self.marketplaces_root().join(marketplace_id)
    }

    /// 插件版本缓存目录：`<root>/cache/<市场id>/<插件名>/`（其下是各版本目录）。
    pub fn plugin_cache_dir(&self, marketplace_id: &str, plugin_name: &str) -> PathBuf {
        self.cache_root().join(marketplace_id).join(plugin_name)
    }

    /// 当前版本目录：`<root>/cache/<市场id>/<插件名>/<版本>/`（install_path 落列值）。
    pub fn version_dir(&self, marketplace_id: &str, plugin_name: &str, version: &str) -> PathBuf {
        self.plugin_cache_dir(marketplace_id, plugin_name).join(version)
    }
}

// ---------- 错误 ----------

/// 激活失败。全部变体携带可读中文消息（Display 直出，可经 toast 展示）；
/// 除 [`ActivateError::Promote`] 外，失败时 SQLite 指针未变更、正式版本目录不存在。
#[derive(Debug, Clone, PartialEq)]
pub enum ActivateError {
    /// 市场 id / 插件名 / 版本名不能安全地用作缓存路径段（含具体原因）。
    /// 布局路径段是启动清理孤儿判定的对照基准，`.` 开头等形态会与暂存残留混淆，
    /// 在进入激活前一律拒绝。
    SegmentInvalid { label: String, value: String, reason: String },
    /// 来源目录不存在或不是目录。
    SourceMissing { path: PathBuf },
    /// staging 目录创建 / 分配失败。
    StagePrepare { reason: String },
    /// staging 拷贝中途失败（来源不可读、目标写失败；staging 已清理）。
    StageCopy { path: PathBuf, reason: String },
    /// 来源包含符号链接：商店技能视为不可信内容（spec 第 8 节），拷贝层不跟随
    /// （不给缓存留 symlink 形态、杜绝来源内 symlink 环导致的无限递归）。
    SymlinkRejected { path: PathBuf },
    /// 来源包含非常规文件（fifo / socket 等特殊文件），拒绝安装。
    UnsupportedEntry { path: PathBuf },
    /// SQLite 事务失败：行与指针未变更（事务回滚），staging 已清理。
    TransactionFailed { reason: String },
    /// staging 提升为正式版本目录失败（事务已提交、指针已指向新版本；staging 已清理）。
    /// 该状态与「事务后 rename 前杀进程」时序等价：以指针为权威，磁盘版本缺失由
    /// 重装恢复，读侧扫描对缺失根跳过，不做事务级回滚。
    Promote { target: PathBuf, reason: String },
}

impl fmt::Display for ActivateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SegmentInvalid { label, value, reason } => {
                write!(f, "{label}不能用作缓存路径段：{value:?}（{reason}）")
            }
            Self::SourceMissing { path } => write!(f, "安装来源不存在或不是目录：{}", path.display()),
            Self::StagePrepare { reason } => write!(f, "准备暂存目录失败：{reason}"),
            Self::StageCopy { path, reason } => {
                write!(f, "拷贝插件内容失败：{}（{reason}）", path.display())
            }
            Self::SymlinkRejected { path } => write!(
                f,
                "插件包含符号链接，已拒绝安装：{}（商店技能视为不可信内容，不接受符号链接）",
                path.display()
            ),
            Self::UnsupportedEntry { path } => {
                write!(f, "插件包含不支持的非常规文件，已拒绝安装：{}", path.display())
            }
            Self::TransactionFailed { reason } => {
                write!(f, "写入安装记录失败，本次安装未生效：{reason}")
            }
            Self::Promote { target, reason } => write!(
                f,
                "暂存内容生效失败（暂存已清理）：{}（{reason}）；安装记录已提交，请重装该插件",
                target.display()
            ),
        }
    }
}

impl std::error::Error for ActivateError {}

// ---------- 激活 ----------

/// 一次原子激活的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateOutcome {
    /// 当前版本缓存目录绝对路径（`skill_plugins.install_path` 落列值）。
    pub install_path: PathBuf,
    /// 事务写行闭包返回的磁盘保留版本列表（编排层已据此写 versions 列；此处原样
    /// 透出供审计与断言）。
    pub kept_versions: Vec<String>,
    /// 本次裁剪掉的版本目录名（超出保留上限的最旧历史版本）。
    pub pruned: Vec<String>,
    /// 裁剪失败的条目（可读中文摘要）。激活不因裁剪失败而失败——指针与新版本目录
    /// 均已生效，残留目录由启动清理兜底；调用方据此写日志或投递 error 事件。
    pub prune_failures: Vec<String>,
}

/// 原子激活一个插件版本（spec 第 3 节顺序：staging 拷贝 → 事务写指针 → rename → 裁剪）。
///
/// - `marketplace_id` / `plugin_name`：缓存路径段（市场 id 与插件名，来自清单解析）；
/// - `version`：版本解析结果（编排层按「插件清单 version > 市场条目 version > git sha
///   短码」解析后传入），同时用作缓存目录名；
/// - `source_dir`：已就位的来源目录（市场快照内的插件目录或临时 clone 落地目录），
///   激活是拷贝，来源被原样保留；
/// - `write_row`：SQLite 事务内执行的写行逻辑（新装 [`store::insert_plugin`] / 升级
///   [`store::append_plugin_version`]，第二参数即事务前已确定的版本目录绝对路径，
///   供 install_path 落列），返回磁盘保留版本列表（含新版本与历史版本，旧 → 新）；
///   闭包内不得再取 `Db` 锁（事务已持锁，`lock_conn()` 非重入）。
///
/// 失败语义见 [`ActivateError`]：staging 失败 / 事务失败均不产生正式版本目录；
/// promote 失败时指针已提交（以指针为权威的兜底见模块注释）。
pub fn activate_plugin_version(
    layout: &MarketplaceLayout,
    db: &craft_store::Db,
    marketplace_id: &str,
    plugin_name: &str,
    version: &str,
    source_dir: &Path,
    write_row: impl FnOnce(&Connection, &Path) -> StoreResult<Vec<String>>,
) -> Result<ActivateOutcome, ActivateError> {
    // 0. 前置校验：三个路径段均可安全作目录名；来源目录在场。
    //    失败不触任何磁盘与数据库状态（校验全部发生在副作用之前）。
    ensure_segment("市场 id", marketplace_id)?;
    ensure_segment("插件名", plugin_name)?;
    ensure_segment("版本名", version)?;
    if !source_dir.is_dir() {
        return Err(ActivateError::SourceMissing { path: source_dir.to_path_buf() });
    }
    let plugin_dir = layout.plugin_cache_dir(marketplace_id, plugin_name);
    let target = plugin_dir.join(version);

    // 1. 准备 staging（`.<版本>.stage-XXXXXXXX`）：插件目录按需创建（0700）。
    create_dir_mode_0700(&plugin_dir).map_err(|e| ActivateError::StagePrepare { reason: e })?;
    let staging = allocate_staging_dir(&plugin_dir, version)?;

    // 2. 拷贝来源到 staging（拷贝不是 rename：保留来源）。失败即清 staging 返回。
    if let Err(err) = copy_dir_recursive(source_dir, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }

    // 3. SQLite 事务写指针：失败回滚（指针不变），清 staging，缓存目录不生效。
    let kept_versions = match db.transaction(|conn| write_row(conn, &target)) {
        Ok(kept) => kept,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(ActivateError::TransactionFailed { reason: err.to_string() });
        }
    };

    // 4. staging rename 为正式版本目录；同版本重装（目标已存在）先删旧再落位。
    //    此后指针已指向本版本，任何失败都以指针为权威、不回滚事务。
    if target.exists()
        && let Err(err) = std::fs::remove_dir_all(&target)
    {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(ActivateError::Promote { target, reason: format!("移除旧版本目录失败：{err}") });
    }
    if let Err(err) = std::fs::rename(&staging, &target) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(ActivateError::Promote { target, reason: err.to_string() });
    }

    // 5. 按事务返回的保留列表裁剪最旧历史版本。裁剪失败只记录、不失败（见模块注释）。
    let (pruned, prune_failures) = prune_versions(&plugin_dir, &kept_versions);

    Ok(ActivateOutcome { install_path: target, kept_versions, pruned, prune_failures })
}

/// 校验版本字符串可安全地用作缓存目录段（目录名 = 版本解析结果的前提）：
/// 非空、≤128 字符、不以 `.` 开头（避免与暂存 / 隐藏残留混淆）、不含 `/` 或 `\`
///（防路径穿越）、不含空白与控制字符（semver / sha 短码 / 任意 tag 文本均通过）。
pub fn validate_version_segment(version: &str) -> Result<(), ActivateError> {
    ensure_segment("版本名", version)
}

/// 路径段校验统一入口（市场 id / 插件名 / 版本名共用同一规则与错误变体）。
fn ensure_segment(label: &str, value: &str) -> Result<(), ActivateError> {
    let reject = |reason: String| ActivateError::SegmentInvalid {
        label: label.to_string(),
        value: value.to_string(),
        reason,
    };
    if value.is_empty() {
        return Err(reject("不能为空".to_string()));
    }
    if value.len() > MAX_SEGMENT_LEN {
        return Err(reject(format!("超过 {MAX_SEGMENT_LEN} 字符")));
    }
    if value.starts_with('.') {
        return Err(reject("不能以点开头（会与暂存 / 隐藏残留混淆，启动清理无法区分）".to_string()));
    }
    if value.contains('/') || value.contains('\\') {
        return Err(reject("不能包含路径分隔符".to_string()));
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(reject("不能包含空白或控制字符".to_string()));
    }
    Ok(())
}

// ---------- 启动清理 ----------

/// 启动清理报告（供调用方写日志；静默语义指不打扰用户，不指不留痕）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// 删除的暂存 / 隐藏残留条目（`.stage-*` 等层内直接隐藏项）。
    pub removed_stages: Vec<PathBuf>,
    /// 删除的孤儿条目（不在 versions 保留列表内的版本目录、无安装行的插件目录、
    /// 散落文件）。
    pub removed_orphans: Vec<PathBuf>,
    /// 清理失败摘要（可读中文；权限异常等。残留无害，下轮启动再试）。
    pub failures: Vec<String>,
}

impl CleanupReport {
    /// 是否无任何动作（测试与日志判定用）。
    pub fn is_empty(&self) -> bool {
        self.removed_stages.is_empty() && self.removed_orphans.is_empty() && self.failures.is_empty()
    }
}

/// 应用启动时清理缓存残留（spec 第 3 节：崩溃一致性以 SQLite 指针为权威，缓存自愈）。
///
/// 扫描 `cache/` 三个层级的**直接子项**（cache 根 / 市场层 / 插件层）：
/// - 隐藏条目（`.` 开头：`.stage-*` 暂存与其他隐藏残留）一律删除——staging 只是中间
///   产物，崩溃后永不合法存活，不依赖数据库判定；市场 id / 插件名 / 版本名在激活时
///   已被 [`ensure_segment`] 拒绝 `.` 开头，正常内容不会落入该判定；
/// - 有安装行的插件：不在 `versions` 保留列表内的版本目录与散落文件按孤儿删除；
///   **保留列表内的目录即使磁盘缺失也不补**（rename 前崩溃导致指针指向缺失版本时，
///   读侧以指针为准跳过该根，重装是唯一恢复路径，清理层不做假修复）；
/// - 无安装行的插件（刚卸载 / 市场级联删除后崩溃残留）：整个插件目录按孤儿删除；
/// - 数据库读失败：只清 staging，跳过孤儿判定（宁漏勿误，失败写进报告）。
///
/// 边界：清理只动上述三层的直接子项，**不进入版本目录内部**——插件本体内容可能
/// 含 `.` 开头的目录（如 `.claude-plugin/` 清单目录），那是安装产物不是残留；
/// `marketplaces/` 快照树也不在本函数职责内——市场快照的生命周期（添加时创建、
/// 移除时删除）由市场源编排（票 #5）管理，本函数只自愈 `cache/` 侧。
pub fn cleanup_cache(layout: &MarketplaceLayout, db: &craft_store::Db) -> CleanupReport {
    let mut report = CleanupReport::default();
    let cache_dir = layout.cache_root();
    if !cache_dir.is_dir() {
        return report;
    }

    // 第一遍：层内直接隐藏条目（不依赖数据库；每层只看直接子项，不下探版本目录）
    remove_hidden_entries(&cache_dir, &mut report);

    // 第二遍：孤儿版本目录（依赖数据库保留列表；读失败则跳过本轮孤儿判定）
    let kept = db.with(|conn| {
        store::list_plugins(conn).map(|rows| {
            rows.into_iter()
                .map(|row| {
                    (
                        (row.marketplace_id, row.name),
                        row.versions.into_iter().collect::<HashSet<String>>(),
                    )
                })
                .collect::<HashMap<_, _>>()
        })
    });
    let kept = match kept {
        Ok(map) => map,
        Err(err) => {
            report.failures.push(format!("读取已安装插件失败，本轮跳过孤儿版本清理：{err}"));
            return report;
        }
    };
    remove_orphan_versions(&cache_dir, &kept, &mut report);
    report
}

/// 清理三个层级的直接隐藏子项：cache 根 → 各市场目录 → 各插件目录，各层只删
/// `.` 开头的直接子项，不进入任何非隐藏子目录内部（边界见 [`cleanup_cache`] 注释）。
fn remove_hidden_entries(cache_dir: &Path, report: &mut CleanupReport) {
    remove_hidden_at(cache_dir, report);
    for market in list_dir(cache_dir, report) {
        if market.file_name().to_string_lossy().starts_with('.') {
            continue; // 市场层隐藏项已随上层删除
        }
        let market_dir = market.path();
        if !market_dir.is_dir() {
            continue; // 散落文件是孤儿清理的辖区
        }
        remove_hidden_at(&market_dir, report);
        for plugin in list_dir(&market_dir, report) {
            if plugin.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let plugin_dir = plugin.path();
            if plugin_dir.is_dir() {
                remove_hidden_at(&plugin_dir, report);
            }
        }
    }
}

/// 删除单个层目录下的直接隐藏子项（目录整删、文件直删；失败记录、下轮再试）。
fn remove_hidden_at(dir: &Path, report: &mut CleanupReport) {
    for entry in list_dir(dir, report) {
        if !entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match removed {
            Ok(()) => report.removed_stages.push(path),
            Err(err) => report.failures.push(format!("清理暂存残留失败：{}（{err}）", path.display())),
        }
    }
}

/// 按数据库保留列表清理孤儿版本目录与散落文件（cache 根 / 市场层 / 插件层三层）。
fn remove_orphan_versions(
    cache_dir: &Path,
    kept: &HashMap<(String, String), HashSet<String>>,
    report: &mut CleanupReport,
) {
    for market in list_dir(cache_dir, report) {
        let market_id = market.file_name().to_string_lossy().to_string();
        let market_dir = market.path();
        if !market_dir.is_dir() {
            remove_orphan_entry(&market_dir, report);
            continue;
        }
        for plugin in list_dir(&market_dir, report) {
            let plugin_name = plugin.file_name().to_string_lossy().to_string();
            let plugin_dir = plugin.path();
            if !plugin_dir.is_dir() {
                remove_orphan_entry(&plugin_dir, report);
                continue;
            }
            let Some(kept_versions) = kept.get(&(market_id.clone(), plugin_name)) else {
                // 无安装行：整个插件目录（含其下任何残留）都是孤儿
                match std::fs::remove_dir_all(&plugin_dir) {
                    Ok(()) => report.removed_orphans.push(plugin_dir),
                    Err(err) => report
                        .failures
                        .push(format!("清理无主插件缓存失败：{}（{err}）", plugin_dir.display())),
                }
                continue;
            };
            for version in list_dir(&plugin_dir, report) {
                let name = version.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue; // 隐藏条目已由第一遍清理，跳过避免重复计数
                }
                if kept_versions.contains(&name) {
                    continue; // 保留列表内的版本目录不动（磁盘缺失也不补，见模块注释）
                }
                remove_orphan_entry(&version.path(), report);
            }
        }
    }
}

/// 删除单个孤儿条目（目录整删、文件直删），结果进报告。
fn remove_orphan_entry(path: &Path, report: &mut CleanupReport) {
    let removed = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match removed {
        Ok(()) => report.removed_orphans.push(path.to_path_buf()),
        Err(err) => report.failures.push(format!("清理孤儿缓存失败：{}（{err}）", path.display())),
    }
}

/// 读目录条目；失败写报告返回空（清理尽力而为，不放大成硬错误）。
fn list_dir(dir: &Path, report: &mut CleanupReport) -> Vec<std::fs::DirEntry> {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().collect(),
        Err(err) => {
            report.failures.push(format!("读取缓存目录失败：{}（{err}）", dir.display()));
            Vec::new()
        }
    }
}

// ---------- 内部工具 ----------

/// staging 目录分配：`.<版本>.stage-<8 位 hex>`；独占创建撞车（极小概率）换后缀重试。
fn allocate_staging_dir(plugin_dir: &Path, version: &str) -> Result<PathBuf, ActivateError> {
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..STAGE_NAME_RETRIES {
        let candidate = plugin_dir.join(format!(".{version}.stage-{}", random_suffix()));
        // create_dir（非 create_dir_all）：父目录已建；失败视为撞车换名重试
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(err) => last_err = Some(err),
        }
    }
    Err(ActivateError::StagePrepare {
        reason: format!(
            "暂存目录创建重试 {STAGE_NAME_RETRIES} 次仍失败：{}",
            last_err.map_or_else(|| "未知原因".to_string(), |e| e.to_string())
        ),
    })
}

/// 随机后缀：pid + 进程内单调计数 + 纳秒时间戳的 SHA-256 前 8 位 hex
/// （复用 craft-core 的 sha256_hex，不引入新依赖；进程内计数保证同进程内唯一）。
/// 编排层（orchestrator）的临时目录命名共用同一实现。
pub(crate) fn random_suffix() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let material = format!("{}-{seq}-{nanos}", std::process::id());
    sha256_hex(material.as_bytes()).chars().take(STAGE_SUFFIX_LEN).collect()
}

/// 递归拷贝目录（staging 准备）：目录一律 0700；文件按内容拷贝（`fs::copy` 保留
/// 来源权限位）；符号链接与非常规文件（fifo / socket 等）拒绝——见
/// [`ActivateError::SymlinkRejected`] / [`ActivateError::UnsupportedEntry`]。
/// 编排层（orchestrator）组装安装载荷（声明技能路径 / 清单副本）共用同一实现：
/// 不可信内容不接受符号链接的信任边界在拷贝入口一次性兜住。
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), ActivateError> {
    std::fs::create_dir_all(dst)
        .map_err(|e| ActivateError::StageCopy { path: dst.to_path_buf(), reason: e.to_string() })?;
    set_dir_mode_0700(dst);
    let entries = std::fs::read_dir(src)
        .map_err(|e| ActivateError::StageCopy { path: src.to_path_buf(), reason: e.to_string() })?;
    for entry in entries {
        let entry = entry.map_err(|e| ActivateError::StageCopy { path: src.to_path_buf(), reason: e.to_string() })?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        // symlink_metadata 不跟随链接：symlink 一律拒绝（信任边界，见错误变体注释）
        let meta = std::fs::symlink_metadata(&from)
            .map_err(|e| ActivateError::StageCopy { path: from.clone(), reason: e.to_string() })?;
        if meta.file_type().is_symlink() {
            return Err(ActivateError::SymlinkRejected { path: from });
        }
        if meta.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if meta.is_file() {
            std::fs::copy(&from, &to)
                .map_err(|e| ActivateError::StageCopy { path: from.clone(), reason: e.to_string() })?;
        } else {
            return Err(ActivateError::UnsupportedEntry { path: from });
        }
    }
    Ok(())
}

/// 按保留列表裁剪插件目录下的历史版本目录；返回（已裁剪名，失败摘要）。
/// 隐藏条目（`.` 开头）跳过——那是启动清理的辖区；保留列表内的名与不在磁盘上的名忽略。
fn prune_versions(plugin_dir: &Path, kept: &[String]) -> (Vec<String>, Vec<String>) {
    let kept_set: HashSet<&str> = kept.iter().map(String::as_str).collect();
    let mut pruned = Vec::new();
    let mut failures = Vec::new();
    let entries = match std::fs::read_dir(plugin_dir) {
        Ok(entries) => entries,
        Err(err) => {
            failures.push(format!("读取版本目录失败：{}（{err}）", plugin_dir.display()));
            return (pruned, failures);
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || kept_set.contains(name.as_str()) {
            continue;
        }
        let path = plugin_dir.join(&name);
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => pruned.push(name),
            Err(err) => failures.push(format!("裁剪历史版本失败：{}（{err}）", path.display())),
        }
    }
    (pruned, failures)
}

/// 建目录并设 0700（对齐数据目录惯例；非 unix 平台跳过权限设置）。
/// 逐级创建而非 create_dir_all：中间层（cache 根、市场 id 层）也不能落 umask 默认
/// 权限，整棵缓存树必须 0700。
fn create_dir_mode_0700(dir: &Path) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut prefix = dir.to_path_buf();
    while !prefix.exists() {
        missing.push(prefix.clone());
        match prefix.parent() {
            Some(parent) => prefix = parent.to_path_buf(),
            None => break,
        }
    }
    for path in missing.into_iter().rev() {
        std::fs::create_dir(&path).map_err(|e| format!("{}（{e}）", path.display()))?;
        set_dir_mode_0700(&path);
    }
    Ok(())
}

#[cfg(unix)]
fn set_dir_mode_0700(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_dir: &Path) {}
