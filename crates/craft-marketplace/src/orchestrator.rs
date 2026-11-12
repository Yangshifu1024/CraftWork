//! 商店核心编排（spec 第 2 / 4 / 6 节，票 #5）：市场刷新、安装、升级、回滚、卸载、
//! 插件启停与更新检测取数，把 [`crate::manifest`]（清单解析）、[`crate::cache`]（原子
//! 激活）、[`crate::store`]（两张表）、[`crate::audit`]（审计）与 craft-git 传输串成
//! 完整动作。
//!
//! # 职责边界
//!
//! - 本模块是 crate 内唯一接触网络与 git 的位置：市场快照刷新与外部插件仓库下载经
//!   `craft_git::transport`（浅克隆 depth-1、超时重试、加速回退均在传输层完成）；
//!   代理选项与加速前缀经 [`NetworkOptions`] 由调用方从 `craft-core::network::ProxyConfig`
//!   解析后传入（`gix_proxy_options()` 产物 + 加速前缀），本模块不读设置。
//! - 磁盘生命周期：临时目录（clone 落地、安装载荷组装）一律放在布局根
//!   `skills-marketplace/.tmp/` 下（与 `marketplaces/` 同文件系统，rename 替换快照
//!   不跨设备），RAII [`TempDirGuard`] 兜底清理；崩溃残留由 [`cleanup_tmp`] 在应用
//!   启动时整树清除。市场快照与版本化缓存的正式目录仍由 [`crate::cache`] 语义管理。
//! - 快照仓库的 HEAD 短码读取（[`read_snapshot_head_sha`]）是纯文本读取：craft-git
//!   的 engine 未暴露 rev-parse 门面（本票不改 craft-git），而浅克隆产物的引用恒为
//!   松散文件，`packed-refs` 兜底即可覆盖。该 sha 用于版本解析的末级回退（spec 第 4 节：
//!   插件清单 version > 市场条目 version > git sha 短码）与 source_sha pin。
//!
//! # 互斥（spec 第 6 节）
//!
//! 安装 / 升级 / 回滚 / 卸载 / 市场刷新共享 [`with_store_lock`] 的进程内互斥
//!（`std::sync::Mutex` 静态实例）：同一时刻仅一个商店写动作在跑，避免 staging
//! 与快照替换互相撕裂。**应用内互斥，跨进程文件锁非目标**（Tauri 单进程，
//! spec 第 6 节明示）。锁持有毒化按「前一个动作已死，放行后续」处理（`into_inner`）。
//!
//! # 失败语义
//!
//! - [`Orchestrator::refresh_market`] 失败返回 Err（由调用方决定多市场循环是否继续），
//!   失败原因落 `marketplaces.last_refresh_failure` 并写审计；旧快照保留（clone 后先在
//!   临时目录 probe 校验，通过才替换；替换用 rename 前移旧快照、失败原样换回）。
//! - 安装 / 升级在原子激活（staging → 事务 → rename → 裁剪）前完成全部校验
//!   （清单 name 一致性、版本解析、载荷组装），失败不留半成品缓存目录。
//! - 全部动作（含失败）写 `audit_log`（[`crate::audit::audit_action`]）。

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use craft_core::network::GixProxyOptions;
use craft_store::db::{StoreError, StoreResult};
use rusqlite::Connection;

use crate::audit::{self, audit_action};
use crate::cache::{self, MarketplaceLayout};
use crate::manifest::{self, MarketEntry, PluginManifest};
use crate::store;
use crate::update_check::{self, UpdateStatus};

/// 插件版本保留上限（spec 第 4 节：当前 + 2 历史）。
const KEEP_LAST_VERSIONS: usize = 3;
/// HEAD 短码长度（与 `craft_git::transport` 的 SHORT_SHA_LEN 对齐：git 默认 7 位）。
const SHORT_SHA_LEN: usize = 7;
/// 技能清单文件名（与 `craft_tools::skills` 的 SKILL_FILE_NAME 对齐；本 crate 不依赖
/// craft-tools，常量以注释约定同步，扫描票 #6 接线时双侧不变）。
const SKILL_FILE_NAME: &str = "SKILL.md";

// ---------- 错误 ----------

/// 编排失败。领域层错误（存储 / 清单 / 激活 / git 传输）原样透传保留结构，
/// 编排自身判定失败落 [`OrchestrateError::Other`]；Display 全部可读中文（可直出 toast）。
#[derive(Debug)]
pub enum OrchestrateError {
    /// 商店表读写失败（[`crate::store`] 透传）。
    Store(StoreError),
    /// 清单探测 / 解析失败（[`crate::manifest`] 透传）。
    Manifest(manifest::ManifestError),
    /// 原子激活失败（[`crate::cache`] 透传）。
    Activate(cache::ActivateError),
    /// git 传输失败（craft-git 归一化错误透传）。
    Git(craft_git::Error),
    /// 编排自身判定失败（前置校验、来源缺失、版本无法确定等；可读中文）。
    Other(String),
}

impl fmt::Display for OrchestrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(err) => write!(f, "{err}"),
            Self::Manifest(err) => write!(f, "{err}"),
            Self::Activate(err) => write!(f, "{err}"),
            Self::Git(err) => write!(f, "{err}"),
            Self::Other(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for OrchestrateError {}

impl From<StoreError> for OrchestrateError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

impl From<manifest::ManifestError> for OrchestrateError {
    fn from(err: manifest::ManifestError) -> Self {
        Self::Manifest(err)
    }
}

impl From<cache::ActivateError> for OrchestrateError {
    fn from(err: cache::ActivateError) -> Self {
        Self::Activate(err)
    }
}

impl From<craft_git::Error> for OrchestrateError {
    fn from(err: craft_git::Error) -> Self {
        Self::Git(err)
    }
}

/// 编排入口统一返回类型。
pub type OResult<T> = Result<T, OrchestrateError>;

// ---------- 互斥 ----------

/// 商店写动作进程内互斥（见模块注释「互斥」；静态实例，全部编排入口共享）。
static STORE_OP_LOCK: Mutex<()> = Mutex::new(());

/// 在商店写动作互斥内执行 `f`。安装 / 升级 / 回滚 / 卸载 / 市场刷新都经本入口
/// 串行化；锁被毒化（前一动作 panic）时恢复内部值继续——前一个动作已死，不应
/// 永久阻塞后续动作。并发语义见模块注释（应用内互斥，跨进程文件锁非目标）。
pub fn with_store_lock<T>(f: impl FnOnce() -> T) -> T {
    let guard = STORE_OP_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let result = f();
    drop(guard);
    result
}

// ---------- 网络配置 ----------

/// 商店下载的网络配置（调用方从设置解析组装；本模块不读设置）：
/// 代理选项来自 `craft_core::network::ProxyConfig::gix_proxy_options()`，
/// 加速前缀来自 `normalize_accelerator_prefix` 的产物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkOptions {
    /// gix 传输代理选项（None = 不干预，gix 自读环境变量）。
    pub proxy: Option<GixProxyOptions>,
    /// GitHub 加速前缀（Some 非空时对 github.com host 改写并启用失败回退）。
    pub accelerator_prefix: Option<String>,
    /// 单次尝试整体超时（默认 90 秒，对齐 `craft_git::transport::DEFAULT_TIMEOUT`）。
    pub timeout: Duration,
    /// 网络类失败重试次数（默认 3 次，对齐 `DEFAULT_RETRIES`）。
    pub retries: u32,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        Self {
            proxy: None,
            accelerator_prefix: None,
            timeout: craft_git::transport::DEFAULT_TIMEOUT,
            retries: craft_git::transport::DEFAULT_RETRIES,
        }
    }
}

impl NetworkOptions {
    /// 组装 craft-git 的克隆请求（默认浅克隆 depth-1；dest 与 URL 由调用方给）。
    fn clone_request(&self, url: String, dest: PathBuf) -> craft_git::transport::CloneRequest {
        let mut req = craft_git::transport::CloneRequest::new(url, dest);
        req.proxy = self.proxy.clone();
        req.accelerator_prefix = self.accelerator_prefix.clone();
        req.timeout = self.timeout;
        req.retries = self.retries;
        req
    }
}

// ---------- 结果结构 ----------

/// 市场刷新结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshOutcome {
    pub marketplace_id: String,
    /// 新快照 HEAD 短码。
    pub head_sha: String,
    /// 本次是否经「加速失败 → 原始 URL 回退」成功。
    pub fell_back: bool,
    /// 重新探测出的清单形态（[`crate::manifest::manifest_kind`] 常量）。
    pub manifest_kind: String,
    /// 重新探测出的条目数。
    pub entry_count: usize,
}

/// 安装 / 升级共用结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// `name@marketplace`。
    pub plugin_id: String,
    /// 版本目录名（= 指针 = UI 展示）。
    pub version: String,
    /// 版本解析来源（`plugin-manifest` / `market-entry` / `git-sha`）。
    pub version_source: &'static str,
    /// 当前版本缓存目录绝对路径。
    pub install_path: PathBuf,
    /// 磁盘保留版本列表（事务返回）。
    pub kept_versions: Vec<String>,
    /// 本次裁剪掉的旧版本目录名。
    pub pruned: Vec<String>,
    /// 进入缓存的技能数（递归数 skills/ 下 SKILL.md）。
    pub skill_count: usize,
}

/// 回滚结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackOutcome {
    pub plugin_id: String,
    /// 回滚前指针。
    pub from_version: String,
    /// 回滚后指针。
    pub to_version: String,
    /// 回滚后的当前版本目录绝对路径。
    pub install_path: PathBuf,
}

/// 卸载结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallOutcome {
    pub plugin_id: String,
    /// 删除的全部版本目录名（含历史）。
    pub removed_versions: Vec<String>,
    /// 从 `skills.disabled` 清掉的残留路径条数。
    pub pruned_disabled: usize,
    /// 磁盘删除失败摘要（行已删、残留由启动清理兜底；正常为空）。
    pub disk_failures: Vec<String>,
}

/// 单个已装插件的更新检测结果（商店列表角标数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginUpdateStatus {
    /// `name@marketplace`。
    pub plugin_id: String,
    pub name: String,
    /// 当前版本指针。
    pub current_version: String,
    /// 双轴判定结果。
    pub status: UpdateStatus,
}

// ---------- 临时目录 ----------

/// 临时目录守卫：drop 时整树删除（clone 落地与载荷组装共用；位置见模块注释）。
struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 在布局根 `.tmp/` 下分配临时目录（随机后缀防并发撞名；临时内容不落正式布局，
/// 目录权限随创建时的 umask，正式缓存目录的 0700 由 [`crate::cache`] 激活保证）。
fn alloc_tmp_dir(layout: &MarketplaceLayout) -> std::io::Result<TempDirGuard> {
    let dir = layout.root().join(".tmp").join(format!("op-{}", cache::random_suffix()));
    std::fs::create_dir_all(&dir)?;
    Ok(TempDirGuard(dir))
}

/// 清理布局根下的临时目录树（应用启动时调用；崩溃残留的 clone 落地 / 载荷目录在此
/// 整树清除）。目录不存在（从未跑过商店动作）视为已清理，返回 Ok。
pub fn cleanup_tmp(layout: &MarketplaceLayout) -> std::io::Result<()> {
    let tmp_root = layout.root().join(".tmp");
    if tmp_root.exists() {
        std::fs::remove_dir_all(&tmp_root)?;
    }
    Ok(())
}

// ---------- 快照 HEAD 读取 ----------

/// 读取市场快照仓库的 HEAD 短码（纯文本读取；机制见模块注释）。
///
/// 支持：`.git/HEAD` 为符号引用（`ref: refs/heads/<branch>`，clone 产物常态）时读
/// 松散引用文件，缺文件回退 `packed-refs` 查找；分离 HEAD（直接是 40 位 sha）原样取。
/// 任一步失败（非 git 目录、文件缺失）返回 None——调用方按「无 sha」处理（版本解析
/// 末级回退缺失、source_sha 不 pin、sha 轴更新检测退化为 None 不误报）。
pub fn read_snapshot_head_sha(snapshot: &Path) -> Option<String> {
    let git_dir = snapshot.join(".git");
    if !git_dir.is_dir() {
        return None;
    }
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let sha = if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        // 松散引用文件优先；clone 刚完成时引用恒为松散文件，packed-refs 是兜底
        match std::fs::read_to_string(git_dir.join(reference)) {
            Ok(text) => text.trim().to_string(),
            Err(_) => find_in_packed_refs(&git_dir.join("packed-refs"), reference)?,
        }
    } else {
        head.to_string()
    };
    let sha = sha.trim();
    if sha.is_empty() {
        return None;
    }
    Some(sha.chars().take(SHORT_SHA_LEN).collect())
}

/// packed-refs 文本查找：`<40 位 sha> <refname>` 行；未命中返回 None。
fn find_in_packed_refs(path: &Path, reference: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((sha, name)) = line.split_once(' ') && name.trim() == reference {
            return Some(sha.trim().to_string());
        }
    }
    None
}

// ---------- 来源与版本解析 ----------

