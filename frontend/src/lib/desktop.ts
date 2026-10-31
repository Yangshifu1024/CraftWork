import * as AppService from "../../bindings/craftwork/internal/service/appservice";
import * as AuditService from "../../bindings/craftwork/internal/service/auditservice";
import * as ChatService from "../../bindings/craftwork/internal/service/chatservice";
import * as DocumentService from "../../bindings/craftwork/internal/service/documentservice";
import * as ProviderService from "../../bindings/craftwork/internal/service/providerservice";
import * as SettingsService from "../../bindings/craftwork/internal/service/settingsservice";
import * as TerminalService from "../../bindings/craftwork/internal/service/terminalservice";
import * as ThreadService from "../../bindings/craftwork/internal/service/threadservice";
import * as WorkspaceService from "../../bindings/craftwork/internal/service/workspaceservice";
import type { Api } from "./api-types";
import type { DiffResult, ThreadSnapshot, UsageSummary } from "./types";

const emptyTotals = { inputTokens: 0, outputTokens: 0, cacheTokens: 0, costMicros: 0, runs: 0 };

const emptyUsage: UsageSummary = {
  today: { ...emptyTotals },
  week: { ...emptyTotals },
  month: { ...emptyTotals },
  byModel: [],
  byDay: [],
};

/** 桌面实现：所有调用都通过 Wails3 生成的绑定进入 Go。 */
export const desktopApi: Api = {
  app: {
    info: () => AppService.Info(),
    openExternal: (url) => AppService.OpenExternal(url),
    pickFiles: async () => (await AppService.PickFiles()) ?? [],
    writeClipboard: async (text) => AppService.WriteClipboard(text),
    readClipboard: async () => AppService.ReadClipboard(),
  },
  settings: {
    get: () => SettingsService.Get(),
    save: (settings) => SettingsService.Save(settings),
  },
  threads: {
    list: async (mode, includeArchived = false) =>
      (await ThreadService.List(mode, includeArchived)) ?? [],
    search: async (mode, query) => (await ThreadService.Search(mode, query)) ?? [],
    create: (mode, workspaceID, title) => ThreadService.Create(mode, workspaceID, title),
    rename: (threadID, title) => ThreadService.Rename(threadID, title),
    setModel: (threadID, modelID) => ThreadService.SetModel(threadID, modelID),
    archive: (threadID, archived) => ThreadService.Archive(threadID, archived),
    remove: (threadID) => ThreadService.Delete(threadID),
    snapshot: async (threadID) =>
      (await ThreadService.Snapshot(threadID)) as unknown as ThreadSnapshot,
  },
  chat: {
    send: (threadID, text, attachments = []) => ChatService.Send(threadID, text, attachments),
    regenerate: (threadID) => ChatService.Regenerate(threadID),
    cancel: (threadID) => ChatService.Cancel(threadID),
    decideApproval: (approvalID, allow, scope) =>
      ChatService.DecideApproval(approvalID, allow, scope),
  },
  workspace: {
    list: async () => (await WorkspaceService.List()) ?? [],
    pickFolder: () => WorkspaceService.PickFolder(),
    add: (path, trust) => WorkspaceService.Add(path, trust),
    setActive: (workspaceID) => WorkspaceService.SetActive(workspaceID),
    active: () => WorkspaceService.Active(),
    setTrust: (workspaceID, trust) => WorkspaceService.SetTrust(workspaceID, trust),
    remove: (workspaceID) => WorkspaceService.Remove(workspaceID),
    listDir: async (workspaceID, path) =>
      (await WorkspaceService.ListDir(workspaceID, path)) ?? [],
    readFile: (workspaceID, path) => WorkspaceService.ReadFile(workspaceID, path),
    writeFile: (workspaceID, path, content, expectedHash) =>
      WorkspaceService.WriteFile(workspaceID, path, content, expectedHash),
    computeDiff: async (oldText, newText) => {
      const raw = await WorkspaceService.ComputeDiff(oldText, newText);
      return {
        additions: raw.additions,
        deletions: raw.deletions,
        truncated: raw.truncated,
        hunks: (raw.hunks ?? []).map((hunk) => ({
          header: hunk.header,
          lines: hunk.lines ?? [],
        })),
      } as DiffResult;
    },
    gitStatuses: async (workspaceID) => {
      const raw = (await WorkspaceService.GitStatuses(workspaceID)) ?? {};
      const result: Record<string, string> = {};
      for (const [path, value] of Object.entries(raw)) {
        if (value) {
          result[path] = value;
        }
      }
      return result;
    },
  },
  providers: {
    list: async () => {
      const raw = (await ProviderService.List()) ?? [];
      return raw.map((provider) => ({ ...provider, models: provider.models ?? [] }));
    },
    warnings: async () => (await ProviderService.Warnings()) ?? [],
    setKey: (providerID, key) => ProviderService.SetKey(providerID, key),
    clearKey: (providerID) => ProviderService.ClearKey(providerID),
    test: (providerID) => ProviderService.Test(providerID),
  },
  audit: {
    list: async (limit) => (await AuditService.List(limit)) ?? [],
    usage: async () => {
      const raw = (await AuditService.Usage()) ?? emptyUsage;
      return {
        ...raw,
        byModel: raw.byModel ?? [],
        byDay: raw.byDay ?? [],
      } as UsageSummary;
    },
    backups: async (limit) => (await AuditService.Backups(limit)) ?? [],
    restore: (backupID) => AuditService.Restore(backupID),
  },
  documents: {
    preview: (workspaceID, path) => DocumentService.Preview(workspaceID, path),
  },
  terminal: {
    list: async () => (await TerminalService.List()) ?? [],
    create: (name, cwd) => TerminalService.Create(name, cwd),
    write: (sessionID, data) => TerminalService.Write(sessionID, data),
    resize: (sessionID, cols, rows) => TerminalService.Resize(sessionID, cols, rows),
    close: (sessionID) => TerminalService.Close(sessionID),
  },
};
