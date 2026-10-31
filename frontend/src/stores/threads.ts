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
import { EVENT } from "../lib/types";
import { uid } from "../lib/utils";
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

function ensureMessage(messages: ChatMessage[], messageID: string, threadID: string): ChatMessage[] {
  if (messages.some((message) => message.id === messageID)) {
    return messages.map((message) =>
      message.id === messageID ? { ...message, status: "streaming" } : message,
    );
  }
  return [
    ...messages,
    {
      id: messageID,
      threadId: threadID,
      role: "assistant",
      status: "streaming",
      createdAt: Date.now(),
      parts: [],
    },
  ];
}

function appendText(
  messages: ChatMessage[],
  messageID: string,
  partID: string,
  kind: "text" | "reasoning",
  text: string,
): ChatMessage[] {
  return messages.map((message) => {
    if (message.id !== messageID) {
      return message;
    }
    const index = message.parts.findIndex((part) => part.id === partID);
    if (index < 0) {
      return {
        ...message,
        parts: [...message.parts, { id: partID, kind, seq: message.parts.length, data: { text } }],
      };
    }
    const parts = message.parts.map((part, partIndex) => {
      if (partIndex !== index) {
        return part;
      }
      return { ...part, data: { ...part.data, text: `${part.data?.text ?? ""}${text}` } };
    });
    return { ...message, parts };
  });
}

function upsertPart(
  messages: ChatMessage[],
  messageID: string,
  threadID: string,
  part: ChatPart,
): ChatMessage[] {
  let found = false;
  const next = messages.map((message) => {
    if (message.id !== messageID) {
      return message;
    }
    found = true;
    const index = message.parts.findIndex((item) => item.id === part.id);
    if (index < 0) {
      return { ...message, parts: [...message.parts, part] };
    }
    const parts = message.parts.map((item, itemIndex) =>
      itemIndex === index ? { ...item, ...part } : item,
    );
    return { ...message, parts };
  });
  if (found) {
    return next;
  }
  return [
    ...next,
    {
      id: messageID,
      threadId: threadID,
      role: "assistant",
      status: "streaming",
      createdAt: Date.now(),
      parts: [part],
    },
  ];
}

function setMessageStatus(
  messages: ChatMessage[],
  messageID: string | null,
  status: ChatMessage["status"],
): ChatMessage[] {
  if (!messageID) {
    return messages;
  }
  return messages.map((message) => (message.id === messageID ? { ...message, status } : message));
}

function updateApprovalPart(
  messages: ChatMessage[],
  partID: string,
  data: Partial<ApprovalPartData>,
): ChatMessage[] {
  return messages.map((message) => ({
    ...message,
    parts: message.parts.map((part) =>
      part.id === partID && part.kind === "approval"
        ? { ...part, data: { ...part.data, ...data } }
        : part,
    ),
  }));
}

function updateThread(
  threads: Record<Mode, ThreadView[]>,
  threadID: string,
  patch: Partial<ThreadView>,
): Record<Mode, ThreadView[]> {
  return {
    code: threads.code.map((thread) => (thread.id === threadID ? { ...thread, ...patch } : thread)),
    work: threads.work.map((thread) => (thread.id === threadID ? { ...thread, ...patch } : thread)),
  };
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
    const activeThreadID = state.activeID[mode];
    let messages = state.messages;
    let running = state.running;
    let runningMessageID = state.runningMessageID;
    let pending = state.pendingApprovals;
    let threads = state.threads;
    let refreshNeeded = false;

    for (const event of batch.events) {
      if (event.threadId?.startsWith("terminal:")) {
        continue;
      }
      if (activeThreadID && event.threadId && event.threadId !== activeThreadID) {
        if (event.type === EVENT.threadUpdated) {
          refreshNeeded = true;
        }
        continue;
      }

      switch (event.type) {
        case EVENT.runStarted: {
          const messageID = event.data?.messageId as string | undefined;
          if (!messageID) {
            break;
          }
          runningMessageID = messageID;
          running = true;
          messages = ensureMessage(messages, messageID, activeThreadID ?? "");
          if (activeThreadID) {
            threads = updateThread(threads, activeThreadID, { status: "running" });
          }
          break;
        }
        case EVENT.textDelta: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = appendText(messages, runningMessageID, event.data.partId, "text", event.data.text ?? "");
          break;
        }
        case EVENT.textCompleted: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "text",
            seq: 0,
            data: { text: event.data.text ?? "" },
          });
          break;
        }
        case EVENT.reasoningDelta: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = appendText(messages, runningMessageID, event.data.partId, "reasoning", event.data.text ?? "");
          break;
        }
        case EVENT.reasoningCompleted: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "reasoning",
            seq: 0,
            data: { text: event.data.text ?? "" },
          });
          break;
        }
        case EVENT.toolStarted:
        case EVENT.toolCompleted: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "tool",
            seq: 0,
            data: event.data,
          });
          break;
        }
        case EVENT.approvalRequested: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "approval",
            seq: 0,
            data: event.data,
          });
          pending = [
            ...pending.filter((item) => item.approvalId !== event.data.approvalId),
            event.data as ApprovalPartData,
          ];
          if (activeThreadID) {
            threads = updateThread(threads, activeThreadID, { status: "awaiting" });
          }
          break;
        }
        case EVENT.approvalResolved: {
          if (!event.data?.partId) {
            break;
          }
          messages = updateApprovalPart(messages, event.data.partId, {
            status: event.data.status,
            decision: event.data.decision,
            scope: event.data.scope,
          });
          pending = pending.filter((item) => item.approvalId !== event.data.approvalId);
          if (activeThreadID) {
            threads = updateThread(threads, activeThreadID, { status: "running" });
          }
          break;
        }
        case EVENT.todoUpdated: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "todo",
            seq: 0,
            data: event.data,
          });
          break;
        }
        case EVENT.error: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "error",
            seq: 0,
            data: event.data,
          });
          break;
        }
        case EVENT.notice: {
          if (!runningMessageID || !event.data?.partId) {
            break;
          }
          messages = upsertPart(messages, runningMessageID, activeThreadID ?? "", {
            id: event.data.partId,
            kind: "notice",
            seq: 0,
            data: event.data,
          });
          break;
        }
        case EVENT.runCompleted: {
          messages = setMessageStatus(messages, runningMessageID, "complete");
          running = false;
          runningMessageID = null;
          if (activeThreadID) {
            threads = updateThread(threads, activeThreadID, { status: "idle" });
          }
          refreshNeeded = true;
          break;
        }
        case EVENT.runCanceled: {
          messages = setMessageStatus(messages, runningMessageID, "incomplete");
          running = false;
          runningMessageID = null;
          if (activeThreadID) {
            threads = updateThread(threads, activeThreadID, { status: "idle" });
          }
          refreshNeeded = true;
          break;
        }
        case EVENT.runFailed: {
          messages = setMessageStatus(messages, runningMessageID, "error");
          running = false;
          runningMessageID = null;
          if (activeThreadID) {
            threads = updateThread(threads, activeThreadID, { status: "error" });
          }
          refreshNeeded = true;
          break;
        }
        case EVENT.threadUpdated: {
          if (event.data?.thread) {
            threads = updateThread(threads, event.data.thread.id, event.data.thread);
          } else if (activeThreadID && event.data?.status) {
            threads = updateThread(threads, activeThreadID, {
              status: event.data.status as ThreadView["status"],
            });
          }
          break;
        }
        default:
          break;
      }
    }

    set({ messages, running, runningMessageID, pendingApprovals: pending, threads });
    if (refreshNeeded) {
      void get().refresh(mode);
    }
  },
}));
