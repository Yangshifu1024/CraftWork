// 本文件负责 SQLite 存储的打开、关闭与迁移：建立数据目录、配置连接参数、
// 按版本执行建表语句，并提供数据库对象的公共入口。
//
// 具体的数据读写分散在同包的其他文件里（会话、事件、审批、审计、用量等）。

package store

import (
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"

	_ "modernc.org/sqlite"
)

// ErrNotFound 表示查询的记录不存在。
var ErrNotFound = errors.New("记录不存在")

// DB 是本地 SQLite 存储。所有写入串行化，避免多 goroutine 争抢 seq。
type DB struct {
	// sqlDB 是底层连接池；读操作可以并发，写操作由 writeMu 串行化。
	sqlDB *sql.DB
	// path 是数据库文件路径，供设置页与日志展示。
	path string
	// writeMu 保护所有写入路径，保证事务之间不会互相争抢锁。
	writeMu sync.Mutex
}

// Open 打开（或创建）数据库并执行迁移。
//
// 连接串里固定了几个关键参数：busy_timeout 等待锁释放、WAL 提升读写并发、
// 外键约束默认开启。目录权限设为仅当前用户可访问。
func Open(path string) (*DB, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return nil, fmt.Errorf("创建数据库目录: %w", err)
	}
	dsn := "file:" + path + "?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=foreign_keys(1)"
	sqlDB, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("打开数据库: %w", err)
	}
	// 连接数保持在较小范围，既够读并发，也不会让 SQLite 积累过多锁竞争。
	sqlDB.SetMaxOpenConns(4)
	sqlDB.SetMaxIdleConns(4)

	db := &DB{sqlDB: sqlDB, path: path}
	// 迁移失败时必须关闭连接，避免泄漏文件句柄。
	if err := db.migrate(); err != nil {
		sqlDB.Close()
		return nil, err
	}
	return db, nil
}

// Close 关闭数据库。
func (db *DB) Close() error {
	return db.sqlDB.Close()
}

// Path 返回数据库文件路径。
func (db *DB) Path() string {
	return db.path
}

// NowMillis 返回当前毫秒时间戳，供服务层统一时间口径。
func NowMillis() int64 {
	return nowMillis()
}

