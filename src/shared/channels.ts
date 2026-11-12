// IPC 通道表：通道名集中定义，避免跨进程字符串散落。
// 渲染层系统操作唯一出口：经 tauri-bridge 暴露的 window.craftwork 调用；src-tauri 的
// craftwork_invoke 按方法名一一分发（三处同步接线：本文件签名 / tauri-bridge / src-tauri/src/lib.rs
// dispatch；接线完整性由 src-tauri/tests/dispatch_test.rs 对拍保证）。
import type {
  AppInfo,
  ApprovalScope,
  AttachmentContent,
  AuditEntry,
  Backup,
  CatalogImportInput,
  CatalogMeta,
  CatalogModelView,
  CatalogProviderView,
  ConnectionResult,
  FetchedModelsResult,
  DiffResult,
  DocumentPreview,
  FileContent,
  FileNode,
  LocatedFile,
  MCPServerInput,
  MCPServerView,
  McpTestResult,
  ModelInput,
  NetworkProbeTargetView,
  NetworkProbeView,
  PersonalizationInfo,
  ProviderInput,
  ProviderView,
  Settings,
  ShellEnvStatus,
  ShellOption,
  SkillView,
  Snapshot,
  StoreMarketsResult,
  StoreRefreshOutcome,
  StorePluginView,
  StorePluginDetail,
  StorageCleanupResult,
  StorageOverview,
  StorageTableRow,
  StorageThreadsView,
  CustomAgentProfile,
  CustomAgentProfileInput,
  SubAgentSettings,
  SubagentPart,
  ThreadView,
  UsageSummary,
  WorkspaceView,
} from "./types";

/** 后端 → 渲染层的事件推送通道（tauri event listen 订阅） */
export const PushChannels = {
  /** 会话事件批量推送（Batch 载荷，渲染层 applyBatch 是唯一事件入口） */
  Events: "craftwork:events",
  /** 全局 toast 提示（后端主动，如密钥文件损坏警告） */
  Toast: "craftwork:toast",
  /** 应用菜单动作（macOS 菜单栏点击；渲染层据此打开浮层/分节） */
  Menu: "craftwork:menu",
} as const;

/** craftwork:toast 通道的载荷 */
export interface ToastPayload {
  tone: "info" | "ok" | "warn" | "danger";
  title: string;
  message?: string;
}

/** craftwork:menu 通道的载荷：macOS 菜单栏动作（src-tauri/src/app_menu.rs 发出）。
 *  section 是设置页分区 id（SettingsSection），由渲染层校验后再使用。 */
export interface MenuActionPayload {
  action: "settings";
  section: string;
}

/** 渲染层 → 后端的全部请求通道与签名（tauri-bridge 据此以 Proxy 实现 window.craftwork，
 *  全部转发到统一入口 craftwork_invoke）。 */
export interface CraftworkApi {
  // ---------- 应用 ----------
  getInfo(): Promise<AppInfo>;
  openExternal(url: string): Promise<void>;
  pickFiles(): Promise<string[]>;
  pickFolder(): Promise<string>;
  savePastedAttachment(name: string, base64: string): Promise<string>;
  readAttachment(path: string): Promise<AttachmentContent>;
  clipboardWrite(text: string): Promise<boolean>;
  clipboardRead(): Promise<string>;
  showInFolder(path: string): Promise<void>;
  /** 用系统文件管理器打开应用数据目录（路径由服务端持有，不接受客户端传路径） */
  openDataDir(): Promise<void>;
  minimiseWindow(): Promise<void>;
  toggleMaximiseWindow(): Promise<void>;
  /** 主窗口当前是否最大化（自绘标题栏的最大化/还原图标据此切换，决策 D22） */
  windowIsMaximised(): Promise<boolean>;
  closeWindow(): Promise<void>;

