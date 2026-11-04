//! 数据库打开、PRAGMA 与迁移链（src/main/store/db.ts 的直译）。
//! 历史迁移 SQL 与 Node 版逐字一致：Electron 版写入的库在 Rust 版直接可读可迁移。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

pub use craft_core::bus::now_millis;

/// 当前 schema 版本（与 Node 版 SCHEMA_VERSION 同步演进，发版两侧必须一致）。
pub const SCHEMA_VERSION: i64 = 7;

#[derive(Debug)]
pub enum StoreError {
    NotFound(String),
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound(msg) => write!(f, "{msg}"),
            StoreError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound("记录不存在".into()),
            other => StoreError::Other(other.to_string()),
        }
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

pub fn not_found<T>(msg: impl Into<String>) -> StoreResult<T> {
    Err(StoreError::NotFound(msg.into()))
}

/// rusqlite 等底层错误 → StoreError 的统一入口（map_err(other_err)）。
pub fn other_err(err: impl std::fmt::Display) -> StoreError {
    StoreError::Other(err.to_string())
}

/// SQLite 存储句柄：单连接 + Mutex 串行（Node 版为单线程 + writeMu，语义等价——
/// 「seq 分配与插入同事务」在串行化下天然成立）。WAL 下如需并行读可另开只读连接。
pub struct Db {
    conn: Mutex<Connection>,
    file_path: PathBuf,
}

impl Db {
    pub fn open(path: &Path) -> StoreResult<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Other(format!("创建数据目录失败：{e}")))?;
            set_dir_mode_0700(dir);
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Other(format!("打开数据库失败：{e}")))?;
        let _ = conn.busy_timeout(std::time::Duration::from_millis(5000));
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| StoreError::Other(e.to_string()))?;
        conn.pragma_update(None, "foreign_keys", "1").map_err(|e| StoreError::Other(e.to_string()))?;
        migrate(&conn)?;
        Ok(Db {
            conn: Mutex::new(conn),
            file_path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.file_path
    }

    /// 持锁访问连接（单语句读写在锁内完成）。
    pub fn with<R>(&self, f: impl FnOnce(&Connection) -> StoreResult<R>) -> StoreResult<R> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&conn)
    }

    /// 持锁获取连接守卫（多语句顺序执行；Guard Deref 到 Connection）。
    pub fn lock_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 事务：fn 内出错自动回滚。事务内不得再获取本锁（防自死锁）。
    pub fn transaction<R>(&self, f: impl FnOnce(&Connection) -> StoreResult<R>) -> StoreResult<R> {
        let conn = self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let tx = conn.unchecked_transaction().map_err(|e| StoreError::Other(e.to_string()))?;
        match f(&tx) {
            Ok(value) => {
                tx.commit().map_err(|e| StoreError::Other(e.to_string()))?;
                Ok(value)
            }
            Err(err) => {
                // Transaction drop 时自动回滚
                Err(err)
            }
        }
    }

    pub fn close(self) -> StoreResult<()> {
        let conn = self.conn.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.close().map_err(|(_, e)| StoreError::Other(e.to_string()))
    }
}

#[cfg(unix)]
fn set_dir_mode_0700(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_dir: &Path) {}

// ---------- 迁移 ----------
// 历史迁移不修改（与 Node 版逐字一致）；新表结构一律追加新版本。

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS threads (
  id TEXT PRIMARY KEY, mode TEXT NOT NULL, title TEXT NOT NULL,
  model_id TEXT NOT NULL DEFAULT '', workspace_id TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'idle', archived INTEGER NOT NULL DEFAULT 0,
  tokens INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS threads_mode_updated ON threads(mode, archived, updated_at DESC);

CREATE TABLE IF NOT EXISTS workspaces (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
  trust TEXT NOT NULL DEFAULT 'restricted',
  created_at INTEGER NOT NULL, last_opened_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, message_id TEXT NOT NULL,
  run_id TEXT, seq INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL,
  created_at INTEGER NOT NULL, UNIQUE(thread_id, seq)
);
CREATE INDEX IF NOT EXISTS events_thread_seq ON events(thread_id, seq);

CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, message_id TEXT NOT NULL,
  status TEXT NOT NULL, model_id TEXT NOT NULL DEFAULT '',
  started_at INTEGER NOT NULL, finished_at INTEGER,
  error_code TEXT, error_message TEXT
);
CREATE INDEX IF NOT EXISTS runs_thread ON runs(thread_id, started_at);

CREATE TABLE IF NOT EXISTS approvals (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, run_id TEXT NOT NULL,
  tool_call_id TEXT NOT NULL, tool TEXT NOT NULL, risk TEXT NOT NULL,
  decision TEXT, scope TEXT, requested_at INTEGER NOT NULL, decided_at INTEGER
);
CREATE INDEX IF NOT EXISTS approvals_pending ON approvals(thread_id, decision);

