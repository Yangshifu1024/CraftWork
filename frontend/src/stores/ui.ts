import { create } from "zustand";

export interface Toast {
  id: string;
  title: string;
  description?: string;
  tone?: "info" | "ok" | "warn" | "danger";
  onClick?: () => void;
}

export interface ConfirmRequest {
  title: string;
  description?: string;
  detail?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
}

export interface PromptRequest {
  title: string;
  label?: string;
  value?: string;
  placeholder?: string;
  confirmText?: string;
}

export type SettingsTab = "general" | "mode" | "models" | "tools" | "mcp" | "appearance" | "advanced";
export type DataTab = "audit" | "usage" | "backup";

interface UiState {
  toasts: Toast[];
  paletteOpen: boolean;
  settingsOpen: boolean;
  settingsTab: SettingsTab;
  dataOpen: boolean;
  dataTab: DataTab;
  confirm: ConfirmRequest | null;
  confirmResolver: ((value: boolean) => void) | null;
  prompt: PromptRequest | null;
  promptResolver: ((value: string | null) => void) | null;
  toast(toast: Omit<Toast, "id">): void;
  dismissToast(id: string): void;
  openPalette(): void;
  closePalette(): void;
  openSettings(tab?: SettingsTab): void;
  closeSettings(): void;
  openData(tab?: DataTab): void;
  closeData(): void;
  askConfirm(request: ConfirmRequest): Promise<boolean>;
  resolveConfirm(value: boolean): void;
  askPrompt(request: PromptRequest): Promise<string | null>;
  resolvePrompt(value: string | null): void;
}

export const useUiStore = create<UiState>((set, get) => ({
  toasts: [],
  paletteOpen: false,
  settingsOpen: false,
  settingsTab: "general",
  dataOpen: false,
  dataTab: "audit",
  confirm: null,
  confirmResolver: null,
  prompt: null,
  promptResolver: null,

  toast(toast) {
    const id = `toast_${Date.now()}_${Math.random().toString(16).slice(2, 6)}`;
    set((state) => ({ toasts: [...state.toasts, { ...toast, id }] }));
    setTimeout(() => get().dismissToast(id), 4200);
  },

  dismissToast(id) {
    set((state) => ({ toasts: state.toasts.filter((toast) => toast.id !== id) }));
  },

  openPalette() {
    set({ paletteOpen: true });
  },
  closePalette() {
    set({ paletteOpen: false });
  },
  openSettings(tab) {
    set({ settingsOpen: true, settingsTab: tab ?? get().settingsTab });
  },
  closeSettings() {
    set({ settingsOpen: false });
  },
  openData(tab) {
    set({ dataOpen: true, dataTab: tab ?? "audit" });
  },
  closeData() {
    set({ dataOpen: false });
  },

  askConfirm(request) {
    return new Promise<boolean>((resolve) => {
      set({ confirm: request, confirmResolver: resolve });
    });
  },

  resolveConfirm(value) {
    const resolver = get().confirmResolver;
    set({ confirm: null, confirmResolver: null });
    resolver?.(value);
  },

  askPrompt(request) {
    return new Promise<string | null>((resolve) => {
      set({ prompt: request, promptResolver: resolve });
    });
  },

  resolvePrompt(value) {
    const resolver = get().promptResolver;
    set({ prompt: null, promptResolver: null });
    resolver?.(value);
  },
}));