  // ---------- 会话 ----------
  threadList(mode: string, includeArchived: boolean): Promise<ThreadView[]>;
  threadSearch(mode: string, query: string): Promise<ThreadView[]>;
  threadCreate(mode: string, workspaceId: string, title: string): Promise<ThreadView>;
  threadRename(threadId: string, title: string): Promise<void>;
  threadSetModel(threadId: string, modelId: string): Promise<void>;
  /** 会话级思考档位（空串=清除，不发 reasoning 参数） */
  threadSetReasoning(threadId: string, level: string): Promise<void>;
  threadArchive(threadId: string, archived: boolean): Promise<void>;
  threadDelete(threadId: string): Promise<void>;
  /** 一键清理临时会话（内置 conversation 工作区下当前模式、未归档的线程）；运行中/等待审批的跳过 */
  threadClearTemp(mode: string): Promise<{ removed: number; skipped: number }>;
  threadSnapshot(threadId: string): Promise<Snapshot>;
  threadSnapshotPage(threadId: string, limit: number, beforeSeq: number): Promise<Snapshot>;
  /** 标记会话已读（写阅读时刻并广播 thread.updated，侧栏未读点随之清除） */
  threadMarkRead(threadId: string): Promise<void>;
  /** 应用角标：未读或待审批的主会话去重数；传 0 或 null 清除（macOS 停靠栏；Windows/Linux 暂为 no-op） */
  setBadgeCount(count: number | null): Promise<void>;

  // ---------- 子代理（决策 D15） ----------
  /** 某主会话派生的子代理状态列表（含终态；主链路是 subagent.updated 事件流）。
   *  右侧面板「子代理」视图的兜底来源：重启后跨会话索引为空，打开面板时按当前会话
   *  经它恢复历史派发记录。返回形状与 subagent 部件载荷一致（渲染层写进同一索引）。
   *  子会话本身用 threadSnapshot(agentId) 读取，与普通会话同一条通道。 */
  subagentList(threadId: string): Promise<SubagentPart[]>;
  /** 子代理全局配置（开关、并发上限、内置两型的模型与迭代上限）。
   *  独立键存储，与 Settings 分开读写；首次读取会从旧 Settings 字段兼容回落一次。 */
  subagentSettingsGet(): Promise<SubAgentSettings>;
  subagentSettingsSave(next: SubAgentSettings): Promise<void>;
  /** 自定义子代理定义列表（含停用的；按创建顺序） */
  subagentProfilesList(): Promise<CustomAgentProfile[]>;
  /** 新建自定义子代理（名称唯一、不得占用内置 general/explore） */
  subagentProfileCreate(input: CustomAgentProfileInput): Promise<CustomAgentProfile>;
  /** 更新自定义子代理（id 不可变，未提供的字段保持原值） */
  subagentProfileUpdate(id: string, input: CustomAgentProfileInput): Promise<CustomAgentProfile>;
  /** 删除自定义子代理，返回删除后的列表 */
  subagentProfileDelete(id: string): Promise<CustomAgentProfile[]>;
  /** 启停自定义子代理（停用后不出现在模型可见的 agent_type 里） */
  subagentProfileSetEnabled(id: string, enabled: boolean): Promise<CustomAgentProfile[]>;

  // ---------- 对话 ----------
  chatCreateAndStart(
    mode: string,
    workspaceId: string,
    text: string,
    attachments: string[],
    modelRef: string,
    reasoningLevel: string,
    /** 计划模式：本轮只调研出计划，写类操作强制逐次审批 */
    planMode: boolean,
    /** worktree：发送时创建独立 git 工作树并绑定到会话（会话级，不可解绑） */
    useWorktree: boolean,
  ): Promise<{ threadId: string; runId: string }>;
  chatSend(threadId: string, text: string, attachments: string[], planMode: boolean, useWorktree: boolean): Promise<string>;
  chatSteer(threadId: string, text: string, attachments: string[], planMode: boolean): Promise<{ queued: boolean; runId?: string }>;
  chatRegenerate(threadId: string): Promise<string>;
  chatCancel(threadId: string): Promise<boolean>;
  chatDecideApproval(approvalId: string, allow: boolean, scope: ApprovalScope, feedback?: string): Promise<void>;
/** 读取后台命令任务的输出尾部（只读展示，不触发模型调用；任务号无效时拒绝）。 */
chatBashOutput(threadId: string, taskId: string, tailBytes?: number): Promise<{ output: string; state: string; exitCode: number | null; totalBytes: number }>;
/** 终止后台命令任务（整组杀含子进程）；用户主动操作，记审计。 */
chatKillShell(threadId: string, taskId: string): Promise<{ killed: boolean }>;
/** 跳过在途的 wait 等待（D21）：run 继续推进，模型立即核实等待目的；用户主动操作，记审计。无在途等待时拒绝。 */
chatCancelWait(threadId: string): Promise<{ canceled: boolean }>;