CREATE TABLE IF NOT EXISTS audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
  thread_id TEXT, run_id TEXT, tool TEXT NOT NULL, risk TEXT NOT NULL,
  decision TEXT NOT NULL, scope TEXT, detail TEXT
);
CREATE INDEX IF NOT EXISTS audit_ts ON audit_log(ts DESC);
CREATE INDEX IF NOT EXISTS audit_thread ON audit_log(thread_id, ts DESC);

CREATE TABLE IF NOT EXISTS usage (
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, thread_id TEXT,
  model_id TEXT NOT NULL, input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0, cache_tokens INTEGER NOT NULL DEFAULT 0,
  reasoning_tokens INTEGER NOT NULL DEFAULT 0, cost_micros INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS usage_ts ON usage(ts DESC);
CREATE INDEX IF NOT EXISTS usage_model ON usage(model_id, ts DESC);

CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS backups (
  id TEXT PRIMARY KEY, path TEXT NOT NULL, backup_path TEXT NOT NULL,
  hash TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS backups_created ON backups(created_at DESC);
"#;

const SCHEMA_V2: &str = r#"
ALTER TABLE threads ADD COLUMN title_auto INTEGER NOT NULL DEFAULT 0;
UPDATE threads SET title_auto = 1 WHERE title = '新对话';
"#;

const SCHEMA_V3: &str = r#"
CREATE INDEX IF NOT EXISTS usage_thread ON usage(thread_id, ts DESC);
"#;

const SCHEMA_V4: &str = r#"
ALTER TABLE usage ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;
"#;

const SCHEMA_V5: &str = r#"
ALTER TABLE workspaces ADD COLUMN purpose TEXT NOT NULL DEFAULT 'project';
"#;

const SCHEMA_V6: &str = r#"
CREATE TABLE IF NOT EXISTS workspace_folders (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  path TEXT NOT NULL UNIQUE, position INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS workspace_folders_ws ON workspace_folders(workspace_id, position);
INSERT INTO workspace_folders (id, workspace_id, path, position, created_at)
  SELECT 'wfo_' || id, id, path, 0, created_at FROM workspaces;
"#;

/// V7：一次性清理遗留空会话——无任何事件、自动命名、标题「新对话」的线程；
/// 有事件或被手动改名（title_auto = 0）的会话一律保留。
pub const SCHEMA_V7: &str = r#"
DELETE FROM threads WHERE title = '新对话' AND title_auto = 1 AND NOT EXISTS (SELECT 1 FROM events WHERE events.thread_id = threads.id);
"#;

const MIGRATIONS: [&str; 8] = ["", SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7];

fn migrate(conn: &Connection) -> StoreResult<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0);
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    for v in (version + 1)..=SCHEMA_VERSION {
        let sql = MIGRATIONS[v as usize];
        if sql.is_empty() {
            continue;
        }
        let apply = || -> StoreResult<()> {
            let tx = conn.unchecked_transaction().map_err(|e| StoreError::Other(e.to_string()))?;
            tx.execute_batch(sql).map_err(|e| StoreError::Other(e.to_string()))?;
            tx.pragma_update(None, "user_version", v).map_err(|e| StoreError::Other(e.to_string()))?;
            tx.commit().map_err(|e| StoreError::Other(e.to_string()))?;
            Ok(())
        };
        if let Err(err) = apply() {
            return Err(StoreError::Other(format!("数据库迁移到版本 {v} 失败：{err}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 全新库_迁移到最新版本() {
        let dir = std::env::temp_dir().join(format!("craftwork-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("craftwork.db")).unwrap();
        let version: i64 = db
            .with(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0)))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        // 关键表存在
        for table in ["threads", "workspaces", "workspace_folders", "events", "runs", "approvals", "audit_log", "usage", "settings", "backups"] {
            let count: i64 = db
                .with(|conn| {
                    Ok(conn
                        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?", [table], |row| row.get(0))
                        .unwrap_or(0))
                })
                .unwrap();
            assert_eq!(count, 1, "表 {table} 应存在");
        }
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 事务_出错自动回滚() {
        let dir = std::env::temp_dir().join(format!("craftwork-dbtx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("craftwork.db")).unwrap();
        let err = db
            .transaction::<()>(|conn| {
                conn.execute("INSERT INTO settings (key, value) VALUES ('k', 'v')", [])?;
                Err(StoreError::Other("故意失败".into()))
            })
            .unwrap_err();
        assert!(matches!(err, StoreError::Other(_)));
        let count: i64 = db
            .with(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0)).unwrap_or(0)))
            .unwrap();
        assert_eq!(count, 0, "事务内写入应已回滚");
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }
}
