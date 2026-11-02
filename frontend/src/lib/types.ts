/**
 * 前端共享类型层。
 *
 * 三类内容集中在这里：
 * 1. 直接复用 Go 生成的类型（应用信息、设置、会话视图、用量等）；
 * 2. 前端专用的视图类型与字面量联合类型（消息、部件、事件信封等）；
 * 3. `EVENT` 事件名常量，Go 侧推送事件与前端订阅必须使用同一套字符串。
 *
 * 修改事件名或载荷结构时，需要同时同步 `internal/events/events.go`，
 * 否则前端会收不到或解析不了对应事件。
 */
import type {
  ProviderView as ApiProviderView,
  AppInfo,
  CatalogImportInput,
  CatalogMeta,
  CatalogModelView,
  CatalogProviderView,
  ConnectionResult,
  DocumentPreview,
  FileContent,
  FileNode,
  MCPServerView,
  MCPToolView,
  ModelInput,
  ModelView,
  ProviderInput,
  Settings,
  TerminalView,
  ThreadView,
  WorkspaceView,
} from "../../bindings/github.com/yangshifu1024/craftwork/internal/service";
import type {
  UsageSummary as ApiUsageSummary,
  AuditEntry,
  Backup,
  DailyUsage,
  ModelUsage,
  Snapshot,
  UsageTotals,
} from "../../bindings/github.com/yangshifu1024/craftwork/internal/store";

/**
 * 把 Go 生成的类型原样再导出。
 *
 * 业务代码统一从本文件导入，避免到处写 `../../bindings/...` 的长路径；
 * 同时让「前端类型来自后端生成」这件事只有这一处依赖。
 */
export type {
  AppInfo,
  AuditEntry,
  Backup,
  CatalogImportInput,
  CatalogMeta,
  CatalogModelView,
  CatalogProviderView,
  ConnectionResult,
  DailyUsage,
  DocumentPreview,
  FileContent,
  FileNode,
  MCPServerView,
  MCPToolView,
  ModelInput,
  ModelUsage,
  ModelView,
  ProviderInput,
  Settings,
  TerminalView,
  ThreadView,
  UsageTotals,
  WorkspaceView,
};

/** 差异中的一行：`kind` 区分新增、删除与上下文，行号用于左侧 gutter。 */
export interface DiffLine {
  kind: string;
  old?: number;
  new?: number;
  text: string;
}

/** 差异中的一个区块：`header` 是 `@@ -a,b +c,d @@` 形式的区块头。 */
export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

/** 差异结果：数组在前端统一为非空数组。 */
export interface DiffResult {
  additions: number;
  deletions: number;
  hunks: DiffHunk[];
  truncated: boolean;
}

/**
 * 提供商视图：模型数组在前端统一为非空数组。
 *
 * 后端可能返回 nil 切片，桌面适配器会补成 `[]`，因此这里把 `models` 重新声明为必填。
 */
export interface ProviderView extends Omit<ApiProviderView, "models"> {
  models: ModelView[];
}

/**
 * 用量聚合：数组在前端统一为非空数组。
 *
 * 与 `ProviderView` 同理，把两个统计数组收紧成必填，
 * 图表面板可以直接遍历而不用判空。
 */
export interface UsageSummary extends Omit<ApiUsageSummary, "byModel" | "byDay"> {
  byModel: ModelUsage[];
  byDay: DailyUsage[];
}

/** 工作模式：`code` 面向编码任务，`work` 面向办公任务，两者各有会话列表。 */
export type Mode = "code" | "work";
/** 右侧面板的页签：文件、差异、终端、文档。 */
export type PanelTab = "files" | "diff" | "terminal" | "document";
/** 界面主题：跟随系统、深色或浅色。 */
export type Theme = "dark" | "light" | "system";
/** 审批档位：分级审批、严格审批、只读、自由（放行一切，切换前需确认）。 */
export type ApprovalLevel = "graded" | "strict" | "readonly" | "free";
/** 消息角色。 */
export type MessageRole = "user" | "assistant";
/** 消息状态：流式输出中、正常完成、被取消、出错。 */
export type MessageStatus = "streaming" | "complete" | "incomplete" | "error";

/**
 * 消息部件类型。
 *
 * 一条消息由多个部件按 `seq` 顺序拼成：正文、推理、工具调用、审批卡片、
 * 待办清单、错误、提示、草稿与强调块。
 */
export type PartKind = "text" | "reasoning" | "tool" | "approval" | "todo" | "error" | "notice" | "draft" | "callout";

/**
 * 消息部件。
 *
 * `data` 的具体结构由 `kind` 决定，对应下面各个 `*PartData` 类型；
 * 因为事件载荷来自后端且类型较松，这里保留 `any`，由渲染组件自行按 kind 收窄。
 */
export interface ChatPart {
  id: string;
  kind: PartKind;
  seq: number;
  // biome-ignore lint/suspicious/noExplicitAny: 后端事件载荷类型较松，由渲染组件按 kind 收窄到各 *PartData
  data: any;
}

