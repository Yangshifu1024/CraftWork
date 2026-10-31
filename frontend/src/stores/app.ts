import { create } from "zustand";
import { api } from "../lib/api";
import type { AppInfo, Mode, ProviderView, Settings, Theme, WorkspaceView } from "../lib/types";
import { useUiStore } from "./ui";

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

function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}

function applyMode(mode: Mode): void {
  document.documentElement.dataset.mode = mode;
}

interface AppState {
  booted: boolean;
  info: AppInfo | null;
  settings: Settings;
  mode: Mode;
  theme: Theme;
  workspaces: WorkspaceView[];
  activeWorkspace: WorkspaceView | null;
  providers: ProviderView[];
  boot(): Promise<void>;
  setMode(mode: Mode): Promise<void>;
  setTheme(theme: Theme): Promise<void>;
  saveSettings(patch: Partial<Settings>): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  setActiveWorkspace(workspaceID: string): Promise<void>;
  addWorkspace(path: string, trust: string): Promise<void>;
  pickAndAddWorkspace(): Promise<void>;
  refreshProviders(): Promise<void>;
}

export const useAppStore = create<AppState>((set, get) => ({
  booted: false,
  info: null,
  settings: DEFAULT_SETTINGS,
  mode: "code",
  theme: "dark",
  workspaces: [],
  activeWorkspace: null,
  providers: [],

  async boot() {
    try {
      const [info, settings, workspaces, active, keyWarnings] = await Promise.all([
        api.app.info(),
        api.settings.get(),
        api.workspace.list(),
        api.workspace.active().catch(() => null),
        api.providers.warnings().catch(() => [] as string[]),
      ]);
      applyTheme(settings.theme as Theme);
      applyMode(settings.mode as Mode);
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
      set({ booted: true });
    }
  },

  async setMode(mode) {
    if (mode === get().mode) {
      return;
    }
    applyMode(mode);
    set({ mode });
    await get().saveSettings({ mode });
  },

  async setTheme(theme) {
    applyTheme(theme);
    set({ theme });
    await get().saveSettings({ theme });
  },

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

  async refreshWorkspaces() {
    const workspaces = await api.workspace.list();
    const currentID = get().activeWorkspace?.id;
    const active = workspaces.find((workspace) => workspace.id === currentID) ?? workspaces[0] ?? null;
    set({ workspaces, activeWorkspace: active });
  },

  async setActiveWorkspace(workspaceID) {
    const workspace = await api.workspace.setActive(workspaceID);
    set({ activeWorkspace: workspace });
    await get().saveSettings({ activeWorkspace: workspace.id });
    await get().refreshWorkspaces();
  },

  async addWorkspace(path, trust) {
    const workspace = await api.workspace.add(path, trust);
    set({ activeWorkspace: workspace });
    await get().saveSettings({ activeWorkspace: workspace.id });
    await get().refreshWorkspaces();
  },

  async pickAndAddWorkspace() {
    const path = await api.workspace.pickFolder();
    if (path) {
      await get().addWorkspace(path, "restricted");
    }
  },

  async refreshProviders() {
    const providers = await api.providers.list();
    set({ providers });
  },
}));
