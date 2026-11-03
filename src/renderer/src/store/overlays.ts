// 浮层状态：命令面板 / 设置 / 数据面板（互斥开关 + Esc 层级）。
import { create } from "zustand";

export type OverlayKind = "none" | "palette" | "settings" | "data" | "confirm" | "prompt";

interface OverlaysState {
  overlay: OverlayKind;
  /** 确认框参数（danger 变体等） */
  confirm: { title: string; message: string; danger?: boolean; onOk(): void } | null;
  /** 输入框参数（重命名等） */
  prompt: { title: string; message: string; value: string; onOk(value: string): void } | null;
  open(kind: OverlayKind): void;
  close(): void;
  openConfirm(opts: { title: string; message: string; danger?: boolean; onOk(): void }): void;
  openPrompt(opts: { title: string; message: string; value: string; onOk(value: string): void }): void;
}

export const useOverlays = create<OverlaysState>((set) => ({
  overlay: "none",
  confirm: null,
  prompt: null,
  open(kind) {
    set({ overlay: kind });
  },
  close() {
    set({ overlay: "none", confirm: null, prompt: null });
  },
  openConfirm(opts) {
    set({ confirm: opts, overlay: "confirm" });
  },
  openPrompt(opts) {
    set({ prompt: opts, overlay: "prompt" });
  },
}));
