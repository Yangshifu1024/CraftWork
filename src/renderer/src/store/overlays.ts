// 浮层状态：命令面板 / 设置（互斥全屏浮层）+ 确认框 / 输入框（独立可叠加层，
// 出现在设置页等全屏浮层之上，不顶掉底层浮层）+ composer 配置 picker（排他）。
import { create } from "zustand";

export type OverlayKind = "none" | "palette" | "settings";
export type PickerKind = "model" | "level";
export type SettingsSection = "general" | "appearance" | "providers" | "permissions" | "usage" | "audit" | "backups";

interface OverlaysState {
  overlay: OverlayKind;
  /** composer 配置 picker（模型 / 审批档位），同一时刻只开一个。 */
  picker: PickerKind | null;
  /** 打开设置时要定位的分区（一次性意图，读取后由设置页消费）。 */
  settingsSection: SettingsSection | null;
  /** 确认框参数（danger 变体等）；独立于全屏浮层，可叠加在其上。 */
  confirm: { title: string; message: string; danger?: boolean; onOk(): void } | null;
  /** 输入框参数（重命名等）；独立于全屏浮层，可叠加在其上。 */
  prompt: { title: string; message: string; value: string; onOk(value: string): void } | null;
  /** 「创建项目」对话框（名称/源文件夹多选/信任确认两视图）。 */
  createProject: boolean;
  open(kind: OverlayKind, section?: SettingsSection): void;
  close(): void;
  closeConfirm(): void;
  closePrompt(): void;
  openCreateProject(): void;
  closeCreateProject(): void;
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
  createProject: false,
  open(kind, section) {
    // 全屏浮层与 composer picker 互斥：打开任一浮层时收起 picker。
    set({ overlay: kind, picker: null, settingsSection: section ?? null });
  },
  close() {
    set({ overlay: "none", confirm: null, prompt: null, picker: null, settingsSection: null });
  },
  // 确认框 / 输入框为独立层，只清自身；全局单例：新请求覆盖旧请求并清掉另一类未决状态
  // （旧回调按丢弃收尾，与既有语义一致）。
  closeConfirm() {
    set({ confirm: null });
  },
  closePrompt() {
    set({ prompt: null });
  },
  openConfirm(opts) {
    set({ confirm: opts, prompt: null, picker: null });
  },
  openPrompt(opts) {
    set({ prompt: opts, confirm: null, picker: null });
  },
  openPicker(kind) {
    set({ picker: kind });
  },
  closePicker() {
    set({ picker: null });
  },
  openCreateProject() {
    set({ createProject: true, picker: null });
  },
  closeCreateProject() {
    set({ createProject: false });
  },
}));
