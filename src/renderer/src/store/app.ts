// 全局状态：会话、消息、设置与界面偏好。
// 事件驱动的会话状态转移都在这里完成（渲染层唯一状态壳）。
import { create } from "zustand";
import type { Batch, Envelope } from "../../../shared/events";
import type {
  MessageSnapshot,
  PartSnapshot,
  Settings,
  ThreadView,
  WorkspaceView,
} from "../../../shared/types";
import type { ApprovalPart } from "../../../shared/events";
import { api } from "../lib/api";
import { applyTheme, saveThemePref, watchSystemTheme } from "../lib/theme";
import { emptyDraftTable, loadDrafts, saveDrafts } from "../lib/drafts";
import type { DraftTable } from "../lib/drafts";

export type Mode = "code" | "work";

/** 消息 + 部件的运行时可变视图（快照结构 + 流式增量）。 */
export interface ChatState {
  messages: MessageSnapshot[];
  running: boolean;
  lastSeq: number;
}

/** 草稿槽：活动会话为空（草稿态）时输入区的未发送内容；modelRef 为草稿态选定的模型引用（空 = 用全局默认）。 */
export interface DraftState {
  text: string;
  attachments: string[];
  modelRef: string;
}

interface AppState {
  booted: boolean;
  mode: Mode;
  threads: Record<Mode, ThreadView[]>;
  activeThreadId: Record<Mode, string>;
  /** 草稿槽按归属分桶：key 为工作区 id，空串 "" 为内置临时会话槽；持久化经 lib/drafts。 */
  drafts: Record<Mode, Record<string, DraftState>>;
  /** 各模式当前草稿槽归属（空串 = 临时槽）；活动会话非空时不起作用。 */
  draftWsId: Record<Mode, string>;
  workspaces: WorkspaceView[];
  settings: Settings;
  chats: Record<string, ChatState>;
  sidebarVisible: boolean;
  themeResolved: "theme-zai-light" | "theme-zai-dark";

  boot(): Promise<void>;
  setMode(mode: Mode): void;
  refreshThreads(mode?: Mode): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  selectThread(id: string): Promise<void>;
  /** 进入当前模式的草稿态：活动会话置空并切到 workspaceId 槽（缺省临时槽），不创建线程、不覆盖槽内容。 */
  newDraft(workspaceId?: string): void;
  /** 写当前草稿槽（按 mode + draftWsId 归属），并同步落本地存储。 */
  setDraft(mode: Mode, patch: Partial<DraftState>): void;
  deleteThread(id: string): Promise<void>;
  renameThread(id: string, title: string): Promise<void>;
  applyBatch(batch: Batch): void;
  saveSettings(next: Partial<Settings>): Promise<void>;
  toggleSidebar(): void;
}

function emptyChat(): ChatState {
  return { messages: [], running: false, lastSeq: 0 };
}

/** 稳定空草稿槽：槽尚无内容时的回落（引用恒定，避免 zustand selector 每次返回新对象触发重渲染）。 */
export const EMPTY_DRAFT: DraftState = { text: "", attachments: [], modelRef: "" };

/** 当前模式的草稿槽（draftWsId 指向归属）；槽尚无内容时为 undefined。 */
export function draftSlot(state: AppState, mode: Mode): DraftState | undefined {
  return state.drafts[mode][state.draftWsId[mode]];
}

/** 草稿态生效模型引用：槽选定引用优先，回落全局默认。 */
export function effectiveModelRef(state: AppState, mode: Mode): string {
  return draftSlot(state, mode)?.modelRef || state.settings.defaultModel;
}

