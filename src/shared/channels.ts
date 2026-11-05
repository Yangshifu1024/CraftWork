// IPC 通道表：通道名集中定义，避免跨进程 channel 字符串散落。
// renderer 经 preload 暴露的 window.craftwork 调用；main 侧 ipcMain.handle 一一对应。
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

/** main → renderer 的事件推送通道 */
export const PushChannels = {
  /** 会话事件批量推送（Envelope[]） */
  Events: "craftwork:events",
  /** 全局 toast 提示（main 主动，如密钥文件损坏警告） */
  Toast: "craftwork:toast",
} as const;

export interface ToastPayload {
  tone: "info" | "ok" | "warn" | "danger";
  title: string;
  message?: string;
}

/** renderer → main 的全部请求通道与签名（preload 按此实现 window.craftwork）。 */
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
  threadArchive(threadId: string, archived: boolean): Promise<void>;
  threadDelete(threadId: string): Promise<void>;
  threadSnapshot(threadId: string): Promise<Snapshot>;
  threadSnapshotPage(threadId: string, limit: number, beforeSeq: number): Promise<Snapshot>;

  // ---------- 对话 ----------
  chatCreateAndStart(
    mode: string,
    workspaceId: string,
    text: string,
    attachments: string[],
    modelRef: string,
  ): Promise<{ threadId: string; runId: string }>;
  chatSend(threadId: string, text: string, attachments: string[]): Promise<string>;
  chatSteer(threadId: string, text: string, attachments: string[]): Promise<{ queued: boolean; runId?: string }>;
  chatRegenerate(threadId: string): Promise<string>;
  chatCancel(threadId: string): Promise<boolean>;
  chatDecideApproval(approvalId: string, allow: boolean, scope: ApprovalScope): Promise<void>;

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
  catalogLookup(providerId: string, modelId: string): Promise<CatalogModelView>;
  catalogImport(input: CatalogImportInput): Promise<ProviderView>;

  // ---------- 工作区与文件 ----------
  workspaceList(): Promise<WorkspaceView[]>;
  workspaceAdd(path: string, trust: string): Promise<WorkspaceView[]>;
  workspaceAddProject(name: string, folders: string[], trust: string): Promise<WorkspaceView>;
  workspaceSetActive(workspaceId: string): Promise<WorkspaceView>;
  workspaceSetTrust(workspaceId: string, trust: string): Promise<void>;
  workspaceRename(workspaceId: string, name: string): Promise<WorkspaceView[]>;
  workspaceRemove(workspaceId: string): Promise<void>;
  workspaceListDir(workspaceId: string, folderId: string, rel: string): Promise<FileNode[]>;
  workspaceReadFile(workspaceId: string, folderId: string, rel: string): Promise<FileContent>;
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

/** CraftworkApi 的通道名映射：方法名 → "craftwork:<camel>"，供 preload/main 共用。 */
export function channelOf(method: string): string {
  return `craftwork:${method}`;
}
