/**
 * 会话事件合并。
 *
 * 这里把事件批量通道推送的事件批次（Batch）换算成聊天界面需要的会话状态：
 * 消息列表、流式文本增量、工具与审批部件、运行标记，以及会话列表里的状态字段。
 *
 * 本文件只做纯计算：不读写 store，也不调用后端命令，因此可以脱离界面单独测试。
 */
import type { ApprovalPartData, Batch, ChatMessage, ChatPart, Envelope, Mode, ThreadView } from "../lib/types";
import { EVENT } from "../lib/types";

/**
 * 会话事件合并处理涉及的 store 字段。
 *
 * 合并入口只接收这些字段，不依赖完整的会话 store，方便测试时按需构造。
 */
export interface ThreadSlice {
  messages: ChatMessage[];
  running: boolean;
  runningMessageID: string | null;
  pendingApprovals: ApprovalPartData[];
  threads: Record<Mode, ThreadView[]>;
}

/**
 * 合并结果：在 {@link ThreadSlice} 基础上多出一个刷新标记。
 *
 * `refreshNeeded` 为真表示批次里出现了其他会话的变更，调用方需要重新拉取会话列表。
 */
export interface ReducedThread extends ThreadSlice {
  refreshNeeded: boolean;
}

/** 事件类型字符串的联合类型，取自 `EVENT` 常量对象的全部取值。 */
type EventType = (typeof EVENT)[keyof typeof EVENT];

/**
 * 合并过程中使用的可变草稿。
 *
 * 在 store 字段之外补充「当前会话 id」与「是否需要刷新列表」，
 * 处理函数直接修改草稿，最后由 {@link reduceThread} 一次性返回结果。
 */
interface Draft extends ThreadSlice {
  activeThreadID: string;
  refreshNeeded: boolean;
}

/** 单个事件的处理函数签名：读取事件并就地修改草稿。 */
type Handler = (draft: Draft, event: Envelope) => void;

/**
 * 占位处理函数。
 *
 * 用于本文件不负责的事件类型；实际效果由其他 store 或后端通知部件承载。
 */
function noop(): void {
  // 由其他 store 或后端通知部件承载。
}

/**
 * 取出事件里携带的部件 id。
 *
 * 事件载荷是松散的 `any`，这里做一次类型检查，缺失时返回空字符串，
 * 让调用方以「没有部件 id」为由跳过处理。
 */
function partIdOf(event: Envelope): string {
  return typeof event.data?.partId === "string" ? event.data.partId : "";
}

/**
 * 确保消息列表中存在指定 id 的 assistant 消息，并把它标记为流式输出中。
 *
 * 事件可能早于消息列表加载完成就到达，所以找不到消息时要补一条空消息；
 * 已存在的消息只更新状态，避免覆盖已有的部件。
 */
function ensureMessage(messages: ChatMessage[], messageID: string, threadID: string): ChatMessage[] {
  if (messages.some((message) => message.id === messageID)) {
    return messages.map((message) => (message.id === messageID ? { ...message, status: "streaming" } : message));
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

/**
 * 把一段文本增量追加到指定消息的指定部件上。
 *
 * 增量事件只带片段内容，因此需要按部件 id 找到之前的文本再拼接；
 * 部件还不存在时新建一个，`seq` 取当前部件数量，保证顺序与后端一致。
 * 其他消息和部件保持原引用，避免整棵消息树重新渲染。
 */
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
      // 文本增量合并：旧内容在前、新片段在后，拼成完整正文。
      return { ...part, data: { ...part.data, text: `${part.data?.text ?? ""}${text}` } };
    });
    return { ...message, parts };
  });
}

/**
 * 按部件 id 新增或合并一个部件。
 *
 * 同一部件可能先以「开始」事件出现、再以「完成」事件带完整数据到达，
 * 因此已存在时做浅合并（新字段覆盖旧字段），不存在时追加到消息末尾。
 * 消息本身也不存在时补一条 assistant 流式消息，避免事件丢失。
 */
