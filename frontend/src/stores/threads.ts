import { create } from "zustand";
import { api } from "../lib/api";
import { onBatch } from "../lib/bus";
import type {
  ApprovalPartData,
  AttachmentInfo,
  Batch,
  ChatMessage,
  ChatPart,
  Mode,
  ThreadSnapshot,
  ThreadView,
} from "../lib/types";
import { uid } from "../lib/utils";
import { reduceThread, updateThread } from "./threadReducer";
import { useAppStore } from "./app";
import { useUiStore } from "./ui";

interface ThreadState {
  threads: Record<Mode, ThreadView[]>;
  activeID: Record<Mode, string | null>;
  messages: ChatMessage[];
  running: boolean;
  runningMessageID: string | null;
  loading: boolean;
  pendingApprovals: ApprovalPartData[];
  searchResults: ThreadView[] | null;
  attachments: AttachmentInfo[];
  connected: boolean;
  hasEarlier: boolean;
  loadingEarlier: boolean;
  connect(): void;
  boot(): Promise<void>;
  refresh(mode?: Mode): Promise<void>;
  select(threadID: string): Promise<void>;
  loadEarlier(): Promise<void>;
  create(): Promise<void>;
  duplicate(threadID: string): Promise<void>;
  rename(threadID: string, title: string): Promise<void>;
  archive(threadID: string, archived: boolean): Promise<void>;
  remove(threadID: string): Promise<void>;
  setModel(threadID: string, modelID: string): Promise<void>;
  send(text: string, attachments?: string[]): Promise<void>;
  cancel(): Promise<void>;
  regenerate(): Promise<void>;
  decide(approvalID: string, allow: boolean, scope: string): Promise<void>;
  search(query: string): Promise<void>;
  clearSearch(): void;
  addAttachments(paths: string[]): void;
  removeAttachment(path: string): void;
  clearAttachments(): void;
  applyBatch(batch: Batch): void;
}

function normalizeMessages(threadID: string, snapshot: ThreadSnapshot): ChatMessage[] {
  return (snapshot.messages ?? []).map((message) => ({
    id: message.id,
    threadId: threadID,
    role: message.role,
    status: (message.status as ChatMessage["status"]) ?? "complete",
    createdAt: message.createdAt,
    parts: (message.parts ?? []).map((part) => ({
      id: part.id,
      kind: part.kind as ChatPart["kind"],
      seq: part.seq,
      data: part.data,
    })),
  }));
}

function collectApprovals(messages: ChatMessage[]): ApprovalPartData[] {
  const result: ApprovalPartData[] = [];
  for (const message of messages) {
    for (const part of message.parts) {
      if (part.kind === "approval" && part.data?.status === "waiting") {
        result.push(part.data as ApprovalPartData);
      }
    }
  }
  return result;
}

