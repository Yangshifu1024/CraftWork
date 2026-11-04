// 浮层状态：命令面板 / 设置 / 数据面板（互斥开关 + Esc 层级）+ composer 配置 picker（排他）。
import { create } from "zustand";

export type OverlayKind = "none" | "palette" | "settings" | "confirm" | "prompt";
export type PickerKind = "model" | "level";
export type SettingsSection = "general" | "appearance" | "providers" | "permissions" | "usage" | "audit" | "backups";

interface OverlaysState {
  overlay: OverlayKind;
  /** composer 配置 picker（模型 / 审批档位），同一时刻只开一个。 */
  picker: PickerKind | null;
  /** 打开设置时要定位的分区（一次性意图，读取后由设置页消费）。 */
  settingsSection: SettingsSection | null;
  /** 确认框参数（danger 变体等） */
  confirm: { title: string; message: string; danger?: boolean; onOk(): void } | null;
  /** 输入框参数（重命名等） */
  prompt: { title: string; message: string; value: string; onOk(value: string): void } | null;
  open(kind: OverlayKind, section?: SettingsSection): void;
  close(): void;
  openConfirm(opts: { title: string; message: string; danger?: boolean; onOk(): void }): void;
  openPrompt(opts: { title: string; message: string; value: string; onOk(value: string): void }): void;
  openPicker(kind: PickerKind): void;
  closePicker(): void;
}

export const useOverlays = create<OverlaysState>((set) => ({
  overlay: "none",
  picker: null,
  settingsSection: null,
  confirm: null,
  prompt: null,
  open(kind, section) {
    // 全屏浮层与 composer picker 互斥：打开任一浮层时收起 picker。
    set({ overlay: kind, picker: null, settingsSection: section ?? null });
  },
  close() {
    set({ overlay: "none", confirm: null, prompt: null, picker: null, settingsSection: null });
  },
  openConfirm(opts) {
    set({ confirm: opts, overlay: "confirm", picker: null });
  },
  openPrompt(opts) {
    set({ prompt: opts, overlay: "prompt", picker: null });
  },
  openPicker(kind) {
    set({ picker: kind });
  },
  closePicker() {
    set({ picker: null });
  },
}));
