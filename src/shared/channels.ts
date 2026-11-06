// IPC 通道表：通道名集中定义，避免跨进程字符串散落。
// 渲染层系统操作唯一出口：经 tauri-bridge 暴露的 window.craftwork 调用；src-tauri 的
// craftwork_invoke 按方法名一一分发（三处同步接线：本文件签名 / tauri-bridge / src-tauri/src/lib.rs
// dispatch；接线完整性由 src-tauri/tests/dispatch_test.rs 对拍保证）。
import type { PlanInfo,
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
  MCPServerView,
  ModelInput,
  ProviderInput,
  ProviderView,
  Settings,
  Snapshot,
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
} as const;

/** craftwork:toast 通道的载荷 */
export interface ToastPayload {
  tone: "info" | "ok" | "warn" | "danger";
  title: string;
  message?: string;
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
  minimiseWindow(): Promise<void>;
  toggleMaximiseWindow(): Promise<void>;
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
  chatDecideApproval(approvalId: string, allow: boolean, scope: ApprovalScope): Promise<void>;

  // ---------- 计划 ----------
  /** 会话的最新计划（无则 null）。file_path 非空且未批准才显示「批准并执行」提示条 */
  planGet(threadId: string): Promise<PlanInfo | null>;
  /** 批准计划：approved 置 1（提示条消失），随后前端发送确认消息开始执行 */
  planApprove(planId: string): Promise<void>;
  /** 忽略计划：提示条不再出现，等待用户直接输入继续 */
  planIgnore(planId: string): Promise<void>;
  /** 用系统默认应用打开计划 md 文件（仅允许 plans 目录内） */
  planOpen(path: string): Promise<void>;

  // ---------- 输入预测 ----------
  /** 一轮任务正常结束后预计算的下一句建议提示词（docs/input-prediction/spec.md；瞬态内存缓存，无则 null） */
  inputPredictionGet(threadId: string): Promise<string | null>;

  // ---------- 设置 ----------
  settingsGet(): Promise<Settings>;
  settingsSave(next: Settings): Promise<Settings>;

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
  workspaceGitStatuses(workspaceId: string): Promise<Record<string, string>>;
  documentPreview(workspaceId: string, folderId: string, rel: string): Promise<DocumentPreview>;

  // ---------- 审计 / 用量 / 备份 ----------
  auditList(limit: number): Promise<AuditEntry[]>;
  auditUsage(): Promise<UsageSummary>;
  auditBackups(limit: number): Promise<Backup[]>;
  auditRestore(backupId: string): Promise<void>;

  // ---------- MCP ----------
  mcpList(): Promise<MCPServerView[]>;
  mcpAdd(name: string, commandLine: string): Promise<MCPServerView>;
  mcpRemove(serverId: string): Promise<void>;
  mcpConnect(serverId: string): Promise<MCPServerView>;
  mcpDisconnect(serverId: string): Promise<void>;
}

/** 方法名 → 通道名映射（"craftwork:<method>"）。当前无调用方：Tauri 版统一入口
 *  craftwork_invoke 按方法名分发，tauri-bridge 以 Proxy 直接转发，无需按通道调用
 *  （Electron 版按通道 invoke 的遗留助手，去留待定）。 */
export function channelOf(method: string): string {
  return `craftwork:${method}`;
}
