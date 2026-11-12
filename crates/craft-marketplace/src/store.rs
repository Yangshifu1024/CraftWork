//! 技能商店两张表（marketplaces / skill_plugins，迁移 V14）的读写封装。
//!
//! 取锁纪律（AGENTS.md 已知坑）：`Db::lock_conn()` 是非重入互斥锁，同一表达式内
//! 嵌套两次取锁必死锁。本模块所有函数一律是接收 `&Connection` 的纯函数，由调用方
//! 取锁传入——crate 内部绝不自己取锁，不制造嵌套死锁面。多步操作（如级联删除）
//! 如需原子性，由调用方包 `Db::transaction`（事务内不得再取锁）。
//!
//! 分工边界：本模块只管表行。磁盘缓存目录（市场快照、版本化缓存）的创建、改名、
//! 裁剪与删除由编排层（spec 第 3 节原子激活）负责，两侧行为以 SQLite 行为权威。

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use craft_store::db::{now_millis, not_found, other_err, StoreError, StoreResult};

/// 插件 id 的唯一形态：`name@marketplace`。同一插件名可出现在多个市场下，
/// 引用必须始终带市场（对应 spec 第 2 节与 AGENTS.md「provider/model 引用」对称约定）。
pub fn plugin_id(name: &str, marketplace_id: &str) -> String {
    format!("{name}@{marketplace_id}")
}

/// 插件来源类型（skill_plugins.source_kind 列；spec 第 2 节市场条目 source 的两种形态）。
pub mod source_kind {
    /// 市场条目相对路径：插件在市场仓库内，安装时从市场快照直接拷贝，不二次下载。
    pub const ENTRY_RELATIVE: &str = "entry-relative";
    /// 独立仓库：按条目 source（owner/repo 或 git URL）二次 clone。
    pub const REPO: &str = "repo";
}

// ---------- 市场源（marketplaces） ----------

/// 已添加市场源行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketplaceRow {
    /// 清单 name（主键）。
    pub id: String,
    /// 用户输入原文（`https://` URL 或 `owner/repo` 缩写）。
    pub source: String,
    /// 展开后的 git 仓库 URL。
    pub repo_url: String,
    /// 最近一次成功刷新时间（毫秒）；None = 从未刷新成功。
    pub last_refreshed_at: Option<i64>,
    /// 最近一次刷新失败原因（可读中文）；成功刷新时清空。
    pub last_refresh_failure: Option<String>,
    /// 添加时间（毫秒）。
    pub created_at: i64,
}

/// 新增市场源入参。
pub struct NewMarketplace<'a> {
    pub id: &'a str,
    pub source: &'a str,
    pub repo_url: &'a str,
}

/// 添加市场源。重复 id 拒绝并给可读提示（spec 第 2 节：重复添加同名市场拒绝，提示先移除）。
pub fn add_marketplace(conn: &Connection, market: NewMarketplace<'_>) -> StoreResult<()> {
    let inserted = conn.execute(
        "INSERT INTO marketplaces (id, source, repo_url, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![market.id, market.source, market.repo_url, now_millis()],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
            Err(StoreError::Other(format!("市场源已存在：{}，请先移除后再添加", market.id)))
        }
        Err(e) => Err(other_err(e)),
    }
}

/// 读取单个市场源。
pub fn get_marketplace(conn: &Connection, id: &str) -> StoreResult<MarketplaceRow> {
    conn.query_row("SELECT * FROM marketplaces WHERE id = ?1", [id], to_marketplace)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound(format!("市场源不存在：{id}")),
            other => other_err(other),
        })
}