  // ---------- 计划 ----------
  // 计划审阅走审批卡（approval.* 通道），无独立计划通道：plans 行是后端档案（compact 续读用），
  // 审批卡「查看计划」直接把卡片持有的正文交给右侧栏计划视图展示。

  // ---------- 输入预测 ----------
  /** 一轮任务正常结束后预计算的下一句建议提示词（`crates/craft-agent/src/prediction.rs` 模块头；瞬态内存缓存，无则 null） */
  inputPredictionGet(threadId: string): Promise<string | null>;

  // ---------- 设置 ----------
  settingsGet(): Promise<Settings>;
  settingsSave(next: Settings): Promise<Settings>;
  /** 可选 shell 清单（设置 → 通用）：路径 / 家族 / 是否当前登录 shell / 会读的配置文件（D18） */
  shellsList(): Promise<ShellOption[]>;
  /** 重新捕获当前选中 shell 的环境（只回传概况，不含变量值）；未选中时返回空概况 */
  shellEnvRefresh(): Promise<ShellEnvStatus>;
  /** 个性化全局指令（<数据目录>/AGENTS.md）读取 */
  personalizationGet(): Promise<PersonalizationInfo>;
  /** 个性化全局指令保存（覆盖写入；空串删除文件；上限 100KB） */
  personalizationSave(content: string): Promise<PersonalizationInfo>;

  // ---------- 提供商与模型 ----------
  providerList(): Promise<ProviderView[]>;
  providerSave(input: ProviderInput): Promise<ProviderView>;
  providerDelete(providerId: string): Promise<ProviderView[]>;
  providerSetEnabled(providerId: string, enabled: boolean): Promise<ProviderView[]>;
  providerRename(providerId: string, name: string): Promise<ProviderView[]>;
  /** 启停单个模型（spec §8）：返回刷新后的提供商列表 */
  providerSetModelEnabled(providerId: string, modelId: string, enabled: boolean): Promise<ProviderView[]>;
  /** 新增模型：提供商内 ID 唯一，返回刷新后的该提供商视图 */
  providerAddModel(providerId: string, model: ModelInput): Promise<ProviderView>;
  /** 更新模型：按 modelId 定位整行更新，ID 不可变，返回刷新后的该提供商视图 */
  providerUpdateModel(providerId: string, modelId: string, model: ModelInput): Promise<ProviderView>;
  /** 删除模型：返回刷新后的提供商列表 */
  providerRemoveModel(providerId: string, modelId: string): Promise<ProviderView[]>;
  /** 拉取模型（spec §7）：按协议 GET 端点模型列表并按 ID 去重合并 */
  providerFetchModels(providerId: string): Promise<FetchedModelsResult>;
  providerSetKey(providerId: string, key: string): Promise<void>;
  providerClearKey(providerId: string): Promise<void>;
  providerTest(providerId: string): Promise<ConnectionResult>;
  providerWarnings(): Promise<string[]>;
  /** 网络页公开端点探测（决策 D14 第 6 节修订，与鉴权级 providerTest 语义不同）：
   *  并发 GET 常用提供商端点根路径（不带密钥），任意 HTTP 状态码都算连通。
   *  id 缺省全测（目录序数组），传单 id 只测一行（行内刷新）。 */
  networkProbeEndpoints(id?: string): Promise<NetworkProbeView[]>;
  /** 网络页探测行清单（D14 修订）：探测开始前就把九行固定画出来，探测只改某一行的状态。
   *  行不随探测结果增删——整块换列表 + 多路扫光并发曾诱发 WebView2 绘制残影。 */
  networkProbeTargets(): Promise<NetworkProbeTargetView[]>;
  catalogMeta(): Promise<CatalogMeta>;
  catalogProviders(query: string): Promise<CatalogProviderView[]>;
  catalogModels(providerId: string): Promise<CatalogModelView[]>;
  /** 死通道：dispatch 未接线（调用报「尚未接线」），登记于 dispatch_test 的 UNWIRED_ALLOWLIST，死代码清理时决定去留或接线 */
  catalogLookup(providerId: string, modelId: string): Promise<CatalogModelView>;
  catalogImport(input: CatalogImportInput): Promise<ProviderView>;

