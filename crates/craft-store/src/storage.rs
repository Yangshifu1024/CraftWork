//! 存储占用查询：只读聚合，服务设置 → 存储分节。
//!
//! 这里只有 SQL 事实：哪个会话写了多少事件字节、哪张表占了多少页、每个会话有多少行。
//! **不判断什么能删**——「attachments 目录能不能清」「worktrees 下哪个目录是孤儿」
//! 是应用层语义（`src-tauri/src/storage.rs`），本模块不知道那些目录的存在。
//!
//! 口径（写进界面文案的同一份定义，改动时同步）：
//! - **事件字节** = `LENGTH(CAST(payload AS BLOB))` 求和。payload 是 JSON 文本，这是
//!   它在库里实际占用的近似值；不含索引页与行头开销，故**不是**该会话在库文件里的
//!   精确占比——存储页把它标成「数据库」，不标「独占字节」。
//! - **表体积** = `dbstat` 的 `pgsize` 求和（真实页占用，含索引）。`dbstat` 是
//!   libsqlite3-sys 的 bundled 构建开启的编译期虚拟表（`SQLITE_ENABLE_DBSTAT_VTAB`）；
//!   万一某个构建没有它，返回 `Ok(None)` 让调用方隐藏明细——**不做** `page_count`
//!   之类的估算兜底，给一个错数字比不给更糟。

use std::collections::HashMap;

use rusqlite::Connection;

use crate::db::StoreResult;

fn other_err(e: impl std::fmt::Display) -> crate::db::StoreError {
    crate::db::StoreError::Other(e.to_string())
}

/// 每个会话的事件载荷字节（只含 events 表；无事件的会话不出现在结果里）。
pub fn thread_event_bytes(conn: &Connection) -> StoreResult<HashMap<String, u64>> {
    let mut stmt = conn
        .prepare("SELECT thread_id, SUM(LENGTH(CAST(payload AS BLOB))) FROM events GROUP BY thread_id")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([], |row| {
            let thread_id: String = row.get(0)?;
            // SUM 在空集上为 NULL，但 GROUP BY 的分组至少有一行，仍按 0 兜底
            let bytes: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
            Ok((thread_id, bytes.max(0) as u64))
        })
        .map_err(other_err)?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 单会话的行数统计（会话行副标题「N 条事件 · M 次运行 · K 条审批」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadRowCounts {
    pub events: u64,
    pub runs: u64,
    pub approvals: u64,
}

/// 按会话汇总各表的行数。三张表各一次聚合查询后合并，避免三次全表扫描写三份代码。
pub fn thread_row_counts(conn: &Connection) -> StoreResult<HashMap<String, ThreadRowCounts>> {
    let mut counts: HashMap<String, ThreadRowCounts> = HashMap::new();
    for (field, table) in ["events", "runs", "approvals"].iter().enumerate() {
        let sql = format!("SELECT thread_id, COUNT(*) FROM {table} GROUP BY thread_id");
        let mut stmt = conn.prepare(&sql).map_err(other_err)?;
        let rows = stmt
            .query_map([], |row| {
                let thread_id: String = row.get(0)?;
                let n: i64 = row.get(1)?;
                Ok((thread_id, n.max(0) as u64))
            })
            .map_err(other_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(other_err)?;
        for (thread_id, n) in rows {
            let entry = counts.entry(thread_id).or_default();
            match field {
                0 => entry.events = n,
                1 => entry.runs = n,
                _ => entry.approvals = n,
            }
        }
    }
    Ok(counts)
}

/// 数据库表级体积（字节，降序）。`dbstat` 不可用时返回 `Ok(None)`。
///
/// 一次调用会遍历整个库的页表，大库上是毫秒到秒级开销——**只在用户展开明细时调用**，
/// 不要放进首屏路径。
pub fn db_table_bytes(conn: &Connection) -> StoreResult<Option<Vec<(String, u64)>>> {
    let mut stmt = match conn.prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY 2 DESC") {
        Ok(stmt) => stmt,
        Err(_) => return Ok(None),
    };
    let rows = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let bytes: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
            Ok((name, bytes.max(0) as u64))
        })
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(Some(rows))
}