function upsertPart(messages: ChatMessage[], messageID: string, threadID: string, part: ChatPart): ChatMessage[] {
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
    const parts = message.parts.map((item, itemIndex) => (itemIndex === index ? { ...item, ...part } : item));
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

/**
 * 更新单条消息的状态。
 *
 * 没有正在处理的消息 id 时原样返回，避免把整份消息数组无意义地重建一次。
 */
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

/**
 * 清除所有引导消息的「待送达」标记。
 *
 * 运行中引导显示后，收到本会话任意后续活动事件（新 run、流式增量、
 * 工具开始）即视为排队引导已被并入模型请求；运行终态也清除——
 * 引导会留在会话记录里，随下一次运行的模型历史重建生效。
 * 没有标记时原样返回，避免无意义的整表重建。
 */
function clearSteerQueued(messages: ChatMessage[]): ChatMessage[] {
  if (!messages.some((message) => message.steerQueued)) {
    return messages;
  }
  return messages.map((message) =>
    message.steerQueued ? { ...message, steerQueued: false } : message,
  );
}

/**
 * 就地更新审批部件里的字段。
 *
 * 只匹配 id 相同且类型为审批的部件，其他部件原样返回；
 * 典型用法是收到 `approval.resolved` 后把状态从「等待」改成「已批准 / 已拒绝」。
 */
function updateApprovalPart(messages: ChatMessage[], partID: string, data: Partial<ApprovalPartData>): ChatMessage[] {
  return messages.map((message) => ({
    ...message,
    parts: message.parts.map((part) =>
      part.id === partID && part.kind === "approval" ? { ...part, data: { ...part.data, ...data } } : part,
    ),
  }));
}

/**
 * 在会话列表中就地合并某个会话的字段。
 *
 * code 与 work 两个模式各有一份列表，这里同时扫描，只替换 id 命中的那一条，
 * 其他会话保持原引用。返回新的 `Record`，不改动传入的对象。
 */
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

/**
 * 用自定义函数改写某个会话。
 *
 * 与 {@link updateThread} 的区别是需要读取原会话再做增量计算，
 * 例如用量事件要在已有的 token 计数上累加。
 */
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

/**
 * 把整块部件事件写入当前正在流式输出的消息。
 *
 * 用于工具、审批、待办、通知、错误等「一次发送完整数据」的事件；
 * 没有运行中的消息或事件缺少部件 id 时直接忽略。
 */
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

/**
 * 把文本类增量事件追加到当前正在流式输出的消息。
 *
 * 与 {@link upsertFromEvent} 的区别是只拼接正文，不整体替换部件数据。
 */
function appendFromEvent(draft: Draft, kind: "text" | "reasoning", event: Envelope): void {
  const partID = partIdOf(event);
  if (!draft.runningMessageID || !partID) {
    return;
  }
  draft.messages = appendText(draft.messages, draft.runningMessageID, partID, kind, String(event.data?.text ?? ""));
}

/**
 * 收尾一次运行。
 *
 * 依次做四件事：给当前消息写入终态、把事件携带的运行时长回填为「已工作」工时、
 * 清空运行标记、把会话状态改成终态；最后标记需要刷新列表，
 * 让侧边栏的排序与统计跟上数据库里的最新结果。
 */
function finishRun(draft: Draft, messageStatus: ChatMessage["status"], threadStatus: string, event: Envelope): void {
  draft.messages = setMessageStatus(draft.messages, draft.runningMessageID, messageStatus);
  // 运行结束事件带后端计算的耗时，回填给消息用于「已工作 N」折叠显示；
  // 与历史快照的回填字段一致，两条路径拿到的都是持久化事实。
  const durationMs = Number(event.data?.durationMs ?? 0);
  if (durationMs > 0 && draft.runningMessageID) {
    const messageID = draft.runningMessageID;
    draft.messages = draft.messages.map((message) =>
      message.id === messageID ? { ...message, workDurationMs: durationMs } : message,
    );
  }
  // 运行终态后不再有「即将投递」的语义：取消或失败时未投递的引导
  // 会留在会话记录里，随下一次运行的模型历史重建生效。
  draft.messages = clearSteerQueued(draft.messages);
  draft.running = false;
  draft.runningMessageID = null;
  if (draft.activeThreadID) {
    draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: threadStatus });
  }
  draft.refreshNeeded = true;
}