/// 条目相对路径规范化（防穿越快照根）：剥 `./` 前缀与空白；拒绝绝对路径与
/// `..` 组件。快照根 / 空路径返回空 PathBuf（join 后即快照根本身）。
fn normalize_entry_relative_path(source_ref: &str) -> Result<PathBuf, String> {
    let mut text = source_ref.trim();
    while let Some(stripped) = text.strip_prefix("./") {
        text = stripped;
    }
    let text = text.trim();
    if text.is_empty() || text == "." {
        return Ok(PathBuf::new());
    }
    let path = Path::new(text);
    // has_root 而非 is_absolute：Windows 上 `/abs`（无盘符）的 is_absolute 是 false，
    // 但它拼到快照根之外同样构成逃逸/歧义，一并拒绝（unix 上两者等价）。
    if path.is_absolute() || path.has_root() {
        return Err(format!("条目来源路径不能是绝对路径：{source_ref:?}"));
    }
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(format!("条目来源路径不能包含 ..（防穿越快照根）：{source_ref:?}"));
        }
    }
    Ok(path.to_path_buf())
}

/// REPO 条目来源展开：含协议头（`://`）原样使用（含测试用的 `file://` 本地仓库）；
/// 否则按 `owner/repo` 缩写展开为 GitHub URL（加速改写与回退由传输层按 host 判定）。
fn expand_source_url(source_ref: &str) -> String {
    if source_ref.contains("://") {
        source_ref.to_string()
    } else {
        format!("https://github.com/{source_ref}")
    }
}

/// 从 git 仓库 URL / 本地路径提取仓库名（形态 4 裸技能集的市场 id 回退）：
/// 取路径最后一段并剥 `.git` 后缀（`https://github.com/o/r.git` → `r`，
/// `/tmp/fixture-repo` → `fixture-repo`）。空形态兜底 `repo`（probe 侧仅形态 4 消费，
/// 其余形态不影响市场 id）。
fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() { "repo".to_string() } else { name.to_string() }
}

/// 版本解析（spec 第 4 节回退链）：插件清单 version > 市场条目 version > git sha 短码。
/// 返回（版本目录名，来源标注）；解析结果必须能安全用作缓存路径段
///（[`cache::validate_version_segment`]）。
fn resolve_version(
    manifest_version: Option<&str>,
    entry_version: Option<&str>,
    sha: Option<&str>,
) -> OResult<(String, &'static str)> {
    fn clean(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    if let Some(version) = clean(manifest_version) {
        cache::validate_version_segment(version).map_err(OrchestrateError::from)?;
        return Ok((version.to_string(), "plugin-manifest"));
    }
    if let Some(version) = clean(entry_version) {
        cache::validate_version_segment(version).map_err(OrchestrateError::from)?;
        return Ok((version.to_string(), "market-entry"));
    }
    if let Some(sha) = sha.map(str::trim).filter(|s| !s.is_empty()) {
        let short: String = sha.chars().take(SHORT_SHA_LEN).collect();
        cache::validate_version_segment(&short).map_err(OrchestrateError::from)?;
        return Ok((short, "git-sha"));
    }
    Err(OrchestrateError::Other(
        "无法确定插件版本：插件清单与市场条目均未声明 version，且无法读取来源 git sha".to_string(),
    ))
}

/// 安装 / 升级的来源材料（已就位目录 + 落列元数据 + 探测到的插件清单）。
struct SourceMaterial {
    /// 来源目录（快照内条目目录，或临时 clone 落地目录；生命周期由编排函数持有）。
    source_dir: PathBuf,
    /// `skill_plugins.source_kind` 落列值。
    source_kind: String,
    /// `skill_plugins.source_ref` 落列值（条目 source 原文）。
    source_ref: String,
    /// `skill_plugins.source_sha` 落列值（git sha 短码；无 git 信息为 None）。
    source_sha: Option<String>,
    /// 来源目录内探测到的插件清单（None = 无清单，如条目级 skills 形态）。
    plugin_manifest: Option<PluginManifest>,
}

// ---------- 编排器 ----------

/// 商店编排器：布局 + 数据库句柄 + 网络配置。全部入口见各方法文档；
/// 网络配置缺省用 [`NetworkOptions::default`]（系统代理不干预、无加速、默认超时重试）。
pub struct Orchestrator<'a> {
    layout: &'a MarketplaceLayout,
    db: &'a craft_store::Db,
    network: NetworkOptions,
}

impl<'a> Orchestrator<'a> {
    /// 以默认网络配置构造。
    pub fn new(layout: &'a MarketplaceLayout, db: &'a craft_store::Db) -> Self {
        Self { layout, db, network: NetworkOptions::default() }
    }

    /// 指定网络配置构造（生产侧从设置解析代理与加速前缀后传入）。
    pub fn with_network(layout: &'a MarketplaceLayout, db: &'a craft_store::Db, network: NetworkOptions) -> Self {
        Self { layout, db, network }
    }

    // ---------- 市场源添加 / 移除（服务层接线票 #7 补齐：services 只做校验与转发） ----------

