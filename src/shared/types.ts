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
  /** 父会话 id（空串 = 主会话）；非空表示这是子代理的子会话（决策 D15，不进侧栏） */
  parentThreadId: string;
  /** 子代理可寻址短名（空串 = 非子代理会话） */
  agentName: string;
  /** 子代理类型（general / explore；空串 = 非子代理会话） */
  agentType: string;
  /** 已完成但用户尚未查看（后端按「完成时刻 > 阅读时刻」判定；子会话恒为 false） */
  hasUnread: boolean;
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
  | "callout"
  /** 子代理状态卡（决策 D15）：同一 partId 的多次事件合并成一张卡 */
  | "subagent";

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
  approvalLevel: PolicyLevel;
  defaultModel: string;
  /** 全局默认思考档位（空串=未选择） */
  reasoningLevel: string;
  fallbackModel: string;
  activeWorkspace: string;
  language: string;
  fontSize: number;
  codeTheme: string;  autoUpdate: boolean;
  notifications: boolean;
  /** 关闭窗口时退出应用（决策 D20）；false = 隐藏到托盘，run / 后台命令 / MCP 继续 */
  quitOnClose: boolean;
  updateChannel: string;
  telemetry: boolean;
  /** 单次运行的模型调用轮数上限；0 = 不限制（默认）。设置页可调 0~1000。 */
  maxIterations: number;
  /** 运行日志等级：off | error | info | debug（会话日志 <数据目录>/logs 的记录密度） */
  logsLevel: "off" | "error" | "info" | "debug";
  /** 选中的 shell 绝对路径（决策 D18）；空串 = 不使用，命令跑在最小环境里。
   *  非空时该 shell 的启动配置会在每条命令前被执行，其环境经黑名单清洗后交给子进程。 */
  shellPath: string;
  /** 流式输出贴底的平滑（弹簧）动画开关；false=即时跟随（直接跳到底） */
  smoothFollow: boolean;
  /** 输入预测：任务结束后静置 5 秒在输入框给出下一句建议，Tab 应用（`crates/craft-agent/src/prediction.rs` 模块头） */
  inputPrediction: boolean;
  /** 技能总开关（决策 D11）：关闭后不注入技能清单、skill 工具报错、斜杠调用不匹配 */
  skillsEnabled: boolean;
  /** 出站代理模式（决策 D14）：system（环境变量 > 系统代理 > 直连）| none | custom */
  networkProxyMode: "system" | "none" | "custom";
  /** 自定义代理地址（仅 custom 模式生效；保存时无协议头自动补 http://；含凭据时明文存本地库） */
  networkProxyUrl: string;
  /** GitHub 加速总开关：github.com / raw.githubusercontent.com 访问经前缀改写加速（第三方服务，国内网络建议开启） */
  networkGithubAcceleratorEnabled: boolean;
  /** 加速前缀（保存时归一补尾斜杠；默认 gh-proxy.com，可换任意同类前缀式服务） */
  networkGithubAcceleratorPrefix: string;
  /** 子代理总开关（决策 D15）：关闭后不再提供派发工具，已派发的照常完成 */
  subagentsEnabled: boolean;
  /** 同一对话内同时运行的子代理数量上限（1~8） */
  subagentMaxConcurrent: number;
}

// ---------- 设置 ----------

/** 可选 shell（设置 → 通用 → Shell 环境，决策 D18）。 */
export interface ShellOption {
  /** 绝对路径 */
  path: string;
  /** zsh | bash | fish | csh | nu | sh | other */
  kind: string;
  /** 是否当前登录 shell（$SHELL 指向同一个文件） */
  current: boolean;
  /** 选中后会被执行的配置文件（只含实际存在的） */
  configs: string[];
}

/** shell 环境概况：只回传数量与路径，**不回传变量值**（值与密钥同类，不进渲染层）。 */
export interface ShellEnvStatus {
  /** 生效的 shell 路径；空串 = 未启用 */
  shellPath: string;
  /** 实际读到的配置文件 */
  configs: string[];
  /** 清洗后保留的变量数 */
  varCount: number;
  /** 捕获失败的原因；null = 成功 */
  error: string | null;
}

/** 右栏「后台任务」tab 的行数据：本会话仍在运行的后台命令任务（`shell_exec` 的
 *  `run_in_background` 与「前台超时转后台」两条来源）。只含 Running——终态任务不保留
 *  展示（`crates/craft-tools/src/shell_tasks.rs` 的模块头与 `chat_shell_tasks`）。
 *
 *  `state` 仍保留完整四态：后端按 `TaskState::as_str` 原样回传，收紧成字面量 `"running"`
 *  会让「后端换了过滤条件」这类漂移在类型层被静默掩盖。 */
