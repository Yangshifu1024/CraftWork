// 领域类型：跨主进程与渲染进程共享的结构定义。
// 所有时间戳统一毫秒；ID 形如「前缀_16位hex」。

// ---------- 策略 ----------

export type Risk = "R0" | "R1" | "R2" | "R3" | "R4";
/** 审批档位：分级（默认）/ 自由。历史档位 strict/readonly 已移除，旧存储值由后端回落分级。 */
export type PolicyLevel = "graded" | "free";
export type PolicyAction = "allow" | "ask" | "deny";
export type ApprovalScope = "once" | "session" | "workspace";

/** 策略评估结果：风险等级 + 判定理由 + 目标（路径/命令），审批卡片据此展示。 */
export interface Assessment {
  risk: Risk;
  reason: string;
  target: string;
}

// ---------- 会话与工作区 ----------

export interface ThreadView {
  id: string;
  mode: string; // code | work
  title: string;
  modelId: string;
  modelName: string;
  /** 思考档位（空串=未选择） */
  reasoningLevel: string;
  workspaceId: string;
  status: string; // idle | running | awaiting | error
  archived: boolean;
  tokens: number;
  contextTokens: number;
  contextCacheTokens: number;
  contextSpeed: number;
  createdAt: number;
  updatedAt: number;
}

export interface WorkspaceFolder {
  id: string;
  path: string;
  position: number;
}

export interface WorkspaceView {
  id: string;
  name: string;
  path: string;
  trust: string; // trusted | restricted
  purpose: string; // project | conversation
  lastOpenedAt: number;
  folders: WorkspaceFolder[];
}

export interface FileNode {
  name: string;
  path: string;
  type: "dir" | "file";
  size: number;
  modified: number;
}

export interface FileContent {
  workspaceId: string;
  folderId: string;
  path: string;
  content: string;
  hash: string;
  size: number;
  modified: number;
  binary: boolean;
  truncated: boolean;
  language: string;
}

export interface LocatedFile {
  workspaceId: string;
  folderId: string;
  path: string;
}

// ---------- 快照（会话历史重建结果） ----------

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

/** 消息内部件：payload 形状随 kind 变化（与 events.ts 的事件载荷对应）。 */
export interface PartSnapshot {
  id: string;
  kind: PartKind;
  seq: number;
  payload: Record<string, unknown>;
}

export interface MessageSnapshot {
  id: string;
  role: "user" | "assistant";
  parts: PartSnapshot[];
  status: "complete" | "streaming" | "incomplete" | "error";
  seq: number; // 最后部件 seq（翻页游标）
  createdAt: number; // 消息起始时刻（毫秒）；0 = 未知（旧事件流未携带）
  workDurationMs?: number;
}

export interface Snapshot {
  thread: ThreadView;
  messages: MessageSnapshot[];
  pendingApprovals: ApprovalCard[];
  lastSeq: number;
  oldestSeq: number;
  hasMore: boolean;
}

/** 待审批卡片（快照恢复用，载荷与事件中的审批部件一致）。 */
export interface ApprovalCard {
  partId: string;
  approvalId: string;
  toolCallId: string;
  tool: string;
  risk: string;
  title: string;
  target: string;
  detail: string;
  facts: Array<{ label: string; value: string }>;
  args: Record<string, unknown>;
  allowPermanent: boolean;
  status: "waiting" | "approved" | "denied" | "canceled";
  decision?: string;
  scope?: string;
}

// ---------- 设置 ----------

export interface Settings {
  theme: "system" | "dark" | "light";
  mode: "code" | "work";
  panelWidth: number;
  approvalLevel: PolicyLevel;
  defaultModel: string;
  /** 全局默认思考档位（空串=未选择） */
  reasoningLevel: string;
  fallbackModel: string;
  activeWorkspace: string;
  language: string;
  fontSize: number;
  codeTheme: string;
  autoUpdate: boolean;
  notifications: boolean;
  updateChannel: string;
  telemetry: boolean;
  maxIterations: number;
}

// ---------- 提供商与模型 ----------

export interface Capabilities {
  stream: boolean;
  tools: boolean;
  vision: boolean;
  reasoning: boolean;
  structured: boolean;
}

export interface Cost {
  inputPerMTok: number;
  outputPerMTok: number;
  cacheReadPerMTok: number;
  cacheWritePerMTok: number;
}

