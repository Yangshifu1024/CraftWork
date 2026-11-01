import type {
  AppInfo,
  AuditEntry,
  Backup,
  ConnectionResult,
  DiffResult,
  DocumentPreview,
  FileContent,
  FileNode,
  MCPServerView,
  ProviderView,
  Settings,
  TerminalView,
  ThreadSnapshot,
  ThreadView,
  UsageSummary,
  WorkspaceView,
} from "./types";

/** 前端与 Go 后端之间的唯一接口。桌面与浏览器 Mock 实现同一套方法。 */
export interface Api {
  app: {
    info(): Promise<AppInfo>;
    openExternal(url: string): Promise<void>;
    pickFiles(): Promise<string[]>;
    writeClipboard(text: string): Promise<boolean>;
    readClipboard(): Promise<string>;
  };
  settings: {
    get(): Promise<Settings>;
    save(settings: Settings): Promise<Settings>;
  };
  threads: {
    list(mode: string, includeArchived?: boolean): Promise<ThreadView[]>;
    search(mode: string, query: string): Promise<ThreadView[]>;
    create(mode: string, workspaceID: string, title: string): Promise<ThreadView>;
    rename(threadID: string, title: string): Promise<void>;
    setModel(threadID: string, modelID: string): Promise<void>;
    archive(threadID: string, archived: boolean): Promise<void>;
    remove(threadID: string): Promise<void>;
    snapshot(threadID: string): Promise<ThreadSnapshot>;
    snapshotPage(threadID: string, limit: number, beforeSeq: number): Promise<ThreadSnapshot>;
  };
  chat: {
    send(threadID: string, text: string, attachments?: string[]): Promise<string>;
    regenerate(threadID: string): Promise<string>;
    cancel(threadID: string): Promise<boolean>;
    decideApproval(approvalID: string, allow: boolean, scope: string): Promise<void>;
  };
  workspace: {
    list(): Promise<WorkspaceView[]>;
    pickFolder(): Promise<string>;
    add(path: string, trust: string): Promise<WorkspaceView>;
    setActive(workspaceID: string): Promise<WorkspaceView>;
    active(): Promise<WorkspaceView | null>;
    setTrust(workspaceID: string, trust: string): Promise<void>;
    remove(workspaceID: string): Promise<void>;
    listDir(workspaceID: string, path: string): Promise<FileNode[]>;
    readFile(workspaceID: string, path: string): Promise<FileContent>;
    writeFile(workspaceID: string, path: string, content: string, expectedHash: string): Promise<void>;
    computeDiff(oldText: string, newText: string): Promise<DiffResult>;
    gitStatuses(workspaceID: string): Promise<Record<string, string>>;
  };
  providers: {
    list(): Promise<ProviderView[]>;
    warnings(): Promise<string[]>;
    setKey(providerID: string, key: string): Promise<void>;
    clearKey(providerID: string): Promise<void>;
    test(providerID: string): Promise<ConnectionResult>;
  };
  mcp: {
    list(): Promise<MCPServerView[]>;
    add(name: string, command: string): Promise<MCPServerView>;
    remove(serverID: string): Promise<void>;
    connect(serverID: string): Promise<MCPServerView>;
    disconnect(serverID: string): Promise<void>;
  };
  audit: {
    list(limit: number): Promise<AuditEntry[]>;
    usage(): Promise<UsageSummary>;
    backups(limit: number): Promise<Backup[]>;
    restore(backupID: string): Promise<void>;
  };
  documents: {
    preview(workspaceID: string, path: string): Promise<DocumentPreview>;
  };
  terminal: {
    list(): Promise<TerminalView[]>;
    create(name: string, cwd: string): Promise<TerminalView>;
    write(sessionID: string, data: string): Promise<void>;
    resize(sessionID: string, cols: number, rows: number): Promise<void>;
    close(sessionID: string): Promise<void>;
  };
}
