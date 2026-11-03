// 工作区服务：目录管理、文件树/读取/写入（含备份与外部修改检测）、Git 状态、文档预览。
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import type { DocumentPreview, FileContent, FileNode, LocatedFile, WorkspaceView } from "../../shared/types";
import { EventType } from "../../shared/events";
import type { Recorder } from "../core/record";
import { isBinary, run, sha256Hex, writeFileAtomic, backupName } from "../core/platform";
import { resolveAt, resolveExisting, relative } from "../policy/resolve";
import { isIgnoredDir, humanSize } from "../tools/registry";
import { addAudit, addBackup } from "../store/records";
import type { DB } from "../store/db";
import {
  deleteWorkspace,
  getWorkspace,
  listWorkspaces,
  replaceWorkspaceFolders,
  updateWorkspacePurpose,
  updateWorkspaceTrust,
  upsertWorkspace,
  type WorkspaceWithFolders,
} from "../store/workspaces";
import { docKind, docReadFile } from "../doc";
import { diff as computeDiffLines } from "../core/editor";

const MAX_READ_BYTES = 10 * 1024 * 1024; // 编辑器读取上限
const MAX_TEXT_BYTES = 2 * 1024 * 1024; // 文本截断上限
const PREVIEW_TEXT_LIMIT = 8000;

const IGNORED_WALK = new Set([".git", "node_modules", "dist", "bin", ".task"]);

function toView(ws: WorkspaceWithFolders): WorkspaceView {
  return {
    id: ws.id,
    name: ws.name,
    path: ws.path,
    trust: ws.trust,
    purpose: ws.purpose,
    lastOpenedAt: ws.lastOpenedAt,
    folders: ws.folders.map((f) => ({ id: f.id, path: f.path, position: f.position })),
  };
}

function normalizeFolder(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) throw new Error("目录路径不能为空");
  const abs = resolveAt(trimmed, ".").abs;
  try {
    const abs2 = realpathSync.native(abs);
    const st = statSync(abs2);
    if (!st.isDirectory()) throw new Error("路径不是目录");
    return abs2;
  } catch (err) {
    if (err instanceof Error && err.message === "路径不是目录") throw err;
    throw new Error("目录不存在或无法访问");
  }
}

export class WorkspaceService {
  constructor(
    private readonly db: DB,
    private readonly recorder: Recorder,
    private readonly backupsDir: string,
  ) {}

  list(): WorkspaceView[] {
    return listWorkspaces(this.db).map(toView);
  }

  active(): WorkspaceView | null {
    const all = this.list();
    return all[0] ?? null;
  }

  add(path: string, trust: string): WorkspaceView {
    const abs = normalizeFolder(path);
    if (this.isConversationPath(abs)) throw new Error("内置临时会话目录不能作为项目添加");
    const name = basename(abs);
    const ws = upsertWorkspace(this.db, { name, path: abs, trust: trust === "trusted" ? "trusted" : "restricted", purpose: "project" });
    // 主目录也落一条文件夹记录（folder 路径全局唯一，避免两套授权范围交叉）
    const folders = getWorkspace(this.db, ws.id).folders;
    if (!folders.some((f) => f.path === abs)) {
      replaceWorkspaceFolders(this.db, ws.id, [abs, ...folders.map((f) => f.path)]);
    }
    this.publishChanged(ws.id, { active: true });
    return toView(getWorkspace(this.db, ws.id));
  }