// migrate 按 PRAGMA user_version 逐版执行迁移。
//
// 已有结构通过新增迁移版本演进，不修改历史迁移；所有步骤与版本号写入在
// 同一个事务里完成，避免中途失败留下半成品。
func (db *DB) migrate() error {
	var version int
	if err := db.sqlDB.QueryRow("PRAGMA user_version").Scan(&version); err != nil {
		return fmt.Errorf("读取数据库版本: %w", err)
	}
	if version >= schemaVersion {
		return nil
	}

	tx, err := db.sqlDB.Begin()
	if err != nil {
		return fmt.Errorf("开始迁移事务: %w", err)
	}
	defer tx.Rollback()

	if version < 1 {
		if _, err := tx.Exec(schemaV1); err != nil {
			return fmt.Errorf("执行数据库迁移 v1: %w", err)
		}
	}
	if version < 2 {
		if _, err := tx.Exec(schemaV2); err != nil {
			return fmt.Errorf("执行数据库迁移 v2: %w", err)
		}
	}
	if version < 3 {
		if _, err := tx.Exec(schemaV3); err != nil {
			return fmt.Errorf("执行数据库迁移 v3: %w", err)
		}
	}
	if version < 4 {
		if _, err := tx.Exec(schemaV4); err != nil {
			return fmt.Errorf("执行数据库迁移 v4: %w", err)
		}
	}
	if version < 5 {
		if _, err := tx.Exec(schemaV5); err != nil {
			return fmt.Errorf("执行数据库迁移 v5: %w", err)
		}
	}
	if version < 6 {
		if _, err := tx.Exec(schemaV6); err != nil {
			return fmt.Errorf("执行数据库迁移 v6: %w", err)
		}
	}
	if _, err := tx.Exec(fmt.Sprintf("PRAGMA user_version = %d", schemaVersion)); err != nil {
		return fmt.Errorf("写入数据库版本: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("提交数据库迁移: %w", err)
	}
	return nil
}

// schemaVersion 是当前数据库结构版本，对应 PRAGMA user_version。
const schemaVersion = 6

// schemaV6 是第 6 版数据库结构：工作区支持挂载多个源文件夹。
//
// 一个工作区（项目）可以包含多个目录：第一个是「主要目录」，作为会话的
// 工作目录；其余是「附加目录」，同样纳入 Agent 的可访问范围。历史工作区
// 的主目录原样迁移为 position = 0 的一条；workspaces.path 列保留并继续
// 表示主要目录，供去重与内置临时目录逻辑沿用。
//
// folder 路径同样唯一：同一目录不能既是 A 工作区的主目录又是 B 工作区的
// 附加目录，避免两套授权范围交叉。
const schemaV6 = `
CREATE TABLE IF NOT EXISTS workspace_folders (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  path TEXT NOT NULL UNIQUE,
  position INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS workspace_folders_ws ON workspace_folders(workspace_id, position);
INSERT INTO workspace_folders (id, workspace_id, path, position, created_at)
  SELECT 'wfo_' || id, id, path, 0, created_at FROM workspaces;
`

// schemaV3 是第 3 版数据库结构：给用量表加会话维度索引。
//
// 上下文占用显示需要按会话取最近一次用量，没有该索引时每次列表
// 刷新都要全表扫描。
const schemaV3 = `
CREATE INDEX IF NOT EXISTS usage_thread ON usage(thread_id, ts DESC);
`

// schemaV4 是第 4 版数据库结构：用量表补记模型调用耗时。
//
// 输出速度 = 输出 token ÷ 调用耗时，把耗时落库才能在重启或重新拉取
// 会话列表后继续展示最近一轮的真实速度，而不是显示为空。
const schemaV4 = `
ALTER TABLE usage ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;
`

// schemaV5 是第 5 版数据库结构：工作区增加用途标记。
//
// purpose = project 是用户添加的项目目录（历史数据的默认值）；
// purpose = conversation 是应用内置的临时会话目录，绑定它的会话
// 即「临时会话」，界面上据此做展示区分。
const schemaV5 = `
ALTER TABLE workspaces ADD COLUMN purpose TEXT NOT NULL DEFAULT 'project';
`

// schemaV1 是第 1 版数据库结构：会话、工作区、事件、运行、审批、审计、
// 用量、设置与备份九张表，以及配套索引。
//
// 事件表按 thread_id + seq 唯一，seq 由写入方在事务内分配；其余表用 AUTOINCREMENT
// 主键或字符串标识，软删除通过 deleted_at 表达。
const schemaV1 = `
CREATE TABLE IF NOT EXISTS threads (
  id TEXT PRIMARY KEY,
  mode TEXT NOT NULL,
  title TEXT NOT NULL,
  model_id TEXT NOT NULL DEFAULT '',
  workspace_id TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'idle',
  archived INTEGER NOT NULL DEFAULT 0,
  tokens INTEGER NOT NULL DEFAULT 0,
  deleted_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS threads_mode_updated ON threads(mode, archived, updated_at DESC);

CREATE TABLE IF NOT EXISTS workspaces (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL UNIQUE,
  trust TEXT NOT NULL DEFAULT 'restricted',
  created_at INTEGER NOT NULL,
  last_opened_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  run_id TEXT,
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(thread_id, seq)
);
CREATE INDEX IF NOT EXISTS events_thread_seq ON events(thread_id, seq);

CREATE TABLE IF NOT EXISTS runs (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  status TEXT NOT NULL,
  model_id TEXT NOT NULL DEFAULT '',
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  error_code TEXT,
  error_message TEXT
);
CREATE INDEX IF NOT EXISTS runs_thread ON runs(thread_id, started_at);

CREATE TABLE IF NOT EXISTS approvals (
  id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  run_id TEXT NOT NULL,
  tool_call_id TEXT NOT NULL,
  tool TEXT NOT NULL,
  risk TEXT NOT NULL,
  decision TEXT,
  scope TEXT,
  requested_at INTEGER NOT NULL,
  decided_at INTEGER
);
CREATE INDEX IF NOT EXISTS approvals_pending ON approvals(thread_id, decision);

CREATE TABLE IF NOT EXISTS audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  thread_id TEXT,
  run_id TEXT,
  tool TEXT NOT NULL,
  risk TEXT NOT NULL,
  decision TEXT NOT NULL,
  scope TEXT,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS audit_ts ON audit_log(ts DESC);
CREATE INDEX IF NOT EXISTS audit_thread ON audit_log(thread_id, ts DESC);

CREATE TABLE IF NOT EXISTS usage (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  thread_id TEXT,
  model_id TEXT NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_tokens INTEGER NOT NULL DEFAULT 0,
  reasoning_tokens INTEGER NOT NULL DEFAULT 0,
  cost_micros INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS usage_ts ON usage(ts DESC);
CREATE INDEX IF NOT EXISTS usage_model ON usage(model_id, ts DESC);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backups (
  id TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  backup_path TEXT NOT NULL,
  hash TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS backups_created ON backups(created_at DESC);
`

// schemaV2 增加会话标题来源标记。
//
// title_auto = 1 表示会话刚创建、标题仍为“新对话”，首轮消息可以自动命名；
// 用户手动改名或自动命名完成后置为 0，后续不再覆盖。
const schemaV2 = `
ALTER TABLE threads ADD COLUMN title_auto INTEGER NOT NULL DEFAULT 0;
UPDATE threads SET title_auto = 1 WHERE title = '新对话';
`
