// IPC 接线：CraftworkApi 的全部方法注册到 ipcMain.handle。
// 通道名与 preload 一一对应；每个 handler 做参数校验并把领域错误原样转字符串给渲染层。
import { ipcMain, type BrowserWindow } from "electron";
import { channelOf } from "../shared/channels";
import type { AppService } from "./services/app";
import type { AuditService } from "./services/audit";
import type { ChatService } from "./services/chat";
import type { MCPService } from "./services/mcp";
import type { ProviderService } from "./services/providers";
import type { SettingsService } from "./services/settings";
import type { TerminalService } from "./services/terminal";
import type { ThreadService } from "./services/threads";
import type { WorkspaceService } from "./services/workspaces";

export interface Services {
  app: AppService;
  threads: ThreadService;
  chat: ChatService;
  settings: SettingsService;
  providers: ProviderService;
  workspaces: WorkspaceService;
  audit: AuditService;
  mcp: MCPService;
  terminal: TerminalService;
}

/** 统一包装：领域异常 → 拒绝（渲染层拿到 message 展示）；同步实现自动 Promise 化。 */
function handle(method: string, impl: (...args: never[]) => unknown): void {
  ipcMain.handle(channelOf(method), async (_event, ...args: unknown[]) => {
    return await Promise.resolve((impl as unknown as (...a: unknown[]) => unknown)(...args));
  });
}