/// 全部市场源，按添加时间升序（UI 按添加顺序展示）。
pub fn list_marketplaces(conn: &Connection) -> StoreResult<Vec<MarketplaceRow>> {
    let mut stmt = conn
        .prepare("SELECT * FROM marketplaces ORDER BY created_at ASC, id ASC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([], to_marketplace)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 移除市场源（默认保护语义）：市场下仍有已安装插件时拒绝，提示先卸载或改用级联删除。
/// 磁盘缓存目录的清理由编排层负责。
pub fn remove_marketplace(conn: &Connection, id: &str) -> StoreResult<()> {
    let plugins = list_plugins_by_marketplace(conn, id)?;
    if !plugins.is_empty() {
        return Err(StoreError::Other(format!(
            "市场 {} 下仍有 {} 个已安装插件，请先卸载插件，或使用级联删除",
            id,
            plugins.len()
        )));
    }
    let deleted = conn.execute("DELETE FROM marketplaces WHERE id = ?1", [id]).map_err(other_err)?;
    if deleted == 0 {
        return not_found(format!("市场源不存在：{id}"));
    }
    Ok(())
}

/// 级联移除市场源：先删该市场全部插件行，再删市场行，返回级联删除的插件数
/// （调用方可用于提示「已随市场卸载 N 个插件」）。两条 DELETE 顺序执行，
/// 如需原子性请包 `Db::transaction`；磁盘缓存目录的清理由编排层负责。
pub fn remove_marketplace_cascade(conn: &Connection, id: &str) -> StoreResult<usize> {
    let plugins = list_plugins_by_marketplace(conn, id)?;
    conn.execute("DELETE FROM skill_plugins WHERE marketplace_id = ?1", [id]).map_err(other_err)?;
    let deleted = conn.execute("DELETE FROM marketplaces WHERE id = ?1", [id]).map_err(other_err)?;
    if deleted == 0 {
        return not_found(format!("市场源不存在：{id}"));
    }
    Ok(plugins.len())
}

/// 刷新成功：写最近刷新时间并清空失败原因。
pub fn set_marketplace_refreshed(conn: &Connection, id: &str, ts: i64) -> StoreResult<()> {
    let updated = conn
        .execute(
            "UPDATE marketplaces SET last_refreshed_at = ?2, last_refresh_failure = NULL WHERE id = ?1",
            rusqlite::params![id, ts],
        )
        .map_err(other_err)?;
    if updated == 0 {
        return not_found(format!("市场源不存在：{id}"));
    }
    Ok(())
}

/// 刷新失败：只记失败原因，不动 last_refreshed_at（上次成功时间仍用于刷新节流，spec 第 11 节）。
pub fn set_marketplace_refresh_failure(conn: &Connection, id: &str, failure: &str) -> StoreResult<()> {
    let updated = conn
        .execute(
            "UPDATE marketplaces SET last_refresh_failure = ?2 WHERE id = ?1",
            rusqlite::params![id, failure],
        )
        .map_err(other_err)?;
    if updated == 0 {
        return not_found(format!("市场源不存在：{id}"));
    }
    Ok(())
}

fn to_marketplace(row: &rusqlite::Row<'_>) -> rusqlite::Result<MarketplaceRow> {
    Ok(MarketplaceRow {
        id: row.get("id")?,
        source: row.get("source")?,
        repo_url: row.get("repo_url")?,
        last_refreshed_at: row.get("last_refreshed_at")?,
        last_refresh_failure: row.get("last_refresh_failure")?,
        created_at: row.get("created_at")?,
    })
}

// ---------- 已安装插件（skill_plugins） ----------

/// 已安装插件行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginRow {
    /// `name@marketplace`（主键）。
    pub id: String,
    pub marketplace_id: String,
    /// 插件名（不带市场后缀，与市场清单条目一致才可安装，spec 第 2 节）。
    pub name: String,
    /// 生效版本指针。缓存路径段 = 已安装清单 = UI 展示三者一致（spec 第 4 节）。
    pub current_version: String,
    /// 磁盘保留的版本目录名（JSON 数组，含 current_version 与历史版本）。
    pub versions: Vec<String>,
    /// 当前版本（current_version 指针）缓存目录绝对路径。
    pub install_path: String,
    /// 来源类型（`source_kind::ENTRY_RELATIVE` / `source_kind::REPO`）。
    pub source_kind: String,
    /// 来源 URL / 路径（市场条目 source 解析结果）。
    pub source_ref: String,
    /// 安装时 pin 的 git sha（双轴更新检测用，spec 第 4 节）；无 pin 为 None。
    pub source_sha: Option<String>,
    /// 插件级启停（spec 第 5 节两层开关的插件层；停用后该插件全部技能根退出扫描）。
    pub enabled: bool,
    /// 安装时间（毫秒）。
    pub installed_at: i64,
    /// 最近一次变更（安装 / 指针切换 / 启停）时间（毫秒）。
    pub updated_at: i64,
}

/// 新增已安装插件入参（原子激活第 2 步：SQLite 事务内写入 / 更新行，spec 第 3 节）。
pub struct NewPlugin<'a> {
    pub id: &'a str,
    pub marketplace_id: &'a str,
    pub name: &'a str,
    pub current_version: &'a str,
    /// 磁盘保留的版本目录名（至少含当前版本，否则拒绝）。
    pub versions: &'a [String],
    pub install_path: &'a str,
    pub source_kind: &'a str,
    pub source_ref: &'a str,
    /// 安装时 pin 的 git sha；无 pin 传 None。
    pub source_sha: Option<&'a str>,
}

