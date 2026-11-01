/**
 * 全局界面状态：通知条、命令面板与设置 / 数据面板的开关，以及「确认框」「输入框」这类
 * 以 Promise 形式被调用的弹层。
 *
 * 弹层的思路：调用方 `await useUiStore.getState().askConfirm(...)` 就能拿到用户的选择，
 * store 负责保存请求与 resolve 函数，常驻挂载的 ConfirmDialog / PromptDialog 负责渲染。
 */
import { create } from "zustand";

/**
 * 一条浮层通知。
 *
 * - `id` 由 store 生成，用于区分同一时间出现的多条通知；
 * - `tone` 决定图标与配色，缺省按 info 处理；
 * - `onClick` 存在时整条通知变成可点击区域（例如「查看」跳转）。
 */
export interface Toast {
  /** 通知唯一标识，由 `toast()` 自动生成。 */
  id: string;
  /** 标题，一行内说明发生了什么。 */
  title: string;
  /** 补充说明，可省略。 */
  description?: string;
  /** 语气：普通信息、成功、警告、错误。 */
  tone?: "info" | "ok" | "warn" | "danger";
  /** 点击通知时的回调；存在时通知会显示为可操作样式。 */
  onClick?: () => void;
}

/**
 * 一次确认框请求。
 *
 * `title` 是必填的主标题；`detail` 用于展示路径、文件名等单行细节；
 * `danger` 为真时确认按钮使用危险样式，提示这是破坏性操作。
 */
export interface ConfirmRequest {
  /** 主标题。 */
  title: string;
  /** 说明文字。 */
  description?: string;
  /** 单行细节，例如文件路径。 */
  detail?: string;
  /** 确认按钮文案，缺省为「确认」。 */
  confirmText?: string;
  /** 取消按钮文案，缺省为「取消」。 */
  cancelText?: string;
  /** 是否为危险操作，决定图标与按钮配色。 */
  danger?: boolean;
}

/**
 * 一次单行输入框请求。
 *
 * 用于重命名会话、输入命令前缀等只需要一个字符串的场景。
 */
export interface PromptRequest {
  /** 主标题。 */
  title: string;
  /** 输入框上方的说明文字。 */
  label?: string;
  /** 输入框初始值。 */
  value?: string;
  /** 占位提示。 */
  placeholder?: string;
  /** 确认按钮文案，缺省为「确定」。 */
  confirmText?: string;
}

/** 设置弹窗的分区标识。 */
export type SettingsTab = "general" | "mode" | "models" | "tools" | "mcp" | "appearance" | "advanced";
/** 数据弹窗的分区标识：审计、用量、备份。 */
export type DataTab = "audit" | "usage" | "backup";

/** 放大预览的图片：data URL 与展示名。 */
export interface ImagePreviewRequest {
  /** 图片的 data URL（后端读取附件后拼出）。 */
  url: string;
  /** 展示名，通常取路径最后一段。 */
  name: string;
}

/**
 * 界面 store 的状态与操作。
 *
 * 关键状态：
 * - `confirm` / `confirmResolver`：当前待处理的确认请求与它的 resolve 函数；
 * - `prompt` / `promptResolver`：当前待处理的输入请求与它的 resolve 函数；
 * - 两者同时只允许存在一个，后发起的会覆盖前一个（实际使用中不会并发）。
 */
