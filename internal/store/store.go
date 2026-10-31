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
	sqlDB   *sql.DB
	path    string
	writeMu sync.Mutex
}

// Open 打开（或创建）数据库并执行迁移。
func Open(path string) (*DB, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return nil, fmt.Errorf("创建数据库目录: %w", err)
	}
	dsn := "file:" + path + "?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=foreign_keys(1)"
	sqlDB, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("打开数据库: %w", err)
	}
	sqlDB.SetMaxOpenConns(4)
	sqlDB.SetMaxIdleConns(4)

	db := &DB{sqlDB: sqlDB, path: path}
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

const schemaVersion = 1

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