    /// 添加市场源（spec 第 2 节）：clone 到临时目录 → probe 探测清单定市场 id →
    /// 重名拒绝（提示先移除）→ 落 `marketplaces` 行 → 快照落位（与刷新同一套原子
    /// 替换语义，添加后即有快照、无需紧接着刷一次）→ 写审计。
    ///
    /// `source` 为用户输入原文（`https://` URL 或 `owner/repo` 缩写，输入合法性由
    /// 服务层校验），`repo_url` 为展开后的 git 仓库 URL。任一步失败返回 Err：行未落
    /// 则直接透传；行已落但快照落位失败则回滚删行并清掉残留快照（添加是原子动作，
    /// 失败后应用内不留「半添加」市场）。失败动作也写审计（对齐刷新失败语义）。
    pub fn add_market(&self, source: &str, repo_url: &str) -> OResult<RefreshOutcome> {
        with_store_lock(|| {
            let tmp = alloc_tmp_dir(self.layout)
                .map_err(|e| OrchestrateError::Other(format!("创建临时目录失败：{e}")))?;
            let dest = tmp.path().join("clone");
            let outcome = match craft_git::transport::clone_repo(self.network.clone_request(repo_url.to_string(), dest.clone())) {
                Ok(outcome) => outcome,
                Err(err) => {
                    let reason = err.to_string();
                    let _ = self.db.with(|conn| {
                        audit::record_audit(
                            conn,
                            audit_action::MARKET_ADD,
                            &format!("添加市场源失败：{repo_url}，{reason}"),
                        )
                    });
                    return Err(err.into());
                }
            };

            let snapshot_sha = read_snapshot_head_sha(&dest).unwrap_or_else(|| outcome.head_sha.clone());
            let repo_name = repo_name_from_url(repo_url);
            let manifest = match manifest::probe_market(&dest, &repo_name, Some(&snapshot_sha)) {
                Ok(manifest) => manifest,
                Err(err) => {
                    let reason = err.to_string();
                    let _ = self.db.with(|conn| {
                        audit::record_audit(
                            conn,
                            audit_action::MARKET_ADD,
                            &format!("添加市场源失败：{repo_url}，{reason}"),
                        )
                    });
                    return Err(err.into());
                }
            };
            // 重名拒绝（spec 第 2 节）：市场 id 来自清单 name，重复添加同名市场提示先移除
            if self.db.with(|conn| store::get_marketplace(conn, &manifest.id)).is_ok() {
                let _ = self.db.with(|conn| {
                    audit::record_audit(
                        conn,
                        audit_action::MARKET_ADD,
                        &format!("添加市场源失败：{repo_url}，市场源已存在：{}，请先移除后再添加", manifest.id),
                    )
                });
                return Err(OrchestrateError::Other(format!(
                    "市场源已存在：{}，请先移除后再添加",
                    manifest.id
                )));
            }

            self.db.with(|conn| {
                store::add_marketplace(
                    conn,
                    store::NewMarketplace { id: &manifest.id, source, repo_url },
                )
            })?;
            // 快照落位；失败则回滚刚落的行与残留快照（添加保持原子性，见函数注释）
            if let Err(err) = self.install_snapshot(&dest, &manifest.id, tmp.path()) {
                let _ = self.db.with(|conn| store::remove_marketplace(conn, &manifest.id));
                let snapshot_dir = self.layout.marketplace_snapshot_dir(&manifest.id);
                if snapshot_dir.exists() {
                    let _ = std::fs::remove_dir_all(&snapshot_dir);
                }
                return Err(err);
            }
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::MARKET_ADD,
                    &format!(
                        "添加市场源：{}（{}，{} 个条目，HEAD {}）",
                        manifest.id, repo_url, manifest.entries.len(), snapshot_sha
                    ),
                )
            });
            Ok(RefreshOutcome {
                marketplace_id: manifest.id,
                head_sha: outcome.head_sha,
                fell_back: outcome.fell_back,
                manifest_kind: manifest.manifest_kind,
                entry_count: manifest.entries.len(),
            })
        })
    }

    /// 移除市场源（保护语义，spec 第 6 节）：市场下仍有已安装插件时拒绝（提示先卸载，
    /// 级联删除由后续 UI 需求另行接线）；移除成功后删除市场快照目录（磁盘删除失败
    /// 不阻塞——行已删、指针为权威，残留目录在审计 detail 注明），全部写审计。
    pub fn remove_market(&self, marketplace_id: &str) -> OResult<()> {
        with_store_lock(|| {
            // 先读行拿来源原文（供审计摘要）；不存在直接透传 NotFound
            let market = match self.db.with(|conn| store::get_marketplace(conn, marketplace_id)) {
                Ok(row) => row,
                Err(err) => return Err(err.into()),
            };
            self.db.with(|conn| store::remove_marketplace(conn, marketplace_id))?;
            let snapshot_dir = self.layout.marketplace_snapshot_dir(marketplace_id);
            let disk_note = if snapshot_dir.exists() {
                match std::fs::remove_dir_all(&snapshot_dir) {
                    Ok(()) => String::new(),
                    Err(err) => format!("，快照目录删除失败（残留 {}）：{err}", snapshot_dir.display()),
                }
            } else {
                "，无快照".to_string()
            };
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::MARKET_REMOVE,
                    &format!("移除市场源：{marketplace_id}（来源 {}）{disk_note}", market.source),
                )
            });
            Ok(())
        })
    }

    // ---------- 市场刷新 ----------

    /// 刷新单个市场源（spec 第 2 节）：浅克隆到临时目录 → 重新探测清单并校验 →
    /// 原子替换 `marketplaces/<id>/` 快照 → 更新 `last_refreshed_at`（清空失败原因）。
    ///
    /// 失败（clone / 探测 / 校验 / 替换任一步）返回 Err，失败原因落
    /// `last_refresh_failure` 并写审计（成功也写审计）；**旧快照保留**——探测在临时
    /// 目录上进行，通过校验才开始替换；替换先把旧快照 rename 到临时目录内（同文件
    /// 系统），落位失败原样换回。单市场失败不影响其他市场（由调用方循环本方法）。
    pub fn refresh_market(&self, marketplace_id: &str) -> OResult<RefreshOutcome> {
        with_store_lock(|| {
            let market = match self.db.with(|conn| store::get_marketplace(conn, marketplace_id)) {
                Ok(row) => row,
                // 市场不存在：无行可记失败原因，直接透传
                Err(err) => return Err(err.into()),
            };
            match self.refresh_market_inner(marketplace_id, &market.repo_url) {
                Ok(outcome) => {
                    let _ = self.db.with(|conn| {
                        store::set_marketplace_refreshed(conn, marketplace_id, craft_store::db::now_millis())
                    });
                    let _ = self.db.with(|conn| {
                        audit::record_audit(
                            conn,
                            audit_action::MARKET_REFRESH,
                            &format!(
                                "刷新市场：{marketplace_id}（{} 个条目，HEAD {}{}）",
                                outcome.entry_count,
                                outcome.head_sha,
                                if outcome.fell_back { "，经加速回退" } else { "" }
                            ),
                        )
                    });
                    Ok(outcome)
                }
                Err(err) => {
                    // 失败落列 + 审计；记录失败不覆盖原始错误
                    let reason = err.to_string();
                    let _ = self
                        .db
                        .with(|conn| store::set_marketplace_refresh_failure(conn, marketplace_id, &reason));
                    let _ = self.db.with(|conn| {
                        audit::record_audit(
                            conn,
                            audit_action::MARKET_REFRESH,
                            &format!("刷新市场失败：{marketplace_id}，{reason}"),
                        )
                    });
                    Err(err)
                }
            }
        })
    }

    /// 刷新主体（互斥内）：clone → probe 校验 → 快照替换。网络与磁盘失败统一经
    /// [`OrchestrateError`]。
    fn refresh_market_inner(&self, marketplace_id: &str, repo_url: &str) -> OResult<RefreshOutcome> {
        let tmp = alloc_tmp_dir(self.layout)
            .map_err(|e| OrchestrateError::Other(format!("创建临时目录失败：{e}")))?;
        let dest = tmp.path().join("clone");

        let outcome = craft_git::transport::clone_repo(self.network.clone_request(repo_url.to_string(), dest.clone()))?;

        // 先在临时目录上探测校验：不是技能源 / 清单改名都不替换旧快照
        let snapshot_sha = read_snapshot_head_sha(&dest).unwrap_or_else(|| outcome.head_sha.clone());
        let manifest = manifest::probe_market(&dest, marketplace_id, Some(&snapshot_sha))?;
        if manifest.id != marketplace_id {
            return Err(OrchestrateError::Other(format!(
                "市场清单 name（{}）与已添加市场 id（{marketplace_id}）不一致，请移除后重新添加",
                manifest.id
            )));
        }

        // 快照替换：旧快照先移入临时目录（同文件系统 rename），落位失败原样换回。
        self.install_snapshot(&dest, marketplace_id, tmp.path())?;

        Ok(RefreshOutcome {
            marketplace_id: marketplace_id.to_string(),
            head_sha: outcome.head_sha,
            fell_back: outcome.fell_back,
            manifest_kind: manifest.manifest_kind,
            entry_count: manifest.entries.len(),
        })
    }

    // ---------- 安装 / 升级（共享主体） ----------

    /// 安装插件（spec 第 2 节）：从市场快照解析条目 → 取来源（条目相对路径从快照
    /// 拷贝；repo 形态经传输层浅克隆到临时目录）→ 读插件清单定版本（清单 version >
    /// 条目 version > git sha 短码）→ 组装载荷（skills 收集规则见 [`Self::build_payload`]）→
    /// 原子激活 → 落 `skill_plugins` 行 → 写审计。
    ///
    /// 插件清单 name 与条目 name 不一致拒绝安装；重复安装拒绝；市场快照不存在
    ///（从未刷新）拒绝并提示先刷新。
    pub fn install_plugin(&self, marketplace_id: &str, plugin_name: &str) -> OResult<InstallOutcome> {
        with_store_lock(|| {
            let plugin_id = store::plugin_id(plugin_name, marketplace_id);
            if self.db.with(|conn| store::get_plugin(conn, &plugin_id)).is_ok() {
                return Err(OrchestrateError::Other(format!("插件已安装：{plugin_id}，如需重装请先卸载")));
            }
            let outcome = self.activate_new_version(marketplace_id, plugin_name, UpgradeMode::Fresh)?;
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::PLUGIN_INSTALL,
                    &format!(
                        "安装插件：{plugin_id}（版本 {}，{} 个技能）",
                        outcome.version, outcome.skill_count
                    ),
                )
            });
            Ok(outcome)
        })
    }

    /// 升级插件（spec 第 4 节）：拉取新版 → 新版本目录激活 → 指针切换 → 裁剪；
    /// 保留上限 [`KEEP_LAST_VERSIONS`]。用户的插件级启停选择（`enabled` 列）不受
    /// 影响（UPDATE 不触碰该列）。新版本与当前指针同版本时原地重装（缓存目录替换）。
    pub fn update_plugin(&self, marketplace_id: &str, plugin_name: &str) -> OResult<InstallOutcome> {
        with_store_lock(|| {
            let plugin = self.db.with(|conn| store::get_plugin(conn, &store::plugin_id(plugin_name, marketplace_id)))?;
            let outcome = self.activate_new_version(marketplace_id, plugin_name, UpgradeMode::Upgrade(&plugin))?;
            let reinstalled = outcome.version == plugin.current_version;
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::PLUGIN_UPDATE,
                    &if reinstalled {
                        format!("重装插件：{}（版本 {} 不变，原地重装）", plugin.id, outcome.version)
                    } else {
                        format!("升级插件：{}（{} → {}）", plugin.id, plugin.current_version, outcome.version)
                    },
                )
            });
            Ok(outcome)
        })
    }

    /// 安装 / 升级共享主体（互斥内调用）：快照解析条目 → 取来源（REPO 形态在此
    /// 浅克隆，全程只取一次）→ 校验插件清单 name 一致性 → 版本解析 → 组装载荷 →
    /// 原子激活（写行逻辑按 [`UpgradeMode`] 区分新装 / 升级）。
    fn activate_new_version(
        &self,
        marketplace_id: &str,
        plugin_name: &str,
        mode: UpgradeMode<'_>,
    ) -> OResult<InstallOutcome> {
        let snapshot = self.require_snapshot(marketplace_id)?;
        let snapshot_sha = read_snapshot_head_sha(&snapshot);
        let manifest = manifest::probe_market(&snapshot, marketplace_id, snapshot_sha.as_deref())?;
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.name == plugin_name)
            .ok_or_else(|| {
                OrchestrateError::Other(format!("市场 {marketplace_id} 中不存在插件条目：{plugin_name}"))
            })?;
        let mut tmp = alloc_tmp_dir(self.layout)
            .map_err(|e| OrchestrateError::Other(format!("创建临时目录失败：{e}")))?;
        let source = self.gather_source_material_in(&snapshot, entry, &mut tmp)?;
        if let Some(pm) = &source.plugin_manifest && pm.name != plugin_name {
            return Err(OrchestrateError::Other(format!(
                "插件清单 name（{}）与市场条目 name（{plugin_name}）不一致，拒绝安装",
                pm.name
            )));
        }
        let (version, version_source) = resolve_version(
            source.plugin_manifest.as_ref().and_then(|pm| pm.version.as_deref()),
            entry.version.as_deref(),
            source.source_sha.as_deref(),
        )?;
        let (payload_dir, skill_count) = self.build_payload(&tmp, &snapshot, &source, entry)?;

        let plugin_id = store::plugin_id(plugin_name, marketplace_id);
        let kind = source.source_kind.clone();
        let source_ref = source.source_ref.clone();
        let sha = source.source_sha.clone();
        let id = plugin_id.clone();
        let version_for_row = version.clone();
        let outcome = cache::activate_plugin_version(
            self.layout,
            self.db,
            marketplace_id,
            plugin_name,
            &version,
            &payload_dir,
            |conn, target| {
                let install_path = target.to_string_lossy().to_string();
                match mode {
                    UpgradeMode::Fresh => {
                        store::insert_plugin(
                            conn,
                            store::NewPlugin {
                                id: &id,
                                marketplace_id,
                                name: plugin_name,
                                current_version: &version_for_row,
                                versions: std::slice::from_ref(&version_for_row),
                                install_path: &install_path,
                                source_kind: &kind,
                                source_ref: &source_ref,
                                source_sha: sha.as_deref(),
                            },
                        )?;
                        Ok(vec![version_for_row.clone()])
                    }
                    UpgradeMode::Upgrade(existing) => {
                        let kept = store::append_plugin_version_with_path(
                            conn,
                            &existing.id,
                            &version_for_row,
                            &install_path,
                            KEEP_LAST_VERSIONS,
                        )?;
                        store::update_plugin_source(conn, &existing.id, &kind, &source_ref, sha.as_deref())?;
                        Ok(kept)
                    }
                }
            },
        )?;
        Ok(InstallOutcome {
            plugin_id,
            version,
            version_source,
            install_path: outcome.install_path,
            kept_versions: outcome.kept_versions,
            pruned: outcome.pruned,
            skill_count,
        })
    }

    // ---------- 回滚 ----------

    /// 回滚插件到保留列表内的历史版本（spec 第 4 节）：指针切回 + install_path 同步；
    /// versions 数组不变，因此可以再切回去。目标版本目录磁盘缺失（promote 前崩溃的
    /// 残留指针态）拒绝回滚并提示重装。
    pub fn rollback_plugin(
        &self,
        marketplace_id: &str,
        plugin_name: &str,
        target_version: &str,
    ) -> OResult<RollbackOutcome> {
        with_store_lock(|| {
            let plugin = self.db.with(|conn| store::get_plugin(conn, &store::plugin_id(plugin_name, marketplace_id)))?;
            let target_dir = self.layout.version_dir(marketplace_id, &plugin.name, target_version);
            if !target_dir.is_dir() {
                return Err(OrchestrateError::Other(format!(
                    "版本 {target_version} 的缓存目录缺失，无法回滚（请升级或重装该插件恢复）：{}",
                    plugin.id
                )));
            }
            let install_path = target_dir.to_string_lossy().to_string();
            self.db.with(|conn| {
                store::set_plugin_version_pointer_with_path(conn, &plugin.id, target_version, &install_path)
            })?;
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::PLUGIN_ROLLBACK,
                    &format!("回滚插件：{} → {target_version}", plugin.id),
                )
            });
            Ok(RollbackOutcome {
                plugin_id: plugin.id,
                from_version: plugin.current_version,
                to_version: target_version.to_string(),
                install_path: target_dir,
            })
        })
    }

    // ---------- 卸载 ----------

    /// 卸载插件（spec 第 4 节）：事务内删 `skill_plugins` 行并清理 `skills.disabled`
    /// 残留（[`prune_disabled_paths`] 按插件缓存目录前缀过滤），随后删除整个插件缓存
    /// 目录（全部版本）。磁盘删除失败不回滚（行已删、指针为权威，残留由启动清理
    /// [`crate::cache::cleanup_cache`] 兜底），失败摘要进结果与审计。
    pub fn uninstall_plugin(&self, marketplace_id: &str, plugin_name: &str) -> OResult<UninstallOutcome> {
        with_store_lock(|| {
            let plugin = self.db.with(|conn| store::get_plugin(conn, &store::plugin_id(plugin_name, marketplace_id)))?;
            let plugin_dir = self.layout.plugin_cache_dir(marketplace_id, &plugin.name);
            let pruned_disabled = self.db.transaction(|conn| {
                store::remove_plugin(conn, &plugin.id)?;
                prune_disabled_paths(conn, std::slice::from_ref(&plugin_dir))
            })?;
            let mut disk_failures = Vec::new();
            if plugin_dir.exists() && let Err(err) = std::fs::remove_dir_all(&plugin_dir) {
                disk_failures.push(format!("删除插件缓存目录失败：{}（{err}）", plugin_dir.display()));
            }
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::PLUGIN_UNINSTALL,
                    &format!(
                        "卸载插件：{}（删除 {} 个版本目录，清理禁用记录 {pruned_disabled} 条{}）",
                        plugin.id,
                        plugin.versions.len(),
                        if disk_failures.is_empty() {
                            String::new()
                        } else {
                            format!("，磁盘残留：{}", disk_failures.join("；"))
                        }
                    ),
                )
            });
            Ok(UninstallOutcome {
                plugin_id: plugin.id,
                removed_versions: plugin.versions,
                pruned_disabled,
                disk_failures,
            })
        })
    }

    // ---------- 插件启停 ----------

    /// 插件级启停（spec 第 5 节两层开关的插件层）+ 审计。停用后该插件全部技能根
    /// 退出扫描（扫描票 #6 接线）；升级重装不影响本选择（`enabled` 列独立于指针）。
    pub fn set_plugin_enabled(&self, marketplace_id: &str, plugin_name: &str, enabled: bool) -> OResult<()> {
        with_store_lock(|| {
            let plugin_id = store::plugin_id(plugin_name, marketplace_id);
            self.db.with(|conn| store::set_plugin_enabled(conn, &plugin_id, enabled))?;
            let _ = self.db.with(|conn| {
                audit::record_audit(
                    conn,
                    audit_action::PLUGIN_SET_ENABLED,
                    &if enabled { format!("启用插件：{plugin_id}") } else { format!("停用插件：{plugin_id}") },
                )
            });
            Ok(())
        })
    }

    // ---------- 更新检测（编排级取数） ----------

    /// 对单个市场下全部已装插件做双轴更新检测（spec 第 4 节；判定规则见
    /// [`update_check::compare_for_update`]）。纯本地比对（读快照 + probe 清单），
    /// 不 clone、不二次下载：entry-relative 条目的可用 sha = 快照 HEAD 短码；
    /// repo 条目不二次 clone 拿不到新 sha，sha 轴无法证明 → 按 none 不误报。
    ///
    /// 市场快照不存在（从未刷新）报错提示先刷新；单个条目清单损坏只影响该条目
    ///（插件清单按缺失处理，退条目 version），不放大为整体失败。
    pub fn check_updates(&self, marketplace_id: &str) -> OResult<Vec<PluginUpdateStatus>> {
        // 只读比对不进互斥：与写动作并发的最坏后果是读到替换中的快照报一次误判，
        // 下一轮刷新即自愈；写动作串行化不为此加长持锁
        let snapshot = self.require_snapshot(marketplace_id)?;
        let snapshot_sha = read_snapshot_head_sha(&snapshot);
        let manifest = manifest::probe_market(&snapshot, marketplace_id, snapshot_sha.as_deref())?;
        let plugins = self.db.with(|conn| store::list_plugins_by_marketplace(conn, marketplace_id))?;

        let mut statuses = Vec::with_capacity(plugins.len());
        for plugin in plugins {
            let Some(entry) = manifest.entries.iter().find(|e| e.name == plugin.name) else {
                // 上游清单已移除该条目：无法证明有更新，不误报
                statuses.push(PluginUpdateStatus {
                    plugin_id: plugin.id,
                    name: plugin.name,
                    current_version: plugin.current_version,
                    status: UpdateStatus::None,
                });
                continue;
            };
            let is_entry_relative = entry.source.kind == store::source_kind::ENTRY_RELATIVE;
            // 条目目录内的插件清单版本参与「可用版本」解析（与安装侧版本链一致）；
            // 清单损坏按缺失处理（宽容，见函数注释）
            let plugin_manifest_version = if is_entry_relative {
                let plugin_dir = match normalize_entry_relative_path(&entry.source.source_ref) {
                    Ok(rel) => snapshot.join(rel),
                    Err(_) => snapshot.clone(),
                };
                manifest::probe_plugin_manifest(&plugin_dir).ok().flatten().and_then(|pm| pm.version)
            } else {
                None
            };
            let available_version = plugin_manifest_version.or_else(|| entry.version.clone());
            let available_sha = if is_entry_relative { snapshot_sha.clone() } else { None };
            let status = update_check::compare_for_update(
                &plugin.current_version,
                plugin.source_sha.as_deref(),
                available_version.as_deref(),
                available_sha.as_deref(),
            );
            statuses.push(PluginUpdateStatus {
                plugin_id: plugin.id,
                name: plugin.name,
                current_version: plugin.current_version,
                status,
            });
        }
        Ok(statuses)
    }

    // ---------- 内部 ----------

    /// 把临时目录内的 clone 产物原子落位为 `marketplaces/<id>/` 快照。
    ///
    /// 与刷新 / 添加两个编排共用：旧快照先移入 `tmp`（同文件系统 rename），新快照
    /// rename 落位失败原样换回。中间窗口（旧已移走、新未落位）快照缺失——进程被杀
    /// 残留由下次刷新重建、安装侧按「快照不存在，请先刷新」报错自愈。
    /// 旧快照移入 `tmp` 后随调用方的 [`TempDirGuard`] drop 清理。
    fn install_snapshot(&self, cloned: &Path, marketplace_id: &str, tmp: &Path) -> OResult<()> {
        let market_dir = self.layout.marketplace_snapshot_dir(marketplace_id);
        if let Some(parent) = market_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| OrchestrateError::Other(format!("创建市场快照目录失败：{e}")))?;
        }
        let previous = tmp.join("previous-snapshot");
        if market_dir.exists() {
            std::fs::rename(&market_dir, &previous)
                .map_err(|e| OrchestrateError::Other(format!("移出旧市场快照失败：{e}")))?;
        }
        if let Err(err) = std::fs::rename(cloned, &market_dir) {
            if previous.exists() {
                let _ = std::fs::rename(&previous, &market_dir);
            }
            return Err(OrchestrateError::Other(format!("落位新市场快照失败：{err}")));
        }
        Ok(())
    }

    /// 市场快照目录必须存在（安装 / 升级 / 检测的前置条件）。
    fn require_snapshot(&self, marketplace_id: &str) -> OResult<PathBuf> {
        let snapshot = self.layout.marketplace_snapshot_dir(marketplace_id);
        if !snapshot.is_dir() {
            return Err(OrchestrateError::Other(format!(
                "市场快照不存在：{marketplace_id}，请先刷新该市场"
            )));
        }
        Ok(snapshot)
    }

    /// 取来源材料（共享临时目录版；clone 落地与后续载荷组装同目录，见
    /// [`Self::activate_new_version`]）。
    fn gather_source_material_in(
        &self,
        snapshot: &Path,
        entry: &MarketEntry,
        tmp: &mut TempDirGuard,
    ) -> OResult<SourceMaterial> {
        match entry.source.kind.as_str() {
            kind if kind == store::source_kind::ENTRY_RELATIVE => {
                let rel = normalize_entry_relative_path(&entry.source.source_ref)
                    .map_err(OrchestrateError::Other)?;
                let source_dir = snapshot.join(rel);
                if !source_dir.is_dir() {
                    return Err(OrchestrateError::Other(format!(
                        "条目来源目录不存在：{}（{}）",
                        source_dir.display(),
                        entry.source.source_ref
                    )));
                }
                Ok(SourceMaterial {
                    plugin_manifest: manifest::probe_plugin_manifest(&source_dir)?,
                    source_dir,
                    source_kind: store::source_kind::ENTRY_RELATIVE.to_string(),
                    source_ref: entry.source.source_ref.clone(),
                    source_sha: read_snapshot_head_sha(snapshot),
                })
            }
            kind if kind == store::source_kind::REPO => {
                let url = expand_source_url(&entry.source.source_ref);
                let dest = tmp.path().join("clone");
                let outcome = craft_git::transport::clone_repo(self.network.clone_request(url, dest.clone()))?;
                let sha = if outcome.head_sha.is_empty() { None } else { Some(outcome.head_sha) };
                Ok(SourceMaterial {
                    plugin_manifest: manifest::probe_plugin_manifest(&dest)?,
                    source_dir: dest,
                    source_kind: store::source_kind::REPO.to_string(),
                    source_ref: entry.source.source_ref.clone(),
                    source_sha: sha,
                })
            }
            other => Err(OrchestrateError::Other(format!("未知的条目来源类型：{other}"))),
        }
    }

    /// [`Self::gather_source_material_in`] 的独立临时目录版（需要只读来源元数据
    /// 而不与载荷组装共享临时目录时使用；用后即弃）。
    #[allow(dead_code)]
    fn gather_source_material(&self, snapshot: &Path, entry: &MarketEntry) -> OResult<SourceMaterial> {
        let mut tmp = alloc_tmp_dir(self.layout)
            .map_err(|e| OrchestrateError::Other(format!("创建临时目录失败：{e}")))?;
        self.gather_source_material_in(snapshot, entry, &mut tmp)
    }

    /// 组装安装载荷（进入缓存的最终内容）：skills 收集规则 + 插件清单副本。
    ///
    /// - 条目级 `entry_skills` 数组存在（anthropics/skills strict:false 形态）：仅拷贝
    ///   声明路径，保持声明路径的相对结构落到载荷 `skills/` 下（扫描层递归发现
    ///   SKILL.md，目录层级不影响）。声明路径先按快照根解析、回退条目目录（两种
    ///   上游形态兼容）；均不存在报错。
    /// - 未声明（常规插件）：拷贝来源目录的 `skills/` 全目录 + `commands/` 目录
    ///   （commands 映射为技能是扫描票 #6 的事，本票只负责把目录放进缓存）。
    ///   两者都不存在视为空插件拒绝（避免装出空目录占位）。
    /// - 插件清单副本（探测到时）按其在来源目录内的相对路径原位拷入载荷（版本回读
    ///   与详情页复用 [`manifest::probe_plugin_manifest`] 同一探测链）。
    ///
    /// 返回（载荷目录，技能数）。载荷内出现符号链接会在激活的拷贝阶段被拒绝
    ///（不可信内容信任边界，见 [`crate::cache`]）。
    fn build_payload(
        &self,
        tmp: &TempDirGuard,
        snapshot: &Path,
        source: &SourceMaterial,
        entry: &MarketEntry,
    ) -> OResult<(PathBuf, usize)> {
        let payload = tmp.path().join("payload");
        std::fs::create_dir_all(&payload)
            .map_err(|e| OrchestrateError::Other(format!("创建载荷目录失败：{e}")))?;
        let payload_skills = payload.join("skills");

        match &entry.entry_skills {
            Some(declared) => {
                for raw in declared {
                    let rel = normalize_entry_relative_path(raw).map_err(OrchestrateError::Other)?;
                    if rel.as_os_str().is_empty() {
                        return Err(OrchestrateError::Other(format!(
                            "条目级技能路径不能是仓库根：{raw:?}（应声明技能子目录）"
                        )));
                    }
                    // 先快照根后条目目录：anthropics 形态（source "./"）两者等价，
                    // 非根来源条目声明快照根路径时由回退覆盖
                    let origin = match [snapshot.join(&rel), source.source_dir.join(&rel)]
                        .into_iter()
                        .find(|candidate| candidate.is_dir())
                    {
                        Some(dir) => dir,
                        None => {
                            return Err(OrchestrateError::Other(format!(
                                "条目声明的技能路径不存在：{raw:?}（在市场快照与条目目录内均未找到）"
                            )));
                        }
                    };
                    cache::copy_dir_recursive(&origin, &payload_skills.join(&rel))?;
                }
            }
            None => {
                let skills_dir = source.source_dir.join("skills");
                let commands_dir = source.source_dir.join("commands");
                let has_skills = skills_dir.is_dir();
                let has_commands = commands_dir.is_dir();
                if !has_skills && !has_commands {
                    return Err(OrchestrateError::Other(format!(
                        "插件 {} 没有可安装的技能内容（无 skills/ 与 commands/ 目录）",
                        entry.name
                    )));
                }
                if has_skills {
                    cache::copy_dir_recursive(&skills_dir, &payload_skills)?;
                }
                if has_commands {
                    cache::copy_dir_recursive(&commands_dir, &payload.join("commands"))?;
                }
            }
        }

        // 插件清单副本：按来源目录内相对路径原位拷入（symlink 清单拒绝，与载荷一致）
        if let Some(pm) = &source.plugin_manifest {
            let rel = pm.manifest_path.strip_prefix(&source.source_dir).map_err(|_| {
                OrchestrateError::Other(format!(
                    "插件清单路径不在来源目录内：{}（来源 {}）",
                    pm.manifest_path.display(),
                    source.source_dir.display()
                ))
            })?;
            let meta = std::fs::symlink_metadata(&pm.manifest_path).map_err(|e| {
                OrchestrateError::Other(format!("读取插件清单失败：{}（{e}）", pm.manifest_path.display()))
            })?;
            if meta.file_type().is_symlink() {
                return Err(OrchestrateError::Other(format!(
                    "插件清单是符号链接，已拒绝安装：{}",
                    pm.manifest_path.display()
                )));
            }
            let target = payload.join(rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| OrchestrateError::Other(format!("创建清单副本目录失败：{e}")))?;
            }
            std::fs::copy(&pm.manifest_path, &target).map_err(|e| {
                OrchestrateError::Other(format!("拷贝插件清单副本失败：{}（{e}）", pm.manifest_path.display()))
            })?;
        }

        let skill_count = count_skills(&payload_skills);
        Ok((payload, skill_count))
    }
}