interface UiState {
  /** 当前显示的通知列表。 */
  toasts: Toast[];
  /** 命令面板是否打开。 */
  paletteOpen: boolean;
  /** 设置弹窗是否打开。 */
  settingsOpen: boolean;
  /** 设置弹窗当前分区。 */
  settingsTab: SettingsTab;
  /** 数据弹窗是否打开。 */
  dataOpen: boolean;
  /** 数据弹窗当前分区。 */
  dataTab: DataTab;
  /** 当前待确认的确认框请求；null 表示没有确认框。 */
  confirm: ConfirmRequest | null;
  /** 确认框的 Promise 落点；用户选择后由 `resolveConfirm` 调用。 */
  confirmResolver: ((value: boolean) => void) | null;
  /** 当前待输入的输入框请求；null 表示没有输入框。 */
  prompt: PromptRequest | null;
  /** 输入框的 Promise 落点；用户提交或取消后由 `resolvePrompt` 调用。 */
  promptResolver: ((value: string | null) => void) | null;
  /** 当前放大预览的图片；null 表示预览浮层关闭。 */
  imagePreview: ImagePreviewRequest | null;
  /** 弹出一条通知，4.2 秒后自动消失。 */
  toast(toast: Omit<Toast, "id">): void;
  /** 立刻移除指定通知。 */
  dismissToast(id: string): void;
  /** 打开命令面板。 */
  openPalette(): void;
  /** 关闭命令面板。 */
  closePalette(): void;
  /** 打开设置弹窗；不传分区时保持上次的分区。 */
  openSettings(tab?: SettingsTab): void;
  /** 关闭设置弹窗。 */
  closeSettings(): void;
  /** 打开数据弹窗；不传分区时回到审计分区。 */
  openData(tab?: DataTab): void;
  /** 关闭数据弹窗。 */
  closeData(): void;
  /** 弹出确认框，返回用户是否点了确认。 */
  askConfirm(request: ConfirmRequest): Promise<boolean>;
  /** 结束确认流程，把结果交还给 `askConfirm` 的调用方。 */
  resolveConfirm(value: boolean): void;
  /** 弹出输入框，返回用户输入的内容；取消时返回 null。 */
  askPrompt(request: PromptRequest): Promise<string | null>;
  /** 结束输入流程，把结果交还给 `askPrompt` 的调用方。 */
  resolvePrompt(value: string | null): void;
  /** 打开图片放大预览浮层。 */
  openImagePreview(request: ImagePreviewRequest): void;
  /** 关闭图片预览浮层。 */
  closeImagePreview(): void;
}

/**
 * 全局界面状态容器。
 *
 * 初始状态：没有通知、所有弹层关闭，设置停在「通用」、数据面板停在「审计」。
 */
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
  imagePreview: null,

  // 生成唯一 id 后追加到列表，并启动 4.2 秒的自动消失计时。
  // 计时结束调用 dismissToast，用户提前手动关闭也只是提前移除，不会出错。
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
  // 不指定分区时保留用户上次停留的位置，避免每次打开都跳回第一页。
  openSettings(tab) {
    set({ settingsOpen: true, settingsTab: tab ?? get().settingsTab });
  },
  closeSettings() {
    set({ settingsOpen: false });
  },
  // 数据面板每次打开默认回到审计分区，除非调用方明确指定。
  openData(tab) {
    set({ dataOpen: true, dataTab: tab ?? "audit" });
  },
  closeData() {
    set({ dataOpen: false });
  },

  // 确认流程：把 resolve 存进 store，让 Promise 悬停；等 ConfirmDialog 调用
  // resolveConfirm 时再落定。这样调用方可以像普通异步函数一样 `await` 用户的选择。
  askConfirm(request) {
    return new Promise<boolean>((resolve) => {
      set({ confirm: request, confirmResolver: resolve });
    });
  },

  // 先清空请求与 resolver，再调用 resolve：即使调用方的后续代码立刻又发起新的确认框，
  // 也不会被旧 resolver 干扰。resolver 可能为 null（例如被重复调用），用可选调用保护。
  resolveConfirm(value) {
    const resolver = get().confirmResolver;
    set({ confirm: null, confirmResolver: null });
    resolver?.(value);
  },

  // 输入流程与确认流程同构，区别是结果类型为 string | null（null 表示取消）。
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

  openImagePreview(request) {
    set({ imagePreview: request });
  },

  closeImagePreview() {
    set({ imagePreview: null });
  },
}));
