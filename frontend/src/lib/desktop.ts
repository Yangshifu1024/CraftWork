/**
 * 桌面端 API 适配器。
 *
 * 把 Wails3 生成的服务绑定包装成统一的 `Api` 形状：收敛可空返回、
 * 补齐空数组、把 Go 侧的切片与指针类型转换成前端期望的结构。
 *
 * 浏览器预览使用 mock.ts 的同一套接口，因此这里只做「形状对齐」，
 * 不承载业务规则；参数校验、权限与审计都在 Go 侧完成。
 */
import * as AppService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/appservice";
import * as AuditService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/auditservice";
import * as ChatService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/chatservice";
import * as DocumentService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/documentservice";
import * as MCPService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/mcpservice";
import * as ProviderService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/providerservice";
import * as SettingsService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/settingsservice";
import * as TerminalService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/terminalservice";
import * as ThreadService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/threadservice";
import * as WorkspaceService from "../../bindings/github.com/yangshifu1024/craftwork/internal/service/workspaceservice";
import type { DiffResult, ProviderView, ThreadSnapshot, UsageSummary } from "./types";

/** 一份空的用量计数，用作汇总对象里各时间段的初始值。 */
const emptyTotals = { inputTokens: 0, outputTokens: 0, cacheTokens: 0, costMicros: 0, runs: 0 };

/** 用量汇总的兜底值：后端没有返回数据时用它，保证界面各区块都有可渲染的字段。 */
const emptyUsage: UsageSummary = {
  today: { ...emptyTotals },
  week: { ...emptyTotals },
  month: { ...emptyTotals },
  byModel: [],
  byDay: [],
};

/**
 * 桌面实现：所有调用都通过 Wails3 生成的绑定进入 Go。
 *
 * 分组与 `Api` 的命名空间保持一致；每个方法尽量把「可能为 null / undefined」
 * 的返回值收敛成前端可直接消费的值，避免调用方到处判空。
 */
/** 附件图片 data URL 的读取缓存：同一路径只读一次，缩略图与预览共用。 */
const attachmentCache = new Map<string, Promise<string | null>>();