export interface ShellTaskView {
  /** `bash_<16hex>`；读取输出 / 终止都按它与会话一同引用 */
  id: string;
  /** 一行命令摘要（展示时由界面截断，不在这里裁） */
  command: string;
  state: "running" | "completed" | "killed" | "failed";
  /** 退出码；运行中与 Killed / 启动失败为 null */
  exitCode: number | null;
  /** 进程组组长 pid（终止时整组回收） */
  pid: number;
  /** 启动时刻（毫秒时间戳）；运行时长由界面按它实时计算（轮询快照的时长会停在采样点） */
  startedAt: number;
  /** 是否由「前台超时转后台」产生（文案要说明来源） */
  fromTimeout: boolean;
}

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

/** 单个套餐额度窗口（D34）：key ∈ five_hour | weekly | monthly，used_percent 为已用百分比。 */
export interface QuotaWindow {
  key: "five_hour" | "weekly" | "monthly";
  usedPercent: number;
  /** 重置时间（RFC3339 / ISO）；上游未给时为空串 */
  resetsAt: string;
}

/** 提供商套餐额度视图（D34，providerQuota 通道）：两家端点均未官方文档化，解析为
 *  防御式——窗口缺失即不出现；null = 提供商不在额度查询白名单（第三方中转）或
 *  无密钥/无套餐，渲染层据此隐藏 chip。余额（balanceText）为扩展位：OpenCode 的
 *  Console 余额走登录态授权、智谱端点不回金额，两家当前均为 null。 */
