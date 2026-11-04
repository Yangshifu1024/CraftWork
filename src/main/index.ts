// 应用组合根：进程装配，不含业务逻辑；控制启动顺序（有依赖）与退出清理。
// 启动顺序：单实例锁 → 数据目录 → 存储 → 密钥 → 事件总线 → 策略/工具 → 服务 → 窗口 → 通知。
import { app, BrowserWindow, Notification } from "electron";
import { existsSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { PushChannels } from "../shared/channels";
import type { Batch } from "../shared/events";
import { Bus } from "./core/bus";
import { Recorder } from "./core/record";
import { DB } from "./store/db";
import { interruptStaleRuns } from "./store/runs";
import { appendEvent } from "./store/events";
import { backfillThreadWorkspaces } from "./store/workspaces";
import { SecretStore } from "./core/secret";
import { Engine } from "./policy/engine";
import { Registry } from "./tools/registry";
import { fsList, fsRead, fsGlob, fsGrep, fsWrite, fsEdit, applyPatch } from "./tools/fs";
import { shellExec, gitStatus, gitDiff, gitLog, gitCommit, docRead, docEdit, xlsxSetCell, todoWrite } from "./tools/git_shell";
import { Catalog } from "./model/catalog";
import { ModelFactory } from "./model/factory";
import { Grants } from "./agent/grants";
import { Approvals } from "./agent/approvals";
import { Runner } from "./agent/runner";
import { ensureConversationWorkspace, WorkspaceService } from "./services/workspaces";
import { SettingsService } from "./services/settings";
import { ThreadService } from "./services/threads";
import { ChatService } from "./services/chat";
import { ProviderService } from "./services/providers";
import { AuditService } from "./services/audit";
import { AppService } from "./services/app";
import { MCPService } from "./services/mcp";
import { registerIpc, type Services } from "./ipc";
import { parseLevel } from "./policy/engine";

// 应用常量
const APP_NAME = "CraftWork";
const APP_VERSION = "0.1.0";
const WINDOW_MIN_WIDTH = 960;
const WINDOW_MIN_HEIGHT = 640;
const WINDOW_DEFAULT_WIDTH = 1440;
const WINDOW_DEFAULT_HEIGHT = 900;

let mainWindow: BrowserWindow | null = null;

/** 数据目录：CRAFTWORK_HOME（测试/开发隔离）优先，否则 ~/.craftwork。 */
function resolveDataDir(): string {
  const override = process.env["CRAFTWORK_HOME"];
  const dir = override && override.trim() ? override.trim() : join(homedir(), ".craftwork");
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  return dir;
}

function createWindow(preloadPath: string): BrowserWindow {
  const win = new BrowserWindow({
    width: WINDOW_DEFAULT_WIDTH,
    height: WINDOW_DEFAULT_HEIGHT,
    minWidth: WINDOW_MIN_WIDTH,
    minHeight: WINDOW_MIN_HEIGHT,
    show: false,
    title: APP_NAME,
    titleBarStyle: "hidden",
    trafficLightPosition: { x: 12, y: 16 },
    backgroundColor: "#161616",
    webPreferences: {
      preload: preloadPath,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      spellcheck: false,
    },
  });

  // 主窗口一律拒绝 popup；外链经 AppService 确认通道打开
  win.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  win.webContents.on("will-navigate", (event, url) => {
    // 阻止渲染层发起的任何导航（页面是本地资源）
    if (!url.startsWith("file://") && !url.startsWith("http://localhost") && !url.startsWith("http://127.0.0.1")) {
      event.preventDefault();
    }
  });

  win.on("ready-to-show", () => win.show());
  win.on("closed", () => {
    if (mainWindow === win) mainWindow = null;
  });

  if (!app.isPackaged && process.env["ELECTRON_RENDERER_URL"]) {
    void win.loadURL(process.env["ELECTRON_RENDERER_URL"]);
  } else {
    void win.loadFile(join(__dirname, "../renderer/index.html"));
  }
  return win;
}

async function bootstrap(): Promise<void> {
  // 单实例锁：在任何数据打开之前（第二个实例连数据库都不该碰）
  // CRAFTWORK_HOME 隔离必须先于单实例锁：userData（含 localStorage）与锁目录一并指向隔离目录，
  // 否则多个 dev/冒烟实例共享同一份浏览器存储，草稿槽等本地状态互相污染。
  const homeOverride = process.env["CRAFTWORK_HOME"]?.trim();
  if (homeOverride) {
    mkdirSync(homeOverride, { recursive: true, mode: 0o700 });
    app.setPath("userData", homeOverride);
  }
  if (!app.requestSingleInstanceLock()) {
    app.quit();
    return;
  }
  app.on("second-instance", () => {
    if (mainWindow) {
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.focus();
    }
  });

  const dataDir = resolveDataDir();
  mkdirSync(join(dataDir, "attachments"), { recursive: true, mode: 0o700 });

  // 存储：上次退出的运行与挂起审批统一收尾
  const db = DB.open(join(dataDir, "craftwork.db"));
  dbRef = db;
  const interrupted = interruptStaleRuns(db, (rec) => {
    // 事件总线尚未建立：这里只需要落库
    appendEvent(db, rec);
  });
  if (interrupted > 0) {
    console.log(`[craftwork] 已收尾 ${interrupted} 个上次中断的运行`);
  }

  // 密钥：损坏不阻止启动（用户在设置页重填）
  let secrets: SecretStore;
  try {
    secrets = SecretStore.open(dataDir);
  } catch {
    secrets = SecretStore.newEmpty(dataDir);
  }
  for (const warning of secrets.warnings()) {
    console.warn(`[craftwork] ${warning}`);
  }

  // 事件总线：窗口未创建前事件丢弃（推送目标为空即可，批量回放由渲染层主动拉快照）
  const bus = new Bus((batch: Batch) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send(PushChannels.Events, batch satisfies Batch);
    }
  });
  busRef = bus;
  const recorder = new Recorder(db, bus);

  // 策略与工具
  const engine = new Engine();
  const registry = new Registry({ backupDir: join(dataDir, "backups"), db });
  registry.register(fsList);
  registry.register(fsRead);
  registry.register(fsGlob);
  registry.register(fsGrep);
  registry.register(fsWrite);
  registry.register(fsEdit);
  registry.register(applyPatch);
  registry.register(shellExec);
  registry.register(gitStatus);
  registry.register(gitDiff);
  registry.register(gitLog);
  registry.register(gitCommit);
  registry.register(docRead);
  registry.register(docEdit);
  registry.register(xlsxSetCell);
  registry.register(todoWrite);

  // 模型与 Agent
  const catalog = new Catalog(db);
  const models = new ModelFactory(catalog, {
    getFor: (providerId, envNames) => secrets.getFor(providerId, envNames),
  });
  const grants = new Grants(db);
  const approvals = new Approvals(db, recorder, grants);
  const runner = new Runner({
    db,
    records: recorder,
    registry,
    engine,
    approvals,
    models,
    attachmentDir: join(dataDir, "attachments"),
    dataDir,
  });
  runner.setNotifier((threadId, title, message) => {
    notify(threadId, title, message);
  });

  // 服务
  const settings = new SettingsService(db, recorder, engine, catalog);
  const providers = new ProviderService(db, secrets, catalog);
  providers.setFactory(models);
  models.setFallbackResolver(() => settings.get().fallbackModel);
  runner.setMaxIterationsResolver(() => settings.get().maxIterations);
  const threads = new ThreadService(db, recorder, catalog, settings);
  const chat = new ChatService(db, recorder, runner, approvals, catalog);
  const workspaces = new WorkspaceService(db, recorder, join(dataDir, "backups"));
  const audit = new AuditService(db);
  const appService = new AppService(APP_VERSION, dataDir);
  const mcp = new MCPService(db);

  // 内置临时会话目录（首启保证可聊）+ 历史无工作区会话归入临时目录
  try {
    const conversation = ensureConversationWorkspace(db, dataDir);
    backfillThreadWorkspaces(db, conversation.id);
  } catch (err) {
    console.warn(`[craftwork] 临时会话目录初始化失败：${String(err)}`);
  }

  // 恢复设置中的审批档位（活动工作区只是 UI 偏好，不进策略状态）
  engine.setLevel(parseLevel(settings.get().approvalLevel));

  // IPC
  const services: Services = { app: appService, threads, chat, settings, providers, workspaces, audit, mcp };
  registerIpc(services);

  // 窗口
  // electron-vite 的 preload 产物是 ESM（index.mjs）
  const preloadPath = join(__dirname, "../preload/index.mjs");
  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      mainWindow = createWindow(preloadPath);
    }
  });
  mainWindow = createWindow(preloadPath);
}

/** 系统通知：窗口失焦时才发；threadId 作为通知分组标识，点击聚焦窗口。 */
function notify(threadId: string, title: string, message: string): void {
  if (mainWindow && !mainWindow.isDestroyed() && mainWindow.isFocused()) return;
  if (!Notification.isSupported()) return;
  const notification = new Notification({ title, body: message, silent: false });
  notification.on("click", () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.show();
      mainWindow.focus();
    }
    void threadId;
  });
  notification.show();
}

app.whenReady().then(() => {
  // 开发态 Dock 图标：assets/logo.png。打包产物由 electron-builder 从该 png 生成的 icns 提供，无需手动设置。
  if (process.platform === "darwin" && !app.isPackaged) {
    const iconPath = join(app.getAppPath(), "assets", "logo.png");
    if (existsSync(iconPath)) app.dock?.setIcon(iconPath);
  }
  bootstrap().catch((err) => {
    console.error("[craftwork] 启动失败：", err);
    app.quit();
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

app.on("will-quit", () => {
  busRef?.close();
  try {
    dbRef?.close();
  } catch {
    // 已关闭
  }
});

// 模块级引用（bootstrap 中赋值，退出时清理）
let busRef: Bus | null = null;
let dbRef: DB | null = null;