  // ---------- 工作区与文件 ----------
  workspaceList(): Promise<WorkspaceView[]>;
  workspaceAdd(path: string, trust: string): Promise<WorkspaceView[]>;
  workspaceAddProject(name: string, folders: string[], trust: string): Promise<WorkspaceView>;
  /** 死通道：dispatch 未接线（调用报「尚未接线」），登记于 dispatch_test 的 UNWIRED_ALLOWLIST，死代码清理时决定去留或接线 */
  workspaceSetActive(workspaceId: string): Promise<WorkspaceView>;
  workspaceSetTrust(workspaceId: string, trust: string): Promise<void>;
  workspaceRename(workspaceId: string, name: string): Promise<WorkspaceView[]>;
/** 编辑项目源文件夹：整体替换文件夹列表，首项即主要目录（内置临时会话目录不可编辑）。 */
workspaceReplaceFolders(workspaceId: string, folders: string[]): Promise<WorkspaceView>;
  workspaceRemove(workspaceId: string): Promise<void>;
  workspaceListDir(workspaceId: string, folderId: string, rel: string): Promise<FileNode[]>;
  workspaceReadFile(workspaceId: string, folderId: string, rel: string): Promise<FileContent>;
  /** 死通道：dispatch 未接线（调用报「尚未接线」），登记于 dispatch_test 的 UNWIRED_ALLOWLIST，死代码清理时决定去留或接线 */
  workspaceLocateFile(workspaceId: string, rel: string): Promise<LocatedFile>;
  workspaceWriteFile(
    workspaceId: string,
    folderId: string,
    rel: string,
    content: string,
    expectedHash: string,
  ): Promise<void>;
  workspaceComputeDiff(oldText: string, newText: string): Promise<DiffResult>;
  /** 全量项目工作区的 Git 状态（`workspaceId → 状态文本`）。后端按全部项目遍历，
   *  不接受单个工作区参数——早先签名带一个 `workspaceId` 但实体从不读取它。 */
  workspaceGitStatuses(): Promise<Record<string, string>>;
  documentPreview(workspaceId: string, folderId: string, rel: string): Promise<DocumentPreview>;

  // ---------- 审计 / 用量 / 备份 ----------
  auditList(limit: number): Promise<AuditEntry[]>;
  auditUsage(): Promise<UsageSummary>;
  auditBackups(limit: number): Promise<Backup[]>;
  auditRestore(backupId: string): Promise<void>;

  // ---------- 存储（设置 → 数据与统计 → 存储，决策 D27） ----------
  /** 首屏概览：数据库体积 + 数据目录各类体积 + 运行门禁原因（非空即禁止一切清理） */
  storageOverview(): Promise<StorageOverview>;
  /** 会话占用（按项目分组）。大对象按需获取——点开会话卡才调用 */
  storageThreads(): Promise<StorageThreadsView>;
  /** 表级体积明细（dbstat）。不可用的构建返回空数组，界面隐藏明细而不是给估算值 */
  storageDbTables(): Promise<StorageTableRow[]>;
  /** 执行一次清理，返回释放字节与逐条失败；有任务在跑时服务端直接拒绝 */
  storageCleanup(kind: string): Promise<StorageCleanupResult>;