/// 库里全部会话 id（含子代理子会话、含归档）。
///
/// 孤儿判定的白名单来源：磁盘上按会话 id 命名的文件，其 id 不在此集合里即为孤儿。
/// 列举**全部**会话而不是「可见会话」，因为子会话语的日志/后台输出同样是合法文件，
/// 漏掉它们会把活着的文件误判成孤儿。
pub fn all_thread_ids(conn: &Connection) -> StoreResult<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM threads").map_err(other_err)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(other_err)?
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 库里登记的全部计划文件路径（`plans.file_path`，跳过空串）。
pub fn all_plan_file_paths(conn: &Connection) -> StoreResult<Vec<String>> {
    let mut stmt = conn.prepare("SELECT file_path FROM plans WHERE file_path <> ''").map_err(other_err)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 某个会话登记的全部计划文件路径（删会话时据此清理正文文件）。
///
/// 必须在 `plans` 行删除**之前**调用——行一旦删掉就无从知道哪些文件属于该会话。
pub fn thread_plan_paths(conn: &Connection, thread_id: &str) -> StoreResult<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT file_path FROM plans WHERE thread_id = ?1 AND file_path <> ''")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([thread_id], |row| row.get::<_, String>(0))
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows)
}

/// 计划文件的归属映射：文件 basename → `thread_id`。
///
/// 用 basename 而不是完整路径作键：`plans.file_path` 记的是绝对路径，而计划目录随
/// `CRAFTWORK_HOME` 变化——同一份库搬到别的数据目录下，完整路径就全对不上了。
pub fn plan_file_owners(conn: &Connection) -> StoreResult<HashMap<String, String>> {
    let mut stmt = conn
        .prepare("SELECT thread_id, file_path FROM plans WHERE file_path <> ''")
        .map_err(other_err)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(other_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other_err)?;
    Ok(rows
        .into_iter()
        .filter_map(|(thread_id, path)| {
            std::path::Path::new(&path)
                .file_name()
                .map(|name| (name.to_string_lossy().to_string(), thread_id))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn temp_db(name: &str) -> (std::path::PathBuf, Db) {
        let dir = std::env::temp_dir().join(format!("craftwork-storage-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("craftwork.db")).unwrap();
        (dir, db)
    }

    fn insert_event(conn: &Connection, thread_id: &str, seq: i64, payload: &str) {
        conn.execute(
            "INSERT INTO events (id, thread_id, message_id, seq, kind, payload, created_at) VALUES (?1, ?2, 'm', ?3, 'text', ?4, 0)",
            rusqlite::params![format!("{thread_id}-{seq}"), thread_id, seq, payload],
        )
        .unwrap();
    }

    #[test]
    fn thread_event_bytes_groups_by_thread() {
        let (dir, db) = temp_db("bytes");
        db.with(|conn| {
            insert_event(conn, "th_a", 1, "12345");
            insert_event(conn, "th_a", 2, "123");
            insert_event(conn, "th_b", 1, "1");
            Ok(())
        })
        .unwrap();
        let map = db.with(thread_event_bytes).unwrap();
        assert_eq!(map.get("th_a"), Some(&8), "同会话多事件应累加");
        assert_eq!(map.get("th_b"), Some(&1));
        assert_eq!(map.len(), 2, "无事件的会话不出现");
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn thread_row_counts_reports_all_three_tables() {
        let (dir, db) = temp_db("rows");
        db.with(|conn| {
            insert_event(conn, "th_a", 1, "1");
            insert_event(conn, "th_a", 2, "1");
            conn.execute(
                "INSERT INTO runs (id, thread_id, message_id, status, started_at) VALUES ('r1', 'th_a', 'm', 'done', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO approvals (id, thread_id, run_id, tool_call_id, tool, risk, requested_at) VALUES ('ap1', 'th_a', 'r1', 'tc', 'fs_write', 'R1', 0)",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        let counts = db.with(thread_row_counts).unwrap();
        let row = counts.get("th_a").expect("th_a 应有行数统计");
        assert_eq!(row.events, 2);
        assert_eq!(row.runs, 1);
        assert_eq!(row.approvals, 1);
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// dbstat 在 bundled 构建里可用（libsqlite3-sys 编译时开 SQLITE_ENABLE_DBSTAT_VTAB）。
    /// 这条同时钉住「明细依赖编译期开关」这件事：真不可用时返回 None，调用方隐藏明细。
    #[test]
    fn db_table_bytes_reports_events_table() {
        let (dir, db) = temp_db("tables");
        db.with(|conn| {
            insert_event(conn, "th_a", 1, &"x".repeat(4096));
            Ok(())
        })
        .unwrap();
        let tables = db.with(db_table_bytes).unwrap();
        match tables {
            Some(rows) => {
                let events = rows.iter().find(|(name, _)| name == "events").expect("应含 events 表");
                assert!(events.1 > 0, "events 表应占用正数页字节");
            }
            None => panic!("bundled 构建应提供 dbstat；若某平台缺失需在此处显式放宽"),
        }
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }
}