  addProject(name: string, folders: string[], trust: string): WorkspaceView {
    const cleaned: string[] = [];
    for (const f of folders) {
      const abs = normalizeFolder(f);
      if (this.isConversationPath(abs)) throw new Error("内置临时会话目录不能作为项目添加");
      if (!cleaned.includes(abs)) cleaned.push(abs);
    }
    if (cleaned.length === 0) throw new Error("请至少选择一个目录");
    // 任一目录已属其他工作区则拒绝
    for (const ws of listWorkspaces(this.db)) {
      if (ws.folders.some((f) => cleaned.includes(f.path))) {
        throw new Error(`目录已被工作区「${ws.name}」使用`);
      }
    }
    let finalName = name.trim() || basename(cleaned[0] ?? "");
    const taken = new Set(listWorkspaces(this.db).map((w) => w.name));
    let suffix = 2;
    while (taken.has(finalName)) {
      finalName = `${name.trim() || basename(cleaned[0] ?? "")} (${suffix++})`;
    }
    const ws = upsertWorkspace(this.db, {
      name: finalName,
      path: cleaned[0] ?? "",
      trust: trust === "trusted" ? "trusted" : "restricted",
      purpose: "project",
    });
    replaceWorkspaceFolders(this.db, ws.id, cleaned);
    this.publishChanged(ws.id, { active: true });
    return toView(getWorkspace(this.db, ws.id));
  }

  setActive(workspaceId: string): WorkspaceView {
    const ws = getWorkspace(this.db, workspaceId);
    this.publishChanged(ws.id, { active: true });
    return toView(ws);
  }

  setTrust(workspaceId: string, trust: string): void {
    const ws = getWorkspace(this.db, workspaceId);
    if (ws.purpose === "conversation") throw new Error("内置临时会话目录不可修改信任级别");
    updateWorkspaceTrust(this.db, workspaceId, trust === "trusted" ? "trusted" : "restricted");
    this.publishChanged(workspaceId, { trust });
  }

  remove(workspaceId: string): void {
    const ws = getWorkspace(this.db, workspaceId);
    if (ws.purpose === "conversation") throw new Error("内置临时会话目录不可移除");
    deleteWorkspace(this.db, workspaceId);
    this.recorder.emit(EventType.WorkspaceUpdated, undefined, undefined, { workspaceId, removed: true });
  }

  // ---------- 文件 ----------