/// 安装 / 升级模式（激活写行逻辑的分支依据）。
enum UpgradeMode<'p> {
    /// 新装：insert 行（重复 id 由唯一约束兜底拒绝）。
    Fresh,
    /// 升级：append 版本 + 切指针 + 同步来源列（existing 为当前行）。
    Upgrade(&'p store::PluginRow),
}

/// 递归统计目录下 SKILL.md 数量（安装结果的技能数；目录缺失计 0）。
fn count_skills(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && entry.file_name().to_string_lossy() == SKILL_FILE_NAME {
            count += 1;
        } else if path.is_dir() {
            count += count_skills(&path);
        }
    }
    count
}

// ---------- skills.disabled 清理 ----------

/// `skills.disabled` 设置键（与 `craft_tools::skills::SKILLS_DISABLED_KEY` 对齐；
/// 本 crate 不依赖 craft-tools，常量以注释约定同步，扫描票 #6 接线时双侧不变）。
pub const SKILLS_DISABLED_KEY: &str = "skills.disabled";

/// 清理 `skills.disabled` 中的已卸载插件路径（spec 第 6 节卸载语义）：读取 settings
/// 表的 [`SKILLS_DISABLED_KEY`]（JSON 字符串数组，元素为 SKILL.md 绝对路径），剔除
/// 落在 `removed_prefixes`（目录前缀）之下的条目后写回；无变化不写（避免无谓写）。
/// 清空整列时删除键（对齐 `craft_tools::skills::save_disabled_paths` 的空列表语义）。
///
/// 前缀判定按路径段边界：`/cache/m/x` 是 `/cache/m/xy` 下条目的误判前缀，须以 `/`
/// 分隔或整串相等才算命中。格式异常（键缺失、非数组）按「无可清理」返回 0 不报错
///（宁漏勿误：禁用记录残留只是管理页多一条死条目，不应阻断卸载）。
/// 须在调用方事务内执行（与删行同事务保证原子；`&Connection` 由调用方传入）。
pub fn prune_disabled_paths(conn: &Connection, removed_prefixes: &[PathBuf]) -> StoreResult<usize> {
    use craft_store::records::{delete_setting, get_setting, set_setting};
    if removed_prefixes.is_empty() {
        return Ok(0);
    }
    let prefixes: Vec<String> =
        removed_prefixes.iter().map(|p| p.to_string_lossy().trim_end_matches('/').to_string()).collect();
    let Some(value) = get_setting(conn, SKILLS_DISABLED_KEY)? else {
        return Ok(0);
    };
    let Some(items) = value.as_array() else {
        return Ok(0);
    };
    let mut kept: Vec<serde_json::Value> = Vec::with_capacity(items.len());
    let mut removed = 0usize;
    for item in items {
        let Some(text) = item.as_str() else {
            kept.push(item.clone());
            continue;
        };
        if is_under_any_prefix(text, &prefixes) {
            removed += 1;
        } else {
            kept.push(item.clone());
        }
    }
    if removed > 0 {
        if kept.is_empty() {
            delete_setting(conn, SKILLS_DISABLED_KEY)?;
        } else {
            set_setting(conn, SKILLS_DISABLED_KEY, &serde_json::Value::Array(kept))?;
        }
    }
    Ok(removed)
}

