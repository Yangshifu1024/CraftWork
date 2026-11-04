// preload 桥：把 CraftworkApi 的全部方法以 invoke 通道暴露给渲染层。
// contextIsolation 开启；渲染层不持有任何 Node 能力，所有系统操作经此桥转主进程。
import { contextBridge, ipcRenderer } from "electron";
import { channelOf, PushChannels, type CraftworkApi } from "../shared/channels";

type ApiMethod = keyof CraftworkApi;

// 依据接口自动生成桥对象：每个方法一个 invoke 通道
function bridge(): CraftworkApi {
  const api = {} as Record<string, unknown>;
  const methods: ApiMethod[] = [
    "getInfo",
    "openExternal",
    "pickFiles",
    "pickFolder",
    "savePastedAttachment",
    "readAttachment",
    "clipboardWrite",
    "clipboardRead",
    "showInFolder",
    "minimiseWindow",
    "toggleMaximiseWindow",
    "closeWindow",
    "threadList",
    "threadSearch",
    "threadCreate",
    "threadRename",
    "threadSetModel",
    "threadArchive",
    "threadDelete",
    "threadSnapshot",
    "threadSnapshotPage",
    "chatCreateAndStart",
    "chatSend",
    "chatSteer",
    "chatRegenerate",
    "chatCancel",
    "chatDecideApproval",
    "settingsGet",
    "settingsSave",
    "providerList",
    "providerSave",
    "providerDelete",
    "providerSetEnabled",
    "providerRename",
    "providerSetModelEnabled",
    "providerAddModel",
    "providerUpdateModel",
    "providerRemoveModel",
    "providerSetKey",
    "providerClearKey",
    "providerTest",
    "providerWarnings",
    "catalogMeta",
    "catalogProviders",
    "catalogModels",
    "catalogLookup",
    "catalogImport",
    "workspaceList",
    "workspaceAdd",
    "workspaceAddProject",
    "workspaceSetActive",
    "workspaceSetTrust",
    "workspaceRemove",
    "workspaceListDir",
    "workspaceReadFile",
    "workspaceLocateFile",
    "workspaceWriteFile",
    "workspaceComputeDiff",
    "workspaceGitStatuses",
    "documentPreview",
    "auditList",
    "auditUsage",
    "auditBackups",
    "auditRestore",
    "mcpList",
    "mcpAdd",
    "mcpRemove",
    "mcpConnect",
    "mcpDisconnect",
    "terminalCreate",
    "terminalWrite",
    "terminalResize",
    "terminalDispose",
  ];
  for (const method of methods) {
    api[method] = (...args: unknown[]) => ipcRenderer.invoke(channelOf(method), ...args);
  }
  return api as unknown as CraftworkApi;
}

contextBridge.exposeInMainWorld("craftwork", bridge());

/** 渲染层事件订阅助手（挂到 window.craftworkPush）。 */
contextBridge.exposeInMainWorld("craftworkPush", {
  onEvents: (callback: (batch: unknown) => void): (() => void) => {
    const handler = (_e: unknown, payload: unknown): void => callback(payload);
    ipcRenderer.on(PushChannels.Events, handler);
    return () => ipcRenderer.removeListener(PushChannels.Events, handler);
  },
  onTerminalData: (callback: (payload: unknown) => void): (() => void) => {
    const handler = (_e: unknown, payload: unknown): void => callback(payload);
    ipcRenderer.on(PushChannels.TerminalData, handler);
    return () => ipcRenderer.removeListener(PushChannels.TerminalData, handler);
  },
  onTerminalExit: (callback: (payload: unknown) => void): (() => void) => {
    const handler = (_e: unknown, payload: unknown): void => callback(payload);
    ipcRenderer.on(PushChannels.TerminalExit, handler);
    return () => ipcRenderer.removeListener(PushChannels.TerminalExit, handler);
  },
  onToast: (callback: (payload: unknown) => void): (() => void) => {
    const handler = (_e: unknown, payload: unknown): void => callback(payload);
    ipcRenderer.on(PushChannels.Toast, handler);
    return () => ipcRenderer.removeListener(PushChannels.Toast, handler);
  },
});
