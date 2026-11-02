/**
 * 右侧面板的界面状态：是否展开、是否最大化、宽度、是否处于窄窗口，
 * 以及 Code / Work 两种模式各自记忆的当前标签页。
 *
 * 面板不保存业务数据，只保存「怎么显示」；文件、文档、差异等数据在 workspace store 里。
 */
import { create } from "zustand";
import type { Mode, PanelTab } from "../lib/types";
import { clamp } from "../lib/utils";

/**
 * 面板宽度的合法区间（像素）。
 *
 * 默认宽度取最小值：面板是辅助区域，初始应把更多空间留给对话区，
 * 需要更宽时再由用户拖动分隔条。
 */
export const PANEL_MIN_WIDTH = 320;
/** 面板最大宽度（像素），避免在宽屏上挤压对话区。 */
export const PANEL_MAX_WIDTH = 760;

/**
 * 面板 store 的状态与操作。
 *
 * 关键约束：面板关闭时一定不是最大化状态，避免下次打开时突然铺满对话区。
 */
interface PanelState {
  /** 面板是否可见。 */
  open: boolean;
  /** 面板是否最大化（铺满对话区，仅宽窗口生效）。 */
  maximized: boolean;
  /** 面板宽度（像素），拖拽调整时限制在 PANEL_MIN_WIDTH–PANEL_MAX_WIDTH 之间。 */
  width: number;
  /** 当前是否处于窄窗口（由 AppShell 的媒体查询写入）。 */
  narrow: boolean;
  /** 每种模式各自记住的标签页，切换模式时回到各自上次的位置。 */
  tabs: Record<Mode, PanelTab>;
  /** 设置右侧面板是否可见；关闭时同时退出最大化。 */
  setOpen(open: boolean): void;
  /** 翻转面板可见性；关闭时同时退出最大化。 */
  toggle(): void;
  /** 设置最大化；最大化会自动展开面板。 */
  setMaximized(maximized: boolean): void;
  /** 翻转最大化状态；面板关闭时先展开再最大化。 */
  toggleMaximized(): void;
  /** 设置面板宽度，内部会夹取到合法区间。 */
  setWidth(width: number): void;
  /** 响应窄窗口变化：进入窄窗口时收起面板，离开时自动展开。 */
  setNarrow(narrow: boolean): void;
  /** 切换某个模式的标签页，并确保面板可见。 */
  setTab(mode: Mode, tab: PanelTab): void;
}

/**
 * 右侧面板的状态容器。
 *
 * 初始状态：默认展开、非最大化、宽度取最小宽度 320，Code 模式停在「文件」、Work 模式停在「文档」。
 */
export const usePanelStore = create<PanelState>((set, get) => ({
  open: true,
  maximized: false,
  width: PANEL_MIN_WIDTH,
  narrow: false,
  tabs: { code: "files", work: "document" },

  // 关闭面板时必须清掉最大化标记，否则再次打开会直接铺满。
  setOpen(open) {
    set({ open, maximized: open ? get().maximized : false });
  },
  // 翻转可见性，规则与 setOpen 一致。
  toggle() {
    const next = !get().open;
    set({ open: next, maximized: next ? get().maximized : false });
  },
  // 最大化必然要求面板可见；取消最大化不改变可见性。
  setMaximized(maximized) {
    set({ maximized, open: maximized ? true : get().open });
  },
  // 面板关着时按「最大化」= 先打开并最大化；否则直接翻转。
  toggleMaximized() {
    const current = get();
    if (!current.open) {
      set({ open: true, maximized: true });
      return;
    }
    set({ maximized: !current.maximized });
  },
  // 宽度做上下限夹取，防止拖出过窄或过宽的面板。
  setWidth(width) {
    set({ width: clamp(width, PANEL_MIN_WIDTH, PANEL_MAX_WIDTH) });
  },
  // 窄窗口下面板改为浮层，会遮住对话区，所以进入窄窗口时自动收起；
  // 回到宽窗口时再自动展开，减少用户手动操作。值没变时直接返回，避免重复 set。
  setNarrow(narrow) {
    if (narrow === get().narrow) {
      return;
    }
    if (narrow) {
      set({ narrow, open: false, maximized: false });
    } else {
      set({ narrow, open: true });
    }
  },
  // 切换标签页通常来自「打开文件 / 打开差异 / 打开文档」这类动作，
  // 用户期望立刻看到面板，所以这里顺带把面板打开。
  setTab(mode, tab) {
    set((state) => ({ tabs: { ...state.tabs, [mode]: tab }, open: true }));
  },
}));
