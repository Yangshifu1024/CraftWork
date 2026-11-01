import type {
  ApprovalPartData,
  Batch,
  ChatMessage,
  ChatPart,
  Envelope,
  Mode,
  ThreadView,
} from "../lib/types";
import { EVENT } from "../lib/types";

/** 线程事件归约涉及的 store 字段。 */
export interface ThreadSlice {
  messages: ChatMessage[];
  running: boolean;
  runningMessageID: string | null;
  pendingApprovals: ApprovalPartData[];
  threads: Record<Mode, ThreadView[]>;
}

export interface ReducedThread extends ThreadSlice {
  refreshNeeded: boolean;
}

type EventType = (typeof EVENT)[keyof typeof EVENT];

interface Draft extends ThreadSlice {
  activeThreadID: string;
  refreshNeeded: boolean;
}

type Handler = (draft: Draft, event: Envelope) => void;

function noop(): void {
  // 由其他 store 或后端通知部件承载。
}

function partIdOf(event: Envelope): string {
  return typeof event.data?.partId === "string" ? event.data.partId : "";
}

function ensureMessage(
  messages: ChatMessage[],
  messageID: string,
  threadID: string,
): ChatMessage[] {
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

export function updateThread(
  threads: Record<Mode, ThreadView[]>,
  threadID: string,
  patch: Partial<ThreadView>,
): Record<Mode, ThreadView[]> {
  return {
    code: threads.code.map((thread) => (thread.id === threadID ? { ...thread, ...patch } : thread)),
    work: threads.work.map((thread) => (thread.id === threadID ? { ...thread, ...patch } : thread)),
  };
}

function updateThreadWith(
  threads: Record<Mode, ThreadView[]>,
  threadID: string,
  update: (thread: ThreadView) => ThreadView,
): Record<Mode, ThreadView[]> {
  return {
    code: threads.code.map((thread) => (thread.id === threadID ? update(thread) : thread)),
    work: threads.work.map((thread) => (thread.id === threadID ? update(thread) : thread)),
  };
}

/** 把部件事件写入当前流式消息。 */
function upsertFromEvent(draft: Draft, kind: ChatPart["kind"], event: Envelope): void {
  const partID = partIdOf(event);
  if (!draft.runningMessageID || !partID) {
    return;
  }
  draft.messages = upsertPart(draft.messages, draft.runningMessageID, draft.activeThreadID, {
    id: partID,
    kind,
    seq: 0,
    data: event.data,
  });
}

function appendFromEvent(
  draft: Draft,
  kind: "text" | "reasoning",
  event: Envelope,
): void {
  const partID = partIdOf(event);
  if (!draft.runningMessageID || !partID) {
    return;
  }
  draft.messages = appendText(
    draft.messages,
    draft.runningMessageID,
    partID,
    kind,
    String(event.data?.text ?? ""),
  );
}

function finishRun(
  draft: Draft,
  messageStatus: ChatMessage["status"],
  threadStatus: string,
): void {
  draft.messages = setMessageStatus(draft.messages, draft.runningMessageID, messageStatus);
  draft.running = false;
  draft.runningMessageID = null;
  if (draft.activeThreadID) {
    draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: threadStatus });
  }
  draft.refreshNeeded = true;
}

