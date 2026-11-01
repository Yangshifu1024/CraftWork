// 本文件负责 SQLite 存储的打开、关闭与迁移：建立数据目录、配置连接参数、
// 按版本执行建表语句，并提供数据库对象的公共入口。
//
// 具体的数据读写分散在同包的其他文件里（线程、事件、审批、审计、用量等）。

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

// migrate 按 PRAGMA user_version 判断是否需要建表。
//
// 当前只有第 1 版结构；后续修改表结构时应新增迁移版本，而不是修改已有的建表语句。
// 建表与版本号写入在同一个事务里完成，避免中途失败留下半成品。
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

	if _, err := tx.Exec(schemaV1); err != nil {
		return fmt.Errorf("执行数据库迁移: %w", err)
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
const schemaVersion = 1

// schemaV1 是第 1 版数据库结构：线程、工作区、事件、运行、审批、审计、
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