export const useApp = create<AppState>((set, get) => ({
  booted: false,
  mode: "code",
  threads: { code: [], work: [] },
  activeThreadId: { code: "", work: "" },
  drafts: emptyDraftTable(),
  draftWsId: { code: "", work: "" },
  workspaces: [],
  settings: {
    theme: "system",
    mode: "code",
    panelWidth: 320,
    approvalLevel: "graded",
    defaultModel: "",
    fallbackModel: "",
    activeWorkspace: "",
    language: "zh-CN",
    fontSize: 15,
    codeTheme: "dark",
    autoUpdate: true,
    notifications: true,
    updateChannel: "stable",
    telemetry: false,
    maxIterations: 300,
  },
  chats: {},
  sidebarVisible: true,
  themeResolved: "theme-zai-dark",

  async boot() {
    const [settings, threadsCode, threadsWork, workspaces] = await Promise.all([
      api.settingsGet(),
      api.threadList("code", false),
      api.threadList("work", false),
      api.workspaceList(),
    ]);
    // 主题以 settings 为事实源：校准启动缓存；system 偏好下跟随系统亮暗变化。
    saveThemePref(settings.theme);
    const themeResolved = applyTheme(settings.theme);
    watchSystemTheme(
      () => useApp.getState().settings.theme,
      (resolved) => useApp.setState({ themeResolved: resolved }),
    );
    set({
      booted: true,
      settings,
      themeResolved,
      mode: settings.mode === "work" ? "work" : "code",
      threads: { code: threadsCode, work: threadsWork },
      workspaces,
      // 启动恢复各槽草稿文本与附件；槽归属不恢复（恒为临时槽），
      // activeThreadId 选择逻辑不动：有会话选中最近会话，即「恢复草稿但不自动进入草稿态」。
      drafts: loadDrafts(),
      activeThreadId: {
        code: threadsCode[0]?.id ?? "",
        work: threadsWork[0]?.id ?? "",
      },
    });
    const active = get().activeThreadId[get().mode];
    if (active) await get().selectThread(active);
  },

  setMode(mode) {
    set({ mode });
    const active = get().activeThreadId[mode];
    if (active) void get().selectThread(active);
    void api.settingsSave({ ...get().settings, mode }).then((s) => set({ settings: s }));
  },

  async refreshThreads(mode) {
    const m = mode ?? get().mode;
    const list = await api.threadList(m, false);
    set((state) => ({ threads: { ...state.threads, [m]: list } }));
  },

  async refreshWorkspaces() {
    const workspaces = await api.workspaceList();
    set({ workspaces });
  },

  async selectThread(id) {
    const mode = get().mode;
    set((state) => ({
      activeThreadId: { ...state.activeThreadId, [mode]: id },
      chats: state.chats[id] ? state.chats : { ...state.chats, [id]: state.chats[id] ?? emptyChat() },
    }));
    if (!get().chats[id] || get().chats[id]!.messages.length === 0) {
      const snap = await api.threadSnapshot(id);
      set((state) => ({
        chats: {
          ...state.chats,
          [id]: {
            messages: snap.messages,
            running: snap.thread.status === "running" || snap.thread.status === "awaiting",
            lastSeq: snap.lastSeq,
          },
        },
      }));
    }
  },

  newDraft(workspaceId = "") {
    const mode = get().mode;
    // 草稿态：活动会话置空并切到归属槽（缺省临时槽），不调线程创建（草稿是纯渲染层状态）；
    // 槽已有内容原样保留（重复 ⌘N 回到同一条草稿）。
    set((state) => ({
      activeThreadId: { ...state.activeThreadId, [mode]: "" },
      draftWsId: { ...state.draftWsId, [mode]: workspaceId },
    }));
  },

  setDraft(mode, patch) {
    const wsId = get().draftWsId[mode];
    const prev = get().drafts;
    const slot = prev[mode][wsId] ?? EMPTY_DRAFT;
    const drafts = { ...prev, [mode]: { ...prev[mode], [wsId]: { ...slot, ...patch } } };
    saveDrafts(drafts);
    set({ drafts });
  },

  async deleteThread(id) {
    await api.threadDelete(id);
    const mode = get().mode;
    await get().refreshThreads(mode);
    if (get().activeThreadId[mode] === id) {
      const next = get().threads[mode][0]?.id ?? "";
      set((state) => ({ activeThreadId: { ...state.activeThreadId, [mode]: next } }));
      if (next) await get().selectThread(next);
    }
  },

  async renameThread(id, title) {
    await api.threadRename(id, title);
    await get().refreshThreads();
  },

  /** 事件批量应用：唯一的事件入口（按 type 分发到消息/会话状态）。 */
  applyBatch(batch) {
    for (const event of batch.events) {
      applyEvent(set, get, event);
    }
  },

  async saveSettings(next) {
    const saved = await api.settingsSave({ ...get().settings, ...next });
    set({ settings: saved });
  },

  toggleSidebar() {
    set((state) => ({ sidebarVisible: !state.sidebarVisible }));
  },
}));

