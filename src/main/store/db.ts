// SQLite 存储：事实源。表结构与迁移链以 schemaVersion 演进，历史迁移不修改；
// 写入经 writeMu 串行化（seq 分配与插入必须同事务，避免并发错序）。
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

// 通过 createRequire 引入 node:sqlite，避免打包器把内置模块改写成 npm 包。
const require = createRequire(import.meta.url);
const { DatabaseSync } = require("node:sqlite") as typeof import("node:sqlite");

export type Database = import("node:sqlite").DatabaseSync;
export type Statement = import("node:sqlite").StatementSync;

export const SCHEMA_VERSION = 7;

export class ErrNotFound extends Error {
  constructor(message = "记录不存在") {
    super(message);
    this.name = "ErrNotFound";
  }
}

export function nowMillis(): number {
  return Date.now();
}

/** 全局写互斥：把「检查+写入」的临界区串起来（Node 单线程，异步间隙需要保护时用）。 */
class WriteMutex {
  private queue: Promise<void> = Promise.resolve();

  run<T>(fn: () => T): Promise<T> {
    const next = this.queue.then(fn);
    this.queue = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }
}

export class DB {
  readonly writeMu = new WriteMutex();
  private constructor(private readonly db: Database, private readonly filePath: string) {}

  static open(path: string): DB {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    const db = new DatabaseSync(path);
    db.exec("PRAGMA busy_timeout = 5000");
    db.exec("PRAGMA journal_mode = WAL");
    db.exec("PRAGMA foreign_keys = 1");
    const wrapper = new DB(db, path);
    migrate(db);
    return wrapper;
  }

  get path(): string {
    return this.filePath;
  }

  prepare(sql: string): Statement {
    return this.db.prepare(sql);
  }

  exec(sql: string): void {
    this.db.exec(sql);
  }

  /** 事务：fn 内抛错自动回滚。Node 同步驱动下事务内不能混入异步。 */
  transaction<T>(fn: () => T): T {
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const result = fn();
      this.db.exec("COMMIT");
      return result;
    } catch (err) {
      try {
        this.db.exec("ROLLBACK");
      } catch {
        // 已回滚
      }
      throw err;
    }
  }

  close(): void {
    this.db.close();
  }
}

// ---------- 迁移 ----------
// 历史迁移不修改；新表结构一律追加新版本。

const SCHEMA_V1 = `
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
`;

const SCHEMA_V2 = `
ALTER TABLE threads ADD COLUMN title_auto INTEGER NOT NULL DEFAULT 0;
UPDATE threads SET title_auto = 1 WHERE title = '新对话';
`;

const SCHEMA_V3 = `
CREATE INDEX IF NOT EXISTS usage_thread ON usage(thread_id, ts DESC);
`;

const SCHEMA_V4 = `
ALTER TABLE usage ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0;
`;

const SCHEMA_V5 = `
ALTER TABLE workspaces ADD COLUMN purpose TEXT NOT NULL DEFAULT 'project';
`;

const SCHEMA_V6 = `
CREATE TABLE IF NOT EXISTS workspace_folders (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  path TEXT NOT NULL UNIQUE, position INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS workspace_folders_ws ON workspace_folders(workspace_id, position);
INSERT INTO workspace_folders (id, workspace_id, path, position, created_at)
  SELECT 'wfo_' || id, id, path, 0, created_at FROM workspaces;
`;

// V7：一次性清理遗留空会话——无任何事件、自动命名、标题「新对话」的线程；
// 有事件或被手动改名（title_auto = 0）的会话一律保留。
export const SCHEMA_V7 = `
DELETE FROM threads WHERE title = '新对话' AND title_auto = 1 AND NOT EXISTS (SELECT 1 FROM events WHERE events.thread_id = threads.id);
`;

const MIGRATIONS: string[] = ["", SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4, SCHEMA_V5, SCHEMA_V6, SCHEMA_V7];

function migrate(db: Database): void {
  const row = db.prepare("PRAGMA user_version").get() as { user_version?: number } | undefined;
  let version = row?.user_version ?? 0;
  if (version >= SCHEMA_VERSION) return;
  for (let v = version + 1; v <= SCHEMA_VERSION; v++) {
    const sql = MIGRATIONS[v];
    if (!sql) continue;
    db.exec("BEGIN IMMEDIATE");
    try {
      db.exec(sql);
      db.exec(`PRAGMA user_version = ${v}`);
      db.exec("COMMIT");
      version = v;
    } catch (err) {
      try {
        db.exec("ROLLBACK");
      } catch {
        // 已回滚
      }
      throw new Error(`数据库迁移到版本 ${v} 失败：${String(err)}`);
    }
  }
  void version;
}

export { join as joinPath };