export function registerIpc(services: Services): void {
  // ---------- 应用 ----------
  handle("getInfo", () => services.app.info());
  handle("openExternal", (url) => services.app.openExternal(String(url)));
  handle("pickFiles", () => services.app.pickFiles());
  handle("pickFolder", () => services.app.pickFolder());
  handle("savePastedAttachment", (name, base64) => services.app.savePastedAttachment(String(name), String(base64)));
  handle("readAttachment", (path) => services.app.readAttachment(String(path)));
  handle("clipboardWrite", (text) => services.app.clipboardWrite(String(text)));
  handle("clipboardRead", () => services.app.clipboardRead());
  handle("showInFolder", (path) => services.app.showInFolder(String(path)));
  handle("minimiseWindow", () => services.app.minimiseWindow());
  handle("toggleMaximiseWindow", () => services.app.toggleMaximiseWindow());
  handle("closeWindow", () => services.app.closeWindow());

  // ---------- 会话 ----------
  handle("threadList", (mode, includeArchived) => services.threads.list(String(mode), Boolean(includeArchived)));
  handle("threadSearch", (mode, query) => services.threads.search(String(mode), String(query ?? "")));
  handle("threadCreate", (mode, workspaceId, title) =>
    services.threads.create(String(mode), String(workspaceId ?? ""), String(title ?? "")),
  );
  handle("threadRename", (threadId, title) => services.threads.rename(String(threadId), String(title)));
  handle("threadSetModel", (threadId, modelId) => services.threads.setModel(String(threadId), String(modelId)));
  handle("threadArchive", (threadId, archived) => services.threads.archive(String(threadId), Boolean(archived)));
  handle("threadDelete", (threadId) => services.threads.delete(String(threadId)));
  handle("threadSnapshot", (threadId) => services.threads.snapshot(String(threadId)));
  handle("threadSnapshotPage", (threadId, limit, beforeSeq) =>
    services.threads.snapshotPage(String(threadId), Number(limit), Number(beforeSeq)),
  );

  // ---------- 对话 ----------
  handle("chatCreateAndStart", (mode, workspaceId, text, attachments, modelRef) =>
    services.chat.createAndStart(
      String(mode),
      String(workspaceId ?? ""),
      String(text ?? ""),
      toStrList(attachments),
      String(modelRef ?? ""),
    ),
  );
  handle("chatSend", (threadId, text, attachments) =>
    services.chat.send(String(threadId), String(text ?? ""), toStrList(attachments)),
  );
  handle("chatSteer", (threadId, text, attachments) =>
    services.chat.steer(String(threadId), String(text ?? ""), toStrList(attachments)),
  );
  handle("chatRegenerate", (threadId) => services.chat.regenerate(String(threadId)));
  handle("chatCancel", (threadId) => services.chat.cancel(String(threadId)));
  handle("chatDecideApproval", (approvalId, allow, scope) =>
    services.chat.decideApproval(String(approvalId), Boolean(allow), scope as "once" | "session" | "workspace"),
  );

  // ---------- 设置 ----------
  handle("settingsGet", () => services.settings.get());
  handle("settingsSave", (next) => services.settings.save(next as Parameters<SettingsService["save"]>[0]).settings);

  // ---------- 提供商与模型 ----------
  handle("providerList", () => services.providers.list());
  handle("providerSave", (input) => services.providers.saveProvider(input as Parameters<ProviderService["saveProvider"]>[0]));
  handle("providerDelete", (providerId) => services.providers.deleteProvider(String(providerId)));
  handle("providerSetEnabled", (providerId, enabled) =>
    services.providers.setEnabled(String(providerId), Boolean(enabled)),
  );
  handle("providerRename", (providerId, name) => services.providers.rename(String(providerId), String(name ?? "")));
  handle("providerSetModelEnabled", (providerId, modelId, enabled) =>
    services.providers.setModelEnabled(String(providerId), String(modelId), Boolean(enabled)),
  );
  handle("providerAddModel", (providerId, model) =>
    services.providers.addModel(String(providerId), model as Parameters<ProviderService["addModel"]>[1]),
  );
  handle("providerUpdateModel", (providerId, modelId, model) =>
    services.providers.updateModel(String(providerId), String(modelId), model as Parameters<ProviderService["updateModel"]>[2]),
  );
  handle("providerRemoveModel", (providerId, modelId) =>
    services.providers.removeModel(String(providerId), String(modelId)),
  );
  handle("providerFetchModels", (providerId) => services.providers.fetchModels(String(providerId)));
  handle("providerSetKey", (providerId, key) => services.providers.setKey(String(providerId), String(key ?? "")));
  handle("providerClearKey", (providerId) => services.providers.clearKey(String(providerId)));
  handle("providerTest", (providerId) => services.providers.test(String(providerId)));
  handle("providerWarnings", () => services.providers.warnings());
  handle("catalogMeta", () => services.providers.catalogMeta());
  handle("catalogProviders", (query) => services.providers.catalogProviders(String(query ?? "")));
  handle("catalogModels", (providerId) => services.providers.catalogModels(String(providerId)));
  handle("catalogLookup", (providerId, modelId) => services.providers.lookupCatalogModel(String(providerId), String(modelId)));
  handle("catalogImport", (input) => services.providers.importFromCatalog(input as Parameters<ProviderService["importFromCatalog"]>[0]));

  // ---------- 工作区与文件 ----------
  handle("workspaceList", () => services.workspaces.list());
  handle("workspaceAdd", (path, trust) => services.workspaces.add(String(path), String(trust ?? "")));
  handle("workspaceAddProject", (name, folders, trust) =>
    services.workspaces.addProject(String(name ?? ""), toStrList(folders), String(trust ?? "")),
  );
  handle("workspaceSetActive", (workspaceId) => services.workspaces.setActive(String(workspaceId)));
  handle("workspaceSetTrust", (workspaceId, trust) => services.workspaces.setTrust(String(workspaceId), String(trust ?? "")));
  handle("workspaceRename", (workspaceId, name) => services.workspaces.rename(String(workspaceId), String(name ?? "")));
  handle("workspaceRemove", (workspaceId) => services.workspaces.remove(String(workspaceId)));
  handle("workspaceListDir", (workspaceId, folderId, rel) =>
    services.workspaces.listDir(String(workspaceId), String(folderId ?? ""), String(rel ?? "")),
  );
  handle("workspaceReadFile", (workspaceId, folderId, rel) =>
    services.workspaces.readFile(String(workspaceId), String(folderId ?? ""), String(rel ?? "")),
  );
  handle("workspaceLocateFile", (workspaceId, rel) => services.workspaces.locateFile(String(workspaceId), String(rel ?? "")));
  handle("workspaceWriteFile", (workspaceId, folderId, rel, content, expectedHash) =>
    services.workspaces.writeFile(
      String(workspaceId),
      String(folderId ?? ""),
      String(rel ?? ""),
      String(content ?? ""),
      String(expectedHash ?? ""),
    ),
  );
  handle("workspaceComputeDiff", (oldText, newText) =>
    services.workspaces.computeDiff(String(oldText ?? ""), String(newText ?? "")),
  );
  handle("workspaceGitStatuses", (workspaceId) => services.workspaces.gitStatuses(String(workspaceId)));
  handle("documentPreview", (workspaceId, folderId, rel) =>
    services.workspaces.preview(String(workspaceId), String(folderId ?? ""), String(rel ?? "")),
  );

  // ---------- 审计 / 用量 / 备份 ----------
  handle("auditList", (limit) => services.audit.list(Number(limit) || 0));
  handle("auditUsage", () => services.audit.usage());
  handle("auditBackups", (limit) => services.audit.backups(Number(limit) || 0));
  handle("auditRestore", (backupId) => services.audit.restore(String(backupId)));

  // ---------- MCP ----------
  handle("mcpList", () => services.mcp.list());
  handle("mcpAdd", (name, commandLine) => services.mcp.add(String(name ?? ""), String(commandLine ?? "")));
  handle("mcpRemove", (serverId) => services.mcp.remove(String(serverId)));
  handle("mcpConnect", (serverId) => services.mcp.connect(String(serverId)));
  handle("mcpDisconnect", (serverId) => services.mcp.disconnect(String(serverId)));

  // ---------- 终端 ----------
  handle("terminalCreate", (sessionId, cwd, rows, cols) =>
    services.terminal.create(String(sessionId), String(cwd ?? ""), Number(rows) || 24, Number(cols) || 80),
  );
  handle("terminalWrite", (sessionId, data) => services.terminal.write(String(sessionId), String(data ?? "")));
  handle("terminalResize", (sessionId, rows, cols) =>
    services.terminal.resize(String(sessionId), Number(rows) || 24, Number(cols) || 80),
  );
  handle("terminalDispose", (sessionId) => services.terminal.dispose(String(sessionId)));
}

export type { BrowserWindow };

function toStrList(value: unknown): string[] {
  return Array.isArray(value) ? value.map((v) => String(v)) : [];
}
