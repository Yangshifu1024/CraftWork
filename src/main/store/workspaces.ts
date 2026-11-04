// workspaces + workspace_folders 表读写。
import { newId } from "../../shared/ids";
import type { DB } from "./db";
import { ErrNotFound, nowMillis } from "./db";
import { IdPrefix } from "../../shared/ids";

export interface WorkspaceRow {
  id: string;
  name: string;
  path: string;
  trust: string;
  purpose: string;
  createdAt: number;
  lastOpenedAt: number;
}

export interface WorkspaceFolderRow {
  id: string;
  workspaceId: string;
  path: string;
  position: number;
}

export interface WorkspaceWithFolders extends WorkspaceRow {
  folders: WorkspaceFolderRow[];
}

function toWorkspace(row: Record<string, unknown>): WorkspaceRow {
  return {
    id: String(row["id"] ?? ""),
    name: String(row["name"] ?? ""),
    path: String(row["path"] ?? ""),
    trust: String(row["trust"] ?? "restricted"),
    purpose: String(row["purpose"] ?? "project"),
    createdAt: Number(row["created_at"] ?? 0),
    lastOpenedAt: Number(row["last_opened_at"] ?? 0),
  };
}

function toFolder(row: Record<string, unknown>): WorkspaceFolderRow {
  return {
    id: String(row["id"] ?? ""),
    workspaceId: String(row["workspace_id"] ?? ""),
    path: String(row["path"] ?? ""),
    position: Number(row["position"] ?? 0),
  };
}

/**
 * 按 path 唯一 upsert：已存在沿用原 id，只更新 name/trust/last_opened_at；
 * purpose 由首次创建决定不更新（临时目录语义不可被项目覆盖）。
 */
export function upsertWorkspace(
  db: DB,
  input: { name: string; path: string; trust: string; purpose: string },
): WorkspaceRow {
  const now = nowMillis();
  const existing = db.prepare("SELECT * FROM workspaces WHERE path = ?").get(input.path) as
    | Record<string, unknown>
    | undefined;
  if (existing) {
    const row = toWorkspace(existing);
    db.prepare("UPDATE workspaces SET name = ?, trust = ?, last_opened_at = ? WHERE id = ?").run(
      input.name,
      input.trust,
      now,
      row.id,
    );
    return { ...row, name: input.name, trust: input.trust, lastOpenedAt: now };
  }
  const id = newId(IdPrefix.Workspace);
  db.prepare(
    "INSERT INTO workspaces (id, name, path, trust, purpose, created_at, last_opened_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
  ).run(id, input.name, input.path, input.trust, input.purpose, now, now);
  return { id, name: input.name, path: input.path, trust: input.trust, purpose: input.purpose, createdAt: now, lastOpenedAt: now };
}

export function listWorkspaces(db: DB): WorkspaceWithFolders[] {
  const rows = db.prepare("SELECT * FROM workspaces ORDER BY last_opened_at DESC").all() as Record<
    string,
    unknown
  >[];
  return rows.map((row) => {
    const ws = toWorkspace(row);
    return { ...ws, folders: listWorkspaceFolders(db, ws.id) };
  });
}

export function getWorkspace(db: DB, id: string): WorkspaceWithFolders {
  const row = db.prepare("SELECT * FROM workspaces WHERE id = ?").get(id) as Record<string, unknown> | undefined;
  if (!row) throw new ErrNotFound("工作区不存在");
  const ws = toWorkspace(row);
  return { ...ws, folders: listWorkspaceFolders(db, ws.id) };
}

/**
 * 读取工作区的源文件夹；兜底保证第一条是 workspaces.path 指向的主目录
 * （历史数据或异常删除时自动补一条，避免路径解析失去主根）。
 */
export function listWorkspaceFolders(db: DB, workspaceId: string): WorkspaceFolderRow[] {
  const wsRow = db.prepare("SELECT path FROM workspaces WHERE id = ?").get(workspaceId) as
    | { path?: string }
    | undefined;
  const rows = db
    .prepare("SELECT * FROM workspace_folders WHERE workspace_id = ? ORDER BY position")
    .all(workspaceId) as Record<string, unknown>[];
  const folders = rows.map(toFolder);
  const mainPath = wsRow?.path ?? "";
  if (mainPath && !folders.some((f) => f.path === mainPath)) {
    const added: WorkspaceFolderRow = {
      id: newId(IdPrefix.WorkspaceFolder),
      workspaceId,
      path: mainPath,
      position: 0,
    };
    db.prepare(
      "INSERT OR IGNORE INTO workspace_folders (id, workspace_id, path, position, created_at) VALUES (?, ?, ?, 0, ?)",
    ).run(added.id, workspaceId, mainPath, nowMillis());
    folders.unshift(added);
  }
  return folders;
}

/** 事务内整体替换文件夹列表，并同步 workspaces.path = paths[0]。 */
export function replaceWorkspaceFolders(db: DB, workspaceId: string, paths: string[]): void {
  db.transaction(() => {
    db.prepare("DELETE FROM workspace_folders WHERE workspace_id = ?").run(workspaceId);
    paths.forEach((p, i) => {
      db.prepare(
        "INSERT INTO workspace_folders (id, workspace_id, path, position, created_at) VALUES (?, ?, ?, ?, ?)",
      ).run(newId(IdPrefix.WorkspaceFolder), workspaceId, p, i, nowMillis());
    });
    if (paths.length > 0) {
      const mainPath = paths[0];
      if (mainPath) {
        db.prepare("UPDATE workspaces SET path = ? WHERE id = ?").run(mainPath, workspaceId);
      }
    }
  });
}

export function updateWorkspaceTrust(db: DB, id: string, trust: string): void {
  const res = db.prepare("UPDATE workspaces SET trust = ? WHERE id = ?").run(trust, id);
  if (res.changes === 0) throw new ErrNotFound("工作区不存在");
}

export function updateWorkspacePurpose(db: DB, id: string, purpose: string): void {
  db.prepare("UPDATE workspaces SET purpose = ? WHERE id = ?").run(purpose, id);
}

/** 历史无工作区会话统一归入临时目录（幂等）。 */
export function backfillThreadWorkspaces(db: DB, workspaceId: string): void {
  db.prepare("UPDATE threads SET workspace_id = ? WHERE workspace_id = ''").run(workspaceId);
}

/** 内置临时会话目录的工作区 id（purpose=conversation；启动 ensure 后必然存在，取不到回落空串）。 */
export function defaultConversationWorkspaceId(db: DB): string {
  const row = db
    .prepare("SELECT id FROM workspaces WHERE purpose = 'conversation' ORDER BY created_at LIMIT 1")
    .get() as { id?: string } | undefined;
  return row?.id ?? "";
}

export function deleteWorkspace(db: DB, id: string): void {
  db.transaction(() => {
    db.prepare("DELETE FROM workspace_folders WHERE workspace_id = ?").run(id);
    db.prepare("DELETE FROM workspaces WHERE id = ?").run(id);
  });
}
