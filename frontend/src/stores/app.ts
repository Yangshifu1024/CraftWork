/**
 * 应用级 store。
 *
 * 保存启动信息、设置、主题与模式、工作区列表和提供商列表，
 * 并把「设置已更新」「工作区已更新」两类广播事件同步到全局状态，
 * 同时维护 <html> 上的主题、模式、桌面平台标记，供样式选择器使用。
 */
import { create } from "zustand";
import { api } from "../lib/api";
import { onBatch } from "../lib/bus";
import type { AppInfo, Mode, ProviderView, Settings, Theme, WorkspaceView } from "../lib/types";
import { EVENT } from "../lib/types";
import { useUiStore } from "./ui";

/**
 * 默认设置。
 *
 * 后端读到完整设置之前先用这份值渲染，避免界面出现空档；
 * 字段与 Go 侧的 Settings 结构一一对应。
 */
export const DEFAULT_SETTINGS: Settings = {
  theme: "dark",
  mode: "code",
  panelWidth: 440,
  approvalLevel: "graded",
  defaultModel: "local-demo",
  fallbackModel: "",
  activeWorkspace: "",
  language: "zh-CN",
  fontSize: 13,
  codeTheme: "dark",
  terminalFontSize: 12,
  autoUpdate: true,
  notifications: true,
  updateChannel: "stable",
  telemetry: false,
};

/** 把主题写到根元素的数据属性上，CSS 变量据此切换深浅色。 */
function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

/** 把工作模式写到根元素的数据属性上，样式与布局据此区分 code / work。 */
function applyMode(mode: Mode): void {
  document.documentElement.dataset.mode = mode;
}

/** 事件总线是否已经订阅过，防止重复注册监听。 */
let eventsConnected = false;

/**
 * 订阅设置与工作区变更事件。
 *
 * Settings 与工作区变化由服务端广播；这里统一刷新，避免各组件重复订阅。
 * 设置事件直接整体覆盖本地设置并同步主题、模式；
 * 工作区事件只触发重新拉取列表，具体差异交给后端数据决定。
 */
function connectEvents(): void {
  if (eventsConnected) {
    return;
  }
  eventsConnected = true;
  onBatch((batch) => {
    for (const event of batch.events) {
      if (event.type === EVENT.settingsUpdated && event.data) {
        const settings = event.data as Settings;
        // 事件里可能缺少主题或模式字段，缺失时退回默认值，保证根元素属性始终有值。
        applyTheme((settings.theme as Theme) ?? "dark");
        applyMode((settings.mode as Mode) ?? "code");
        useAppStore.setState({
          settings,
          theme: (settings.theme as Theme) ?? "dark",
          mode: (settings.mode as Mode) ?? "code",
        });
      }
      if (event.type === EVENT.workspaceUpdated) {
        // 刷新失败不打断事件循环，界面继续使用旧的列表。
        void useAppStore
          .getState()
          .refreshWorkspaces()
          .catch(() => undefined);
      }
    }
  });
}

/**
 * 应用级状态与可用操作。
 *
 * 字段是界面读取的只读快照；所有修改都通过下面的操作完成，
 * 需要持久化的操作会顺带写入后端设置。
 */
interface AppState {
  /** 是否已完成启动初始化；完成前界面显示加载态。 */
  booted: boolean;
  /** 应用与平台信息，例如版本号、平台标识。 */
  info: AppInfo | null;
  /** 当前生效的完整设置。 */
  settings: Settings;
  /** 当前工作模式，code 为编码模式，work 为办公模式。 */
  mode: Mode;
  /** 当前主题，深色或浅色。 */
  theme: Theme;
  /** 工作区列表。 */
  workspaces: WorkspaceView[];
  /** 当前选中的工作区；没有可用工作区时为 null。 */
  activeWorkspace: WorkspaceView | null;
  /** 模型提供商列表，含模型与密钥状态。 */
  providers: ProviderView[];
  /** 启动初始化：并行读取应用信息、设置、工作区与密钥告警。 */
  boot(): Promise<void>;
  /** 切换工作模式，并持久化到设置。 */
  setMode(mode: Mode): Promise<void>;
  /** 切换主题，并持久化到设置。 */
  setTheme(theme: Theme): Promise<void>;
  /** 合并保存设置片段，成功后再以服务端返回值覆盖本地。 */
  saveSettings(patch: Partial<Settings>): Promise<void>;
  /** 重新拉取工作区列表，并保持当前选中项（不存在时退回第一条）。 */
  refreshWorkspaces(): Promise<void>;
  /** 切换当前工作区，并持久化。 */
  setActiveWorkspace(workspaceID: string): Promise<void>;
  /** 按路径添加工作区并设为当前工作区。 */
  addWorkspace(path: string, trust: string): Promise<void>;
  /** 弹出系统目录选择框，选中后以受限信任级别添加工作区。 */
  pickAndAddWorkspace(): Promise<void>;
  /** 重新拉取提供商列表。 */
  refreshProviders(): Promise<void>;
}

