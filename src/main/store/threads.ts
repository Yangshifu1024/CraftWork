// threads 表读写。
import type { DB } from "./db";
import { ErrNotFound, nowMillis } from "./db";

export interface ThreadRow {
  id: string;
  mode: string;
  title: string;
  titleAuto: boolean;
  modelId: string;
  workspaceId: string;
  status: string;
  archived: boolean;
  tokens: number;
  createdAt: number;
  updatedAt: number;
}

function toThread(row: Record<string, unknown>): ThreadRow {
  return {
    id: String(row["id"] ?? ""),
    mode: String(row["mode"] ?? "code"),
    title: String(row["title"] ?? ""),
    titleAuto: Number(row["title_auto"] ?? 0) === 1,
    modelId: String(row["model_id"] ?? ""),
    workspaceId: String(row["workspace_id"] ?? ""),
    status: String(row["status"] ?? "idle"),
    archived: Number(row["archived"] ?? 0) === 1,
    tokens: Number(row["tokens"] ?? 0),
    createdAt: Number(row["created_at"] ?? 0),
    updatedAt: Number(row["updated_at"] ?? 0),
  };
}

const COLS =
  "id, mode, title, title_auto, model_id, workspace_id, status, archived, tokens, created_at, updated_at";

export function createThread(db: DB, t: Omit<ThreadRow, "createdAt" | "updatedAt" | "tokens">): ThreadRow {
  const now = nowMillis();
  db.prepare(
    `INSERT INTO threads (${COLS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)`,
  ).run(
    t.id,
    t.mode,
    t.title,
    t.titleAuto ? 1 : 0,
    t.modelId,
    t.workspaceId,
    t.status,
    t.archived ? 1 : 0,
    now,
    now,
  );
  return { ...t, tokens: 0, createdAt: now, updatedAt: now };
}

export function getThread(db: DB, id: string): ThreadRow {
  const row = db.prepare(`SELECT ${COLS} FROM threads WHERE id = ? AND deleted_at IS NULL`).get(id) as
    | Record<string, unknown>
    | undefined;
  if (!row) throw new ErrNotFound("会话不存在");
  return toThread(row);
}

export function listThreads(db: DB, mode: string, includeArchived: boolean): ThreadRow[] {
  const archivedClause = includeArchived ? "" : "AND archived = 0";
  const rows = db
    .prepare(
      `SELECT ${COLS} FROM threads WHERE deleted_at IS NULL AND mode = ? ${archivedClause} ORDER BY updated_at DESC`,
    )
    .all(mode) as Record<string, unknown>[];
  return rows.map(toThread);
}

/** 标题或事件内容匹配的搜索（LIKE + DISTINCT + LIMIT 100）。 */
export function searchThreads(db: DB, mode: string, query: string): ThreadRow[] {
  const like = `%${query}%`;
  const cols = COLS.split(", ")
    .map((c) => `t.${c}`)
    .join(", ");
  const rows = db
    .prepare(
      `SELECT DISTINCT ${cols}
       FROM threads t
       LEFT JOIN events e ON e.thread_id = t.id
       WHERE t.deleted_at IS NULL AND t.mode = ? AND (t.title LIKE ? OR e.payload LIKE ?)
       ORDER BY t.updated_at DESC LIMIT 100`,
    )
    .all(mode, like, like) as Record<string, unknown>[];
  return rows.map(toThread);
}

export function renameThread(db: DB, id: string, title: string): void {
  const res = db
    .prepare("UPDATE threads SET title = ?, title_auto = 0, updated_at = ? WHERE id = ? AND deleted_at IS NULL")
    .run(title, nowMillis(), id);
  if (res.changes === 0) throw new ErrNotFound("会话不存在");
}

/** 条件更新自动命名：仅当仍是自动标题且标题未被改动时生效。返回是否实际更新。 */
export function autoTitleThread(db: DB, id: string, title: string, prev: string): boolean {
  const res = db
    .prepare(
      "UPDATE threads SET title = ?, updated_at = ? WHERE id = ? AND title_auto = 1 AND title = ? AND deleted_at IS NULL",
    )
    .run(title, nowMillis(), id, prev);
  return res.changes > 0;
}

export function setThreadStatus(db: DB, id: string, status: string): void {
  db.prepare("UPDATE threads SET status = ?, updated_at = ? WHERE id = ?").run(status, nowMillis(), id);
}

export function setThreadModel(db: DB, id: string, modelId: string): void {
  db.prepare("UPDATE threads SET model_id = ?, updated_at = ? WHERE id = ?").run(modelId, nowMillis(), id);
}

export function setThreadArchived(db: DB, id: string, archived: boolean): void {
  db.prepare("UPDATE threads SET archived = ?, updated_at = ? WHERE id = ?").run(archived ? 1 : 0, nowMillis(), id);
}

export function touchThread(db: DB, id: string): void {
  db.prepare("UPDATE threads SET updated_at = ? WHERE id = ?").run(nowMillis(), id);
}

export function addThreadTokens(db: DB, id: string, tokens: number): void {
  if (tokens === 0) return;
  db.prepare("UPDATE threads SET tokens = tokens + ? WHERE id = ?").run(tokens, id);
}

/** 物理删除：事务内删 events + approvals + runs + threads。 */
export function deleteThread(db: DB, id: string): void {
  db.transaction(() => {
    db.prepare("DELETE FROM events WHERE thread_id = ?").run(id);
    db.prepare("DELETE FROM approvals WHERE thread_id = ?").run(id);
    db.prepare("DELETE FROM runs WHERE thread_id = ?").run(id);
    db.prepare("DELETE FROM threads WHERE id = ?").run(id);
  });
}

/** 删除工作区下全部会话（事件 / 审批 / 运行一并清理），返回删除数。项目删除时级联调用。 */
export function deleteThreadsByWorkspace(db: DB, workspaceId: string): number {
  const rows = db.prepare("SELECT id FROM threads WHERE workspace_id = ?").all(workspaceId) as Array<{ id: string }>;
  for (const row of rows) deleteThread(db, row.id);
  return rows.length;
}
