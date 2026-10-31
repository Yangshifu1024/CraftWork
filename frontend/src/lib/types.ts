import type {
  AppInfo,
  ConnectionResult,
  DocumentPreview,
  FileContent,
  FileNode,
  ModelView,
  ProviderView as ApiProviderView,
  Settings,
  TerminalView,
  ThreadView,
  WorkspaceView,
} from "../../bindings/craftwork/internal/service";
import type {
  AuditEntry,
  Backup,
  DailyUsage,
  ModelUsage,
  UsageSummary as ApiUsageSummary,
  UsageTotals,
} from "../../bindings/craftwork/internal/store";

export type {
  AppInfo,
  AuditEntry,
  Backup,
  ConnectionResult,
  DailyUsage,
  DocumentPreview,
  FileContent,
  FileNode,
  ModelUsage,
  ModelView,
  Settings,
  TerminalView,
  ThreadView,
  UsageTotals,
  WorkspaceView,
};

/** 差异结果：数组在前端统一为非空数组。 */
export interface DiffLine {
  kind: string;
  old?: number;
  new?: number;
  text: string;
}

export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

export interface DiffResult {
  additions: number;
  deletions: number;
  hunks: DiffHunk[];
  truncated: boolean;
}

/** 提供商视图：模型数组在前端统一为非空数组。 */
export interface ProviderView extends Omit<ApiProviderView, "models"> {
  models: ModelView[];
}

/** 用量聚合：数组在前端统一为非空数组。 */
export interface UsageSummary extends Omit<ApiUsageSummary, "byModel" | "byDay"> {
  byModel: ModelUsage[];
  byDay: DailyUsage[];
}

export type Mode = "code" | "work";
export type PanelTab = "files" | "diff" | "terminal" | "document";
export type Theme = "dark" | "light";
export type ApprovalLevel = "graded" | "strict" | "readonly";
export type MessageRole = "user" | "assistant";
export type MessageStatus = "streaming" | "complete" | "incomplete" | "error";

export type PartKind =
  | "text"
  | "reasoning"
  | "tool"
  | "approval"
  | "todo"
  | "error"
  | "notice"
  | "draft"
  | "callout";

export interface ChatPart {
  id: string;
  kind: PartKind;
  seq: number;
  data: any;
}

export interface ChatMessage {
  id: string;
  threadId: string;
  role: MessageRole;
  status: MessageStatus;
  createdAt: number;
  parts: ChatPart[];
}

export interface TextPartData {
  partId?: string;
  text: string;
}

export interface ReasoningPartData {
  partId?: string;
  text: string;
}

export interface ToolPartData {
  partId: string;
  toolCallId: string;
  name: string;
  args: Record<string, any>;
  risk: string;
  status: "running" | "done" | "error";
  output?: string;
  isError?: boolean;
  durationMs?: number;
  summary?: string;
}

export interface ApprovalFact {
  label: string;
  value: string;
}

export interface ApprovalPartData {
  partId: string;
  approvalId: string;
  toolCallId: string;
  tool: string;
  risk: string;
  title: string;
  target: string;
  detail?: string;
  facts?: ApprovalFact[];
  args?: Record<string, any>;
  allowPermanent: boolean;
  status: "waiting" | "approved" | "denied" | "canceled";
  decision?: string;
  scope?: string;
}

export interface TodoItem {
  text: string;
  status: "pending" | "active" | "done";
}

export interface TodoPartData {
  partId: string;
  items: TodoItem[];
}

export interface ErrorPartData {
  partId: string;
  code: string;
  message: string;
  retryable?: boolean;
}

export interface DraftPartData {
  partId?: string;
  to: string;
  subject: string;
  body: string;
  time?: string;
}

export interface CalloutPartData {
  partId?: string;
  tone?: "info" | "warn" | "ok";
  text: string;
}

export interface NoticePartData {
  partId?: string;
  text: string;
}

export interface Envelope {
  seq: number;
  type: string;
  threadId?: string;
  runId?: string;
  ts: number;
  data: any;
}

export interface Batch {
  v: number;
  threadId?: string;
  events: Envelope[];
}

export interface AttachmentInfo {
  name: string;
  path: string;
  size?: number;
}

export interface ThreadSnapshot {
  thread: {
    id: string;
    mode: Mode;
    title: string;
    modelId: string;
    workspaceId: string;
    status: string;
    archived: boolean;
    tokens: number;
    createdAt: number;
    updatedAt: number;
  };
  messages: Array<{
    id: string;
    threadId: string;
    role: MessageRole;
    status: string;
    createdAt: number;
    parts: Array<{ id: string; kind: string; seq: number; data: any }> | null;
  }> | null;
  lastSeq: number;
  approvals: Array<{
    id: string;
    threadId: string;
    runId: string;
    toolCallId: string;
    tool: string;
    risk: string;
    decision: string;
    scope: string;
  }> | null;
}

export const EVENT = {
  runStarted: "run.started",
  runCompleted: "run.completed",
  runCanceled: "run.canceled",
  runFailed: "run.failed",
  textDelta: "text.delta",
  textCompleted: "text.completed",
  reasoningDelta: "reasoning.delta",
  reasoningCompleted: "reasoning.completed",
  toolStarted: "tool.started",
  toolCompleted: "tool.completed",
  approvalRequested: "approval.requested",
  approvalResolved: "approval.resolved",
  todoUpdated: "todo.updated",
  notice: "notice",
  error: "error",
  threadUpdated: "thread.updated",
  usageUpdated: "usage.updated",
  settingsUpdated: "settings.updated",
  workspaceUpdated: "workspace.updated",
  terminalOutput: "terminal.output",
  terminalExit: "terminal.exit",
} as const;