/// 插入已安装插件行（新安装；重装同 id 拒绝——升级走 `append_plugin_version`，不走本函数）。
/// 默认 enabled = 1。外键约束下市场源必须已存在。
pub fn insert_plugin(conn: &Connection, plugin: NewPlugin<'_>) -> StoreResult<()> {
    if plugin.versions.is_empty() {
        return Err(StoreError::Other("versions 不能为空：至少要含当前版本".into()));
    }
    let versions = serde_json::to_string(plugin.versions).map_err(other_err)?;
    let ts = now_millis();
    let inserted = conn.execute(
        "INSERT INTO skill_plugins (id, marketplace_id, name, current_version, versions, install_path, source_kind, source_ref, source_sha, enabled, installed_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?10)",
        rusqlite::params![
            plugin.id,
            plugin.marketplace_id,
            plugin.name,
            plugin.current_version,
            versions,
            plugin.install_path,
            plugin.source_kind,
            plugin.source_ref,
            plugin.source_sha,
            ts,
        ],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY {
                Err(StoreError::Other(format!("市场源不存在：{}，请先添加对应市场", plugin.marketplace_id)))
            } else {
                Err(StoreError::Other(format!("插件已安装：{}，如需重装请先卸载", plugin.id)))
            }
        }
        Err(e) => Err(other_err(e)),
    }
}

/// 读取单个插件。
pub fn get_plugin(conn: &Connection, id: &str) -> StoreResult<PluginRow> {
    conn.query_row("SELECT * FROM skill_plugins WHERE id = ?1", [id], to_plugin)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound(format!("插件不存在：{id}")),
            other => other_err(other),
        })
}

