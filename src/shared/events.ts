// 事件类型全表：存储与前端渲染的共同协议。
// 这些字符串即协议本身，改动任何一处都属于协议变更。

/** 事件类型全表：值即协议字符串，与 Rust craft-core::contract 的事件类型常量对拍防漂移。 */
export const EventType = {
  RunStarted: "run.started",
  RunCompleted: "run.completed",
  RunCanceled: "run.canceled",
  RunFailed: "run.failed",
  TextDelta: "text.delta",
  TextCompleted: "text.completed",
  ReasoningDelta: "reasoning.delta",
  ReasoningCompleted: "reasoning.completed",
  ToolStarted: "tool.started",
  ToolCompleted: "tool.completed",
  ApprovalRequested: "approval.requested",
  ApprovalResolved: "approval.resolved",
  PlanCreated: "plan.created",
  TodoUpdated: "todo.updated",
  UserMessage: "user.message",
  Notice: "notice",
  ContextCompressed: "context.compressed",
  Error: "error",
  ThreadUpdated: "thread.updated",
  UsageUpdated: "usage.updated",
  SettingsUpdated: "settings.updated",
  WorkspaceUpdated: "workspace.updated",
} as const;

export type EventType = (typeof EventType)[keyof typeof EventType];

/** 事件信封：seq 为会话内单调递增（总线序，用于前端去重排序）。 */
export interface Envelope<T = unknown> {
  seq: number;
  type: EventType;
  threadId?: string;
  runId?: string;
  ts: number;
  data: T;
}

/** 一次批量推送（craftwork:events 通道的载荷；Bus 在后端攒批，渲染层 applyBatch 整批应用）。 */
export interface Batch {
  v: 1;
  threadId?: string;
  events: Envelope[];
}

// ---------- 事件载荷 ----------

export interface RunStartedData {
  messageId: string;
  modelId: string;
}

/** Run 终态（run.completed / run.canceled / run.failed 共用）；code/message 仅失败时携带。 */
export interface RunFinishedData {
  status: "done" | "canceled" | "error";
  code?: string;
  message?: string;
  durationMs: number;
}

export interface TextDeltaData {
  partId: string;
  text: string;
}

export interface ToolCallPart {
  partId: string;
  toolCallId: string;
  name: string;
  args: Record<string, unknown>;
  risk: string;
  status: "running" | "done" | "error";
  output?: string;
  isError?: boolean;
  code?: string;
  durationMs?: number;
  summary: string;
  title: string;
}

export interface ApprovalFact {
  label: string;
  value: string;
}

export interface ApprovalPart {
  partId: string;
  approvalId: string;
  toolCallId: string;
  tool: string;
  risk: string;
  title: string;
  target: string;
  detail: string;
  facts: ApprovalFact[];
  args: Record<string, unknown>;
  allowPermanent: boolean;
  status: "waiting" | "approved" | "denied" | "canceled";
  decision?: string;
  scope?: string;
}

/** 计划已生成：计划正文落盘到 plans 目录并回填路径（仅计划 run 正常完成时发出）。 */
export interface PlanCreatedData {
  planId: string;
  threadId: string;
  filePath: string;
}

export interface ApprovalResolvedData {
  partId: string;
  approvalId: string;
  status: "approved" | "denied" | "canceled";
  decision: string;
  scope: string;
}

export interface TodoItem {
  text: string;
  status: "pending" | "active" | "done";
}

export interface TodoPart {
  partId: string;
  items: TodoItem[];
}

/** 用户消息事件载荷；via="steer" 表示运行中引导已入队（前端显示「待送达」标记）。 */
export interface UserMessageData {
  messageId: string;
  text: string;
  attachments: string[];
  via?: "steer";
}

export interface NoticePart {
  partId: string;
  text: string;
}

export interface ContextCompressedData {
  summary: string;
  removedMessages: number;
  keptMessages: number;
  beforeTokens: number;
  afterTokens: number;
}

export interface ErrorPart {
  partId: string;
  code: string;
  message: string;
  retryable: boolean;
}

export interface ThreadUpdatedData {
  threadId: string;
  mode?: string;
  status?: string;
  thread?: ThreadSummary;
  deleted?: boolean;
}

export interface UsageUpdatedData {
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
  contextTokens?: number;
  cacheTokens?: number;
  durationMs?: number;
  tokensPerSecond?: number;
}

export interface ThreadSummary {
  id: string;
  mode: string;
  title: string;
  modelId: string;
  modelName: string;
  workspaceId: string;
  status: string;
  archived: boolean;
  tokens: number;
  contextTokens: number;
  contextCacheTokens: number;
  contextSpeed: number;
  createdAt: number;
  updatedAt: number;
}

export interface WorkspaceUpdatedData {
  workspaceId: string;
  name?: string;
  trust?: string;
  active?: boolean;
  removed?: boolean;
}

// ---------- 存储事件（DB events 表）的 kind 与载荷 ----------

/** 落库事件 kind：决定快照重建时生成哪种部件。 */
export type EventKind =
  | "user_message"
  | "text"
  | "reasoning"
  | "tool_call"
  | "approval"
  | "todo"
  | "error"
  | "notice"
  | "draft"
  | "callout"
  | "run_started"
  | "run_finished"
  | "context_summary"
  | "approval_resolved"
  | "context_compressed";

/** events 表行（落库形态）：快照重建（Snapshot）的输入；与推送 Envelope 是同一事实的两种形态。 */
export interface StoredEvent {
  id: string;
  threadId: string;
  messageId: string;
  runId?: string | null;
  seq: number;
  kind: EventKind;
  payload: unknown;
  createdAt: number;
}
