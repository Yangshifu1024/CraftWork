//! 数据库打开、PRAGMA 与迁移链。
//! 历史迁移 SQL 与 Node 版逐字一致：Electron 版写入的库在 Rust 版直接可读可迁移。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

pub use craft_core::bus::now_millis;

/// 当前 schema 版本。历史版本与已移除的 Electron 版（Node）双向兼容——老库原位可读；
/// Node 版删除后由本侧单独演进，新增迁移只做加列/加表等向后兼容变更。
pub const SCHEMA_VERSION: i64 = 18;

/// 存储层错误：NotFound 承载业务「不存在」（渲染层按静默/提示处理），Other 为可读中文错误串。
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

/// 构造 NotFound 错误的便捷入口。
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
    /// 打开库：建目录（0700）→ WAL + 外键 PRAGMA → busy_timeout 5s → 跑迁移链。
    pub fn open(path: &Path) -> StoreResult<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Other(format!("创建数据目录失败：{e}")))?;
            set_dir_mode_0700(dir);
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Other(format!("打开数据库失败：{e}")))?;
        let _ = conn.busy_timeout(std::time::Duration::from_millis(5000));
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| StoreError::Other(e.to_string()))?;
        // WAL 官方推荐搭配：提交只写操作系统页缓存、系统择机落盘，不再逐事务强制 fsync
        //（桌面断电最多丢最近几个已提交事务，库文件本身不会损坏）；多会话并行写入不再逐个等盘。
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| StoreError::Other(e.to_string()))?;
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

    /// 数据库在磁盘上的占用（字节）：主库 + WAL 日志 + 共享内存索引三个文件之和。
    ///
    /// 为什么不是单个主库文件大小：WAL 模式下最近的提交先落在 `-wal` 里、在下一次 checkpoint
    /// 之前不回写主库，只报主库会明显偏小（刚写入大量数据时可能只有几百 KB）。文件名后缀
    /// 属于存储层细节，调用方只需要一个总量，故在此汇总。
    /// 读取失败（文件尚未创建等）按 0 计——体积展示不该因单文件缺失而整体失败。
    pub fn disk_usage_bytes(&self) -> u64 {
        let base = self.file_path.to_string_lossy().to_string();
        ["", "-wal", "-shm"]
            .iter()
            .filter_map(|suffix| std::fs::metadata(format!("{base}{suffix}")).ok())
            .map(|meta| meta.len())
            .sum()
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

    /// 关闭并释放连接（消费 self，防止关闭后继续使用）。
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

/// V8：runs 记录触发它的用户消息事件 seq。快照重建给无部件 run 的占位消息定序时，
/// 以 seq（单写者事务内严格递增）为权威先后信号，替代不可靠的毫秒时间戳比较
/// （消息落库与 run 创建常落在同一毫秒，时间戳区分不出先后）。旧行留 NULL，
/// 快照侧对 NULL 回退时间戳兜底，行为与 V7 一致。
pub const SCHEMA_V8: &str = r#"
ALTER TABLE runs ADD COLUMN trigger_seq INTEGER;
"#;

/// V9：会话级思考档位（与 model_id 同生命周期：随会话记忆，threadSetReasoning 更新）。
/// 空串 = 未选择（请求不发任何 reasoning 参数，行为与 V8 一致）。
pub const SCHEMA_V9: &str = r#"
ALTER TABLE threads ADD COLUMN reasoning_level TEXT NOT NULL DEFAULT '';
"#;

/// V10：会话绑定的 git worktree（composer 勾选 worktree 发送时创建并绑定）。
/// 空串 = 未绑定；绑定后该会话所有 run 的工作目录切换到工作树，删除会话时一并清理。
pub const SCHEMA_V10: &str = r#"
ALTER TABLE threads ADD COLUMN worktree_path TEXT NOT NULL DEFAULT '';
ALTER TABLE threads ADD COLUMN worktree_branch TEXT NOT NULL DEFAULT '';
"#;

/// V11：计划数据（计划模式产物）。发送计划模式消息时插入（approved=0、file_path 空），
/// 计划 run 正常完成后把最终正文写入 ~/.craftwork/plans/ 并回填 file_path；
/// 前端「批准并执行」提示条以 file_path 非空且未批准为准（中断/失败不出现）。
pub const SCHEMA_V11: &str = r#"
CREATE TABLE plans (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  run_id TEXT,
  file_path TEXT NOT NULL DEFAULT '',
  approved INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_plans_thread ON plans(thread_id, created_at);
"#;

/// V12：计划的「忽略」标记（提示条「忽略」按钮）：用户明确不想现在处理该计划，
/// 提示条不再出现（与 approved 互不影响；批准依旧走 approved）。
pub const SCHEMA_V12: &str = r#"
ALTER TABLE plans ADD COLUMN ignored INTEGER NOT NULL DEFAULT 0;
"#;

/// V13：用量行新增速率/占用口径列——
/// - prompt_tokens：本次请求完整提示词 token（含缓存读+写，按提供商归一化）；0 = 旧行，读取时回退 input+output；
/// - ttft_ms：首 token 延迟（发起调用 → 首个内容片段）；
/// - gen_window_ms：生成窗口（收流结束 − 首 token），速率分母的事实源；
///   -1 = 无效（取消/窗口过短，unrated 不参与速度统计），0 = 旧行回退全程 duration。
pub const SCHEMA_V13: &str = r#"
ALTER TABLE usage ADD COLUMN prompt_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE usage ADD COLUMN ttft_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE usage ADD COLUMN gen_window_ms INTEGER NOT NULL DEFAULT 0;
"#;

/// V14：技能商店（决策 D12）两张表——
/// - marketplaces：已添加市场源。id 取清单 name（主键）；source 存用户输入原文（URL 或 owner/repo）；
///   repo_url 为展开后的 git URL；last_refreshed_at / last_refresh_failure 记最近一次刷新结果
///   （成功清空失败原因，失败不动成功时间，spec 第 11 节）。
/// - skill_plugins：已安装插件。id 形如 `name@marketplace`；current_version 为生效版本指针，
///   versions 为磁盘保留版本目录名的 JSON 数组（当前 + 历史并存，保留上限由编排层控制）；
///   enabled 为插件级启停（spec 第 5 节两层开关的插件层）。
/// - 时间戳沿用全库惯例 INTEGER 毫秒；外键仅做引用完整性（不带 ON DELETE CASCADE）：
///   「删市场先无插件或显式级联」的语义由 craft-marketplace 存储层显式实现，不做隐式级联。
pub const SCHEMA_V14: &str = r#"
CREATE TABLE marketplaces (
  id TEXT PRIMARY KEY, source TEXT NOT NULL, repo_url TEXT NOT NULL,
  last_refreshed_at INTEGER, last_refresh_failure TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE skill_plugins (
  id TEXT PRIMARY KEY, marketplace_id TEXT NOT NULL REFERENCES marketplaces(id),
  name TEXT NOT NULL, current_version TEXT NOT NULL,
  versions TEXT NOT NULL, install_path TEXT NOT NULL,
  source_kind TEXT NOT NULL, source_ref TEXT NOT NULL, source_sha TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  installed_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX idx_skill_plugins_marketplace ON skill_plugins(marketplace_id);
"#;

/// V15：子代理体系（决策 D15）——子代理是一个子会话，
/// 复用事件表 / 快照 / 压缩 / 用量 / 日志全套机制，只在 threads 上补归属与档案列：
/// - parent_thread_id：父会话 id（空串 = 主会话；子会话不进侧栏与搜索）；
/// - agent_name：可寻址短名（模型经 agent_wait / agent_close 引用；父会话内唯一）；
/// - agent_type：代理类型（general / explore，见 craft-agent::subagent::AgentType）；
/// - agent_depth：嵌套深度（一期恒为 1，递归被禁止，列留给未来放宽）；
/// - spawn_call_id：发起它的工具调用 id（工具结果与子会话的双向关联键）。
///
/// 索引只服务「列某会话的子代理」这一条查询（子会话总量远小于主会话）。
pub const SCHEMA_V15: &str = r#"
ALTER TABLE threads ADD COLUMN parent_thread_id TEXT NOT NULL DEFAULT '';
ALTER TABLE threads ADD COLUMN agent_name TEXT NOT NULL DEFAULT '';
ALTER TABLE threads ADD COLUMN agent_type TEXT NOT NULL DEFAULT '';
ALTER TABLE threads ADD COLUMN agent_depth INTEGER NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN spawn_call_id TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS threads_parent ON threads(parent_thread_id);
"#;

/// V16：子代理档案快照（决策 D15 扩展）——自定义子代理的生效档案（模型 / 迭代上限 /
/// 工具面 / 角色提示词）在派发时快照进子会话。理由：自定义定义可被用户随时改或删，
/// 而历史子会话与卡片必须仍能解释「当时按什么跑的」，与「事件表是事实源、快照要可重建」
/// 一致。空串 = 内置类型或老数据（按 agent_type 现查内置档案即可）。
pub const SCHEMA_V16: &str = r#"
ALTER TABLE threads ADD COLUMN agent_profile TEXT NOT NULL DEFAULT '';
"#;

/// V17：未读标记——会话「已完成但用户尚未查看」需要两个时刻才能判定：
/// - last_done_at：最近一次 run 正常完成（`run.completed`）的时刻；取消/失败不写（那两类
///   侧栏已有独立形态，不算「等你查看的成果」）；
/// - last_read_at：用户最后一次打开该会话的时刻（打开即视为读过）。
///
/// 未读 = last_done_at > last_read_at。用两个绝对时间戳而非布尔位，是因为布尔位在
/// 「打开后又有新完成」时必须靠回调清位，多入口（切换会话/后台完成/重启）容易漏清；
/// 比较时间戳天然幂等，重启后按库内值直接重算，无需补偿逻辑。
pub const SCHEMA_V17: &str = r#"
ALTER TABLE threads ADD COLUMN last_done_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN last_read_at INTEGER NOT NULL DEFAULT 0;
"#;

/// V18：移除审计与写前备份——两张表连同历史记录一并丢弃（不可逆的有意变更）。
///
/// V1 的建表语句原样保留（已发布迁移不可改）：新库先照 V1 建表、再由本版本删除，
/// 与老库升级走同一条链，落地结果一致。
///
/// 为什么是 DROP 而不是「保留表、停止写入」：留一张只增不删且无任何入口的死表，
/// 磁盘只涨不落，下一次看到它的人还得重新判断它是否还有用。
pub const SCHEMA_V18: &str = r#"
DROP TABLE IF EXISTS audit_log;
DROP TABLE IF EXISTS backups;
"#;

/// 迁移链：下标即目标版本号，[0] 为占位空串（使 MIGRATIONS[v] 对应版本 v）。
const MIGRATIONS: [&str; 19] = ["", SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7, SCHEMA_V8, SCHEMA_V9, SCHEMA_V10, SCHEMA_V11, SCHEMA_V12, SCHEMA_V13, SCHEMA_V14, SCHEMA_V15, SCHEMA_V16, SCHEMA_V17, SCHEMA_V18];

/// 逐版本应用迁移：每个版本在独立事务内执行 SQL 并推进 user_version（原子生效或回滚）。
/// 已是最新版本时为 no-op；注意 user_version 大于 SCHEMA_VERSION 的库（更新版本产物）会静默放行。
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
    fn disk_usage_includes_wal_file() {
        let dir = std::env::temp_dir().join(format!("craftwork-db-wal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db_path = dir.join("craftwork.db");
        let db = Db::open(&db_path).unwrap();
        // 写一批数据：WAL 模式下提交先落在 -wal，容器未被 checkpoint 回写
        db.with(|conn| {
            for i in 0..200 {
                conn.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                    rusqlite::params![format!("usage-test-{i}"), "x".repeat(512)],
                )
                .map_err(StoreError::from)?;
            }
            Ok(())
        })
        .unwrap();
        let wal_len = std::fs::metadata(format!("{}-wal", db_path.to_string_lossy()))
            .map(|meta| meta.len())
            .unwrap_or(0);
        let main_len = std::fs::metadata(&db_path).map(|meta| meta.len()).unwrap_or(0);
        assert!(wal_len > 0, "写入后应存在 WAL 文件");
        // 只报主库会漏掉 WAL 里的这段时间线数据；总量必须覆盖两者
        assert!(
            db.disk_usage_bytes() >= main_len + wal_len,
            "磁盘占用应包含 WAL（总 {} < 主库 {main_len} + WAL {wal_len}）",
            db.disk_usage_bytes()
        );
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fresh_db_migrates_to_latest_version() {
        let dir = std::env::temp_dir().join(format!("craftwork-db-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("craftwork.db")).unwrap();
        let version: i64 = db
            .with(|conn| Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap_or(0)))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        // 关键表存在
        for table in ["threads", "workspaces", "workspace_folders", "events", "runs", "approvals", "usage", "settings"] {
            let count: i64 = db
                .with(|conn| {
                    Ok(conn
                        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?", [table], |row| row.get(0))
                        .unwrap_or(0))
                })
                .unwrap();
            assert_eq!(count, 1, "表 {table} 应存在");
        }
        // V18 之后审计与备份表不存在（迁移链末端删表，新库同样走到这里）
        for table in ["audit_log", "backups"] {
            let count: i64 = db
                .with(|conn| {
                    Ok(conn
                        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?", [table], |row| row.get(0))
                        .unwrap_or(0))
                })
                .unwrap();
            assert_eq!(count, 0, "表 {table} 应已被 V18 删除");
        }
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn transaction_auto_rollback_on_error() {
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
