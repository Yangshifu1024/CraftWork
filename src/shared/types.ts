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
  /** 绑定的 git worktree 目录（空串 = 未绑定）；绑定后本会话固定在工作树内执行 */
  worktreePath: string;
  /** 绑定的 worktree 分支名（craftwork/<会话短码>；空串 = 未绑定） */
  worktreeBranch: string;
  status: string; // idle | running | awaiting | error
  archived: boolean;
  tokens: number;
  contextTokens: number;
  contextCacheTokens: number;
  contextSpeed: number;
  /** 最近一轮首 token 延迟（毫秒）；0 = 无记录 */
  contextTtftMs: number;
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
  finishedAt?: number; // 回复完成时刻（= run 结束时间）；运行中缺省——界面时间戳显示它而非 createdAt
}

export interface Snapshot {
  thread: ThreadView;
  messages: MessageSnapshot[];
  pendingApprovals: ApprovalCard[];
  lastSeq: number;
  oldestSeq: number;
  hasMore: boolean;
}

/** 计划数据（计划模式产物）：file_path 非空且未批准时前端显示「批准并执行」提示条。 */
export interface PlanInfo {
  id: string;
  threadId: string;
  runId?: string;
  filePath: string;
  approved: boolean;
  /** 用户点过「忽略」：提示条不再出现 */
  ignored: boolean;
  updatedAt: number;
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
  /** 运行日志等级：off | error | info | debug（会话日志 <数据目录>/logs 的记录密度） */
  logsLevel: "off" | "error" | "info" | "debug";
  /** 流式输出贴底的平滑（弹簧）动画开关；false=即时跟随（直接跳到底） */
  smoothFollow: boolean;
  /** 输入预测：任务结束后静置 5 秒在输入框给出下一句建议，Tab 应用（docs/input-prediction/spec.md） */
  inputPrediction: boolean;
  /** 技能总开关（决策 D11）：关闭后不注入技能清单、skill 工具报错、斜杠调用不匹配 */
  skillsEnabled: boolean;
}

// ---------- 设置 ----------

/** 个性化全局指令（<数据目录>/AGENTS.md）：加载时机与注入规则见 craft-agent::envcontext。 */
export interface PersonalizationInfo {
  /** 文件是否存在（首次使用前为 false，展示空编辑器） */
  exists: boolean;
  /** 文件内容（不存在时为空串） */
  content: string;
  /** 字节数（计数与 100KB 上限展示用） */
  size: number;
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
  /** 自定义请求头（随每次模型请求附加，如网关路由头 x-opencode-session） */
  headers: HeaderPair[];
  models: ModelView[];
}

/** 一个自定义请求头（名称 + 值）；服务端保存时校验（冲突名/换行/重复/上限）。 */
export interface HeaderPair {
  name: string;
  value: string;
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
  /** 自定义请求头：携带时整体替换（服务端校验），缺省保留现值 */
  headers?: HeaderPair[];
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

// ---------- Skills（技能，决策 D11） ----------

export interface SkillView {
  /** frontmatter name；缺失时回退目录名 */
  name: string;
  description: string;
  whenToUse: string;
  /** user = 用户级（数据目录 / ~/.agents），workspace = 工作区级 */
  scope: "user" | "workspace";
  /** craftwork = `.craftwork/skills`，agents = `.agents/skills`（外部生态兼容） */
  source: "craftwork" | "agents";
  /** SKILL.md 绝对路径（启停与删除的标识） */
  path: string;
  /** 技能目录（辅助脚本/资源基准目录） */
  dir: string;
  enabled: boolean;
  /** 诊断信息；非空表示不可加载（缺 frontmatter 字段、超长、同名冲突等） */
  error?: string;
  /** 所属工作区 id；用户级技能为空串（仅 skillsListAll 下发） */
  workspaceId?: string;
  /** 所属工作区名称（管理页分组标题用）；用户级技能为空串（仅 skillsListAll 下发） */
  workspaceName?: string;
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
  /** 用户显示名：git 全局 user.name → 系统用户名 → "CraftWork"（侧栏账户位展示用） */
  userName: string;
}

// ---------- 工具声明（发往模型的规格 & 前端展示共用） ----------

export interface ToolSpecView {
  name: string;
  description: string;
  schema: Record<string, unknown>;
  risk: Risk;
  work: boolean;
}