export const useThreadStore = create<ThreadState>((set, get) => ({
  threads: { code: [], work: [] },
  activeID: { code: null, work: null },
  messages: [],
  running: false,
  runningMessageID: null,
  loading: false,
  pendingApprovals: [],
  searchResults: null,
  attachments: [],
  connected: false,
  hasEarlier: false,
  loadingEarlier: false,

  connect() {
    if (get().connected) {
      return;
    }
    onBatch((batch) => get().applyBatch(batch));
    set({ connected: true });
  },

  async boot() {
    const mode = useAppStore.getState().mode;
    await get().refresh(mode);
    const list = get().threads[mode];
    const current = get().activeID[mode];
    const target = list.find((thread) => thread.id === current) ?? list[0];
    if (target) {
      await get().select(target.id);
    } else {
      set((state) => ({
        messages: [],
        activeID: { ...state.activeID, [mode]: null },
        running: false,
        runningMessageID: null,
        pendingApprovals: [],
      }));
    }
  },

  async refresh(mode) {
    const target = mode ?? useAppStore.getState().mode;
    try {
      const list = await api.threads.list(target, false);
      set((state) => ({ threads: { ...state.threads, [target]: list } }));
    } catch (error) {
      console.error("读取线程列表失败", error);
    }
  },

  async select(threadID) {
    const mode = useAppStore.getState().mode;
    set((state) => ({
      activeID: { ...state.activeID, [mode]: threadID },
      loading: true,
      messages: [],
      pendingApprovals: [],
    }));
    try {
      const snapshot = await api.threads.snapshotPage(threadID, 80, 0);
      const messages = normalizeMessages(threadID, snapshot);
      const lastStreaming = [...messages].reverse().find((message) => message.status === "streaming");
      set({
        messages,
        running: snapshot.thread.status === "running" || snapshot.thread.status === "awaiting",
        runningMessageID: lastStreaming?.id ?? null,
        pendingApprovals: collectApprovals(messages),
        loading: false,
        hasEarlier: snapshot.hasMore ?? false,
        loadingEarlier: false,
        threads: updateThread(get().threads, threadID, {
          status: snapshot.thread.status as ThreadView["status"],
        }),
      });
    } catch (error) {
      console.error("读取线程快照失败", error);
      set({ loading: false });
    }
  },

  async loadEarlier() {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    if (!threadID || !get().hasEarlier || get().loadingEarlier) {
      return;
    }
    const firstMessage = get().messages[0];
    const firstSeq = firstMessage?.parts.reduce(
      (smallest, part) => (part.seq > 0 && part.seq < smallest ? part.seq : smallest),
      Number.MAX_SAFE_INTEGER,
    );
    if (!firstSeq || firstSeq === Number.MAX_SAFE_INTEGER) {
      set({ hasEarlier: false });
      return;
    }
    set({ loadingEarlier: true });
    try {
      const snapshot = await api.threads.snapshotPage(threadID, 80, firstSeq);
      const older = normalizeMessages(threadID, snapshot);
      const current = get().messages;
      const existing = new Set(current.map((message) => message.id));
      const prepended = older.filter((message) => !existing.has(message.id));
      const merged = [...prepended, ...current];
      set({
        messages: merged,
        hasEarlier: snapshot.hasMore ?? false,
        loadingEarlier: false,
        pendingApprovals: collectApprovals(merged),
      });
    } catch (error) {
      set({ loadingEarlier: false });
      useUiStore.getState().toast({
        title: "加载更早消息失败",
        description: String(error),
        tone: "danger",
      });
    }
  },

  async create() {
    const app = useAppStore.getState();
    try {
      const thread = await api.threads.create(app.mode, app.activeWorkspace?.id ?? "", "");
      set((state) => ({
        threads: { ...state.threads, [app.mode]: [thread, ...state.threads[app.mode]] },
        activeID: { ...state.activeID, [app.mode]: thread.id },
        messages: [],
        pendingApprovals: [],
        running: false,
        runningMessageID: null,
      }));
    } catch (error) {
      console.error("新建线程失败", error);
    }
  },

  async duplicate(threadID) {
    const mode = useAppStore.getState().mode;
    const source = get().threads[mode].find((thread) => thread.id === threadID);
    if (!source) {
      return;
    }
    const app = useAppStore.getState();
    const thread = await api.threads.create(
      mode,
      app.activeWorkspace?.id ?? source.workspaceId,
      `${source.title}（副本）`,
    );
    await get().refresh(mode);
    await get().select(thread.id);
  },

  async rename(threadID, title) {
    await api.threads.rename(threadID, title);
    set((state) => ({ threads: updateThread(state.threads, threadID, { title }) }));
  },

  async archive(threadID, archived) {
    await api.threads.archive(threadID, archived);
    const mode = useAppStore.getState().mode;
    await get().refresh(mode);
    if (archived && get().activeID[mode] === threadID) {
      const next = get().threads[mode][0];
      if (next) {
        await get().select(next.id);
      } else {
        set({ messages: [], activeID: { ...get().activeID, [mode]: null } });
      }
    }
  },

  async remove(threadID) {
    await api.threads.remove(threadID);
    const mode = useAppStore.getState().mode;
    await get().refresh(mode);
    if (get().activeID[mode] === threadID) {
      const next = get().threads[mode][0];
      if (next) {
        await get().select(next.id);
      } else {
        set({ messages: [], activeID: { ...get().activeID, [mode]: null } });
      }
    }
  },

  async setModel(threadID, modelID) {
    await api.threads.setModel(threadID, modelID);
    set((state) => ({ threads: updateThread(state.threads, threadID, { modelId: modelID }) }));
  },

  async send(text, attachments = []) {
    const app = useAppStore.getState();
    const mode = app.mode;
    let threadID = get().activeID[mode];
    if (!threadID) {
      await get().create();
      threadID = get().activeID[mode];
    }
    if (!threadID) {
      return;
    }
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) {
      return;
    }

    const userMessage: ChatMessage = {
      id: uid("msg"),
      threadId: threadID,
      role: "user",
      status: "complete",
      createdAt: Date.now(),
      parts: [
        {
          id: uid("part"),
          kind: "text",
          seq: 0,
          data: { text: trimmed, attachments },
        },
      ],
    };
    set((state) => ({ messages: [...state.messages, userMessage], running: true }));
    try {
      await api.chat.send(threadID, trimmed, attachments);
      get().clearAttachments();
    } catch (error) {
      set({ running: false });
      useUiStore.getState().toast({
        title: "发送失败",
        description: String(error),
        tone: "danger",
      });
    }
  },

  async cancel() {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    if (!threadID) {
      return;
    }
    await api.chat.cancel(threadID);
    set({ running: false });
  },

  async regenerate() {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    if (!threadID) {
      return;
    }
    set({ running: true });
    try {
      await api.chat.regenerate(threadID);
    } catch (error) {
      set({ running: false });
      useUiStore.getState().toast({ title: "重新生成失败", description: String(error), tone: "danger" });
    }
  },

  async decide(approvalID, allow, scope) {
    const previous = get().pendingApprovals;
    const nextStatus = allow ? "approved" : "denied";
    set((state) => ({
      pendingApprovals: state.pendingApprovals.filter((item) => item.approvalId !== approvalID),
      messages: state.messages.map((message) => ({
        ...message,
        parts: message.parts.map((part) =>
          part.kind === "approval" && part.data?.approvalId === approvalID
            ? { ...part, data: { ...part.data, status: nextStatus, decision: allow ? "allow" : "deny", scope } }
            : part,
        ),
      })),
    }));
    try {
      await api.chat.decideApproval(approvalID, allow, scope);
    } catch (error) {
      set({ pendingApprovals: previous });
      useUiStore.getState().toast({ title: "审批提交失败", description: String(error), tone: "danger" });
    }
  },

  async search(query) {
    const mode = useAppStore.getState().mode;
    if (!query.trim()) {
      set({ searchResults: null });
      return;
    }
    const results = await api.threads.search(mode, query);
    set({ searchResults: results });
  },

  clearSearch() {
    set({ searchResults: null });
  },

  addAttachments(paths) {
    const existing = new Set(get().attachments.map((item) => item.path));
    const next = paths
      .filter((path) => path && !existing.has(path))
      .map((path) => ({ path, name: path.split("/").pop() ?? path }));
    set((state) => ({ attachments: [...state.attachments, ...next].slice(0, 20) }));
  },

  removeAttachment(path) {
    set((state) => ({ attachments: state.attachments.filter((item) => item.path !== path) }));
  },

  clearAttachments() {
    set({ attachments: [] });
  },

  applyBatch(batch) {
    const state = get();
    const mode = useAppStore.getState().mode;
    const result = reduceThread(
      {
        messages: state.messages,
        running: state.running,
        runningMessageID: state.runningMessageID,
        pendingApprovals: state.pendingApprovals,
        threads: state.threads,
      },
      batch,
      state.activeID[mode] ?? "",
    );
    set({
      messages: result.messages,
      running: result.running,
      runningMessageID: result.runningMessageID,
      pendingApprovals: result.pendingApprovals,
      threads: result.threads,
    });
    if (result.refreshNeeded) {
      void get().refresh(mode);
    }
  },
}));