export interface ModelView {
  id: string;
  ref: string;
  name: string;
  provider: string;
  providerName: string;
  contextK: number;
  maxTokens: number;
  caps: Capabilities;
  cost?: Cost;
  status: string;
  canonicalId: string;
  releaseDate: string;
  /** manual | catalog | fetched（fetched：providerFetchModels 拉取合并时写入，见 spec §7） */
  source: string;
  /** 启用状态：禁用后不进模型选择器 */
  enabled: boolean;
  available: boolean;
  /** 思考档位类型：effort（枚举直传）| budget（Anthropic 预算映射）；空=不支持档位选择 */
  reasoningKind: string;
  /** 思考档位取值集（空=无档位 UI） */
  reasoningLevels: string[];
}

export interface ProviderView {
  id: string;
  name: string;
  kind: "openai" | "anthropic" | "openai-compatible";
  endpoint: string;
  keyRequired: boolean;
  keySource: string;
  keyMasked: string;
  available: boolean;
  /** 启用状态：禁用后该供应商模型不进模型选择器 */
  enabled: boolean;
  /** 文档地址（目录导入时来自 models.dev 的 doc 字段） */
  docUrl: string;
  catalogId: string;
  source: string; // manual | catalog
  models: ModelView[];
}

export interface ModelInput {
  id: string;
  name: string;
  contextK: number;
  maxTokens: number;
  caps?: Partial<Capabilities>;
  cost?: Cost;
  /** 思考档位声明（手动声明，用于不在目录中的网关模型）：缺省 = 编辑时保留现值；
   *  kind 空串 + [] = 显式清除；kind "effort" + 非空集 = 声明档位（注入 reasoning_effort）。 */
  reasoningKind?: string;
  reasoningLevels?: string[];
}

export interface ProviderInput {
  id: string;
  name: string;
  kind: "openai" | "anthropic" | "openai-compatible";
  endpoint: string;
  keyRequired: boolean;
  models: ModelInput[];
}

export interface ConnectionResult {
  ok: boolean;
  message: string;
  latencyMs: number;
  code: string;
}

/** 拉取模型结果（spec §7）：fetched = 拉取到的总数；added = 新增 ID；models = 合并后的最新模型视图。 */
export interface FetchedModelsResult {
  fetched: number;
  added: string[];
  models: ModelView[];
}

// ---------- 模型目录 ----------

export interface CatalogMeta {
  generatedAt: string;
  version: string;
  providerCount: number;
  modelCount: number;
}

export interface CatalogProviderView {
  id: string;
  name: string;
  kind: string;
  endpoint: string;
  doc: string;
  env: string[];
  modelCount: number;
  added: boolean;
}

export interface CatalogModelView {
  id: string;
  name: string;
  contextK: number;
  maxTokens: number;
  caps: Capabilities;
  cost?: Cost;
  status: string;
  releaseDate: string;
}

export interface CatalogImportInput {
  providerId: string;
  all: boolean;
  modelIds: string[];
}

// ---------- 审计 / 用量 / 备份 ----------

/** audit_log 行：每次工具调用与审批决定一条（自由档位下是事后追责的唯一手段）。 */
export interface AuditEntry {
  id: number;
  ts: number;
  threadId: string;
  runId: string;
  tool: string;
  risk: string;
  decision: string;
  scope: string;
  detail: string;
}

export interface UsageByModel {
  modelId: string;
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
}

export interface UsageByDay {
  day: string;
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
}

export interface UsageSummary {
  today: { inputTokens: number; outputTokens: number; costMicros: number };
  week: { inputTokens: number; outputTokens: number; costMicros: number };
  month: { inputTokens: number; outputTokens: number; costMicros: number };
  byModel: UsageByModel[];
  byDay: UsageByDay[];
}

/** 写前备份记录：backupPath 为备份副本，path 为原文件；hash 为备份时原文件 SHA-256。 */
export interface Backup {
  id: string;
  path: string;
  backupPath: string;
  hash: string;
  size: number;
  createdAt: number;
}

// ---------- MCP ----------

export interface MCPServerView {
  id: string;
  name: string;
  command: string;
  args: string[];
  enabled: boolean;
  connected: boolean;
  error?: string;
  tools: { name: string; description: string }[];
}

// ---------- 差异 ----------

export interface DiffLine {
  kind: "ctx" | "add" | "del";
  old: number;
  new: number;
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

// ---------- 文档预览 ----------

export interface DocumentPreview {
  kind: string;
  path: string;
  folderId: string;
  name: string;
  size: number;
  modified: number;
  supported: boolean;
  note: string;
  text: string;
}

// ---------- 附件 ----------

export interface AttachmentContent {
  base64: string;
  mime: string;
}

// ---------- 应用信息 ----------

export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  arch: string;
  dataDir: string;
}

// ---------- 工具声明（发往模型的规格 & 前端展示共用） ----------

export interface ToolSpecView {
  name: string;
  description: string;
  schema: Record<string, unknown>;
  risk: Risk;
  work: boolean;
}