export interface ProviderQuotaView {
  providerId: string;
  providerName: string;
  windows: QuotaWindow[];
  balanceText: string | null;
  /** 本次取数时间（RFC3339） */
  fetchedAt: string;
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

/** 网络页公开端点探测行（D14修订）：
 *  status = "ok" 表示收到任意 HTTP 响应（含 4xx/5xx，网络可达）；
 *  仅 DNS 解析失败 / 连接失败 / 超时才是 "fail"。 */
export interface NetworkProbeView {
  id: string;
  name: string;
  url: string;
  status: "ok" | "fail";
  latencyMs: number;
  httpStatus: number | null;
  error: string | null;
}

/** 网络页探测行清单（D14 修订，`networkProbeTargets`）：界面据此**固定渲染行**，
 *  探测只改某一行的状态，行本身不随结果增删（整块替换曾诱发绘制残影）。 */
export interface NetworkProbeTargetView {
  id: string;
  name: string;
  url: string;
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

// ---------- 子代理（决策 D15） ----------

/** 子代理状态：运行中 / 完成 / 失败 / 被父级中断 / 被模型停止。 */
export type SubagentStatus = "running" | "completed" | "failed" | "interrupted" | "closed";

/** 子代理类型：general 完整工具、explore 只读、custom 用户自定义。 */
export type SubagentType = "general" | "explore" | "custom";

/** 子代理状态列表条目（subagentList 通道返回值；卡片走 subagent 部件）。 */
export interface SubagentView {
  /** 子会话 id（threadSnapshot 可直接读取它的完整过程） */
  agentId: string;
  /** 可寻址短名（父会话内唯一） */
  name: string;
  agentType: SubagentType;
  /** 派发摘要（卡片标题行） */
  description: string;
  status: SubagentStatus;
  /** 子代理使用的模型引用 */
  model: string;
  startedAt: number;
  finishedAt?: number;
  durationMs?: number;
  /** 子会话内已完成的工具调用次数 */
  toolUses: number;
  /** 子会话累计 token */
  tokens: number;
  /** 已用轮数（已完成的模型调用次数；随中间态进度事件实时递增） */
  iterations: number;
  /** 生效轮数上限；0 = 不限（仅配置异常时出现） */
  maxIterations: number;
  /** 派生它的父会话 id（渲染层跨会话索引与「列表跟随当前会话」过滤的依据） */
  parentThreadId: string;
}

/** 子代理部件（kind="subagent"）：父会话里的一张子代理卡。
 *  同一 partId 的 running → 中间态 → 终态多次事件按浅合并收口成一张卡（快照侧同规则）。 */
export interface SubagentPart extends SubagentView {
  partId: string;
  /** 子代理类型的中文展示名（通用 / 探查 / 自定义） */
  agentTypeLabel: string;
  /** 终态结论正文（运行中为空） */
  result: string;
  /** 终态是否需要向父会话回注报告（后台派发为 true；前台结果走工具结果） */
  notify: boolean;
}

// ---------- 子代理配置（决策 D15 扩展：独立键 + 自定义定义） ----------

/** 子代理工具面档位：只读 / 全部 / 指定名单。 */
export type CustomAgentTools =
  | { kind: "readOnly" }
  | { kind: "all" }
  | { kind: "only"; names: string[] };

/** 内置子代理类型的可调默认值（只允许改模型与迭代上限，工具面与提示词是安全属性）。 */
export interface BuiltinAgentProfile {
  /** general | explore */
  agentType: "general" | "explore";
  /** 默认模型引用；空串 = 继承父会话模型 */
  model: string;
  /** 默认迭代上限；0 = 使用封顶值（封顶值见 SubAgentSettings.maxIterationsCap） */
  maxIterations: number;
}

/** 子代理全局配置（settings 独立键 `subagent.config`）。 */
export interface SubAgentSettings {
  enabled: boolean;
  /** 单会话并发上限（1~8） */
  maxConcurrent: number;
  builtinProfiles: BuiltinAgentProfile[];
  /** 轮数上限的封顶值（服务端只读返回）：设置页上限输入的最大值与
   *  「0 = 封顶 N」文案的唯一事实源，前端不得再写死数字。 */
  maxIterationsCap: number;
}

/** 用户自定义子代理（settings 独立键 `subagent.profiles` 的元素）。 */
export interface CustomAgentProfile {
  id: string;
  /** 模型可见的 agent_type 值（小写字母开头，不得占用 general / explore） */
  name: string;
  /** 写进 enum 说明与提示词清单的一句话简介 */
  description: string;
  /** 追加到子代理提示词的角色段 */
  rolePrompt: string;
  /** 默认模型；空串 = 继承父会话模型 */
  model: string;
  /** 默认迭代上限；0 = 使用封顶值（封顶值见 SubAgentSettings.maxIterationsCap） */
  maxIterations: number;
  tools: CustomAgentTools;
  enabled: boolean;
}

/** 新建/更新自定义子代理的入参（更新时未提供的字段保持原值）。 */
export interface CustomAgentProfileInput {
  name?: string;
  description?: string;
  rolePrompt?: string;
  model?: string;
  maxIterations?: number;
  tools?: CustomAgentTools;
  enabled?: boolean;
}

// ---------- 用量 ----------

export interface UsageByModel {
  modelId: string;
  inputTokens: number;
  outputTokens: number;
  costMicros: number;
}

/** 按会话类型的用量聚合（近 30 天）：main = 主会话，general / explore / custom = 子代理类型。 */
export interface UsageByAgentType {
  agentType: string;
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
  byAgentType: UsageByAgentType[];
}

// ---------- MCP ----------

/** MCP 服务器合并视图（配置 + 运行态）。env / headers 只回传条数不回传值——
 *  凭据值只存本地库，进了前端就等于进了截图与剪贴板。 */
export interface MCPServerView {
  id: string;
  name: string;
  transport: "stdio" | "http" | "sse";
  command: string;
  args: string[];
  /** stdio：工作目录（空串 = 继承进程当前目录）。路径不是凭据，可以回传。 */
  cwd: string;
  url: string;
  enabled: boolean;
  timeoutMs: number;
  /** env 只回传条数：凭据进了前端就等于进了截图与剪贴板（spec 第 9 节）。 */
  envCount: number;
  headerCount: number;
  status: "disabled" | "disconnected" | "connecting" | "connected" | "failed";
  toolCount: number;
  error: string;
  serverName: string;
  serverVersion: string;
}

/** 测试连接结果。 */
export interface McpTestResult {
  success: boolean;
  elapsedMs: number;
  serverName?: string;
  serverVersion?: string;
  protocolVersion?: string;
  tools?: Array<{ name: string; description: string }>;
  error?: string;
}

/** MCP 服务器配置（保存表单的载荷；env / headers 为键值对象）。 */
export interface MCPServerInput {
  id?: string;
  name: string;
  transport: "stdio" | "http" | "sse";
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  url?: string;
  headers?: Record<string, string>;
  timeoutMs?: number;
  enabled?: boolean;
}

// ---------- Skills（技能，决策 D11） ----------

export interface SkillView {
  /** frontmatter name；缺失时回退目录名。插件技能为限定名 `插件名:技能名` */
  name: string;
  description: string;
  whenToUse: string;
  /** user = 用户级（数据目录 / ~/.agents / 商店插件缓存），workspace = 工作区级 */
  scope: "user" | "workspace";
  /** craftwork = `.craftwork/skills`，agents = `.agents/skills`（外部生态兼容），
   * marketplace = 技能商店插件缓存（D12） */
  source: "craftwork" | "agents" | "marketplace";
  /** 条目来源形态（spec 第 5 节 commands 映射）：skill = SKILL.md 声明，command = 插件
   *  `commands/*.md` 映射（仅插件域）。后端 SkillMeta 恒下发；可选取值保旧快照兼容 */
  origin?: "skill" | "command";
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

// ---------- 技能商店（决策 D12） ----------

/** 已添加市场源（marketplaces 表行视图；spec 第 2 节）。 */
export interface MarketView {
  /** 市场 id：清单 name（单插件仓库取插件 name，裸技能集取仓库名） */
  id: string;
  /** 用户输入原文（https:// URL 或 owner/repo 缩写） */
  source: string;
  /** 展开后的 git 仓库 URL */
  repoUrl: string;
  /** 最近一次成功刷新时间（毫秒；0 = 从未刷新成功，刷新节流依赖它） */
  lastRefreshedAt: number;
  /** 最近一次刷新失败原因（空串 = 无失败；成功刷新时清空） */
  lastRefreshFailure: string;
  createdAt: number;
}

/** 推荐市场源候选（spec 第 9 节常驻区块；仅候选，添加仍是用户确认动作）。 */
export interface RecommendedMarketView {
  /** 仓库缩写 owner/repo */
  repo: string;
  /** 形态徽标：marketplace | plugin | skills */
  form: "marketplace" | "plugin" | "skills";
  /** 形态徽标中文文案（市场 / 单插件 / 技能集） */
  formLabel: string;
  displayName: string;
  description: string;
}

/** storeListMarkets / storeAddMarket / storeRemoveMarket 的返回载荷。 */
export interface StoreMarketsResult {
  markets: MarketView[];
  recommended: RecommendedMarketView[];
}

/** 单市场刷新结果（storeRefreshMarkets；成功刷新的市场逐条返回）。 */
export interface StoreRefreshOutcome {
  marketplaceId: string;
  /** 新快照 HEAD 短码 */
  headSha: string;
  /** 本次是否经「加速失败 → 原始 URL 回退」成功（spec 第 7 节加速回退） */
  fellBack: boolean;
  /** 重新探测出的清单形态（craftwork-market / claude-market / single-plugin / bare-skills） */
  manifestKind: string;
  entryCount: number;
}

/** 更新角标三态（spec 第 4 节双轴检测：semver 可比比 semver，不可比比来源 pin，无法证明不误报）。 */
export interface StoreUpdateStatus {
  /** none 无更新（含无法证明）；update-available semver 更高；version-changed 同版本内容已变 */
  state: "none" | "update-available" | "version-changed";
  /** 市场侧可用版本原文（仅 update-available） */
  availableVersion?: string;
}

/** 商店插件视图（市场条目 + 已装状态合并；未装条目 installed=false、无安装侧字段）。 */
export interface StorePluginView {
  /** name@marketplace（安装 / 升级 / 回滚 / 卸载 / 启停的标识） */
  id: string;
  marketplaceId: string;
  name: string;
  description: string;
  category: string;
  tags: string[];
  /** 已装 = 当前版本指针；未装 = 市场侧解析版本（清单 version > 条目 version，可能空串） */
  version: string;
  installed: boolean;
  /** 插件级启停（仅已装；停用后该插件全部技能根退出扫描） */
  enabled?: boolean;
  /** 版本历史：磁盘保留的版本目录名（含当前，旧 → 新；仅已装） */
  versions: string[];
  /** 来源 pin：安装时的 git sha 短码（双轴检测的 sha 轴；仅已装，无 pin 为空串） */
  sourceSha: string;
  installedAt: number;
  /** 更新状态三态（未装恒 none） */
  updateStatus: StoreUpdateStatus;
  /** 组件标注：声明了 hooks / agents / MCP 配置（解析仅供详情页标注，当前版本不加载） */
  hasHooks: boolean;
  hasAgents: boolean;
  hasMcp: boolean;
}

/** 插件详情（storeDescribePlugin；已装读缓存，未装基于市场快照离线描述）。 */
export interface StorePluginDetail {
  id: string;
  marketplaceId: string;
  name: string;
  installed: boolean;
  enabled?: boolean;
  /** 当前版本指针（未装为空串） */
  currentVersion: string;
  versions: string[];
  sourceSha: string;
  /** 来源类型：entry-relative（市场仓库内）| repo（独立仓库二次克隆）；未装为空串 */
  sourceKind: string;
  /** 来源原文：相对路径 / owner/repo / git URL */
  sourceRef: string;
  description: string;
  category: string;
  tags: string[];
  hasHooks: boolean;
  hasAgents: boolean;
  hasMcp: boolean;
  /** 技能清单（已装扫缓存、未装扫快照来源目录；name 缺省回退目录名）。
   *  origin = command 时为 commands 映射条目（详情页标「命令」徽标；后端详情扫描当前
   *  仅列 skills/ 目录，字段可选取值兼容扩展） */
  skills: Array<{ name: string; path: string; description: string; origin?: "skill" | "command" }>;
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
  /** 构建期注入的 Git 短哈希（工作区有未提交改动时带 -dirty；无 git 环境为 unknown） */
  gitHash: string;
  /** 构建期注入的编译时间（本地时区，形如 2026-11-08 14:03:21） */
  buildTime: string;
}

// ---------- 存储占用（设置 → 数据与统计 → 存储，决策 D27） ----------
//
// 口径分两半，界面上不合并成单数：**数据库内**（SQLite 里的，含索引页）与
// **数据目录文件**（库之外的文件）。「各会话占用」是归属估算，不是独占字节。

/** 一个可执行的清理动作；kind 即 storageCleanup 的入参。 */
export interface StorageCleanupAction {
  kind: string;
  label: string;
  /** 影响面大、需二次确认对话框（其余走就近气泡） */
  destructive: boolean;
  /** 确认文案：写明后果与不可逆性 */
  confirm: string;
}

/** 数据目录里的一类文件（体积 + 条数 + 能怎么清）。 */
export interface StorageCategoryView {
  kind: string;
  label: string;
  description: string;
  bytes: number;
  files: number;
  /** 条目数达上限提前截止：界面显示「≥」而不是假精确值 */
  truncated: boolean;
  /** 空数组 = 只读展示 */
  cleanups: StorageCleanupAction[];
}

export interface StorageDatabaseView {
  /** 主库 + WAL + 共享内存索引 */
  bytes: number;
  mainBytes: number;
  walBytes: number;
  /** 全部会话 events.payload 字节合计（近似，不含索引页） */
  eventBytes: number;
}

export interface StorageTableRow {
  name: string;
  bytes: number;
}

export interface StorageOverview {
  database: StorageDatabaseView;
  categories: StorageCategoryView[];
  /** 数据目录各类合计（不含数据库） */
  filesTotalBytes: number;
  dataDir: string;
  /** 非空 = 有任务在运行，所有清理动作禁用；内容为可读原因 */
  busyReason: string;
}

export interface StorageThreadRow {
  threadId: string;
  title: string;
  mode: string;
  archived: boolean;
  /** 子代理子会话：不单列，折叠在父会话下 */
  isSubagent: boolean;
  /** 数据库内：events.payload 字节（近似） */
  dbBytes: number;
  /** 数据目录文件合计：日志 + 后台输出 + 工作树 + 计划文件 */
  fileBytes: number;
  logBytes: number;
  shellTaskBytes: number;
  worktreeBytes: number;
  planBytes: number;
  events: number;
  runs: number;
  approvals: number;
  updatedAt: number;
}

export interface StorageProjectGroup {
  workspaceId: string;
  workspaceName: string;
  /** 内置临时会话目录（界面标注「临时会话」） */
  isConversation: boolean;
  dbBytes: number;
  fileBytes: number;
  threadCount: number;
  threads: StorageThreadRow[];
  omittedThreads: number;
}

export interface StorageThreadsView {
  projects: StorageProjectGroup[];
  /** 未归属到任何项目的会话（工作区已删除等历史残留） */
  orphanThreads: StorageThreadRow[];
  threadTotal: number;
}

export interface StorageCleanupResult {
  kind: string;
  freedBytes: number;
  removedItems: number;
  skippedItems: number;
  failures: string[];
}

// ---------- 工具声明（发往模型的规格 & 前端展示共用） ----------

export interface ToolSpecView {
  name: string;
  description: string;
  schema: Record<string, unknown>;
  risk: Risk;
  work: boolean;
}
