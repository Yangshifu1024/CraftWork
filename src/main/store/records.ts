// 审计 / 用量 / 设置 / 备份 表读写。
import { newId } from "../../shared/ids";
import type { AuditEntry, Backup, UsageByDay, UsageByModel, UsageSummary } from "../../shared/types";
import type { DB } from "./db";
import { ErrNotFound } from "./db";
import { IdPrefix } from "../../shared/ids";

// ---------- 审计 ----------

export interface AuditRecord {
  ts?: number;
  threadId?: string;
  runId?: string;
  tool: string;
  risk: string;
  decision: string;
  scope?: string;
  detail?: string;
}

export function addAudit(db: DB, rec: AuditRecord): void {
  db.prepare(
    "INSERT INTO audit_log (ts, thread_id, run_id, tool, risk, decision, scope, detail) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
  ).run(
    rec.ts ?? Date.now(),
    rec.threadId || null,
    rec.runId || null,
    rec.tool,
    rec.risk,
    rec.decision,
    rec.scope || null,
    rec.detail || null,
  );
}

export function listAudit(db: DB, limit: number): AuditEntry[] {
  const n = limit <= 0 || limit > 500 ? 200 : limit;
  const rows = db.prepare("SELECT * FROM audit_log ORDER BY ts DESC, id DESC LIMIT ?").all(n) as Record<
    string,
    unknown
  >[];
  return rows.map((row) => ({
    id: Number(row["id"] ?? 0),
    ts: Number(row["ts"] ?? 0),
    threadId: String(row["thread_id"] ?? ""),
    runId: String(row["run_id"] ?? ""),
    tool: String(row["tool"] ?? ""),
    risk: String(row["risk"] ?? ""),
    decision: String(row["decision"] ?? ""),
    scope: String(row["scope"] ?? ""),
    detail: String(row["detail"] ?? ""),
  }));
}

// ---------- 用量 ----------

export interface UsageRecord {
  threadId?: string;
  modelId: string;
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  reasoningTokens: number;
  costMicros: number;
  durationMs?: number;
}

export function addUsage(db: DB, rec: UsageRecord): void {
  db.prepare(
    "INSERT INTO usage (ts, thread_id, model_id, input_tokens, output_tokens, cache_tokens, reasoning_tokens, cost_micros, duration_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
  ).run(
    Date.now(),
    rec.threadId || null,
    rec.modelId,
    rec.inputTokens,
    rec.outputTokens,
    rec.cacheTokens,
    rec.reasoningTokens,
    rec.costMicros,
    rec.durationMs ?? 0,
  );
}

interface Tokens {
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
}

function sumWindow(db: DB, sinceTs: number): Tokens {
  const row = db
    .prepare(
      "SELECT COALESCE(SUM(input_tokens),0) AS i, COALESCE(SUM(output_tokens),0) AS o, COALESCE(SUM(cost_micros),0) AS c FROM usage WHERE ts >= ?",
    )
    .get(sinceTs) as Record<string, unknown>;
  return { inputTokens: Number(row["i"] ?? 0), outputTokens: Number(row["o"] ?? 0), costMicros: Number(row["c"] ?? 0) };
}

/** 用量汇总：今天/近7天/近30天/按模型近30天/按天近14天；窗口从本地时区今天零点回推。 */
export function usageSummary(db: DB): UsageSummary {
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const day = 24 * 60 * 60 * 1000;

  const today = sumWindow(db, midnight);
  const week = sumWindow(db, midnight - 6 * day);
  const month = sumWindow(db, midnight - 29 * day);

  const modelRows = db
    .prepare(
      "SELECT model_id, COALESCE(SUM(input_tokens),0) AS i, COALESCE(SUM(output_tokens),0) AS o, COALESCE(SUM(cost_micros),0) AS c FROM usage WHERE ts >= ? GROUP BY model_id ORDER BY i + o DESC LIMIT 20",
    )
    .all(midnight - 29 * day) as Record<string, unknown>[];
  const byModel: UsageByModel[] = modelRows.map((row) => ({
    modelId: String(row["model_id"] ?? ""),
    inputTokens: Number(row["i"] ?? 0),
    outputTokens: Number(row["o"] ?? 0),
    costMicros: Number(row["c"] ?? 0),
  }));

  const dayRows = db
    .prepare(
      "SELECT date(ts/1000,'unixepoch','localtime') AS d, COALESCE(SUM(input_tokens),0) AS i, COALESCE(SUM(output_tokens),0) AS o, COALESCE(SUM(cost_micros),0) AS c FROM usage WHERE ts >= ? GROUP BY d ORDER BY d ASC",
    )
    .all(midnight - 13 * day) as Record<string, unknown>[];
  const byDay: UsageByDay[] = dayRows.map((row) => ({
    day: String(row["d"] ?? ""),
    inputTokens: Number(row["i"] ?? 0),
    outputTokens: Number(row["o"] ?? 0),
    costMicros: Number(row["c"] ?? 0),
  }));

  return { today, week, month, byModel, byDay };
}

// ---------- 设置 ----------

export function getSetting<T>(db: DB, key: string, target: T): T {
  const row = db.prepare("SELECT value FROM settings WHERE key = ?").get(key) as { value?: string } | undefined;
  if (!row) return target;
  try {
    return JSON.parse(row.value ?? "") as T;
  } catch {
    return target;
  }
}

export function setSetting(db: DB, key: string, value: unknown): void {
  db.prepare(
    "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
  ).run(key, JSON.stringify(value ?? null));
}

export function deleteSetting(db: DB, key: string): void {
  db.prepare("DELETE FROM settings WHERE key = ?").run(key);
}

// ---------- 备份 ----------

export function addBackup(db: DB, rec: { path: string; backupPath: string; hash: string; size: number }): string {
  const id = newId(IdPrefix.Backup);
  db.prepare(
    "INSERT INTO backups (id, path, backup_path, hash, size, created_at) VALUES (?, ?, ?, ?, ?, ?)",
  ).run(id, rec.path, rec.backupPath, rec.hash, rec.size, Date.now());
  return id;
}

export function getBackup(db: DB, id: string): Backup {
  const row = db.prepare("SELECT * FROM backups WHERE id = ?").get(id) as Record<string, unknown> | undefined;
  if (!row) throw new ErrNotFound("备份不存在");
  return toBackup(row);
}

function toBackup(row: Record<string, unknown>): Backup {
  return {
    id: String(row["id"] ?? ""),
    path: String(row["path"] ?? ""),
    backupPath: String(row["backup_path"] ?? ""),
    hash: String(row["hash"] ?? ""),
    size: Number(row["size"] ?? 0),
    createdAt: Number(row["created_at"] ?? 0),
  };
}

export function listBackups(db: DB, limit: number): Backup[] {
  const n = limit <= 0 || limit > 200 ? 50 : limit;
  const rows = db.prepare("SELECT * FROM backups ORDER BY created_at DESC LIMIT ?").all(n) as Record<string, unknown>[];
  return rows.map(toBackup);
}
