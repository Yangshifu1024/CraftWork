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

export type Mode = "code" | "work";

/** 消息 + 部件的运行时可变视图（快照结构 + 流式增量）。 */
export interface ChatState {
  messages: MessageSnapshot[];
  running: boolean;
  lastSeq: number;
}

interface AppState {
  booted: boolean;
  mode: Mode;
  threads: Record<Mode, ThreadView[]>;
  activeThreadId: Record<Mode, string>;
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
  createThread(workspaceId?: string): Promise<void>;
  deleteThread(id: string): Promise<void>;
  renameThread(id: string, title: string): Promise<void>;
  applyBatch(batch: Batch): void;
  saveSettings(next: Partial<Settings>): Promise<void>;
  toggleSidebar(): void;
}

function emptyChat(): ChatState {
  return { messages: [], running: false, lastSeq: 0 };
}

export const useApp = create<AppState>((set, get) => ({
  booted: false,
  mode: "code",
  threads: { code: [], work: [] },
  activeThreadId: { code: "", work: "" },
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
    terminalFontSize: 12,
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
    set({
      booted: true,
      settings,
      mode: settings.mode === "work" ? "work" : "code",
      threads: { code: threadsCode, work: threadsWork },
      workspaces,
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

  async createThread(workspaceId) {
    const mode = get().mode;
    const thread = await api.threadCreate(mode, workspaceId ?? "", "");
    await get().refreshThreads(mode);
    await get().selectThread(thread.id);
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
      set({ settings: data as unknown as Settings });
      break;
    }
    case "workspace.updated": {
      void get().refreshWorkspaces();
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