export const desktopApi = {
  /** 应用级：版本信息、打开外链、选择文件、粘贴附件、写系统剪贴板。 */
  app: {
    info: () => AppService.Info(),
    openExternal: (url) => AppService.OpenExternal(url),
    // 用户取消选择时后端返回空值，这里统一成空数组。
    pickFiles: async () => (await AppService.PickFiles()) ?? [],
    // 粘贴附件：内容以 base64 交给 Go 落盘，返回附件目录中的绝对路径。
    savePastedAttachment: (name: string, content: string) => AppService.SavePastedAttachment(name, content),
    // 附件图片预览：按路径读取内容并拼成 data URL；失败时返回 null，
    // 调用方回退成普通文件 chip。同一路径在会话内只读一次，
    // 读取失败不缓存失败结果，下次仍可重试。
    readAttachment: (path: string): Promise<string | null> => {
      const cached = attachmentCache.get(path);
      if (cached) {
        return cached;
      }
      const pending = AppService.ReadAttachment(path)
        .then((content) => (content ? `data:${content.mime};base64,${content.base64}` : null))
        .catch(() => null);
      attachmentCache.set(path, pending);
      pending.then((url) => {
        if (url === null) {
          attachmentCache.delete(path);
        }
      });
      return pending;
    },
    writeClipboard: async (text) => AppService.WriteClipboard(text),
  },
  /** 设置读写。 */
  settings: {
    get: () => SettingsService.Get(),
    save: (settings) => SettingsService.Save(settings),
  },
  /** 会话列表、搜索、增删改与分页快照。 */
  threads: {
    list: async (mode, includeArchived = false) => (await ThreadService.List(mode, includeArchived)) ?? [],
    search: async (mode, query) => (await ThreadService.Search(mode, query)) ?? [],
    create: (mode, workspaceID, title) => ThreadService.Create(mode, workspaceID, title),
    rename: (threadID, title) => ThreadService.Rename(threadID, title),
    setModel: (threadID, modelID) => ThreadService.SetModel(threadID, modelID),
    archive: (threadID, archived) => ThreadService.Archive(threadID, archived),
    remove: (threadID) => ThreadService.Delete(threadID),
    snapshotPage: async (threadID, limit, beforeSeq) =>
      // 已有会话的分页快照在后端保证非空；这里把 Wails 的可空返回收敛掉。
      (await ThreadService.SnapshotPage(threadID, limit, beforeSeq)) as ThreadSnapshot,
  },
  /** 对话：发送、运行中引导、重新生成、取消、提交审批决定。 */
  chat: {
    send: (threadID, text, attachments: string[] = []) => ChatService.Send(threadID, text, attachments),
    steer: (threadID, text, attachments: string[] = []) => ChatService.Steer(threadID, text, attachments),
    regenerate: (threadID) => ChatService.Regenerate(threadID),
    cancel: (threadID) => ChatService.Cancel(threadID),
    decideApproval: (approvalID, allow, scope) => ChatService.DecideApproval(approvalID, allow, scope),
  },
  /** 工作区：列表与选中、目录浏览、文件读写、差异与 Git 状态。 */
  workspace: {
    list: async () => (await WorkspaceService.List()) ?? [],
    pickFolder: () => WorkspaceService.PickFolder(),
    add: (path, trust) => WorkspaceService.Add(path, trust),
    addProject: (name, folders, trust) => WorkspaceService.AddProject(name, folders, trust),
    setActive: (workspaceID) => WorkspaceService.SetActive(workspaceID),
    active: () => WorkspaceService.Active(),
    setTrust: (workspaceID, trust) => WorkspaceService.SetTrust(workspaceID, trust),
    listDir: async (workspaceID, folderID, path) => (await WorkspaceService.ListDir(workspaceID, folderID, path)) ?? [],
    readFile: (workspaceID, folderID, path) => WorkspaceService.ReadFile(workspaceID, folderID, path),
    locateFile: async (workspaceID, path) => await WorkspaceService.LocateFile(workspaceID, path),
    writeFile: (workspaceID, folderID, path, content, expectedHash) =>
      WorkspaceService.WriteFile(workspaceID, folderID, path, content, expectedHash),
    computeDiff: async (oldText, newText) => {
      const raw = await WorkspaceService.ComputeDiff(oldText, newText);
      // Go 侧的切片可能是 nil，统一成数组再交给差异视图渲染。
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
      // 只保留有状态的文件，丢掉空字符串，减少前端树的判断分支。
      for (const [path, value] of Object.entries(raw)) {
        if (value) {
          result[path] = value;
        }
      }
      return result;
    },
  },
  /** 提供商：列表、密钥维护、连通性测试。 */
  providers: {
    list: async () => {
      const raw = (await ProviderService.List()) ?? [];
      // 没有模型时补空数组，界面不必再处理 undefined。
      return raw.map((provider) => ({ ...provider, models: provider.models ?? [] }));
    },
    warnings: async () => (await ProviderService.Warnings()) ?? [],
    saveProvider: async (input) => {
      const raw = await ProviderService.SaveProvider(input);
      return { ...raw, models: raw.models ?? [] } as ProviderView;
    },
    deleteProvider: (providerID) => ProviderService.DeleteProvider(providerID),
    setKey: (providerID, key) => ProviderService.SetKey(providerID, key),
    clearKey: (providerID) => ProviderService.ClearKey(providerID),
    test: (providerID) => ProviderService.Test(providerID),
    catalogMeta: () => ProviderService.CatalogMeta(),
    catalogProviders: async (query = "") => (await ProviderService.CatalogProviders(query)) ?? [],
    catalogModels: async (providerID) => (await ProviderService.CatalogModels(providerID)) ?? [],
    lookupCatalogModel: (providerID, modelID) => ProviderService.LookupCatalogModel(providerID, modelID),
    importFromCatalog: async (input) => {
      const raw = await ProviderService.ImportFromCatalog(input);
      return { ...raw, models: raw.models ?? [] } as ProviderView;
    },
  },
  /** MCP 服务器管理（当前为 M4 留桩阶段）。 */
  mcp: {
    list: async () => (await MCPService.List()) ?? [],
    add: (name, command) => MCPService.Add(name, command),
    remove: (serverID) => MCPService.Remove(serverID),
    connect: (serverID) => MCPService.Connect(serverID),
    disconnect: (serverID) => MCPService.Disconnect(serverID),
  },
  /** 审计：操作记录、用量统计、备份与恢复。 */
  audit: {
    list: async (limit) => (await AuditService.List(limit)) ?? [],
    usage: async () => {
      const raw = (await AuditService.Usage()) ?? emptyUsage;
      // 后端没有数据时返回空汇总；有数据但数组为 nil 时同样补齐。
      return {
        ...raw,
        byModel: raw.byModel ?? [],
        byDay: raw.byDay ?? [],
      } as UsageSummary;
    },
    backups: async (limit) => (await AuditService.Backups(limit)) ?? [],
    restore: (backupID) => AuditService.Restore(backupID),
  },
  /** 文档：读取 Word / Excel / PDF 的文本层预览。 */
  documents: {
    preview: (workspaceID, folderID, path) => DocumentService.Preview(workspaceID, folderID, path),
  },
  /** 终端：会话列表、创建、写入、调整尺寸与关闭。 */
  terminal: {
    list: async () => (await TerminalService.List()) ?? [],
    create: (name, cwd) => TerminalService.Create(name, cwd),
    write: (sessionID, data) => TerminalService.Write(sessionID, data),
    resize: (sessionID, cols, rows) => TerminalService.Resize(sessionID, cols, rows),
    close: (sessionID) => TerminalService.Close(sessionID),
  },
};

/**
 * 把 Wails 的 CancellablePromise 归一成普通 Promise。
 *
 * Wails 生成的绑定方法返回带取消能力的 Promise 子类型，直接把它暴露给业务代码
 * 会让类型与浏览器 Mock 不一致，因此这里用条件类型把返回类型剥到 `Awaited<R>`，
 * 调用方只需要关心「一个会 resolve 出结果的普通异步函数」。
 */
type PortMethod<T> = T extends (...args: infer A) => infer R ? (...args: A) => Promise<Awaited<R>> : never;

/**
 * 前端使用的后端端口：由桌面适配器推导，不再手写第二份签名。
 *
 * 以 `desktopApi` 为唯一事实源逐层映射：外层保留命名空间（app / threads / chat …），
 * 内层把每个方法统一成普通 Promise 签名。新增或修改后端服务后重新生成绑定即可，
 * mock.ts 与 api.ts 会按同一份 `Api` 结构做类型检查。
 */
export type Api = {
  [N in keyof typeof desktopApi]: {
    [M in keyof (typeof desktopApi)[N]]: PortMethod<(typeof desktopApi)[N][M]>;
  };
};
