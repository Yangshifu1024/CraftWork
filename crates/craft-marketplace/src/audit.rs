//! 技能商店审计入口：市场源添加 / 移除 / 刷新与插件安装 / 升级 / 回滚 / 卸载 / 启停
//! 全部写 `audit_log`（spec 第 6 节：用户直接操作也落审计）。
//!
//! 走既有 `craft_store::records::add_audit` 通道模式，与 services.rs / craft-agent
//! 的审计行同构：商店动作不发生在会话 run 内，thread_id / run_id 恒空；tool 固定
//! `skill_store`，decision 承载动作名（`audit_action` 常量），risk 为 R0（用户直接
//! 操作，不经审批链），detail 承载可读中文摘要。失败动作（如刷新失败）也落审计，
//! 失败原因写进 detail。
//!
//! 取锁纪律同 store 模块：只提供接收 `&Connection` 的纯函数，由调用方取锁传入。

use rusqlite::Connection;

use craft_store::db::StoreResult;
use craft_store::records::{add_audit, AuditRecord};

/// 审计动作名（写 audit_log.decision 列；工具列固定 `skill_store`）。
pub mod audit_action {
    /// 添加市场源。
    pub const MARKET_ADD: &str = "market_add";
    /// 移除市场源（含级联删除；detail 注明是否级联及随删插件数）。
    pub const MARKET_REMOVE: &str = "market_remove";
    /// 市场源刷新（成功与失败都落审计，失败原因在 detail）。
    pub const MARKET_REFRESH: &str = "market_refresh";
    /// 安装插件。
    pub const PLUGIN_INSTALL: &str = "plugin_install";
    /// 升级插件（detail 含旧版 → 新版）。
    pub const PLUGIN_UPDATE: &str = "plugin_update";
    /// 回滚插件版本（detail 含回滚目标版本）。
    pub const PLUGIN_ROLLBACK: &str = "plugin_rollback";
    /// 卸载插件（含全部历史版本，detail 注明删除的版本目录）。
    pub const PLUGIN_UNINSTALL: &str = "plugin_uninstall";
    /// 插件启停（detail 注明启用 / 停用）。
    pub const PLUGIN_SET_ENABLED: &str = "plugin_set_enabled";
}

/// 写一条商店审计。`action` 取 `audit_action` 常量，`detail` 为可读中文摘要。
pub fn record_audit(conn: &Connection, action: &str, detail: &str) -> StoreResult<()> {
    add_audit(
        conn,
        AuditRecord {
            ts: None,
            thread_id: None,
            run_id: None,
            tool: "skill_store",
            risk: "R0",
            decision: action,
            scope: None,
            detail: Some(detail),
        },
    )
}

#[cfg(test)]
mod tests {
    use craft_store::Db;
    use craft_store::records::list_audit;

    use super::*;

    #[test]
    fn record_audit_covers_all_store_actions_with_readable_detail() {
        let static_seq = std::sync::atomic::AtomicU32::new(0);
        let n = static_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cw-marketplace-audit-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("test.db")).expect("打开测试库失败");

        {
            let conn = db.lock_conn();
            // 八类动作各写一条
            record_audit(&conn, audit_action::MARKET_ADD, "添加市场源：obra/superpowers").unwrap();
            record_audit(&conn, audit_action::MARKET_REMOVE, "移除市场源：ponytail（级联卸载 1 个插件）").unwrap();
            record_audit(&conn, audit_action::MARKET_REFRESH, "刷新市场失败：skills，网络不可达").unwrap();
            record_audit(&conn, audit_action::PLUGIN_INSTALL, "安装插件：engineering@skills（版本 1.0.0）").unwrap();
            record_audit(&conn, audit_action::PLUGIN_UPDATE, "升级插件：engineering@skills（1.0.0 → 2.0.0）").unwrap();
            record_audit(&conn, audit_action::PLUGIN_ROLLBACK, "回滚插件：engineering@skills → 1.0.0").unwrap();
            record_audit(&conn, audit_action::PLUGIN_UNINSTALL, "卸载插件：engineering@skills（含 2 个版本目录）").unwrap();
            record_audit(&conn, audit_action::PLUGIN_SET_ENABLED, "停用插件：engineering@skills").unwrap();

            // 走既有 list_audit 通道读回，断言列语义
            let entries = list_audit(&conn, 100).unwrap();
            assert_eq!(entries.len(), 8);
            for entry in &entries {
                assert_eq!(entry.tool, "skill_store", "商店审计工具列应固定 skill_store");
                assert_eq!(entry.risk, "R0");
                assert_eq!(entry.thread_id, "", "商店动作无会话上下文");
                assert_eq!(entry.run_id, "");
                assert!(!entry.detail.is_empty());
            }
            let decisions: Vec<&str> = entries.iter().map(|e| e.decision.as_str()).collect();
            for action in [
                audit_action::MARKET_ADD,
                audit_action::MARKET_REMOVE,
                audit_action::MARKET_REFRESH,
                audit_action::PLUGIN_INSTALL,
                audit_action::PLUGIN_UPDATE,
                audit_action::PLUGIN_ROLLBACK,
                audit_action::PLUGIN_UNINSTALL,
                audit_action::PLUGIN_SET_ENABLED,
            ] {
                assert!(decisions.contains(&action), "缺少动作 {action}");
            }
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