/**
 * 运行中引导（steer）落库：按消息 id 幂等插入一条用户消息。
 *
 * 引导不走乐观插入，这条事件就是显示的唯一来源；重复推送（多窗口、
 * 事件重放）按 id 去重。刷新或切换会话后整份消息由快照接管。
 */
function appendSteerMessage(draft: Draft, event: Envelope): void {
  const messageID = event.data?.messageId as string | undefined;
  if (!messageID || draft.messages.some((message) => message.id === messageID)) {
    return;
  }
  draft.messages = [
    ...draft.messages,
    {
      id: messageID,
      threadId: draft.activeThreadID,
      role: "user",
      status: "complete",
      createdAt: Date.now(),
      steerQueued: true,
      parts: [
        {
          id: `${messageID}-text`,
          kind: "text",
          seq: 0,
          data: { text: String(event.data?.text ?? ""), attachments: event.data?.attachments ?? [] },
        },
      ],
    },
  ];
}

/**
 * 事件类型到处理函数的映射表。
 *
 * 每种事件一个处理函数；新增 EVENT 时类型检查会强制补表，避免漏处理。
 */
const handlers: Record<EventType, Handler> = {
  // 运行开始：记录正在流式输出的消息 id，并把会话标记为运行中。
  // 接力运行也走这里；开始即视为排队引导即将投递，清掉「待送达」标记。
  [EVENT.runStarted]: (draft, event) => {
    const messageID = event.data?.messageId as string | undefined;
    if (!messageID) {
      return;
    }
    draft.runningMessageID = messageID;
    draft.running = true;
    draft.messages = clearSteerQueued(ensureMessage(draft.messages, messageID, draft.activeThreadID));
    if (draft.activeThreadID) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: "running" });
    }
  },
  // 正文增量：把片段追加到对应文本部件后面；出现新内容说明引导已投递。
  [EVENT.textDelta]: (draft, event) => {
    appendFromEvent(draft, "text", event);
    draft.messages = clearSteerQueued(draft.messages);
  },
  // 正文片段完成：用后端给出的完整文本覆盖该部件。
  [EVENT.textCompleted]: (draft, event) => upsertFromEvent(draft, "text", event),
  // 推理过程增量：与正文分开存放，供界面折叠展示。
  [EVENT.reasoningDelta]: (draft, event) => {
    appendFromEvent(draft, "reasoning", event);
    draft.messages = clearSteerQueued(draft.messages);
  },
  // 推理片段完成：整体覆盖该部件。
  [EVENT.reasoningCompleted]: (draft, event) => upsertFromEvent(draft, "reasoning", event),
  // 工具调用开始：写入工具部件，此时通常只有命令与参数。
  [EVENT.toolStarted]: (draft, event) => {
    upsertFromEvent(draft, "tool", event);
    draft.messages = clearSteerQueued(draft.messages);
  },
  // 工具调用结束：同一部件 id 合并输出、耗时与错误标记。
  [EVENT.toolCompleted]: (draft, event) => upsertFromEvent(draft, "tool", event),
  [EVENT.approvalRequested]: (draft, event) => {
    // 先落一个审批部件，界面据此渲染允许 / 拒绝按钮。
    upsertFromEvent(draft, "approval", event);
    const approval = event.data as ApprovalPartData;
    // 去重后放入待审批队列：同一个审批重复推送时只保留最新一份。
    draft.pendingApprovals = [
      ...draft.pendingApprovals.filter((item) => item.approvalId !== approval.approvalId),
      approval,
    ];
    if (draft.activeThreadID) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: "awaiting" });
    }
  },
  [EVENT.approvalResolved]: (draft, event) => {
    // 审批状态更新：把部件里的等待状态改成最终结果，并记录决定与生效范围。
    const partID = partIdOf(event);
    if (partID) {
      draft.messages = updateApprovalPart(draft.messages, partID, {
        status: event.data.status,
        decision: event.data.decision,
        scope: event.data.scope,
      });
    }
    // 无论部件是否存在，都要把它移出待审批队列。
    draft.pendingApprovals = draft.pendingApprovals.filter((item) => item.approvalId !== event.data?.approvalId);
    if (draft.activeThreadID) {
      draft.threads = updateThread(draft.threads, draft.activeThreadID, { status: "running" });
    }
  },
  // 待办清单更新：整份清单一次性替换。
  [EVENT.todoUpdated]: (draft, event) => upsertFromEvent(draft, "todo", event),
  // 运行中引导落库：插入带「待送达」标记的用户消息气泡。
  [EVENT.userMessage]: appendSteerMessage,
  // 普通提示：例如上下文压缩说明，作为消息内的通知部件展示。
  [EVENT.notice]: (draft, event) => upsertFromEvent(draft, "notice", event),
  // 错误事件：写成错误部件，交给消息组件展示错误码与可读文案。
  [EVENT.error]: (draft, event) => upsertFromEvent(draft, "error", event),
  // 三种运行终态：分别对应完成、取消、失败。
  [EVENT.runCompleted]: (draft, event) => finishRun(draft, "complete", "idle", event),
  [EVENT.runCanceled]: (draft, event) => finishRun(draft, "incomplete", "idle", event),
  [EVENT.runFailed]: (draft, event) => finishRun(draft, "error", "error", event),
  [EVENT.threadUpdated]: (draft, event) => {
    // 事件带整条会话时直接合并；只带状态时更新当前会话的状态字段。
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
    // 令牌数为 0 或没有当前会话时不必改动列表。
    if (!draft.activeThreadID || input + output <= 0) {
      return;
    }
    // 用量是累加语义：在已有计数上加本次输入与输出之和。
    draft.threads = updateThreadWith(draft.threads, draft.activeThreadID, (thread) => ({
      ...thread,
      tokens: (thread.tokens ?? 0) + input + output,
    }));
    // 上下文占用、缓存命中与输出速度是覆盖语义：每轮直接替换为最新值。
    // 只有主对话请求才携带这些字段（自动命名等 sidecar 请求不带），
    // 字段缺失时保持原值，避免 sidecar 小请求压低水位。
    const contextTokens = Number(event.data?.contextTokens ?? 0);
    if (contextTokens > 0) {
      draft.threads = updateThreadWith(draft.threads, draft.activeThreadID, (thread) => ({
        ...thread,
        contextTokens,
        contextCacheTokens: Number(event.data?.cacheTokens ?? 0),
        contextSpeed: Number(event.data?.tokensPerSecond ?? 0),
      }));
    }
  },
  // 上下文压缩的可见文案由后端同时写入的 notice 部件承载。
  [EVENT.contextCompressed]: noop,
  // 设置与工作区事件由 app store 订阅，会话处理只负责会话内状态。
  [EVENT.settingsUpdated]: noop,
  [EVENT.workspaceUpdated]: noop,
  // 终端事件由 terminal store 订阅。
  [EVENT.terminalOutput]: noop,
  [EVENT.terminalExit]: noop,
};

/**
 * reduceThread 是会话事件的唯一合并入口。
 *
 * 纯函数：给定状态与批次返回新状态，便于直接测试，不需要 mock api 或总线。
 * 处理顺序严格按批次里的事件顺序进行，因此同一批次内的增量不会乱序。
 */
export function reduceThread(state: ThreadSlice, batch: Batch, activeThreadID: string): ReducedThread {
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
    // 事件批次过滤：终端事件挂在 "terminal:" 伪会话下，与聊天消息无关，直接跳过。
    if (event.threadId?.startsWith("terminal:")) {
      continue;
    }
    // 当前会话判定：明确属于其他会话的事件不进入本会话的消息与运行状态。
    // 但会话列表仍要跟上，因此遇到其他会话的更新事件只打刷新标记，稍后重新拉取列表。
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
    // 没有登记的事件类型（后端新增、前端尚未跟上）静默忽略，不影响同批次的其他事件。
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