/// 路径段边界前缀判定（语义见 [`prune_disabled_paths`]）。
///
/// 比较前两侧统一成正斜杠：Windows 的禁用记录与前缀都来自 `PathBuf`（反斜杠），
/// 段边界只认 `/` 会让全部条目不命中（实测 windows 上 pruned=0）。
fn is_under_any_prefix(path: &str, prefixes: &[String]) -> bool {
    let path = path.trim_end_matches('/').replace('\\', "/");
    prefixes.iter().any(|prefix| {
        let prefix = prefix.trim_end_matches('/').replace('\\', "/");
        prefix.is_empty()
            || path == prefix
            || (path.starts_with(prefix.as_str()) && path.as_bytes().get(prefix.len()) == Some(&b'/'))
    })
}

// ---------- 单元测试（纯函数层） ----------

#[cfg(test)]
mod unit_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn entry_relative_path_normalizes_and_rejects_escape() {
        // 正常形态
        assert_eq!(normalize_entry_relative_path("./plugins/demo").unwrap(), Path::new("plugins/demo"));
        assert_eq!(normalize_entry_relative_path("./").unwrap(), Path::new(""));
        assert_eq!(normalize_entry_relative_path(".").unwrap(), Path::new(""));
        assert_eq!(normalize_entry_relative_path("  ./x/y  ").unwrap(), Path::new("x/y"));
        assert_eq!(normalize_entry_relative_path("././a/b").unwrap(), Path::new("a/b"));
        // 穿越与绝对路径拒绝
        assert!(normalize_entry_relative_path("../up").is_err());
        assert!(normalize_entry_relative_path("a/../../b").is_err());
        assert!(normalize_entry_relative_path("/abs/path").is_err());
    }

    #[test]
    fn source_url_expands_shorthand_and_keeps_absolute() {
        assert_eq!(expand_source_url("owner/repo"), "https://github.com/owner/repo");
        assert_eq!(expand_source_url("https://example.com/x.git"), "https://example.com/x.git");
        // file:// 本地仓库（测试通道）原样保留
        assert_eq!(expand_source_url("file:///tmp/fixture"), "file:///tmp/fixture");
    }

    #[test]
    fn repo_name_extracts_last_segment_and_strips_git_suffix() {
        assert_eq!(repo_name_from_url("https://github.com/owner/repo.git"), "repo");
        assert_eq!(repo_name_from_url("https://github.com/owner/repo"), "repo");
        assert_eq!(repo_name_from_url("/tmp/fixture-repo/"), "fixture-repo");
        assert_eq!(repo_name_from_url("owner/repo"), "repo");
        // 空形态兜底（理论不可达，probe 侧仅形态 4 消费）
        assert_eq!(repo_name_from_url(""), "repo");
    }

    #[test]
    fn version_resolution_follows_manifest_entry_sha_fallback_chain() {
        // 三级优先：插件清单 > 条目 > sha
        let (version, source) = resolve_version(Some("1.0.0"), Some("2.0.0"), Some("abcdef1234")).unwrap();
        assert_eq!((version.as_str(), source), ("1.0.0", "plugin-manifest"));
        let (version, source) = resolve_version(None, Some("2.0.0"), Some("abcdef1234")).unwrap();
        assert_eq!((version.as_str(), source), ("2.0.0", "market-entry"));
        let (version, source) = resolve_version(None, None, Some("abcdef1234567890")).unwrap();
        assert_eq!((version.as_str(), source), ("abcdef1", "git-sha"), "sha 取 7 位短码");
        // 空白视为未声明
        let (version, source) = resolve_version(Some("  "), Some("2.0.0"), None).unwrap();
        assert_eq!((version.as_str(), source), ("2.0.0", "market-entry"));
        // 全缺报可读错误
        let err = resolve_version(None, None, None).unwrap_err();
        assert!(err.to_string().contains("无法确定插件版本"), "实际 {err}");
        // 不能作路径段的版本（`.` 开头、含路径分隔符）拒绝
        assert!(resolve_version(Some(".hidden"), None, None).is_err());
        assert!(resolve_version(Some("a/b"), None, None).is_err());
    }

    #[test]
    fn snapshot_head_sha_reads_loose_packed_and_detached_refs() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();

        // 松散引用（clone 产物常态）
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/heads/main"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        assert_eq!(read_snapshot_head_sha(dir.path()).as_deref(), Some("0123456"));

        // 松散文件缺失 → packed-refs 兜底
        std::fs::remove_file(git.join("refs/heads/main")).unwrap();
        std::fs::write(
            git.join("packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted \n1234567890abcdef0123456789abcdef01234567 refs/heads/main\n",
        )
        .unwrap();
        assert_eq!(read_snapshot_head_sha(dir.path()).as_deref(), Some("1234567"));

        // 分离 HEAD（直接是 sha）
        std::fs::remove_file(git.join("packed-refs")).unwrap();
        std::fs::write(git.join("HEAD"), "89abcdef0123456789abcdef0123456789abcdef\n").unwrap();
        assert_eq!(read_snapshot_head_sha(dir.path()).as_deref(), Some("89abcde"));

        // 非 git 目录 / 损坏形态 → None（调用方按无 sha 处理）
        assert_eq!(read_snapshot_head_sha(Path::new("/tmp")), None);
        let empty = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(empty.path().join(".git")).unwrap();
        assert_eq!(read_snapshot_head_sha(empty.path()), None, "HEAD 缺失");
    }

    #[test]
    fn prune_disabled_paths_filters_by_segment_boundary_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let db = craft_store::Db::open(&dir.path().join("test.db")).unwrap();
        let inside_a = "/home/x/.craftwork/skills-marketplace/cache/mkt/plugin-a/1.0.0/skills/demo/SKILL.md";
        let inside_b = "/home/x/.craftwork/skills-marketplace/cache/mkt/plugin-b/2.0.0/skills/other/SKILL.md";
        let outside = "/home/x/.craftwork/skills/demo/SKILL.md";
        db.with(|conn| {
            craft_store::records::set_setting(
                conn,
                SKILLS_DISABLED_KEY,
                &serde_json::json!([inside_a, inside_b, outside]),
            )
        })
        .unwrap();

        let prefix_a = PathBuf::from("/home/x/.craftwork/skills-marketplace/cache/mkt/plugin-a");
        db.with(|conn| {
            let removed = prune_disabled_paths(conn, std::slice::from_ref(&prefix_a)).unwrap();
            assert_eq!(removed, 1, "只清 plugin-a 前缀下的条目");
            // 段边界：plugin-xy 前缀不误伤 plugin-a / plugin-b
            let kept = craft_store::records::get_setting(conn, SKILLS_DISABLED_KEY).unwrap().unwrap();
            assert_eq!(
                kept,
                serde_json::json!([inside_b, outside]),
                "plugin-a 被清、plugin-b 与插件外条目保留"
            );
            Ok(())
        })
        .unwrap();

        // 全部清空 → 键删除（对齐 save_disabled_paths 的空列表语义）
        let prefix_b = PathBuf::from("/home/x/.craftwork/skills-marketplace/cache/mkt/plugin-b");
        db.with(|conn| {
            assert_eq!(prune_disabled_paths(conn, &[prefix_b]).unwrap(), 1);
            // outside 是插件缓存外的条目，不在本轮清理范围：键保留（仅剩它）
            let kept = craft_store::records::get_setting(conn, SKILLS_DISABLED_KEY).unwrap().unwrap();
            assert_eq!(kept, serde_json::json!([outside]), "缓存外条目保留，键仍在");
            Ok(())
        })
        .unwrap();

        // 清掉最后一条 → 键删除
        let prefix_outside = PathBuf::from("/home/x/.craftwork/skills");
        db.with(|conn| {
            assert_eq!(prune_disabled_paths(conn, &[prefix_outside]).unwrap(), 1);
            assert_eq!(
                craft_store::records::get_setting(conn, SKILLS_DISABLED_KEY).unwrap(),
                None,
                "清空后键被删除"
            );
            Ok(())
        })
        .unwrap();

        // 无键 / 空前缀列表 / 无变化：均为 0 且不报错
        db.with(|conn| {
            assert_eq!(prune_disabled_paths(conn, &[PathBuf::from("/nowhere")]).unwrap(), 0);
            assert_eq!(prune_disabled_paths(conn, &[]).unwrap(), 0);
            craft_store::records::set_setting(conn, SKILLS_DISABLED_KEY, &serde_json::json!(["/keep/me/SKILL.md"]))
                .unwrap();
            assert_eq!(prune_disabled_paths(conn, &[PathBuf::from("/nowhere")]).unwrap(), 0);
            assert!(
                craft_store::records::get_setting(conn, SKILLS_DISABLED_KEY).unwrap().is_some(),
                "无变化不写回"
            );
            Ok(())
        })
        .unwrap();
        let _ = std::fs::remove_dir_all(dir.path());
    }

    #[test]
    fn store_lock_serializes_concurrent_sections() {
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let events = Arc::new(std::sync::Mutex::new(Vec::<(&'static str, usize)>::new()));
        let mut handles = Vec::new();
        for id in 0..2 {
            let events = Arc::clone(&events);
            handles.push(std::thread::spawn(move || {
                with_store_lock(|| {
                    events.lock().unwrap().push(("enter", id));
                    std::thread::sleep(Duration::from_millis(120));
                    events.lock().unwrap().push(("exit", id));
                });
            }));
        }
        let started = Instant::now();
        for handle in handles {
            handle.join().unwrap();
        }
        let elapsed = started.elapsed();
        // 串行化：两段各 ~120ms，总耗时必须超过单段时长（并行则 ~120ms 内完成）
        assert!(
            elapsed >= Duration::from_millis(240),
            "两段应串行执行（总耗时 {elapsed:?} < 240ms 说明未互斥）"
        );
        // 进入 / 退出严格交替（不出现两次进入交错）
        let sequence = events.lock().unwrap();
        for (index, (kind, _)) in sequence.iter().enumerate() {
            let expected = if index % 2 == 0 { "enter" } else { "exit" };
            assert_eq!(*kind, expected, "第 {index} 项应为 {expected}（实际 {kind}），说明区间交错");
        }
        assert_eq!(sequence.len(), 4);
    }
}

// ---------- 集成测试（fixture + 临时数据目录全链路） ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::MarketplaceLayout;
    use craft_store::records::list_audit;
    use craft_store::Db;
    use std::path::Path;

    const MARKET_ID: &str = "demo-market";
    const PLUGIN: &str = "demo";

    /// 隔离测试环境：临时数据目录（CRAFTWORK_HOME 语义）+ 商店布局 + 独立 SQLite。
    /// 下划线前缀是有意为之：该字段只被「持有」、从不读取，用来绑定临时目录的生命周期
    /// ——字段一旦被丢弃，目录立即删除，布局与数据库随即失效。Rust 对下划线开头的字段
    /// 不做 dead_code 检查，正好表达「存活期守卫」这一语义，也不诱导后来者把它当无用字段删掉。
    struct TestEnv {
        _home: tempfile::TempDir,
        layout: MarketplaceLayout,
        db: Db,
    }

    impl TestEnv {
        fn new(tag: &str) -> Self {
            let home = tempfile::tempdir().expect("创建临时数据目录失败");
            let layout = MarketplaceLayout::new(home.path().join("skills-marketplace"));
            let db = Db::open(&home.path().join("craftwork.db")).expect("打开测试库失败");
            let _ = tag;
            Self { _home: home, layout, db }
        }

        fn orchestrator(&self) -> Orchestrator<'_> {
            Orchestrator::new(&self.layout, &self.db)
        }

        /// 添加市场源行（repo_url 由测试给；本地 fixture 用裸路径 clone）。
        fn add_market(&self, repo_url: &str) {
            self.db
                .with(|conn| {
                    store::add_marketplace(
                        conn,
                        store::NewMarketplace { id: MARKET_ID, source: MARKET_ID, repo_url },
                    )
                })
                .expect("添加市场源失败");
        }

        /// 直接放置市场快照（不经过 clone：条目 version 驱动版本解析的测试用）。
        fn place_snapshot(&self, files: &[(&str, &str)]) {
            write_tree(&self.layout.marketplace_snapshot_dir(MARKET_ID), files);
        }

        fn audit_decisions(&self) -> Vec<String> {
            self.db
                .with(|conn| list_audit(conn, 500).map(|entries| entries.into_iter().map(|e| e.decision).collect()))
                .expect("读取审计失败")
        }

        fn audit_details(&self, action: &str) -> Vec<String> {
            self.db
                .with(|conn| {
                    list_audit(conn, 500).map(|entries| {
                        entries
                            .into_iter()
                            .filter(|e| e.decision == action)
                            .map(|e| e.detail)
                            .collect()
                    })
                })
                .expect("读取审计失败")
        }
    }

    /// 在目录内写最小文件树（相对路径 + 文本内容），父目录按需创建。
    fn write_tree(root: &Path, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("创建 fixture 目录失败");
            }
            std::fs::write(&path, content).expect("写入 fixture 文件失败");
        }
    }

    /// git 可用性探测（本机无 git 时跳过依赖真仓库的测试）。
    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// 在 `dir` 建本地 fixture 仓库（git CLI；写文件 → init → commit），返回 HEAD 7 位
    /// 短码。使用 symbolic-ref 固定分支名（比 `init -b` 兼容更老的 git）。
    fn make_git_repo(dir: &Path, files: &[(&str, &str)]) -> Option<String> {
        if !git_available() {
            return None;
        }
        write_tree(dir, files);
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git").args(args).current_dir(dir).output().expect("git 执行失败");
            assert!(out.status.success(), "git {args:?} 失败：{}", String::from_utf8_lossy(&out.stderr));
            out
        };
        run(&["init"]);
        run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
        run(&["add", "-A"]);
        run(&["-c", "user.name=fixture", "-c", "user.email=fixture@example.com", "commit", "-m", "fixture"]);
        let out = run(&["rev-parse", "--short=7", "HEAD"]);
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// 市场清单 JSON（形态 2 Claude 兼容：本票集成测试统一用该形态）。
    fn market_json(entries: &str) -> String {
        format!(r#"{{"name":"{MARKET_ID}","plugins":[{entries}]}}"#)
    }

    /// 常规插件条目（相对路径来源、条目声明版本）。
    fn versioned_entry(version: &str) -> String {
        format!(r#"{{"name":"{PLUGIN}","description":"演示插件","version":"{version}","source":"./"}}"#)
    }

    /// 磁盘上当前版本目录（layout 计算路径）。
    fn version_dir(env: &TestEnv, version: &str) -> PathBuf {
        env.layout.version_dir(MARKET_ID, PLUGIN, version)
    }

    /// 市场清单标准位置（形态 2）。
    fn market_manifest_rel() -> &'static str {
        crate::manifest::paths::CLAUDE_MARKET
    }

    // ---------- 市场刷新 ----------

    #[test]
    fn refresh_clones_snapshot_probes_manifest_and_records_success() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        let env = TestEnv::new("refresh-ok");
        let fixture = tempfile::tempdir().unwrap();
        let sha = make_git_repo(
            fixture.path(),
            &[
                (".claude-plugin/marketplace.json", &market_json(&versioned_entry("1.0.0"))),
                ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
            ],
        )
        .expect("git fixture 构建失败");
        env.add_market(fixture.path().to_string_lossy().as_ref());

        let outcome = env.orchestrator().refresh_market(MARKET_ID).expect("刷新应成功");
        assert_eq!(outcome.marketplace_id, MARKET_ID);
        assert_eq!(outcome.head_sha, sha, "clone 的 HEAD 短码透出");
        assert!(!outcome.fell_back, "本地路径 clone 无加速改写");
        assert_eq!(outcome.entry_count, 1);
        assert_eq!(outcome.manifest_kind, crate::manifest::manifest_kind::CLAUDE_MARKET);

        // 快照已落位（clone 产物含 .git）
        let snapshot = env.layout.marketplace_snapshot_dir(MARKET_ID);
        assert!(snapshot.is_dir());
        assert!(snapshot.join(".git").is_dir(), "快照是 clone 产物（保留 .git 供 sha 回读）");
        assert!(snapshot.join("skills/demo/SKILL.md").is_file());

        // store：成功时间落列、无失败原因
        env.db
            .with(|conn| {
                let row = store::get_marketplace(conn, MARKET_ID).unwrap();
                assert!(row.last_refreshed_at.is_some(), "刷新成功应写时间");
                assert_eq!(row.last_refresh_failure, None);
                Ok(())
            })
            .unwrap();
        // 审计：成功一条（detail 含条目数与 sha）
        let details = env.audit_details(audit_action::MARKET_REFRESH);
        assert_eq!(details.len(), 1);
        assert!(details[0].contains("刷新市场"), "实际 {}", details[0]);
        assert!(details[0].contains(&sha));

        // 快照 HEAD 回读一致（纯文本读取链路对拍 git 真实值）
        assert_eq!(read_snapshot_head_sha(&snapshot).as_deref(), Some(sha.as_str()));
    }

    #[test]
    fn refresh_failure_keeps_old_snapshot_and_records_failure_audited() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        let env = TestEnv::new("refresh-fail");
        // 先放一份旧快照（含技能文件作「旧内容」标记），刷新目标是不存在的仓库路径
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/old/SKILL.md", "旧内容"),
        ]);
        env.add_market("/tmp/cw-marketplace-no-such-repo-does-not-exist");

        let err = env.orchestrator().refresh_market(MARKET_ID).expect_err("刷新应失败");
        assert!(err.to_string().contains("失败") || err.to_string().contains("不存在"), "实际 {err}");

        // 旧快照保留
        let snapshot = env.layout.marketplace_snapshot_dir(MARKET_ID);
        assert!(snapshot.is_dir(), "失败后旧快照应保留");
        assert!(snapshot.join("skills/old/SKILL.md").is_file(), "旧快照内容未被破坏");

        // store：失败原因落列、成功时间不动
        env.db
            .with(|conn| {
                let row = store::get_marketplace(conn, MARKET_ID).unwrap();
                assert_eq!(row.last_refreshed_at, None, "从未成功刷新");
                assert!(row.last_refresh_failure.is_some(), "失败原因应落列");
                Ok(())
            })
            .unwrap();
        // 审计：失败一条，detail 带原因
        let details = env.audit_details(audit_action::MARKET_REFRESH);
        assert_eq!(details.len(), 1);
        assert!(details[0].contains("刷新市场失败"), "实际 {}", details[0]);
    }

    #[test]
    fn refresh_rejects_non_skill_source_and_renamed_manifest_without_replacing_snapshot() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        // 场景一：clone 成功但不是技能源 → 拒绝替换
        let env = TestEnv::new("refresh-not-source");
        env.place_snapshot(&[(market_manifest_rel(), &market_json(&versioned_entry("1.0.0")))]);
        let fixture = tempfile::tempdir().unwrap();
        make_git_repo(fixture.path(), &[("README.md", "# 普通仓库")]).expect("git fixture 构建失败");
        env.add_market(fixture.path().to_string_lossy().as_ref());
        let err = env.orchestrator().refresh_market(MARKET_ID).expect_err("非技能源应拒绝");
        assert!(err.to_string().contains("不是技能源"), "实际 {err}");
        assert!(
            env.layout.marketplace_snapshot_dir(MARKET_ID).join(".claude-plugin").is_dir(),
            "旧快照保留"
        );

        // 场景二：清单 name 与市场 id 不一致 → 拒绝替换（上游改名需显式移除重加）
        let env = TestEnv::new("refresh-renamed");
        env.place_snapshot(&[(market_manifest_rel(), &market_json(&versioned_entry("1.0.0")))]);
        let fixture = tempfile::tempdir().unwrap();
        let renamed = r#"{"name":"renamed-market","plugins":[]}"#;
        make_git_repo(
            fixture.path(),
            &[(".claude-plugin/marketplace.json", renamed), ("skills/x/SKILL.md", "---\nname: x\n---\n")],
        )
        .expect("git fixture 构建失败");
        env.add_market(fixture.path().to_string_lossy().as_ref());
        let err = env.orchestrator().refresh_market(MARKET_ID).expect_err("改名市场应拒绝");
        assert!(err.to_string().contains("不一致"), "实际 {err}");
        assert!(
            env.layout.marketplace_snapshot_dir(MARKET_ID).join(".claude-plugin").is_dir(),
            "旧快照保留"
        );
        env.db
            .with(|conn| {
                let row = store::get_marketplace(conn, MARKET_ID).unwrap();
                assert!(row.last_refresh_failure.is_some(), "两种拒绝都落失败原因");
                Ok(())
            })
            .unwrap();
    }

    // ---------- 安装（条目相对路径形态） ----------

    #[test]
    fn install_entry_relative_full_flow_pins_row_dir_and_audit() {
        let env = TestEnv::new("install-basic");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
            ("skills/demo/reference.md", "参考"),
            ("commands/run.md", "# 命令"),
        ]);

        let outcome = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装应成功");
        let plugin_id = store::plugin_id(PLUGIN, MARKET_ID);
        assert_eq!(outcome.plugin_id, plugin_id);
        assert_eq!(outcome.version, "1.0.0");
        assert_eq!(outcome.version_source, "market-entry", "无插件清单时条目 version 生效");
        assert_eq!(outcome.skill_count, 1, "skills/ 下 1 个 SKILL.md");
        assert_eq!(outcome.kept_versions, vec!["1.0.0".to_string()]);

        // 磁盘：版本目录含 skills 全目录 + commands/（扫描票 #6 的输入）+ 无清单副本（无清单）
        let dir = version_dir(&env, "1.0.0");
        assert!(dir.join("skills/demo/SKILL.md").is_file());
        assert!(dir.join("skills/demo/reference.md").is_file(), "技能附属文件随目录拷贝");
        assert!(dir.join("commands/run.md").is_file(), "commands/ 目录一并进缓存");
        assert!(!dir.join(".git").exists(), "缓存是拷贝不是 clone");

        // store 行：指针、versions、install_path、来源、默认启用
        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &plugin_id).unwrap();
                assert_eq!(row.current_version, "1.0.0");
                assert_eq!(row.versions, vec!["1.0.0".to_string()]);
                assert_eq!(row.install_path, dir.to_string_lossy().to_string());
                assert_eq!(row.source_kind, store::source_kind::ENTRY_RELATIVE);
                assert_eq!(row.source_ref, "./");
                assert_eq!(row.source_sha, None, "手工快照无 .git，无 sha pin");
                assert!(row.enabled, "新装默认启用");
                Ok(())
            })
            .unwrap();
        // 审计
        let details = env.audit_details(audit_action::PLUGIN_INSTALL);
        assert_eq!(details.len(), 1);
        assert!(details[0].contains("1.0.0") && details[0].contains(&plugin_id), "实际 {}", details[0]);
    }

    #[test]
    fn install_entry_skills_array_copies_only_declared_paths() {
        let env = TestEnv::new("install-declared");
        env.add_market("https://github.com/example/repo");
        let entries = format!(
            r#"{{"name":"{PLUGIN}","description":"anthropics 形态","version":"2.0.0","source":"./","skills":["document/docx","document/pdf"]}}"#
        );
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&entries)),
            ("document/docx/SKILL.md", "---\nname: docx\n---\n正文"),
            ("document/docx/extra.md", "附属"),
            ("document/pdf/SKILL.md", "---\nname: pdf\n---\n正文"),
            ("skills/undeclared/SKILL.md", "---\nname: undeclared\n---\n不该被拷贝"),
            ("commands/x.md", "不该被拷贝"),
        ]);

        let outcome = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装应成功");
        assert_eq!(outcome.skill_count, 2, "仅声明的两个技能");
        let dir = version_dir(&env, "2.0.0");
        // 声明路径保持相对结构落到 skills/ 下
        assert!(dir.join("skills/document/docx/SKILL.md").is_file());
        assert!(dir.join("skills/document/docx/extra.md").is_file());
        assert!(dir.join("skills/document/pdf/SKILL.md").is_file());
        // 未声明内容不进缓存
        assert!(!dir.join("skills/undeclared").exists(), "仅拷贝声明路径");
        assert!(!dir.join("commands").exists(), "声明模式不拷 commands/");
    }

    #[test]
    fn install_repo_entry_clones_plugin_and_pins_clone_sha() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        let env = TestEnv::new("install-repo");
        let fixture = tempfile::tempdir().unwrap();
        let sha = make_git_repo(
            fixture.path(),
            &[
                (".craftwork/plugin.json", r#"{"name":"demo","version":"3.1.0","description":"独立仓库插件"}"#),
                ("skills/only/SKILL.md", "---\nname: only\n---\n正文"),
            ],
        )
        .expect("git fixture 构建失败");
        // 市场条目 source 指向本地 fixture 仓库（file:// 形态，REPO 分类含 :// 即成立）。
        // 路径统一正斜杠并补足第三斜杠：Windows 的 `\` 直接进 JSON 是非法转义
        //（实测 Manifest Parse invalid escape），`file://C:/...` 的盘符会被当 host，
        // 需 `file:///C:/...`；unix 的 display 以 / 开头，两者拼出统一的三斜杠形态。
        let repo_path = fixture.path().to_string_lossy().replace('\\', "/");
        let repo_url = if repo_path.starts_with('/') { format!("file://{repo_path}") } else { format!("file:///{repo_path}") };
        let entries = format!(
            r#"{{"name":"{PLUGIN}","description":"repo 形态","source":"{repo_url}"}}"#
        );
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[(market_manifest_rel(), &market_json(&entries))]);

        let outcome = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装应成功");
        assert_eq!(outcome.version, "3.1.0", "插件清单 version 优先于其他来源");
        assert_eq!(outcome.version_source, "plugin-manifest");

        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &store::plugin_id(PLUGIN, MARKET_ID)).unwrap();
                assert_eq!(row.source_kind, store::source_kind::REPO);
                assert_eq!(row.source_ref, repo_url);
                assert_eq!(row.source_sha.as_deref(), Some(sha.as_str()), "pin clone 的 HEAD 短码");
                Ok(())
            })
            .unwrap();
        // 临时 clone 目录已清理（布局根 .tmp 不留残留）
        let tmp_root = env.layout.root().join(".tmp");
        let residues = std::fs::read_dir(&tmp_root).map(|d| d.count()).unwrap_or(0);
        assert_eq!(residues, 0, "临时目录应被 RAII 清理");
    }

    #[test]
    fn install_falls_back_to_snapshot_sha_version_when_no_version_declared() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        let env = TestEnv::new("install-sha-fallback");
        let fixture = tempfile::tempdir().unwrap();
        let sha = make_git_repo(
            fixture.path(),
            &[
                // 条目无 version、插件目录无清单：裸技能集式回退
                (".claude-plugin/marketplace.json", &market_json(r#"{"name":"demo","source":"./"}"#)),
                ("skills/bare/SKILL.md", "---\nname: bare\n---\n正文"),
            ],
        )
        .expect("git fixture 构建失败");
        env.add_market(fixture.path().to_string_lossy().as_ref());
        // 用刷新把真 clone 快照放到位（同时验证 clone → install 的 sha 链）
        env.orchestrator().refresh_market(MARKET_ID).expect("刷新应成功");

        let outcome = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装应成功");
        assert_eq!(outcome.version, sha, "版本回退 = 快照 HEAD 短码");
        assert_eq!(outcome.version_source, "git-sha");
        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &store::plugin_id(PLUGIN, MARKET_ID)).unwrap();
                assert_eq!(row.source_sha.as_deref(), Some(sha.as_str()));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn install_plugin_manifest_copy_is_cached_for_version_reads() {
        let env = TestEnv::new("install-manifest-copy");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("4.0.0"))),
            (".craftwork/plugin.json", r#"{"name":"demo","version":"4.0.0","hooks":"./hooks.json"}"#),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);
        let outcome = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装应成功");
        assert_eq!(outcome.version, "4.0.0", "插件清单 version 优先");
        assert_eq!(outcome.version_source, "plugin-manifest");
        // 清单副本原位落在缓存（版本回读与详情复用同一探测链）
        let dir = version_dir(&env, "4.0.0");
        let copy = dir.join(crate::manifest::paths::CRAFTWORK_PLUGIN);
        assert!(copy.is_file(), "清单副本按相对路径保留");
        let probed = crate::manifest::probe_plugin_manifest(&dir).expect("缓存内清单可回读").expect("应探到清单");
        assert_eq!(probed.name, PLUGIN);
        assert_eq!(probed.version.as_deref(), Some("4.0.0"));
        assert!(probed.has_hooks, "组件标注随副本保留");
    }

    #[test]
    fn install_rejects_manifest_name_mismatch_and_bad_entries_and_dup_and_missing_snapshot() {
        // name 不一致拒绝（票内明确要求）
        let env = TestEnv::new("install-mismatch");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            (".claude-plugin/plugin.json", r#"{"name":"other-name","version":"9.9.9"}"#),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);
        let err = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).unwrap_err();
        assert!(err.to_string().contains("不一致"), "实际 {err}");
        assert!(matches!(err, OrchestrateError::Other(_)));
        assert!(!version_dir(&env, "9.9.9").exists(), "拒绝安装不留缓存目录");

        // 条目声明的技能路径不存在 → 拒绝
        let env = TestEnv::new("install-missing-skill");
        env.add_market("https://github.com/example/repo");
        let entries = format!(
            r#"{{"name":"{PLUGIN}","version":"1.0.0","source":"./","skills":["document/docx"]}}"#
        );
        env.place_snapshot(&[(market_manifest_rel(), &market_json(&entries))]);
        let err = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).unwrap_err();
        assert!(err.to_string().contains("不存在"), "实际 {err}");

        // 重复安装拒绝
        let env = TestEnv::new("install-dup");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);
        env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("首次安装成功");
        let err = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).unwrap_err();
        assert!(err.to_string().contains("已安装"), "实际 {err}");

        // 市场不存在 / 快照缺失提示先刷新
        let env = TestEnv::new("install-no-market");
        env.db
            .with(|conn| {
                store::add_marketplace(
                    conn,
                    store::NewMarketplace { id: MARKET_ID, source: MARKET_ID, repo_url: "https://github.com/x/y" },
                )
            })
            .unwrap();
        let err = env.orchestrator().install_plugin(MARKET_ID, PLUGIN).unwrap_err();
        assert!(err.to_string().contains("请先刷新"), "实际 {err}");

        // 条目不存在
        let env = TestEnv::new("install-no-entry");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[(market_manifest_rel(), &market_json(&versioned_entry("1.0.0")))]);
        let err = env.orchestrator().install_plugin(MARKET_ID, "no-such").unwrap_err();
        assert!(err.to_string().contains("不存在插件条目"), "实际 {err}");
    }

    // ---------- 升级 / 回滚 / 卸载（全链路） ----------

    #[test]
    fn full_lifecycle_install_update_rollback_update_uninstall() {
        let env = TestEnv::new("lifecycle");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n版本一"),
        ]);

        // 1. 安装 1.0.0
        let orch = env.orchestrator();
        let outcome = orch.install_plugin(MARKET_ID, PLUGIN).expect("安装失败");
        assert_eq!(outcome.version, "1.0.0");
        assert_eq!(outcome.kept_versions, vec!["1.0.0".to_string()]);
        let plugin_id = store::plugin_id(PLUGIN, MARKET_ID);

        // 用户停用（升级后必须保留的选择）
        orch.set_plugin_enabled(MARKET_ID, PLUGIN, false).expect("停用失败");
        env.db
            .with(|conn| {
                assert!(!store::get_plugin(conn, &plugin_id).unwrap().enabled);
                Ok(())
            })
            .unwrap();

        // 2. 升级：快照清单改 2.0.0 → update
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("2.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n版本二"),
        ]);
        let outcome = orch.update_plugin(MARKET_ID, PLUGIN).expect("升级失败");
        assert_eq!(outcome.version, "2.0.0");
        assert_eq!(outcome.pruned, Vec::<String>::new(), "未超上限不裁剪");
        assert!(version_dir(&env, "1.0.0").is_dir(), "旧版本目录保留（回滚依据）");
        assert!(version_dir(&env, "2.0.0").join("skills/demo/SKILL.md").is_file());
        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &plugin_id).unwrap();
                assert_eq!(row.versions, vec!["1.0.0".to_string(), "2.0.0".to_string()]);
                assert_eq!(row.current_version, "2.0.0");
                assert_eq!(
                    row.install_path,
                    version_dir(&env, "2.0.0").to_string_lossy().to_string(),
                    "install_path 同步到新版本目录"
                );
                assert!(!row.enabled, "升级不覆盖用户的启停选择（spec 第 4 节）");
                Ok(())
            })
            .unwrap();

        // 3. 回滚 1.0.0：指针切回、目录不变
        let rolled = orch.rollback_plugin(MARKET_ID, PLUGIN, "1.0.0").expect("回滚失败");
        assert_eq!(rolled.from_version, "2.0.0");
        assert_eq!(rolled.to_version, "1.0.0");
        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &plugin_id).unwrap();
                assert_eq!(row.current_version, "1.0.0");
                assert_eq!(row.versions, vec!["1.0.0".to_string(), "2.0.0".to_string()], "versions 不变");
                assert_eq!(row.install_path, version_dir(&env, "1.0.0").to_string_lossy().to_string());
                Ok(())
            })
            .unwrap();

        // 4. 可再切回（回滚双向）
        orch.rollback_plugin(MARKET_ID, PLUGIN, "2.0.0").expect("再切回失败");
        env.db
            .with(|conn| {
                assert_eq!(store::get_plugin(conn, &plugin_id).unwrap().current_version, "2.0.0");
                Ok(())
            })
            .unwrap();

        // 5. 再升级 3.0.0（指针切换 + versions 追加；1.0.0 仍在保留列表）
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("3.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n版本三"),
        ]);
        let outcome = orch.update_plugin(MARKET_ID, PLUGIN).expect("再升级失败");
        assert_eq!(outcome.version, "3.0.0");
        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &plugin_id).unwrap();
                assert_eq!(row.versions, vec!["1.0.0".to_string(), "2.0.0".to_string(), "3.0.0".to_string()]);
                assert!(!row.enabled, "全程保留停用选择");
                Ok(())
            })
            .unwrap();

        // 6. 升级到 4.0.0：超出保留上限 3，最旧的 1.0.0 被裁剪
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("4.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n版本四"),
        ]);
        let outcome = orch.update_plugin(MARKET_ID, PLUGIN).expect("第四次升级失败");
        assert_eq!(outcome.pruned, vec!["1.0.0".to_string()], "超出上限裁最旧");
        assert!(!version_dir(&env, "1.0.0").exists(), "被裁版本目录已删除");
        assert!(version_dir(&env, "2.0.0").is_dir());
        // 已裁剪版本不可回滚（编排层磁盘目录检查先于 store 保留列表校验命中）
        let err = orch.rollback_plugin(MARKET_ID, PLUGIN, "1.0.0").unwrap_err();
        assert!(
            err.to_string().contains("无法回滚") || err.to_string().contains("不在"),
            "裁剪版本拒绝回滚：{err}"
        );

        // 7. 卸载：行删 + 全版本目录删 + 禁用记录清理 + 审计
        env.db
            .with(|conn| {
                craft_store::records::set_setting(
                    conn,
                    SKILLS_DISABLED_KEY,
                    &serde_json::json!([
                        version_dir(&env, "4.0.0").join("skills/demo/SKILL.md"),
                        "/home/user/.craftwork/skills/handmade/SKILL.md"
                    ]),
                )
            })
            .unwrap();
        let removed = orch.uninstall_plugin(MARKET_ID, PLUGIN).expect("卸载失败");
        assert_eq!(removed.plugin_id, plugin_id);
        assert_eq!(removed.removed_versions, vec!["2.0.0".to_string(), "3.0.0".to_string(), "4.0.0".to_string()]);
        assert_eq!(removed.pruned_disabled, 1, "只清插件缓存前缀下的禁用记录");
        assert!(!env.layout.plugin_cache_dir(MARKET_ID, PLUGIN).exists(), "整个插件缓存目录删除");
        env.db
            .with(|conn| {
                assert!(store::get_plugin(conn, &plugin_id).is_err(), "行已删");
                let kept = craft_store::records::get_setting(conn, SKILLS_DISABLED_KEY).unwrap().unwrap();
                assert_eq!(kept, serde_json::json!(["/home/user/.craftwork/skills/handmade/SKILL.md"]));
                Ok(())
            })
            .unwrap();
        // 卸载后回滚：插件行已删，先撞 NotFound（目录缺失路径已由上方裁剪版本用例覆盖）
        let err = orch.rollback_plugin(MARKET_ID, PLUGIN, "4.0.0").unwrap_err();
        assert!(
            err.to_string().contains("插件不存在") || err.to_string().contains("缓存目录缺失"),
            "实际 {err}"
        );

        // 全程审计：安装 1 / 启停 1 / 升级 3 / 回滚 2 / 卸载 1
        let decisions = env.audit_decisions();
        let count = |name: &str| decisions.iter().filter(|d| d == &name).count();
        assert_eq!(count(audit_action::PLUGIN_INSTALL), 1);
        assert_eq!(count(audit_action::PLUGIN_SET_ENABLED), 1);
        assert_eq!(count(audit_action::PLUGIN_UPDATE), 3);
        assert_eq!(count(audit_action::PLUGIN_ROLLBACK), 2);
        assert_eq!(count(audit_action::PLUGIN_UNINSTALL), 1);
        let update_details = env.audit_details(audit_action::PLUGIN_UPDATE);
        assert!(
            update_details.iter().any(|d| d.contains("1.0.0 → 2.0.0")),
            "升级审计含版本区间（列表按时间降序）：{:?}",
            update_details
        );
        let uninstall_details = env.audit_details(audit_action::PLUGIN_UNINSTALL);
        assert!(uninstall_details[0].contains("3 个版本目录"), "卸载审计含版本数：{}", uninstall_details[0]);
    }

    #[test]
    fn update_reinstalls_same_version_in_place() {
        let env = TestEnv::new("update-same");
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n旧内容"),
        ]);
        let orch = env.orchestrator();
        orch.install_plugin(MARKET_ID, PLUGIN).expect("安装失败");
        // 版本号不变、内容变化（sha 轴升级的常见形态）
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n新内容"),
        ]);
        let outcome = orch.update_plugin(MARKET_ID, PLUGIN).expect("原地重装失败");
        assert_eq!(outcome.version, "1.0.0");
        assert_eq!(outcome.kept_versions, vec!["1.0.0".to_string()], "同版本不重复追加");
        let dir = version_dir(&env, "1.0.0");
        let content = std::fs::read_to_string(dir.join("skills/demo/SKILL.md")).unwrap();
        assert!(content.contains("新内容"), "缓存目录已被新载荷替换");
        env.db
            .with(|conn| {
                let row = store::get_plugin(conn, &store::plugin_id(PLUGIN, MARKET_ID)).unwrap();
                assert_eq!(row.versions, vec!["1.0.0".to_string()]);
                Ok(())
            })
            .unwrap();
        let details = env.audit_details(audit_action::PLUGIN_UPDATE);
        assert!(details[0].contains("原地重装"), "实际 {}", details[0]);
    }

    // ---------- 更新检测（编排级） ----------

    #[test]
    fn check_updates_reports_semver_sha_change_and_missing_entry() {
        let env = TestEnv::new("check-updates");
        env.add_market("https://github.com/example/repo");
        // 快照含三个条目：semver 升级可用 / 无变化 / 即将消失
        let entries = r#"{"name":"bump","version":"1.0.0","source":"./"},
               {"name":"same","version":"2.0.0","source":"./"},
               {"name":"vanish","version":"1.0.0","source":"./"}"#;
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(entries)),
            ("skills/bump/SKILL.md", "---\nname: bump\n---\n"),
            ("skills/same/SKILL.md", "---\nname: same\n---\n"),
            ("skills/vanish/SKILL.md", "---\nname: vanish\n---\n"),
        ]);
        let orch = env.orchestrator();
        for name in ["bump", "same", "vanish"] {
            orch.install_plugin(MARKET_ID, name).expect("安装失败");
        }

        // 快照演进：bump 升到 1.1.0；same 不变；vanish 条目移除
        let entries_after = r#"{"name":"bump","version":"1.1.0","source":"./"},
               {"name":"same","version":"2.0.0","source":"./"}"#;
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(entries_after)),
            ("skills/bump/SKILL.md", "---\nname: bump\n---\n新"),
            ("skills/same/SKILL.md", "---\nname: same\n---\n"),
        ]);

        let statuses = orch.check_updates(MARKET_ID).expect("更新检测失败");
        let by_id = |name: &str| {
            statuses
                .iter()
                .find(|s| s.plugin_id == store::plugin_id(name, MARKET_ID))
                .unwrap_or_else(|| panic!("缺少 {name} 的状态"))
        };
        assert_eq!(
            by_id("bump").status,
            UpdateStatus::UpdateAvailable { available_version: "1.1.0".to_string() },
            "semver 升级 → update-available"
        );
        assert_eq!(by_id("same").status, UpdateStatus::None, "均未变 → none");
        assert_eq!(by_id("vanish").status, UpdateStatus::None, "条目已移除 → 不误报");

        // 版本不可比（sha 装的插件）+ 快照 sha 变化 → version-changed 需要 git fixture
        if git_available() {
            let fixture = tempfile::tempdir().unwrap();
            let sha1 = make_git_repo(
                fixture.path(),
                &[
                    (".claude-plugin/marketplace.json", &market_json(r#"{"name":"demo","source":"./"}"#)),
                    ("skills/demo/SKILL.md", "---\nname: demo\n---\n一"),
                ],
            )
            .expect("git fixture 构建失败");
            let env = TestEnv::new("check-sha-axis");
            env.add_market(fixture.path().to_string_lossy().as_ref());
            env.orchestrator().refresh_market(MARKET_ID).expect("刷新失败");
            env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装失败");
            env.db
                .with(|conn| {
                    let row = store::get_plugin(conn, &store::plugin_id(PLUGIN, MARKET_ID)).unwrap();
                    assert_eq!(row.source_sha.as_deref(), Some(sha1.as_str()));
                    Ok(())
                })
                .unwrap();

            // 上游新提交（内容变化、无版本字段）→ 刷新后 sha 轴命中
            let out = std::process::Command::new("git")
                .args(["commit", "-a", "-m", "second", "--allow-empty"])
                .current_dir(fixture.path())
                .env("GIT_AUTHOR_NAME", "fixture")
                .env("GIT_AUTHOR_EMAIL", "f@example.com")
                .env("GIT_COMMITTER_NAME", "fixture")
                .env("GIT_COMMITTER_EMAIL", "f@example.com")
                .output()
                .expect("git commit 失败");
            assert!(out.status.success());
            env.orchestrator().refresh_market(MARKET_ID).expect("二次刷新失败");
            let statuses = env.orchestrator().check_updates(MARKET_ID).expect("检测失败");
            assert_eq!(
                statuses[0].status,
                UpdateStatus::VersionChanged,
                "无版本字段仅 sha → sha 轴判定"
            );

            // 升级后指针 = 新 sha 短码，sha 相同 → none
            env.orchestrator().update_plugin(MARKET_ID, PLUGIN).expect("升级失败");
            let statuses = env.orchestrator().check_updates(MARKET_ID).expect("检测失败");
            assert_eq!(statuses[0].status, UpdateStatus::None, "升级后再检无更新");
            env.db
                .with(|conn| {
                    let row = store::get_plugin(conn, &store::plugin_id(PLUGIN, MARKET_ID)).unwrap();
                    assert_ne!(row.source_sha.as_deref(), Some(sha1.as_str()), "升级同步新 pin");
                    Ok(())
                })
                .unwrap();
        }

        // 快照缺失（未刷新）报可读错误
        let env = TestEnv::new("check-no-snapshot");
        env.add_market("https://github.com/example/repo");
        let err = env.orchestrator().check_updates(MARKET_ID).unwrap_err();
        assert!(err.to_string().contains("请先刷新"), "实际 {err}");
    }

    // ---------- 互斥（真实编排竞争） ----------

    #[test]
    fn concurrent_install_of_same_plugin_serializes_to_single_install() {
        use std::sync::Arc;
        let env = Arc::new(TestEnv::new("concurrent-install"));
        env.add_market("https://github.com/example/repo");
        env.place_snapshot(&[
            (market_manifest_rel(), &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);

        let mut handles = Vec::new();
        for _ in 0..2 {
            let env = Arc::clone(&env);
            handles.push(std::thread::spawn(move || env.orchestrator().install_plugin(MARKET_ID, PLUGIN)));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().expect("线程不应 panic")).collect();
        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok, 1, "两个并发安装应恰有一个成功（串行化后第二个撞「已安装」）：{results:?}");
        // 最终状态：一行、一个版本目录
        env.db
            .with(|conn| {
                let rows = store::list_plugins(conn).unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].versions, vec!["1.0.0".to_string()]);
                Ok(())
            })
            .unwrap();
        let plugin_versions = std::fs::read_dir(env.layout.plugin_cache_dir(MARKET_ID, PLUGIN))
            .expect("插件缓存目录应存在")
            .count();
        assert_eq!(plugin_versions, 1, "磁盘只有一个版本目录");
    }

    // ---------- 市场源添加 / 移除（服务层接线票 #7） ----------

    /// 本地 fixture 仓库（复用 [`make_git_repo`]）作为添加来源；返回（临时目录、HEAD 短码）。
    fn fixture_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let sha = make_git_repo(dir.path(), files).expect("git fixture 构建失败");
        (dir, sha)
    }

    #[test]
    fn add_market_clones_probes_places_snapshot_and_audits() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        let env = TestEnv::new("add-market-ok");
        let (fixture, sha) = fixture_repo(&[
            (".claude-plugin/marketplace.json", &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);
        let repo_url = fixture.path().to_string_lossy().to_string();

        let outcome = env.orchestrator().add_market(&repo_url, &repo_url).expect("添加应成功");
        assert_eq!(outcome.marketplace_id, MARKET_ID, "市场 id 取自清单 name");
        assert_eq!(outcome.head_sha, sha);
        assert_eq!(outcome.entry_count, 1);

        // 行落库 + 快照落位（添加后即有快照，无需再刷新一次）
        env.db
            .with(|conn| {
                let row = store::get_marketplace(conn, MARKET_ID).unwrap();
                assert_eq!(row.repo_url, repo_url);
                Ok(())
            })
            .unwrap();
        let snapshot = env.layout.marketplace_snapshot_dir(MARKET_ID);
        assert!(snapshot.join(".claude-plugin/marketplace.json").is_file());
        assert!(snapshot.join(".git").is_dir(), "快照是 clone 产物");
        // 审计
        let details = env.audit_details(audit_action::MARKET_ADD);
        assert_eq!(details.len(), 1);
        assert!(details[0].contains("添加市场源"), "实际 {}", details[0]);
    }

    #[test]
    fn add_market_rejects_duplicate_id_and_non_skill_source_without_rows() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        // 重名：同 id 已存在 → 拒绝并提示先移除（行数不变）
        let env = TestEnv::new("add-market-dup");
        let (fixture, _) = fixture_repo(&[
            (".claude-plugin/marketplace.json", &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);
        let repo_url = fixture.path().to_string_lossy().to_string();
        env.add_market(&repo_url);
        let err = env.orchestrator().add_market(&repo_url, &repo_url).unwrap_err();
        assert!(err.to_string().contains("请先移除"), "实际 {err}");
        env.db
            .with(|conn| {
                assert_eq!(store::list_marketplaces(conn).unwrap().len(), 1, "重复添加不产生新行");
                Ok(())
            })
            .unwrap();

        // 非技能源：clone 成功但 probe 失败 → 不落行、审计失败记录
        let (bad, _) = fixture_repo(&[("README.md", "没有任何技能内容")]);
        let bad_url = bad.path().to_string_lossy().to_string();
        let err = env.orchestrator().add_market(&bad_url, &bad_url).unwrap_err();
        assert!(err.to_string().contains("不是技能源"), "实际 {err}");
        env.db
            .with(|conn| {
                assert_eq!(store::list_marketplaces(conn).unwrap().len(), 1, "非技能源不落行");
                Ok(())
            })
            .unwrap();
        let details = env.audit_details(audit_action::MARKET_ADD);
        assert_eq!(details.len(), 2, "成功一条 + 失败一条");
        assert!(details[1].contains("添加市场源失败"), "实际 {}", details[1]);
    }

    #[test]
    fn remove_market_rejects_with_plugins_then_succeeds_and_removes_snapshot() {
        if !git_available() {
            eprintln!("跳过：本机无 git CLI");
            return;
        }
        let env = TestEnv::new("remove-market");
        let (fixture, _) = fixture_repo(&[
            (".claude-plugin/marketplace.json", &market_json(&versioned_entry("1.0.0"))),
            ("skills/demo/SKILL.md", "---\nname: demo\n---\n正文"),
        ]);
        let repo_url = fixture.path().to_string_lossy().to_string();
        env.orchestrator().add_market(&repo_url, &repo_url).expect("添加应成功");

        // 装一个插件后移除被拒（保护语义）
        let snapshot = env.layout.marketplace_snapshot_dir(MARKET_ID);
        let _ = snapshot; // 快照已在
        env.orchestrator().install_plugin(MARKET_ID, PLUGIN).expect("安装应成功");
        let err = env.orchestrator().remove_market(MARKET_ID).unwrap_err();
        assert!(err.to_string().contains("先卸载") || err.to_string().contains("级联"), "实际 {err}");
        env.db
            .with(|conn| {
                assert!(store::get_marketplace(conn, MARKET_ID).is_ok(), "被拒后市场仍在");
                Ok(())
            })
            .unwrap();

        // 卸载插件后移除成功：行删除、快照目录删除、审计
        env.orchestrator().uninstall_plugin(MARKET_ID, PLUGIN).expect("卸载应成功");
        env.orchestrator().remove_market(MARKET_ID).expect("移除应成功");
        env.db
            .with(|conn| {
                assert!(matches!(store::get_marketplace(conn, MARKET_ID), Err(StoreError::NotFound(_))));
                Ok(())
            })
            .unwrap();
        assert!(
            !env.layout.marketplace_snapshot_dir(MARKET_ID).exists(),
            "移除后快照目录删除"
        );
        let details = env.audit_details(audit_action::MARKET_REMOVE);
        assert_eq!(details.len(), 1);
        assert!(details[0].contains("移除市场源"), "实际 {}", details[0]);

        // 不存在的市场：NotFound 透传
        let err = env.orchestrator().remove_market("no-such").unwrap_err();
        assert!(err.to_string().contains("不存在"), "实际 {err}");
    }

    // ---------- 临时目录清理 ----------

    #[test]
    fn cleanup_tmp_removes_crash_residue_and_tolerates_missing_root() {
        let env = TestEnv::new("cleanup-tmp");
        // 从未跑过商店动作：无 .tmp → Ok
        assert!(cleanup_tmp(&env.layout).is_ok());
        // 崩溃残留（模拟：手工放置 .tmp 内容）
        let residue = env.layout.root().join(".tmp").join("op-residue").join("clone");
        std::fs::create_dir_all(&residue).unwrap();
        std::fs::write(residue.join("HEAD"), "ref: refs/heads/main").unwrap();
        cleanup_tmp(&env.layout).expect("清理应成功");
        assert!(!env.layout.root().join(".tmp").exists(), "临时树整删");
        // 正式布局不受影响
        assert!(env.layout.root().exists(), "布局根保留");
    }
}