  // ---------- MCP ----------
  /** mcpList：配置与运行态合并视图（敏感值不回传，见 MCPServerView 注释）。 */
  mcpList(): Promise<MCPServerView[]>;
  /** 保存（新增或编辑）一个服务器；保存即连接，返回合并视图列表。 */
  mcpUpsert(server: MCPServerInput): Promise<MCPServerView[]>;
  /** 删除一个服务器（先断开再删配置）。 */
  mcpRemove(serverId: string): Promise<MCPServerView[]>;
  /** 启用/停用（开关切换即连接/断开，工具随之注册/注销）。 */
  mcpSetEnabled(serverId: string, enabled: boolean): Promise<MCPServerView[]>;
  /** 测试连接：临时连接取工具清单后关闭（不注册工具、不改配置）。 */
  mcpTest(server: MCPServerInput): Promise<McpTestResult>;
  /** 手动重连。 */
  mcpConnect(serverId: string): Promise<MCPServerView[]>;
  /** 手动断开。 */
  mcpDisconnect(serverId: string): Promise<MCPServerView[]>;

  // ---------- Skills（技能，决策 D11） ----------
  /** 扫描技能（用户级 + 指定工作区级；workspaceId 缺省只扫用户级），含禁用与诊断标记 */
  skillsList(workspaceId?: string): Promise<SkillView[]>;
  /** 管理页全量扫描：用户级 + 全部工作区级，每条带 workspaceId / workspaceName 归属 */
  skillsListAll(): Promise<SkillView[]>;
  /** 启停单个技能（按 SKILL.md 绝对路径），返回刷新后的技能列表 */
  skillsSetEnabled(path: string, enabled: boolean, workspaceId?: string): Promise<SkillView[]>;
  /** 删除技能目录（仅允许技能根内的技能目录），返回刷新后的技能列表 */
  skillsRemove(path: string, workspaceId?: string): Promise<SkillView[]>;
  /** 读取技能正文（管理页预览；剥 frontmatter、变量已替换、超长截断） */
  skillsRead(path: string, workspaceId?: string): Promise<{ content: string }>;

  // ---------- 技能商店（决策 D12） ----------
  /** 已添加市场源 + 推荐源候选（运行时不内置任何源，推荐仅是候选） */
  storeListMarkets(): Promise<StoreMarketsResult>;
  /** 添加市场源（仅接受 https:// 仓库地址或 owner/repo 缩写），返回刷新后的市场清单 */
  storeAddMarket(input: string): Promise<StoreMarketsResult>;
  /** 移除市场源（市场下仍有已安装插件时拒绝并提示先卸载），返回刷新后的市场清单 */
  storeRemoveMarket(id: string): Promise<StoreMarketsResult>;
  /** 立即刷新：id 非空刷单源、空串刷全部；单源失败不阻塞其他源（失败原因在市场清单的 lastRefreshFailure） */
  storeRefreshMarkets(id?: string): Promise<StoreRefreshOutcome[]>;
  /** 各市场的插件清单（含未安装条目）+ 已装插件的更新状态（spec 第 4 节双轴检测） */
  storeListPlugins(): Promise<StorePluginView[]>;
  /** 安装插件，返回刷新后的插件清单（变更下一轮对话生效） */
  storeInstallPlugin(id: string): Promise<StorePluginView[]>;
  /** 升级插件（版本保留上限 3、启停选择保留），返回刷新后的插件清单 */
  storeUpdatePlugin(id: string): Promise<StorePluginView[]>;
  /** 一键回滚上一版（指针切回、可再切回），返回刷新后的插件清单 */
  storeRollbackPlugin(id: string): Promise<StorePluginView[]>;
  /** 彻底卸载（含全部历史版本与禁用记录残留），返回刷新后的插件清单 */
  storeUninstallPlugin(id: string): Promise<StorePluginView[]>;
  /** 插件级启停（停用后该插件全部技能根退出扫描），返回刷新后的插件清单 */
  storeSetPluginEnabled(id: string, enabled: boolean): Promise<StorePluginView[]>;
  /** 插件详情：已安装读缓存，未安装基于市场快照离线描述（不 clone 不下载） */
  storeDescribePlugin(id: string): Promise<StorePluginDetail>;
}

/** 方法名 → 通道名映射（"craftwork:<method>"）。当前无调用方：Tauri 版统一入口
 *  craftwork_invoke 按方法名分发，tauri-bridge 以 Proxy 直接转发，无需按通道调用
 *  （Electron 版按通道 invoke 的遗留助手，去留待定）。 */
export function channelOf(method: string): string {
  return `craftwork:${method}`;
}