/** 聊天消息：归属某个会话，带角色、状态、创建时间与部件列表。 */
export interface ChatMessage {
  id: string;
  threadId: string;
  role: MessageRole;
  status: MessageStatus;
  createdAt: number;
  /** 本轮运行耗时（毫秒）。运行结束（含取消/失败）后由后端回填；运行中为 0。 */
  workDurationMs?: number;
  /**
   * 运行中引导消息的「待送达」标记：已显示但尚未并入下一次模型请求。
   * 收到本会话任意后续活动事件（流式增量、工具开始、新 run）即清除。
   */
  steerQueued?: boolean;
  parts: ChatPart[];
}

/** 正文部件的数据。 */
export interface TextPartData {
  partId?: string;
  text: string;
}

/** 推理过程部件的数据，与正文分开展示，可折叠。 */
export interface ReasoningPartData {
  partId?: string;
  text: string;
  /** 本次思考持续的毫秒数；由后端在流式结束时随部件数据落库，旧数据可能缺失。 */
  durationMs?: number;
}

/** 工具调用部件的数据：记录工具名、参数、风险级别、执行状态与输出。 */
export interface ToolPartData {
  partId: string;
  toolCallId: string;
  name: string;
  // biome-ignore lint/suspicious/noExplicitAny: 工具参数结构随工具而异，由各工具卡片自行读取
  args: Record<string, any>;
  risk: string;
  status: "running" | "done" | "error";
  output?: string;
  isError?: boolean;
  durationMs?: number;
  summary?: string;
}

/** 审批卡片上展示的单条事实，例如目标路径、命令内容。 */
export interface ApprovalFact {
  label: string;
  value: string;
}

/** 审批部件的数据：包含展示信息与用户决定后的状态、决定与生效范围。 */
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
  // biome-ignore lint/suspicious/noExplicitAny: 审批参数结构随工具而异，由审批卡片自行读取
  args?: Record<string, any>;
  /** 是否允许「本次会话 / 当前工作区」的永久授权。 */
  allowPermanent: boolean;
  status: "waiting" | "approved" | "denied" | "canceled";
  decision?: string;
  scope?: string;
}

/** 待办清单里的一条任务。 */
export interface TodoItem {
  text: string;
  status: "pending" | "active" | "done";
}

/** 待办部件的数据：整份清单一次性下发。 */
export interface TodoPartData {
  partId: string;
  items: TodoItem[];
}

/** 错误部件的数据：错误码与可读文案，`retryable` 表示是否可重试。 */
export interface ErrorPartData {
  partId: string;
  code: string;
  message: string;
  retryable?: boolean;
}

/** 草稿部件的数据：办公模式下生成的邮件草稿等。 */
export interface DraftPartData {
  partId?: string;
  to: string;
  subject: string;
  body: string;
  time?: string;
}

/** 强调块部件的数据：用于提示、警告或成功信息。 */
export interface CalloutPartData {
  partId?: string;
  tone?: "info" | "warn" | "ok";
  text: string;
}

/** 通知部件的数据：系统提示或上下文压缩说明等简短信息。 */
export interface NoticePartData {
  partId: string;
  text: string;
  summary?: string;
}

/**
 * 事件信封。
 *
 * `seq` 在同一会话内单调递增，`type` 对应 `EVENT` 里的字符串；
 * `threadId` 标记事件归属，终端事件使用 `terminal:` 前缀的伪会话 id。
 */
export interface Envelope {
  seq: number;
  type: string;
  threadId?: string;
  runId?: string;
  ts: number;
  // biome-ignore lint/suspicious/noExplicitAny: 事件载荷结构由 type 决定，消费端按事件类型收窄
  data: any;
}

/** 事件批次：总线把多个事件合并成一批推送，`v` 为协议版本。 */
export interface Batch {
  v: number;
  threadId?: string;
  events: Envelope[];
}

/** 附件信息：路径、显示名与可选的文件大小。 */
export interface AttachmentInfo {
  name: string;
  path: string;
  size?: number;
}

/** 会话快照直接使用 Go 生成的类型，避免前端手写镜像。 */
export type ThreadSnapshot = Snapshot;

/**
 * 事件名常量。
 *
 * 必须与 Go 侧 `internal/events/events.go` 保持一致；
 * `reduceThread` 以这些名字做分发，类型检查会在新增事件时强制补处理函数。
 */
export const EVENT = {
  // 一次运行的生命周期。
  runStarted: "run.started",
  runCompleted: "run.completed",
  runCanceled: "run.canceled",
  runFailed: "run.failed",
  // 正文与推理过程的增量 / 完成事件。
  textDelta: "text.delta",
  textCompleted: "text.completed",
  reasoningDelta: "reasoning.delta",
  reasoningCompleted: "reasoning.completed",
  // 工具调用与审批。
  toolStarted: "tool.started",
  toolCompleted: "tool.completed",
  approvalRequested: "approval.requested",
  approvalResolved: "approval.resolved",
  // 消息内的其他部件。
  todoUpdated: "todo.updated",
  notice: "notice",
  contextCompressed: "context.compressed",
  error: "error",
  // 运行中引导（steer）落库的用户消息，前端据此插入消息气泡。
  userMessage: "user.message",
  // 会话与用量统计。
  threadUpdated: "thread.updated",
  usageUpdated: "usage.updated",
  // 应用级广播与终端事件。
  settingsUpdated: "settings.updated",
  workspaceUpdated: "workspace.updated",
  terminalOutput: "terminal.output",
  terminalExit: "terminal.exit",
} as const;