/// 全部已安装插件，按安装时间升序（扫描按安装顺序逐插件注入技能根，spec 第 5 节）。
pub fn list_plugins(conn: &Connection) -> StoreResult<Vec<PluginRow>> {
    let mut stmt = conn
        .prepare("SELECT * FROM skill_plugins ORDER BY installed_at ASC, id ASC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([], to_plugin)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 某市场下的全部插件，按安装时间升序。
pub fn list_plugins_by_marketplace(conn: &Connection, marketplace_id: &str) -> StoreResult<Vec<PluginRow>> {
    let mut stmt = conn
        .prepare("SELECT * FROM skill_plugins WHERE marketplace_id = ?1 ORDER BY installed_at ASC, id ASC")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([marketplace_id], to_plugin)
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 切换版本指针（回滚用）：目标版本必须已在保留列表内，否则拒绝；versions 数组不变、
/// 只换指针，因此可以再切回去（spec 第 4 节「一键回滚上一版，可再切回」）。
///
/// install_path 不变；回滚编排应走 [`set_plugin_version_pointer_with_path`] 同步落列。
pub fn set_plugin_version_pointer(conn: &Connection, id: &str, version: &str) -> StoreResult<()> {
    let install_path = get_plugin(conn, id)?.install_path;
    set_plugin_version_pointer_with_path(conn, id, version, &install_path)
}

/// [`set_plugin_version_pointer`] 的带路径版本：切指针的同时更新 install_path 列
///（回滚后 install_path 恒等于当前版本目录路径）。
pub fn set_plugin_version_pointer_with_path(
    conn: &Connection,
    id: &str,
    version: &str,
    install_path: &str,
) -> StoreResult<()> {
    let plugin = get_plugin(conn, id)?;
    if !plugin.versions.iter().any(|v| v == version) {
        return not_found(format!("版本 {version} 不在插件 {id} 的保留版本列表中，无法切换"));
    }
    let updated = conn
        .execute(
            "UPDATE skill_plugins SET current_version = ?2, install_path = ?3, updated_at = ?4 WHERE id = ?1",
            rusqlite::params![id, version, install_path, now_millis()],
        )
        .map_err(other_err)?;
    if updated == 0 {
        return not_found(format!("插件不存在：{id}"));
    }
    Ok(())
}

/// 追加版本并切换指针（安装 / 升级用）：新版本追加到数组尾部（已在数组内则先移除旧位置，
/// 幂等），超出 keep_last 时从最旧一端裁剪；返回裁剪后的保留列表（编排层据此删除被裁剪
/// 的磁盘版本目录）。keep_last = 0 报错（至少要保留当前版本）。
///
/// install_path 不变；指针切换伴随目录路径变更的编排走 [`append_plugin_version_with_path`]。
pub fn append_plugin_version(conn: &Connection, id: &str, version: &str, keep_last: usize) -> StoreResult<Vec<String>> {
    let install_path = get_plugin(conn, id)?.install_path;
    append_plugin_version_with_path(conn, id, version, &install_path, keep_last)
}

/// [`append_plugin_version`] 的带路径版本：追加版本、切换指针的同时更新 install_path
/// 列（编排层升级到新版本目录后同步落列，保证 install_path 恒等于当前版本目录路径）。
pub fn append_plugin_version_with_path(
    conn: &Connection,
    id: &str,
    version: &str,
    install_path: &str,
    keep_last: usize,
) -> StoreResult<Vec<String>> {
    if keep_last == 0 {
        return Err(StoreError::Other("版本保留上限必须 ≥ 1".into()));
    }
    let plugin = get_plugin(conn, id)?;
    let mut versions: Vec<String> = plugin.versions.into_iter().filter(|v| v != version).collect();
    versions.push(version.to_string());
    if versions.len() > keep_last {
        versions.drain(..versions.len() - keep_last);
    }
    let updated = conn
        .execute(
            "UPDATE skill_plugins SET current_version = ?2, versions = ?3, install_path = ?4, updated_at = ?5 WHERE id = ?1",
            rusqlite::params![
                id,
                version,
                serde_json::to_string(&versions).map_err(other_err)?,
                install_path,
                now_millis()
            ],
        )
        .map_err(other_err)?;
    if updated == 0 {
        return not_found(format!("插件不存在：{id}"));
    }
    Ok(versions)
}

/// 升级时同步来源信息（source_kind / source_ref / source_sha 列）：编排层拉到新版后，
/// 新 pin（git sha 短码）与来源（条目 source 可能随上游清单变化）一并落列。
pub fn update_plugin_source(
    conn: &Connection,
    id: &str,
    source_kind: &str,
    source_ref: &str,
    source_sha: Option<&str>,
) -> StoreResult<()> {
    let updated = conn
        .execute(
            "UPDATE skill_plugins SET source_kind = ?2, source_ref = ?3, source_sha = ?4, updated_at = ?5 WHERE id = ?1",
            rusqlite::params![id, source_kind, source_ref, source_sha, now_millis()],
        )
        .map_err(other_err)?;
    if updated == 0 {
        return not_found(format!("插件不存在：{id}"));
    }
    Ok(())
}

/// 插件级启停。独立于指针更新（UPDATE 不触碰 enabled 列），因此升级重装天然保留
/// 用户已做的启用 / 禁用选择（spec 第 4 节）。
pub fn set_plugin_enabled(conn: &Connection, id: &str, enabled: bool) -> StoreResult<()> {
    let updated = conn
        .execute(
            "UPDATE skill_plugins SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, enabled as i64, now_millis()],
        )
        .map_err(other_err)?;
    if updated == 0 {
        return not_found(format!("插件不存在：{id}"));
    }
    Ok(())
}

/// 卸载插件行（彻底删除；磁盘全部版本目录的删除由编排层负责）。
pub fn remove_plugin(conn: &Connection, id: &str) -> StoreResult<()> {
    let deleted = conn.execute("DELETE FROM skill_plugins WHERE id = ?1", [id]).map_err(other_err)?;
    if deleted == 0 {
        return not_found(format!("插件不存在：{id}"));
    }
    Ok(())
}

fn to_plugin(row: &rusqlite::Row<'_>) -> rusqlite::Result<PluginRow> {
    let versions_text: String = row.get("versions")?;
    let versions = parse_versions(&versions_text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("versions 列不是合法 JSON 数组：{e}").into(),
        )
    })?;
    Ok(PluginRow {
        id: row.get("id")?,
        marketplace_id: row.get("marketplace_id")?,
        name: row.get("name")?,
        current_version: row.get("current_version")?,
        versions,
        install_path: row.get("install_path")?,
        source_kind: row.get("source_kind")?,
        source_ref: row.get("source_ref")?,
        source_sha: row.get("source_sha")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        installed_at: row.get("installed_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// versions 列 JSON 反序列化。损坏即报错不静默回退——版本指针数据损坏应显式暴露，
/// 由调用方给出可读错误（清缓存重装是唯一恢复路径）。
fn parse_versions(text: &str) -> Result<Vec<String>, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use craft_store::Db;

    use super::*;

    /// 建隔离临时库（测试自用，不进 crate 公共面）。
    fn tmp_db(tag: &str) -> (PathBuf, Db) {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cw-marketplace-{}-{}-{}",
            tag,
            std::process::id(),
            n + 1000 * std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_millis()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("test.db")).expect("打开测试库失败");
        (dir, db)
    }

    fn market<'a>(id: &'a str) -> NewMarketplace<'a> {
        NewMarketplace { id, source: id, repo_url: "https://github.com/example/repo" }
    }

    fn versions(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn plugin<'a>(id: &'a str, market_id: &'a str, vers: &'a [String]) -> NewPlugin<'a> {
        NewPlugin {
            id,
            marketplace_id: market_id,
            name: id.split('@').next().unwrap_or(id),
            current_version: vers.last().map(|s| s.as_str()).unwrap_or("1.0.0"),
            versions: vers,
            install_path: "/tmp/cache/skills-marketplace",
            source_kind: source_kind::ENTRY_RELATIVE,
            source_ref: "./plugins/demo",
            source_sha: Some("abc1234"),
        }
    }

    #[test]
    fn duplicate_market_add_rejected_with_readable_error() {
        let (dir, db) = tmp_db("dup-market");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("superpowers")).unwrap();
            let err = add_marketplace(&conn, market("superpowers")).unwrap_err();
            assert!(matches!(err, StoreError::Other(_)), "重复添加应报可读错误，实际 {err:?}");
            assert!(err.to_string().contains("已存在"));
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_failure_keeps_last_success_time_and_success_clears_failure() {
        let (dir, db) = tmp_db("refresh");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("skills")).unwrap();
            // 成功 → 失败：失败原因落列、成功时间保留（刷新节流依赖它）
            set_marketplace_refreshed(&conn, "skills", 1000).unwrap();
            set_marketplace_refresh_failure(&conn, "skills", "网络不可达").unwrap();
            let row = get_marketplace(&conn, "skills").unwrap();
            assert_eq!(row.last_refreshed_at, Some(1000));
            assert_eq!(row.last_refresh_failure.as_deref(), Some("网络不可达"));
            // 再成功：失败原因被清空
            set_marketplace_refreshed(&conn, "skills", 2000).unwrap();
            let row = get_marketplace(&conn, "skills").unwrap();
            assert_eq!(row.last_refreshed_at, Some(2000));
            assert_eq!(row.last_refresh_failure, None);
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_market_with_plugins_rejected_then_cascade_removes_plugins_and_reports_count() {
        let (dir, db) = tmp_db("cascade");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("anthropics-skills")).unwrap();
            let vers = versions(&["1.0.0"]);
            insert_plugin(&conn, plugin(&plugin_id("document-skills", "anthropics-skills"), "anthropics-skills", &vers)).unwrap();
            insert_plugin(&conn, plugin(&plugin_id("example-skills", "anthropics-skills"), "anthropics-skills", &vers)).unwrap();

            // 保护语义：仍有插件时直接删除被拒绝
            let err = remove_marketplace(&conn, "anthropics-skills").unwrap_err();
            assert!(err.to_string().contains("级联删除") || err.to_string().contains("先卸载"), "实际 {err:?}");
            assert!(get_marketplace(&conn, "anthropics-skills").is_ok(), "拒绝删除后市场应还在");
            assert_eq!(list_plugins_by_marketplace(&conn, "anthropics-skills").unwrap().len(), 2);

            // 级联语义：先删插件行再删市场行，返回级联删除的插件数
            let removed = remove_marketplace_cascade(&conn, "anthropics-skills").unwrap();
            assert_eq!(removed, 2);
            assert!(matches!(get_marketplace(&conn, "anthropics-skills"), Err(StoreError::NotFound(_))));
            assert!(list_plugins_by_marketplace(&conn, "anthropics-skills").unwrap().is_empty());
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_market_without_plugins_succeeds_and_missing_row_reports_not_found() {
        let (dir, db) = tmp_db("remove-market");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("ponytail")).unwrap();
            remove_marketplace(&conn, "ponytail").unwrap();
            assert!(matches!(get_marketplace(&conn, "ponytail"), Err(StoreError::NotFound(_))));
            // 不存在的市场：删除报 NotFound（与「有插件被拒」区分开）
            let err = remove_marketplace(&conn, "ponytail").unwrap_err();
            assert!(matches!(err, StoreError::NotFound(_)), "实际 {err:?}");
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_plugin_rejects_duplicate_and_missing_market_and_empty_versions() {
        let (dir, db) = tmp_db("insert-plugin");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("skills")).unwrap();
            let vers = versions(&["1.0.0"]);
            let pid = plugin_id("engineering", "skills");
            insert_plugin(&conn, plugin(&pid, "skills", &vers)).unwrap();

            // 重复安装拒绝
            let err = insert_plugin(&conn, plugin(&pid, "skills", &vers)).unwrap_err();
            assert!(err.to_string().contains("已安装"), "实际 {err:?}");
            // 市场不存在拒绝（外键 + 可读错误）
            let err = insert_plugin(&conn, plugin(&plugin_id("x", "no-such"), "no-such", &vers)).unwrap_err();
            assert!(err.to_string().contains("市场源不存在"), "实际 {err:?}");
            // 空 versions 拒绝
            let err = insert_plugin(&conn, plugin(&plugin_id("y", "skills"), "skills", &[])).unwrap_err();
            assert!(err.to_string().contains("versions"), "实际 {err:?}");

            let row = get_plugin(&conn, &pid).unwrap();
            assert_eq!(row.versions, vers);
            assert!(row.enabled, "新安装默认启用");
            assert_eq!(row.current_version, "1.0.0");
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_version_moves_pointer_prunes_oldest_and_is_idempotent() {
        let (dir, db) = tmp_db("append-version");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("skills")).unwrap();
            let pid = plugin_id("engineering", "skills");
            insert_plugin(&conn, plugin(&pid, "skills", &versions(&["1.0.0"]))).unwrap();

            // 升到 2.0.0：指针切换 + 数组追加
            let kept = append_plugin_version(&conn, &pid, "2.0.0", 3).unwrap();
            assert_eq!(kept, vec!["1.0.0".to_string(), "2.0.0".to_string()]);
            assert_eq!(get_plugin(&conn, &pid).unwrap().current_version, "2.0.0");

            // 升到 3.0.0、4.0.0（keep_last = 3）：最旧版本被裁剪
            append_plugin_version(&conn, &pid, "3.0.0", 3).unwrap();
            let kept = append_plugin_version(&conn, &pid, "4.0.0", 3).unwrap();
            assert_eq!(kept, vec!["2.0.0".to_string(), "3.0.0".to_string(), "4.0.0".to_string()]);
            let row = get_plugin(&conn, &pid).unwrap();
            assert_eq!(row.versions, vec!["2.0.0".to_string(), "3.0.0".to_string(), "4.0.0".to_string()]);
            assert_eq!(row.current_version, "4.0.0");

            // 幂等：重复追加已保留版本，旧位置移除后追加到尾部、不产生重复项（长度恰为上限时不裁剪）
            let kept = append_plugin_version(&conn, &pid, "3.0.0", 3).unwrap();
            assert_eq!(kept, vec!["2.0.0".to_string(), "4.0.0".to_string(), "3.0.0".to_string()]);

            // keep_last = 0 报错
            assert!(append_plugin_version(&conn, &pid, "5.0.0", 0).is_err());
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rollback_pointer_requires_kept_version_and_keeps_array_so_switching_back_works() {
        let (dir, db) = tmp_db("rollback");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("skills")).unwrap();
            let pid = plugin_id("engineering", "skills");
            insert_plugin(&conn, plugin(&pid, "skills", &versions(&["1.0.0"]))).unwrap();
            append_plugin_version(&conn, &pid, "2.0.0", 3).unwrap();

            // 回滚到上一版：指针切回、数组不变
            set_plugin_version_pointer(&conn, &pid, "1.0.0").unwrap();
            let row = get_plugin(&conn, &pid).unwrap();
            assert_eq!(row.current_version, "1.0.0");
            assert_eq!(row.versions, vec!["1.0.0".to_string(), "2.0.0".to_string()]);

            // 可再切回（回滚是双向指针切换）
            set_plugin_version_pointer(&conn, &pid, "2.0.0").unwrap();
            assert_eq!(get_plugin(&conn, &pid).unwrap().current_version, "2.0.0");

            // 目标版本不在保留列表（已被裁剪）：拒绝
            let err = set_plugin_version_pointer(&conn, &pid, "0.9.0").unwrap_err();
            assert!(matches!(err, StoreError::NotFound(_)), "实际 {err:?}");
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enable_toggle_survives_version_updates() {
        let (dir, db) = tmp_db("enable");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("skills")).unwrap();
            let pid = plugin_id("engineering", "skills");
            insert_plugin(&conn, plugin(&pid, "skills", &versions(&["1.0.0"]))).unwrap();

            // 停用后升级：启停选择保留（spec 第 4 节「升级重装不覆盖启停选择」）
            set_plugin_enabled(&conn, &pid, false).unwrap();
            assert!(!get_plugin(&conn, &pid).unwrap().enabled);
            append_plugin_version(&conn, &pid, "2.0.0", 3).unwrap();
            let row = get_plugin(&conn, &pid).unwrap();
            assert!(!row.enabled, "升级后启用状态应保留");
            assert_eq!(row.current_version, "2.0.0");

            // 重新启用
            set_plugin_enabled(&conn, &pid, true).unwrap();
            assert!(get_plugin(&conn, &pid).unwrap().enabled);

            // 不存在的插件：NotFound
            assert!(matches!(
                set_plugin_enabled(&conn, plugin_id("x", "skills").as_str(), true),
                Err(StoreError::NotFound(_))
            ));
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uninstall_plugin_then_market_removal_succeeds_and_list_order_is_install_order() {
        let (dir, db) = tmp_db("uninstall");
        {
            let conn = db.lock_conn();
            add_marketplace(&conn, market("skills")).unwrap();
            let a = plugin_id("alpha", "skills");
            let b = plugin_id("beta", "skills");
            insert_plugin(&conn, plugin(&a, "skills", &versions(&["1.0.0"]))).unwrap();
            insert_plugin(&conn, plugin(&b, "skills", &versions(&["1.0.0"]))).unwrap();

            // 列表按安装顺序（spec 第 5 节：扫描按安装顺序逐插件注入技能根）
            let listed = list_plugins(&conn).unwrap();
            assert_eq!(listed.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec![a.as_str(), b.as_str()]);

            // 卸载全部插件后，市场回到可直删状态；中途仍留插件时直删仍被拒
            remove_plugin(&conn, &a).unwrap();
            assert!(matches!(get_plugin(&conn, &a), Err(StoreError::NotFound(_))));
            assert!(remove_marketplace(&conn, "skills").is_err(), "仍有插件时直删应被拒");
            remove_plugin(&conn, &b).unwrap();
            remove_marketplace(&conn, "skills").unwrap();
            assert!(matches!(get_marketplace(&conn, "skills"), Err(StoreError::NotFound(_))));
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plugin_id_joins_name_and_marketplace() {
        assert_eq!(plugin_id("engineering", "mattpocock-skills"), "engineering@mattpocock-skills");
    }
}