// ---------- 事件应用 ----------

type SetFn = (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void;
type GetFn = () => AppState;

function upsertPart(
  chat: ChatState,
  messageId: string,
  partId: string,
  kind: PartSnapshot["kind"],
  payload: Record<string, unknown>,
): MessageSnapshot[] {
  const messages = [...chat.messages];
  let target = messages.find((m) => m.id === messageId);
  if (!target) {
    target = { id: messageId, role: "assistant", parts: [], status: "streaming", seq: 0 };
    messages.push(target);
  }
  const parts = [...target.parts];
  const idx = parts.findIndex((p) => p.id === partId);
  const part: PartSnapshot = { id: partId, kind, seq: 0, payload };
  if (idx >= 0) parts[idx] = { ...parts[idx]!, payload: { ...parts[idx]!.payload, ...payload } };
  else parts.push(part);
  target = { ...target, parts };
  const msgIdx = messages.findIndex((m) => m.id === messageId);
  messages[msgIdx] = target;
  return messages;
}

function findMessageIdForPart(chat: ChatState, partId: string): string | null {
  for (const m of chat.messages) {
    if (m.parts.some((p) => p.id === partId)) return m.id;
  }
  return null;
}

function applyEvent(set: SetFn, get: GetFn, event: Envelope): void {
  // workspace.updated 与会话无关（threadId 为空）：草稿态（无活动会话）下也会到达，
  // 必须在 threadId 短路之前处理，否则删项目清槽事件会被丢弃。
  if (event.type === "workspace.updated") {
    const payload = event.data as Record<string, unknown>;
    const removedWsId = payload["removed"] === true && typeof payload["workspaceId"] === "string" ? payload["workspaceId"] : "";
    if (removedWsId) {
      // 删除项目：清掉该工作区在两个模式下的草稿槽（无残留），并同步本地存储。
      set((state) => {
        const drafts: DraftTable = { code: { ...state.drafts.code }, work: { ...state.drafts.work } };
        delete drafts.code[removedWsId];
        delete drafts.work[removedWsId];
        saveDrafts(drafts);
        return { drafts };
      });
    }
    void get().refreshWorkspaces();
    return;
  }
  const state = get();
  const threadId = event.threadId ?? state.activeThreadId[state.mode];
  if (!threadId) return;
  const chat = state.chats[threadId] ?? emptyChat();
  const data = event.data as Record<string, unknown>;

  switch (event.type) {
    case "run.started": {
      const messageId = String(data["messageId"] ?? "");
      setChat(set, threadId, {
        ...chat,
        running: true,
        messages: chat.messages.some((m) => m.id === messageId)
          ? chat.messages
          : [...chat.messages, { id: messageId, role: "assistant", parts: [], status: "streaming", seq: 0 }],
      });
      break;
    }
    case "run.completed":
    case "run.canceled": {
      const messages = chat.messages.map((m) =>
        m.status === "streaming" ? { ...m, status: event.type === "run.completed" ? ("complete" as const) : ("incomplete" as const) } : m,
      );
      setChat(set, threadId, { messages, running: false, lastSeq: chat.lastSeq });
      break;
    }
    case "run.failed": {
      const messages = chat.messages.map((m) => (m.status === "streaming" ? { ...m, status: "error" as const } : m));
      setChat(set, threadId, { messages, running: false, lastSeq: chat.lastSeq });
      break;
    }
    case "text.delta":
    case "reasoning.delta": {
      const partId = String(data["partId"] ?? "");
      let messageId = findMessageIdForPart(chat, partId);
      if (!messageId) {
        // 流式开始：挂到最后一条 streaming 消息（或新建）
        messageId = [...chat.messages].reverse().find((m) => m.status === "streaming")?.id ?? "";
      }
      if (!messageId) break;
      const kind = event.type === "text.delta" ? "text" : "reasoning";
      const messages = upsertAppend(chat, messageId, partId, kind, String(data["text"] ?? ""));
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "text.completed":
    case "reasoning.completed": {
      const partId = String(data["partId"] ?? "");
      const messageId = findMessageIdForPart(chat, partId);
      if (!messageId) break;
      const kind = event.type === "text.completed" ? "text" : "reasoning";
      const messages = upsertPart(chat, messageId, partId, kind, data);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "tool.started": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "tool", data);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "tool.completed": {
      const partId = String(data["partId"] ?? "");
      const messageId = findMessageIdForPart(chat, partId) ?? lastStreamingId(chat);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "tool", data);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "approval.requested": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat) ?? [...chat.messages].reverse()[0]?.id;
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "approval", data);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "approval.resolved": {
      const partId = String(data["partId"] ?? "");
      const messageId = findMessageIdForPart(chat, partId);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "approval", {
        ...approvalPayloadOf(chat, partId),
        status: data["status"],
        decision: data["decision"],
        scope: data["scope"],
      });
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "todo.updated": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "todo", data);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "error": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat) ?? [...chat.messages].reverse()[0]?.id ?? "";
      const messages = upsertPart(chat, messageId, partId, "error", data);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "notice":
    case "context.compressed": {
      const partId = String(data["partId"] ?? event.seq);
      const messageId = lastStreamingId(chat) ?? [...chat.messages].reverse()[0]?.id ?? "";
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "notice", { text: data["text"] ?? data["summary"] ?? "" });
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "user.message": {
      const messageId = String(data["messageId"] ?? "");
      if (chat.messages.some((m) => m.id === messageId)) break;
      const message: MessageSnapshot = {
        id: messageId,
        role: "user",
        parts: [
          {
            id: `part_${messageId}`,
            kind: "text",
            seq: 0,
            payload: { text: data["text"] ?? "", attachments: data["attachments"] ?? [] },
          },
        ],
        status: "complete",
        seq: 0,
      };
      setChat(set, threadId, { ...chat, messages: [...chat.messages, message] });
      break;
    }
    case "thread.updated": {
      if (data["deleted"] === true) {
        void get().refreshThreads();
        break;
      }
      void get().refreshThreads();
      break;
    }
    case "settings.updated": {
      const next = data as unknown as Settings;
      // 主题偏好可能被外部改动：重新应用并同步启动缓存
      if (next.theme !== get().settings.theme) {
        saveThemePref(next.theme);
        set({ themeResolved: applyTheme(next.theme) });
      }
      set({ settings: next });
      break;
    }
    default:
      break;
  }
}