  listDir(workspaceId: string, folderId: string, rel: string): FileNode[] {
    const { root } = this.folderRoot(workspaceId, folderId);
    const target = resolveExisting([root], rel || ".").abs;
    let entries;
    try {
      entries = readdirSync(target, { withFileTypes: true });
    } catch {
      return [];
    }
    const nodes: FileNode[] = [];
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      if (entry.isDirectory() && isIgnoredDir(entry.name)) continue;
      const abs = join(target, entry.name);
      let st;
      try {
        st = statSync(abs);
      } catch {
        continue;
      }
      nodes.push({
        name: entry.name,
        path: rel ? `${rel}/${entry.name}` : entry.name,
        type: entry.isDirectory() ? "dir" : "file",
        size: st.size,
        modified: st.mtimeMs,
      });
    }
    nodes.sort((a, b) => (a.type === b.type ? a.name.localeCompare(b.name) : a.type === "dir" ? -1 : 1));
    return nodes;
  }

  readFile(workspaceId: string, folderId: string, rel: string): FileContent {
    const { ws, root, folder } = this.folderRoot(workspaceId, folderId);
    const abs = resolveExisting([root], rel).abs;
    const st = statSync(abs);
    if (st.isDirectory()) throw new Error("目标是目录");
    if (st.size > MAX_READ_BYTES) throw new Error(`文件过大（${humanSize(st.size)}），超过读取上限`);
    const raw = readFileSync(abs);
    const binary = isBinary(raw);
    let content = "";
    let truncated = false;
    if (!binary) {
      const text = raw.toString("utf8");
      if (Buffer.byteLength(text, "utf8") > MAX_TEXT_BYTES) {
        content = `${Buffer.from(text, "utf8").subarray(0, MAX_TEXT_BYTES).toString("utf8")}\n…（文本已截断）`;
        truncated = true;
      } else {
        content = text;
      }
    }
    return {
      workspaceId: ws.id,
      folderId: folder.id,
      path: relative(root, abs),
      content,
      hash: sha256Hex(raw),
      size: st.size,
      modified: st.mtimeMs,
      binary,
      truncated,
      language: languageOf(abs),
    };
  }

  /** 短路径引用定位：精确命中 → 后缀匹配 → 文件名匹配（均要求唯一）。 */
  locateFile(workspaceId: string, rel: string): LocatedFile {
    const ws = getWorkspace(this.db, workspaceId);
    const clean = rel.trim().replace(/^\.\//, "").replace(/^\/+/, "");
    if (!clean) throw new Error("路径不能为空");
    const roots = ws.folders.map((f) => f.path);
    // 1) 原样命中
    try {
      const resolved = resolveExisting(roots, clean);
      if (statSync(resolved.abs).isFile()) {
        return { workspaceId, folderId: ws.folders[resolved.rootIndex]?.id ?? "", path: relative(roots[0] ?? "", resolved.abs) };
      }
    } catch {
      // 继续
    }
    // 2) 遍历收集：文件名精确 + 路径后缀
    const byName: string[] = [];
    const bySuffix: string[] = [];
    const walk = (dir: string, depth: number): void => {
      if (depth > 6 || byName.length + bySuffix.length > 400) return;
      let entries;
      try {
        entries = readdirSync(dir, { withFileTypes: true });
      } catch {
        return;
      }
      let count = 0;
      for (const entry of entries) {
        if (++count > 200) return;
        if (entry.name.startsWith(".") || IGNORED_WALK.has(entry.name)) continue;
        const abs = join(dir, entry.name);
        if (entry.isDirectory()) {
          walk(abs, depth + 1);
        } else {
          const relPath = relative(roots[0] ?? "", abs);
          if (basename(abs) === clean) byName.push(relPath);
          else if (relPath === clean || relPath.endsWith(`/${clean}`)) bySuffix.push(relPath);
        }
      }
    };
    for (const root of roots) walk(root, 0);
    const pick = (list: string[]): string => {
      if (list.length === 1) return list[0] ?? "";
      if (list.length > 1) throw new Error("匹配到多个文件，请使用更完整的路径");
      return "";
    };
    const hit = pick(bySuffix) || pick(byName);
    if (!hit) throw new Error(`工作区内找不到 ${clean}`);
    const resolved = resolveExisting(roots, hit);
    return { workspaceId, folderId: ws.folders[resolved.rootIndex]?.id ?? "", path: hit };
  }

  /**
   * 写文件：expectedHash 外部修改检测 → 备份 → 原子写 → 审计。
   * 语义：写入前备份可恢复；冲突时拒绝覆盖。
   */
  writeFile(workspaceId: string, folderId: string, rel: string, content: string, expectedHash: string): void {
    const { root } = this.folderRoot(workspaceId, folderId);
    const resolved = resolveExisting([root], rel);
    const abs = resolved.abs;
    const display = relative(root, abs);
    let existing: Buffer | null = null;
    if (existsSync(abs)) {
      existing = readFileSync(abs);
      if (expectedHash && sha256Hex(existing) !== expectedHash) {
        throw new Error("文件已被外部修改，请重新加载后再保存");
      }
    }
    if (existing) {
      mkdirSync(this.backupsDir, { recursive: true, mode: 0o700 });
      const backupPath = join(this.backupsDir, backupName(abs));
      writeFileAtomic(backupPath, existing, 0o600);
      addBackup(this.db, { path: abs, backupPath, hash: sha256Hex(existing), size: existing.length });
    }
    mkdirSync(resolved.abs.slice(0, resolved.abs.lastIndexOf("/")) || "/", { recursive: true });
    writeFileAtomic(abs, Buffer.from(content, "utf8"), 0o644);
    addAudit(this.db, { tool: "editor_write", risk: "R1", decision: "allow", scope: "user", detail: display });
  }

  computeDiff(oldText: string, newText: string) {
    return computeDiffLines(oldText, newText);
  }

  /** Git 状态：path → 状态码（非仓库返回空 map，不报错）。 */
  async gitStatuses(workspaceId: string): Promise<Record<string, string>> {
    const ws = getWorkspace(this.db, workspaceId);
    const result = await run({ file: "git", args: ["status", "--porcelain"], dir: ws.path, timeoutMs: 10_000 });
    const map: Record<string, string> = {};
    if (result.exitCode !== 0) return map;
    for (const line of result.output.split("\n")) {
      if (line.length < 4) continue;
      const code = line.slice(0, 2).trim() || line[0] || "";
      let path = line.slice(3).trim();
      const arrow = path.indexOf(" -> ");
      if (arrow >= 0) path = path.slice(arrow + 4);
      if (path) map[path] = code;
    }
    return map;
  }

  /** 文档预览：docx/xlsx/pdf 走文档读取；其余按纯文本（二进制不支持）。 */
  async preview(workspaceId: string, folderId: string, rel: string): Promise<DocumentPreview> {
    const { root, folder } = this.folderRoot(workspaceId, folderId);
    const abs = resolveExisting([root], rel).abs;
    const st = statSync(abs);
    const base: DocumentPreview = {
      kind: docKind(abs) || "text",
      path: relative(root, abs),
      folderId: folder.id,
      name: basename(abs),
      size: st.size,
      modified: st.mtimeMs,
      supported: false,
      note: "",
      text: "",
    };
    if (docKind(abs)) {
      try {
        const doc = await docReadFile(abs);
        let text = doc.text;
        if (doc.truncated) text += "\n…（文本已截断）";
        return { ...base, supported: true, note: doc.note, text };
      } catch (err) {
        return { ...base, note: err instanceof Error ? err.message : String(err) };
      }
    }
    const raw = readFileSync(abs);
    if (isBinary(raw)) {
      return { ...base, note: "二进制文件，无法预览。" };
    }
    const text = raw.toString("utf8");
    return {
      ...base,
      supported: true,
      text: Buffer.byteLength(text, "utf8") > PREVIEW_TEXT_LIMIT ? `${Buffer.from(text, "utf8").subarray(0, PREVIEW_TEXT_LIMIT).toString("utf8")}…` : text,
    };
  }

  // ---------- 内部 ----------

  private folderRoot(workspaceId: string, folderId: string): { ws: WorkspaceWithFolders; root: string; folder: WorkspaceWithFolders["folders"][number] } {
    const ws = getWorkspace(this.db, workspaceId);
    if (ws.folders.length === 0) throw new Error("工作区没有可用目录");
    if (!folderId) return { ws, root: ws.folders[0]!.path, folder: ws.folders[0]! };
    const folder = ws.folders.find((f) => f.id === folderId);
    if (!folder) throw new Error("源文件夹不存在或已被移除");
    return { ws, root: folder.path, folder };
  }

  private isConversationPath(abs: string): boolean {
    for (const ws of listWorkspaces(this.db)) {
      if (ws.purpose === "conversation" && (ws.path === abs || ws.folders.some((f) => f.path === abs))) return true;
    }
    return false;
  }

  private publishChanged(workspaceId: string, extra: Record<string, unknown>): void {
    this.recorder.emit(EventType.WorkspaceUpdated, undefined, undefined, { workspaceId, ...extra });
  }
}

/** 一次性引导：确保内置临时会话目录存在（被删自动重建），幂等。 */
export function ensureConversationWorkspace(db: DB, dataDir: string): WorkspaceWithFolders {
  const dir = join(dataDir, "workspace", "default");
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const abs = realpathSync.native(dir);
  const existing = listWorkspaces(db).find((w) => w.path === abs);
  if (existing) {
    if (existing.purpose !== "conversation") {
      updateWorkspacePurpose(db, existing.id, "conversation");
      updateWorkspaceTrust(db, existing.id, "trusted");
    }
    return getWorkspace(db, existing.id);
  }
  const ws = upsertWorkspace(db, { name: "临时会话", path: abs, trust: "trusted", purpose: "conversation" });
  return getWorkspace(db, ws.id);
}

/** 按扩展名映射高亮语言。 */
function languageOf(path: string): string {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  const map: Record<string, string> = {
    go: "go", ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript",
    json: "json", md: "markdown", css: "css", html: "html", py: "python", rs: "rust",
    java: "java", yaml: "yaml", yml: "yaml", toml: "toml", sh: "bash", sql: "sql",
  };
  return map[ext] ?? "text";
}