/** 应用级 store：设置、模式、主题、工作区与提供商的唯一入口。 */
export const useAppStore = create<AppState>((set, get) => ({
  booted: false,
  info: null,
  settings: DEFAULT_SETTINGS,
  mode: "code",
  theme: "dark",
  workspaces: [],
  activeWorkspace: null,
  providers: [],

  /**
   * 启动初始化。
   *
   * 并行读取应用信息、设置、工作区、当前工作区与密钥文件告警：
   * 前四项缺一不可，后两项失败时用兜底值继续启动，避免因为某个可选接口
   * 出错就卡在加载页。读取完成后同步根元素属性并写入状态。
   */
  async boot() {
    connectEvents();
    try {
      const [info, settings, workspaces, active, keyWarnings] = await Promise.all([
        api.app.info(),
        api.settings.get(),
        api.workspace.list(),
        // 可选接口：失败时按「没有当前工作区」处理。
        api.workspace.active().catch(() => null),
        // 密钥告警只是提示，读取失败时按空列表处理。
        api.providers.warnings().catch(() => [] as string[]),
      ]);
      applyTheme(settings.theme as Theme);
      applyMode(settings.mode as Mode);
      // macOS 与 Linux / Windows 需要不同的窗口细节样式，这里把平台写到根元素；
      // web 表示浏览器预览，不需要桌面平台标记。
      if (info.platform === "darwin") {
        document.documentElement.dataset.desktop = "darwin";
      } else if (info.platform !== "web") {
        document.documentElement.dataset.desktop = info.platform;
      }
      set({
        info,
        settings,
        mode: (settings.mode as Mode) ?? "code",
        theme: (settings.theme as Theme) ?? "dark",
        workspaces,
        // 后端没有指定当前工作区时，退回列表第一条；列表为空则为 null。
        activeWorkspace: active ?? workspaces[0] ?? null,
        booted: true,
      });
      if (keyWarnings.length > 0) {
        useUiStore.getState().toast({
          title: "密钥文件权限已自动收紧",
          description: keyWarnings.join("；"),
          tone: "warn",
        });
      }
    } catch (error) {
      console.error("应用初始化失败", error);
      // 即使失败也要标记启动完成，否则界面会一直停在加载态。
      set({ booted: true });
    }
  },

  /**
   * 切换工作模式。
   *
   * 与当前模式相同时直接返回，避免无意义的保存请求；
   * 先改根元素属性再更新状态，最后持久化到后端设置。
   */
  async setMode(mode) {
    if (mode === get().mode) {
      return;
    }
    applyMode(mode);
    set({ mode });
    await get().saveSettings({ mode });
  },

  /** 切换主题：先让样式立即生效，再更新状态并持久化。 */
  async setTheme(theme) {
    applyTheme(theme);
    set({ theme });
    await get().saveSettings({ theme });
  },

  /**
   * 保存设置片段。
   *
   * 先与现有设置合并并立即生效（主题、模式要马上看到变化），
   * 再把完整设置交给后端；成功后用后端返回值覆盖本地，
   * 保证界面显示的是真正写入数据库的内容。失败只记录日志，保留本地改动。
   */
  async saveSettings(patch) {
    const next = { ...get().settings, ...patch };
    if (patch.theme) {
      applyTheme(patch.theme as Theme);
    }
    if (patch.mode) {
      applyMode(patch.mode as Mode);
    }
    set({
      settings: next,
      theme: (next.theme as Theme) ?? get().theme,
      mode: (next.mode as Mode) ?? get().mode,
    });
    try {
      const saved = await api.settings.save(next);
      set({ settings: saved });
    } catch (error) {
      console.error("保存设置失败", error);
    }
  },

  /**
   * 重新拉取工作区列表。
   *
   * 尽量保持当前选中项：按 id 在新列表里找回；
   * 若该工作区已被移除，则退回新列表第一条，列表为空时为 null。
   */
  async refreshWorkspaces() {
    const workspaces = await api.workspace.list();
    const currentID = get().activeWorkspace?.id;
    const active = workspaces.find((workspace) => workspace.id === currentID) ?? workspaces[0] ?? null;
    set({ workspaces, activeWorkspace: active });
  },

  /**
   * 切换当前工作区。
   *
   * 以后端返回的工作区对象为准（可能带上最新路径与信任级别），
   * 然后写入设置并刷新列表，让顶部选择器与侧边栏保持一致。
   */
  async setActiveWorkspace(workspaceID) {
    const workspace = await api.workspace.setActive(workspaceID);
    set({ activeWorkspace: workspace });
    await get().saveSettings({ activeWorkspace: workspace.id });
    await get().refreshWorkspaces();
  },

  /**
   * 添加工作区。
   *
   * 新工作区添加后直接设为当前工作区，并持久化这个选择。
   * 信任级别由调用方给出，受限工作区的风险策略由后端执行。
   */
  async addWorkspace(path, trust) {
    const workspace = await api.workspace.add(path, trust);
    set({ activeWorkspace: workspace });
    await get().saveSettings({ activeWorkspace: workspace.id });
    await get().refreshWorkspaces();
  },

  /**
   * 弹出系统目录选择框并添加工作区。
   *
   * 用户取消选择时返回空路径，此时什么都不做；
   * 通过选择框新增的工作区默认使用受限信任级别，需要时再由用户提升。
   */
  async pickAndAddWorkspace() {
    const path = await api.workspace.pickFolder();
    if (path) {
      await get().addWorkspace(path, "restricted");
    }
  },

  /** 重新拉取提供商列表，设置页据此刷新密钥与模型状态。 */
  async refreshProviders() {
    const providers = await api.providers.list();
    set({ providers });
  },
}));