/** 每种事件一个处理函数；新增 EVENT 时类型检查会强制补表。 */
const handlers: Record<EventType, Handler> = {
  [EVENT.runStarted]: (draft, event) => {
    const messageID = event.data?.messageId as string | undefined;
    if (!messageID) {
      return;
    }
    draft.runningMessageID = messageID;
    draft.running = true;
    draft.messages = ensureMessage(draft.messages, messageID, draft.activeThreadID);
    if (draft.activeThreadID) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: "running" });
    }
  },
  [EVENT.textDelta]: (draft, event) => appendFromEvent(draft, "text", event),
  [EVENT.textCompleted]: (draft, event) => upsertFromEvent(draft, "text", event),
  [EVENT.reasoningDelta]: (draft, event) => appendFromEvent(draft, "reasoning", event),
  [EVENT.reasoningCompleted]: (draft, event) => upsertFromEvent(draft, "reasoning", event),
  [EVENT.toolStarted]: (draft, event) => upsertFromEvent(draft, "tool", event),
  [EVENT.toolCompleted]: (draft, event) => upsertFromEvent(draft, "tool", event),
  [EVENT.approvalRequested]: (draft, event) => {
    upsertFromEvent(draft, "approval", event);
    const approval = event.data as ApprovalPartData;
    draft.pendingApprovals = [
      ...draft.pendingApprovals.filter((item) => item.approvalId !== approval.approvalId),
      approval,
    ];
    if (draft.activeThreadID) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: "awaiting" });
    }
  },
  [EVENT.approvalResolved]: (draft, event) => {
    const partID = partIdOf(event);
    if (partID) {
      draft.messages = updateApprovalPart(draft.messages, partID, {
        status: event.data.status,
        decision: event.data.decision,
        scope: event.data.scope,
      });
    }
    draft.pendingApprovals = draft.pendingApprovals.filter(
      (item) => item.approvalId !== event.data?.approvalId,
    );
    if (draft.activeThreadID) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: "running" });
    }
  },
  [EVENT.todoUpdated]: (draft, event) => upsertFromEvent(draft, "todo", event),
  [EVENT.notice]: (draft, event) => upsertFromEvent(draft, "notice", event),
  [EVENT.error]: (draft, event) => upsertFromEvent(draft, "error", event),
  [EVENT.runCompleted]: (draft) => finishRun(draft, "complete", "idle"),
  [EVENT.runCanceled]: (draft) => finishRun(draft, "incomplete", "idle"),
  [EVENT.runFailed]: (draft) => finishRun(draft, "error", "error"),
  [EVENT.threadUpdated]: (draft, event) => {
    if (event.data?.thread) {
      draft.threads = updateThread(draft.threads, event.data.thread.id, event.data.thread);
    } else if (draft.activeThreadID && event.data?.status) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, {
        status: event.data.status,
      });
    }
  },
  [EVENT.usageUpdated]: (draft, event) => {
    const input = Number(event.data?.inputTokens ?? 0);
    const output = Number(event.data?.outputTokens ?? 0);
    if (!draft.activeThreadID || input + output <= 0) {
      return;
    }
    draft.threads = updateThreadWith(draft.threads, draft.activeThreadID, (thread) => ({
      ...thread,
      tokens: (thread.tokens ?? 0) + input + output,
    }));
  },
  // 上下文压缩的可见文案由后端同时写入的 notice 部件承载。
  [EVENT.contextCompressed]: noop,
  // 设置与工作区事件由 app store 订阅，线程归约只负责线程内状态。
  [EVENT.settingsUpdated]: noop,
  [EVENT.workspaceUpdated]: noop,
  // 终端事件由 terminal store 订阅。
  [EVENT.terminalOutput]: noop,
  [EVENT.terminalExit]: noop,
};

/**
 * reduceThread 是线程事件的唯一归约入口。
 *
 * 纯函数：给定状态与批次返回新状态，便于直接测试，不需要 mock api 或总线。
 */
export function reduceThread(
  state: ThreadSlice,
  batch: Batch,
  activeThreadID: string,
): ReducedThread {
  const draft: Draft = {
    messages: state.messages,
    running: state.running,
    runningMessageID: state.runningMessageID,
    pendingApprovals: state.pendingApprovals,
    threads: state.threads,
    activeThreadID,
    refreshNeeded: false,
  };

  for (const event of batch.events) {
    if (event.threadId?.startsWith("terminal:")) {
      continue;
    }
    if (activeThreadID && event.threadId && event.threadId !== activeThreadID) {
      if (event.type === EVENT.threadUpdated) {
        draft.refreshNeeded = true;
      }
      continue;
    }
    const handler = handlers[event.type as EventType];
    if (handler) {
      handler(draft, event);
    }
  }

  return {
    messages: draft.messages,
    running: draft.running,
    runningMessageID: draft.runningMessageID,
    pendingApprovals: draft.pendingApprovals,
    threads: draft.threads,
    refreshNeeded: draft.refreshNeeded,
  };
}
