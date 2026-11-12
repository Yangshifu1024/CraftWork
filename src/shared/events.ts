// 事件类型全表：存储与前端渲染的共同协议。
// 这些字符串即协议本身，改动任何一处都属于协议变更。

import type { DiffResult } from "./types";

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
  /** 计划态切换（纯广播不入库）：enter_plan 工具执行时发出，前端同步 composer 勾选态。 */
  PlanEntered: "plan.entered",
  TodoUpdated: "todo.updated",
  UserMessage: "user.message",
  SteerDelivered: "steer.delivered",
  Notice: "notice",
  ContextCompressed: "context.compressed",
  Error: "error",
  /** 错误已解决：新 run 开始时对上一轮失败（error/interrupted）回填，前端摘除旧错误卡片。 */
  ErrorResolved: "error.resolved",
  ThreadUpdated: "thread.updated",
  UsageUpdated: "usage.updated",
  SettingsUpdated: "settings.updated",
  WorkspaceUpdated: "workspace.updated",
  /** 子代理状态变化（决策 D15）：派发（running）与终态各一次，落库供快照还原卡片。 */
  SubagentUpdated: "subagent.updated",
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
  /** canceled = wait 等待被用户跳过或随会话停止中断（D21）：中性状态，不算失败。 */
  status: "running" | "done" | "error" | "canceled";
  output?: string;
  isError?: boolean;
  code?: string;
  durationMs?: number;
  summary: string;
  title: string;
  /** wait 工具的出局原因（D21）：timeout / task_notification / subagent_report /
   *  steer / user_cancel / run_canceled。仅 wait 落库时携带。 */
  wokeBy?: string;
  /** 后台命令任务编号：`shell_exec` 启动行（run_in_background 或超时转后台）与
   *  `bash_output` 轮询行都携带。启动行有值即表示这次调用已返回、命令仍在后台跑，
   *  界面显示「后台运行」并提供读取输出入口；轮询行据此按任务号合并等待行。 */
  backgroundTaskId?: string | null;
  /** 后台任务的当前状态（`bash_output` 轮询回报）：渲染层据此判断等待行是扫光还是终态，
   *  不解析工具输出正文。旧事件没有该字段。 */
  backgroundTaskState?: "running" | "completed" | "killed" | "failed";
  /** 是否由「前台超时转后台」产生（界面上说明原因） */
  backgroundFromTimeout?: boolean;
  /** 工具返回的图片（MCP）：附件文件名列表，界面经 readAttachment 懒加载缩略图。 */
  attachments?: string[];
  /** 写类工具（fs_write / fs_edit / apply_patch）产生的行级差异，界面展开工具卡时渲染
   *  （D21）。超预算时 hunks 为空、truncated 为真，只剩增删统计。
   *  旧事件没有该字段，界面回退为纯文本输出。 */
  diff?: DiffResult;
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

export interface ApprovalResolvedData {
  partId: string;
  approvalId: string;
  status: "approved" | "denied" | "canceled";
  decision: string;
  scope: string;
  /** 拒绝时用户填写的修改意见长度（反馈本体作为用户消息另行落库，这里只带摘要）。 */
  feedbackChars?: number;
}

export interface TodoItem {
  text: string;
  /** "completed" 为历史事件中的模型别名（旧事件未归一化），渲染时映射回 "done"。 */
  status: "pending" | "active" | "done" | "completed";
}

export interface TodoPart {
  partId: string;
  items: TodoItem[];
}

/** 用户消息事件载荷；via="steer" 表示运行中引导已入队（前端显示「待送达」标记），
 *  via="plan_feedback" 表示计划审阅的修改意见（渲染为普通用户消息，不显示待送达标记）。
 *  skillInvoke 表示斜杠技能调用（text 保留原文 `/名称 参数`，模型历史改写为引导 prompt，
 *  消息流渲染为技能胶囊头 + 参数正文；与 via 正交——普通发送与 steer 均可携带）。 */
export interface UserMessageData {
  messageId: string;
  text: string;
  attachments: string[];
  via?: "steer" | "plan_feedback";
  skillInvoke?: { name: string };
}

/** 引导投递回执：messageId 即引导消息 id（Recorder 自动注入）。
 * nextMessageId 有值表示引导已被消费，后续输出切换到该新 assistant 消息；
 * 缺省表示去重跳过或收尾丢弃（内容已在事件表，仅摘「待送达」标记）。 */
export interface SteerDeliveredData {
  messageId: string;
  nextMessageId?: string;
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

/** 错误已解决事件载荷：messageId 为触发解决的新 run 占位消息（Recorder 自动注入）。 */
export interface ErrorResolvedData {
  messageId: string;
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
  /** 归一化完整提示词（含缓存读+写）；旧网关/旧行缺失时为 0 */
  promptTokens?: number;
  /** 本轮输出字符数（正文+思考），实时速率自校准 charsPerToken 用 */
  outputChars?: number;
  contextTokens?: number;
  cacheTokens?: number;
  durationMs?: number;
  /** 首 token 延迟（毫秒）；0 = 未产出内容 */
  ttftMs?: number;
  /** 网关不给 usage / 取消轮等无效窗口时不携带（unrated，前端显示「—」而非 0） */
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
  | "error_resolved"
  | "context_compressed"
  | "steer_delivered"
  | "subagent";

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
