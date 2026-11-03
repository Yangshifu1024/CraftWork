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
  DiffResult,
  DocumentPreview,
  FileContent,
  FileNode,
  LocatedFile,
  MCPServerView,
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
  /** 终端输出 */
  TerminalData: "craftwork:terminal-data",
  /** 终端退出 */
  TerminalExit: "craftwork:terminal-exit",
  /** 全局 toast 提示（main 主动，如密钥文件损坏警告） */
  Toast: "craftwork:toast",
} as const;

/** 终端数据载荷 */
export interface TerminalDataPayload {
  sessionId: string;
  data: string;
}

export interface TerminalExitPayload {
  sessionId: string;
  exitCode: number;
}

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
  chatSend(threadId: string, text: string, attachments: string[]): Promise<string>;
  chatSteer(threadId: string, text: string, attachments: string[]): Promise<string>;
  chatRegenerate(threadId: string): Promise<string>;
  chatCancel(threadId: string): Promise<boolean>;
  chatDecideApproval(approvalId: string, allow: boolean, scope: ApprovalScope): Promise<void>;

  // ---------- 设置 ----------
  settingsGet(): Promise<Settings>;
  settingsSave(next: Settings): Promise<Settings>;

  // ---------- 提供商与模型 ----------
  providerList(): Promise<ProviderView[]>;
  providerSave(input: ProviderInput): Promise<ProviderView>;
  providerDelete(providerId: string): Promise<void>;
  providerSetKey(providerId: string, key: string): Promise<void>;
  providerClearKey(providerId: string): Promise<void>;
  providerTest(providerId: string): Promise<ConnectionResult>;
  providerWarnings(): Promise<string[]>;
  catalogMeta(): Promise<CatalogMeta>;
  catalogProviders(query: string): Promise<CatalogProviderView[]>;
  catalogModels(providerId: string): Promise<CatalogModelView[]>;
  catalogLogo(providerId: string): Promise<string>;
  catalogLookup(providerId: string, modelId: string): Promise<CatalogModelView>;
  catalogImport(input: CatalogImportInput): Promise<ProviderView>;

  // ---------- 工作区与文件 ----------
  workspaceList(): Promise<WorkspaceView[]>;
  workspaceAdd(path: string, trust: string): Promise<WorkspaceView>;
  workspaceAddProject(name: string, folders: string[], trust: string): Promise<WorkspaceView>;
  workspaceSetActive(workspaceId: string): Promise<WorkspaceView>;
  workspaceSetTrust(workspaceId: string, trust: string): Promise<void>;
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

  // ---------- 终端 ----------
  terminalCreate(sessionId: string, cwd: string, rows: number, cols: number): Promise<void>;
  terminalWrite(sessionId: string, data: string): Promise<void>;
  terminalResize(sessionId: string, rows: number, cols: number): Promise<void>;
  terminalDispose(sessionId: string): Promise<void>;
}

/** CraftworkApi 的通道名映射：方法名 → "craftwork:<camel>"，供 preload/main 共用。 */
export function channelOf(method: string): string {
  return `craftwork:${method}`;
}
