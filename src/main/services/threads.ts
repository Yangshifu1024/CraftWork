// 会话服务：列表/创建/改名/删除/快照（参数校验与编排，业务在 store 与 agent）。
import type { Snapshot, ThreadView } from "../../shared/types";
import { EventType } from "../../shared/events";
import { newId, IdPrefix } from "../../shared/ids";
import type { Recorder } from "../core/record";
import type { DB } from "../store/db";
import {
  createThread,
  deleteThread as dbDeleteThread,
  getThread,
  listThreads,
  renameThread,
  searchThreads,
  setThreadArchived,
  setThreadModel,
  type ThreadRow,
} from "../store/threads";
import { snapshot as buildSnapshot } from "../store/events";
import { defaultConversationWorkspaceId, getWorkspace } from "../store/workspaces";
import { toThreadView } from "../store/view";
import type { Catalog } from "../model/catalog";
import type { SettingsService } from "./settings";

const TITLE_MAX_RUNES = 80;

export function normalizeMode(mode: string): string {
  return mode === "work" ? "work" : "code";
}

export class ThreadService {
  constructor(
    private readonly db: DB,
    private readonly recorder: Recorder,
    private readonly catalog: Catalog,
    private readonly settings: SettingsService,
  ) {}

  list(mode: string, includeArchived: boolean): ThreadView[] {
    return listThreads(this.db, normalizeMode(mode), includeArchived).map((row) => toThreadView(this.db, row));
  }

  search(mode: string, query: string): ThreadView[] {
    const q = query.trim();
    if (!q) return this.list(mode, false);
    return searchThreads(this.db, normalizeMode(mode), q).map((row) => toThreadView(this.db, row));
  }

  create(mode: string, workspaceId: string, title: string): ThreadView {
    const normalizedMode = normalizeMode(mode);
    if (workspaceId) {
      try {
        getWorkspace(this.db, workspaceId);
      } catch {
        throw new Error("工作区不存在，请重新选择工作区后再新建会话");
      }
    }
    // 模型取会话指定 → 全局默认；不可解析则留空（发送前有明确引导）
    const threadModel = this.settings.get().defaultModel;
    const trimmed = title.trim();
    const auto = trimmed === "";
    const row: Omit<ThreadRow, "createdAt" | "updatedAt" | "tokens"> = {
      id: newId(IdPrefix.Thread),
      mode: normalizedMode,
      title: auto ? "新对话" : [...trimmed].slice(0, TITLE_MAX_RUNES).join(""),
      titleAuto: auto,
      modelId: threadModel,
      workspaceId: workspaceId || defaultConversationWorkspaceId(this.db),
      status: "idle",
      archived: false,
    };
    const created = createThread(this.db, row);
    const view = toThreadView(this.db, created);
    this.publishChanged(view.id);
    return view;
  }

  rename(threadId: string, title: string): void {
    const trimmed = title.trim();
    if (!trimmed) throw new Error("标题不能为空");
    if ([...trimmed].length > TITLE_MAX_RUNES) throw new Error("标题过长（最多 80 字）");
    renameThread(this.db, threadId, trimmed);
    this.publishChanged(threadId);
  }

  setModel(threadId: string, modelId: string): void {
    const normalized = this.catalog.normalize(modelId);
    if (!normalized) throw new Error("当前模型不可用，请重新选择模型");
    setThreadModel(this.db, threadId, normalized.ref);
    this.publishChanged(threadId);
  }

  archive(threadId: string, archived: boolean): void {
    const thread = getThread(this.db, threadId);
    if (archived && (thread.status === "running" || thread.status === "awaiting")) {
      throw new Error("会话正在运行，不能归档");
    }
    setThreadArchived(this.db, threadId, archived);
    this.publishChanged(threadId);
  }

  delete(threadId: string): void {
    const thread = getThread(this.db, threadId);
    if (thread.status === "running" || thread.status === "awaiting") {
      throw new Error("会话正在运行，请先取消");
    }
    dbDeleteThread(this.db, threadId);
    this.recorder.emit(EventType.ThreadUpdated, threadId, undefined, { threadId, deleted: true, mode: thread.mode });
  }

  snapshot(threadId: string): Snapshot {
    return buildSnapshot(this.db, threadId);
  }

  snapshotPage(threadId: string, limit: number, beforeSeq: number): Snapshot {
    const n = limit <= 0 ? 80 : Math.min(500, limit);
    return buildSnapshot(this.db, threadId, n, beforeSeq > 0 ? beforeSeq : 0);
  }

  /** 重读并广播 thread.updated（已删除则静默）。 */
  private publishChanged(threadId: string): void {
    try {
      const row = getThread(this.db, threadId);
      const view = toThreadView(this.db, row);
      this.recorder.emit(EventType.ThreadUpdated, threadId, undefined, { threadId, thread: view });
    } catch {
      // 已删除：不广播
    }
  }
}