function approvalPayloadOf(chat: ChatState, partId: string): Record<string, unknown> {
  for (const m of chat.messages) {
    const part = m.parts.find((p) => p.id === partId);
    if (part) return part.payload;
  }
  return {} as ApprovalPart as unknown as Record<string, unknown>;
}

function lastStreamingId(chat: ChatState): string {
  return [...chat.messages].reverse().find((m) => m.status === "streaming")?.id ?? "";
}

/** 流式增量追加：找到 part 追加文本，没有则创建。 */
function upsertAppend(
  chat: ChatState,
  messageId: string,
  partId: string,
  kind: PartSnapshot["kind"],
  text: string,
): MessageSnapshot[] {
  const messages = [...chat.messages];
  const msgIdx = messages.findIndex((m) => m.id === messageId);
  if (msgIdx < 0) return messages;
  const target = messages[msgIdx]!;
  const parts = [...target.parts];
  const idx = parts.findIndex((p) => p.id === partId);
  if (idx >= 0) {
    const prev = parts[idx]!;
    parts[idx] = { ...prev, payload: { ...prev.payload, text: String(prev.payload["text"] ?? "") + text } };
  } else {
    parts.push({ id: partId, kind, seq: 0, payload: { text } });
  }
  messages[msgIdx] = { ...target, parts };
  return messages;
}

function setChat(set: SetFn, threadId: string, chat: ChatState): void {
  set((state) => ({ chats: { ...state.chats, [threadId]: chat } }));
}
