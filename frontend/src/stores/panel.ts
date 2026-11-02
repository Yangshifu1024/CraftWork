/**
 * 右侧面板的界面状态：是否展开、是否最大化、宽度、是否处于窄窗口，
 * 以及多标签页状态（tabs、激活项、最近关闭栈）。
 *
 * 职责边界：
 * - tab 的类型与全部转移函数在 `lib/sidePane.ts`（纯函数库，可单测）；
 * - 本文件是唯一编排者（写出口）：所有打开 / 关闭 / 激活入口都经这里
 *   落账到「工作区记忆桶」，再把当前上下文可见的投影写给组件；组件层
 *   只读状态，从不直接修改 tabs；
 * - 面板不保存业务数据——文件内容、差异、文档、终端输出仍在 workspace
 *   与 terminal store，tab 只记录「显示什么」。
 *
 * 分桶（对齐 ZCode Side Pane）：每个工作区一个记忆桶（tabs + 最近关闭 +
 * 按线程记住的激活项与开合偏好），切换工作区整桶切换，互不串台；桶只在
 * 内存（LRU 上限 50 个），tab 列表不持久化（重启后按入口重建，避免恢复
 * 失效路径）。跨工作区的打开动作写回原工作区的桶，不污染当前视图。
 */
import { create } from "zustand";
import {
  activateTab as activateTabFn,
  closeAllVisibleTabs,
  closeOtherTabs as closeOtherTabsFn,
  closeTab as closeTabFn,
  emptySidePaneState,
  enforceFileTabLimit,
  makeDiffTab,
  makeDocumentTab,
  makeFilesTab,
  makeFileTab,
  makeTerminalTab,
  normalizeOwner,
  openTab as openTabFn,
  pruneThreadTabs,
  pushRecentClosed,
  reorderTabs as reorderTabsFn,
  type SidePaneScope,
  type SidePaneState,
  type SidePaneTab,
  TERMINAL_TAB_ID,
  takeRecentClosed,
  visibleActiveTabId,
  visibleTabs,
} from "../lib/sidePane";
import { clamp } from "../lib/utils";
import { useAppStore } from "./app";
import { useThreadStore } from "./threads";

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
 * 面板偏好的 localStorage 键（spec 第 7 节）。
 *
 * 持久化的只有「上次布局偏好」：宽度与开合。tab 列表刻意不落盘——文件可
 * 由文件树重建、终端 PTY 随进程退出消失，恢复失效路径（文件移动、线程
 * 删除）是纯负担，重启后面板打开显示文件树兜底视图。桌面与 server 模式
 * 行为一致（localStorage 在两种宿主下都可用）。
 */
const PANEL_WIDTH_KEY = "cw-panel-width";
const PANEL_OPEN_KEY = "cw-panel-open";

/** 从 localStorage 读宽度偏好；缺失或损坏时回退默认（最小宽度）。 */
function readStoredWidth(): number {
  try {
    const raw = localStorage.getItem(PANEL_WIDTH_KEY);
    const parsed = raw ? Number.parseInt(raw, 10) : Number.NaN;
    return Number.isFinite(parsed) ? clamp(parsed, PANEL_MIN_WIDTH, PANEL_MAX_WIDTH) : PANEL_MIN_WIDTH;
  } catch {
    return PANEL_MIN_WIDTH;
  }
}

/** 从 localStorage 读开合偏好；只有显式写入的 "false" 视为收起，其余展开。 */
function readStoredOpen(): boolean {
  try {
    return localStorage.getItem(PANEL_OPEN_KEY) !== "false";
  } catch {
    return true;
  }
}

/** 写入宽度偏好；localStorage 不可用时静默跳过。 */
function persistWidth(width: number): void {
  try {
    localStorage.setItem(PANEL_WIDTH_KEY, String(width));
  } catch {
    // 存储不可用（隐私模式等）时放弃持久化，功能不受影响。
  }
}

/** 写入开合偏好；localStorage 不可用时静默跳过。 */
function persistOpen(open: boolean): void {
  try {
    localStorage.setItem(PANEL_OPEN_KEY, open ? "true" : "false");
  } catch {
    // 存储不可用（隐私模式等）时放弃持久化，功能不受影响。
  }
}

/** 工作区记忆桶：tab 状态与按线程记住的偏好。 */
interface PaneBucket {
  /** 该工作区的多 tab 状态（含其他线程的隐藏 tab）。 */
  state: SidePaneState;
  /** 「工作区::线程」到该上下文最后激活 tab 的映射，切回时恢复。 */
  activeTabByThread: Record<string, string>;
  /** 归一化线程 id 到用户显式设置的开合偏好（true = 收起）。 */
  collapsedByThread: Record<string, boolean>;
}

/** 记忆桶数量上限：防止工作区数量增长后内存无限上涨。 */
const BUCKET_LIMIT = 50;

/** 所有工作区的记忆桶；Map 的插入顺序即 LRU 顺序（末尾最新）。 */
const buckets = new Map<string, PaneBucket>();

function newBucket(): PaneBucket {
  return {
    state: terminalOpen
      ? { ...emptySidePaneState(), tabs: [makeTerminalTab({ threadId: null })] }
      : emptySidePaneState(),
    activeTabByThread: {},
    collapsedByThread: {},
  };
}

/**
 * 终端 singleton 的全局开关。
 *
 * 终端是跨工作区、跨线程的公共 tab，而 tab 状态按工作区分桶存储——
 * 如果只写进当前桶，切换工作区后终端 tab 会消失。因此「终端已打开」
 * 用一个全局标志表示：打开时同步补进所有已存在的桶，新建桶时按标志
 * 自带；关闭时从所有桶移除。各桶内的 tab 实例相互独立，激活记忆互不影响。
 */
let terminalOpen = false;

/** 确保桶里带着终端 tab（终端已打开但该桶没有时补建）。 */
function ensureTerminalTab(bucket: PaneBucket): PaneBucket {
  if (!terminalOpen || bucket.state.tabs.some((tab) => tab.id === TERMINAL_TAB_ID)) {
    return bucket;
  }
  bucket.state = { ...bucket.state, tabs: [...bucket.state.tabs, makeTerminalTab({ threadId: null })] };
  return bucket;
}

/** 取工作区的记忆桶，读写都刷新 LRU 顺序；超过上限时淘汰最旧的桶。 */
function bucketFor(workspaceId: string): PaneBucket {
  const existing = buckets.get(workspaceId);
  if (existing) {
    buckets.delete(workspaceId);
    buckets.set(workspaceId, existing);
    return existing;
  }
  const created = newBucket();
  buckets.set(workspaceId, created);
  if (buckets.size > BUCKET_LIMIT) {
    const oldest = buckets.keys().next().value;
    if (oldest !== undefined) {
      buckets.delete(oldest);
    }
  }
  return created;
}

/** 从全局状态读取当前的面板作用域（工作区 + 归一化线程）。 */
function currentScope(): SidePaneScope {
  const app = useAppStore.getState();
  return {
    workspaceId: app.activeWorkspace?.id ?? "",
    threadId: normalizeOwner(useThreadStore.getState().activeID[app.mode]),
  };
}

/** 激活项记忆的键：工作区 + 线程。 */
function ownerKey(scope: SidePaneScope): string {
  return `${scope.workspaceId}::${scope.threadId}`;
}

/**
 * 面板 store 的状态与操作。
 *
 * 关键约束：
 * - 面板关闭时一定不是最大化状态，避免下次打开时突然铺满对话区；
 * - 「当前线程可见 tab 清空」时面板自动收起（并退出最大化）；
 * - 打开类入口总是确保面板展开（沿用旧 setTab 的行为语义）。
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
  /** 当前工作区桶的全部 tab（含其他线程的隐藏 tab）。 */
  tabs: SidePaneTab[];
  /** 当前应显示的激活 tab（已经过可见性兜底）；null 表示没有激活 tab。 */
  activeTabId: string | null;
  /** 最近关闭栈（当前工作区桶的），新的在前。 */
  recentClosed: SidePaneTab[];
  /** 设置右侧面板是否可见；关闭时同时退出最大化并记住该线程的偏好。 */
  setOpen(open: boolean): void;
  /** 翻转面板可见性；关闭时同时退出最大化并记住该线程的偏好。 */
  toggle(): void;
  /** 设置最大化；最大化会自动展开面板。 */
  setMaximized(maximized: boolean): void;
  /** 翻转最大化状态；面板关闭时先展开再最大化。 */
  toggleMaximized(): void;
  /** 设置面板宽度，内部会夹取到合法区间。 */
  setWidth(width: number): void;
  /** 响应窄窗口变化：进入窄窗口时收起面板，离开时自动展开（临时态，不记偏好）。 */
  setNarrow(narrow: boolean): void;
  /** 打开（或聚焦）一个文件 tab。 */
  openFileTab(input: { folderId: string; path: string; title: string }): void;
  /** 打开（或聚焦）一个文档 tab。 */
  openDocumentTab(input: { folderId: string; path: string; title: string; kind: string }): void;
  /** 打开（或聚焦）差异 singleton tab；差异内容由 workspace store 写入。 */
  openDiffTab(title: string): void;
  /** 打开（或聚焦）文件树 singleton tab（工作区公共的导航器）。 */
  openFilesTab(): void;
  /** 打开（或聚焦）终端 singleton tab（全局公共，跨工作区、跨线程可见）。 */
  openTerminalTab(): void;
  /** 激活指定 tab 并记住它为当前上下文的激活项。 */
  activateTab(tabId: string): void;
  /** 关闭一个 tab；关闭的是激活项时自动激活邻居，可见集清空时收起面板。 */
  closeTab(tabId: string): void;
  /** 关闭当前上下文可见 tab 中除目标之外的全部（隐藏 tab 不受影响）。 */
  closeOtherTabs(tabId: string): void;
  /** 关闭当前上下文可见的全部 tab，面板收起。 */
  closeAllTabs(): void;
  /** 从最近关闭栈重开一个 tab；校验当前上下文可见，归属重置为当前线程。 */
  reopenRecentClosed(tabId: string): void;
  /** 清理某个线程名下的全部 tab（线程删除联动）；公共 tab 保留，不入最近关闭栈。 */
  pruneThread(threadId: string): void;
  /** 拖拽排序：把激活 tab 移动到目标 tab 的位置。 */
  reorderTabs(activeTabId: string, overTabId: string): void;
  /** 工作区 / 线程变化后的恢复：切桶、恢复激活项与该线程的开合偏好。 */
  syncScope(): void;
}

/**
 * 右侧面板的状态容器。
 *
 * 初始状态：宽度与开合从 localStorage 恢复（缺失时展开 / 最小宽度），
 * 没有任何 tab（面板内容显示文件树兜底视图）。
 */
export const usePanelStore = create<PanelState>((set, get) => ({
  open: readStoredOpen(),
  maximized: false,
  width: readStoredWidth(),
  narrow: false,
  tabs: [],
  activeTabId: null,
  recentClosed: [],

  // 关闭面板时必须清掉最大化标记，否则再次打开会直接铺满；
  // 显式开合都记入当前线程的偏好（切回该线程时恢复），并持久化开合值。
  setOpen(open) {
    const scope = currentScope();
    bucketFor(scope.workspaceId).collapsedByThread[scope.threadId] = !open;
    persistOpen(open);
    set({ open, maximized: open ? get().maximized : false });
  },
  // 翻转可见性，规则与 setOpen 一致。
  toggle() {
    get().setOpen(!get().open);
  },
  // 最大化必然要求面板可见；取消最大化不改变可见性。展开状态一并持久化。
  setMaximized(maximized) {
    if (maximized && !get().open) {
      persistOpen(true);
    }
    set({ maximized, open: maximized ? true : get().open });
  },
  // 面板关着时按「最大化」= 先打开并最大化；否则直接翻转。隐含的展开一并持久化。
  toggleMaximized() {
    const current = get();
    if (!current.open) {
      persistOpen(true);
      set({ open: true, maximized: true });
      return;
    }
    set({ maximized: !current.maximized });
  },
  // 宽度做上下限夹取（防拖出过窄或过宽），并持久化偏好。
  setWidth(width) {
    const next = clamp(width, PANEL_MIN_WIDTH, PANEL_MAX_WIDTH);
    persistWidth(next);
    set({ width: next });
  },
  // 窄窗口下面板改为浮层，会遮住对话区，所以进入窄窗口时自动收起；
  // 回到宽窗口时再自动展开。这是临时态，不写入线程偏好。
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

  // 下面是 tab 写出口：统一「算新状态 → 写回桶 → 记激活项 → 投影给组件」。
  openFileTab(input) {
    commitTabState(
      set,
      (bucket, scope) => {
        const tab = makeFileTab({
          workspaceId: scope.workspaceId,
          threadId: scope.threadId,
          folderId: input.folderId,
          path: input.path,
          title: input.title,
        });
        return enforceFileTabLimit(openTabFn(bucket.state, tab, scope)).state;
      },
      { ensureOpen: true },
    );
  },

  openDocumentTab(input) {
    commitTabState(
      set,
      (bucket, scope) => {
        const tab = makeDocumentTab({
          workspaceId: scope.workspaceId,
          threadId: scope.threadId,
          folderId: input.folderId,
          path: input.path,
          title: input.title,
          kind: input.kind,
        });
        return enforceFileTabLimit(openTabFn(bucket.state, tab, scope)).state;
      },
      { ensureOpen: true },
    );
  },

  // 差异是 singleton：已存在时只刷新标题并聚焦，内容仍由 workspace store 写。
  openDiffTab(title) {
    commitTabState(
      set,
      (bucket, scope) => {
        const tab = makeDiffTab({ workspaceId: scope.workspaceId, threadId: scope.threadId, title });
        return openTabFn(bucket.state, tab, scope);
      },
      { ensureOpen: true },
    );
  },

  // 文件树是工作区公共 singleton：file tab 激活时经它回到树再开其他文件。
  openFilesTab() {
    commitTabState(
      set,
      (bucket, scope) => {
        const tab = makeFilesTab({ workspaceId: scope.workspaceId, threadId: scope.threadId });
        return openTabFn(bucket.state, tab, scope);
      },
      { ensureOpen: true },
    );
  },

  // 终端是全局公共 singleton：置全局标志并同步补进所有桶，当前桶聚焦。
  openTerminalTab() {
    terminalOpen = true;
    for (const bucket of buckets.values()) {
      ensureTerminalTab(bucket);
    }
    commitTabState(set, (bucket) => activateTabFn(bucket.state, TERMINAL_TAB_ID), { ensureOpen: true });
  },

  activateTab(tabId) {
    commitTabState(set, (bucket) => activateTabFn(bucket.state, tabId));
  },

  closeTab(tabId) {
    // 关闭终端 tab：全局标志复位，其他工作区的桶静默移除；当前桶交给
    // 下面的统一提交（删除 + 入最近关闭栈 + 投影）。按裁决不终止 PTY
    // 会话，重开终端 tab 时会话仍在（会话管理在终端面板内部）。
    if (tabId === TERMINAL_TAB_ID && terminalOpen) {
      terminalOpen = false;
      const currentWs = useAppStore.getState().activeWorkspace?.id ?? "";
      for (const [id, bucket] of buckets.entries()) {
        if (id !== currentWs) {
          bucket.state = closeTabFn(bucket.state, TERMINAL_TAB_ID).state;
        }
      }
    }
    commitTabState(
      set,
      (bucket) => {
        const { state: next, removed } = closeTabFn(bucket.state, tabId);
        if (removed.length === 0) {
          return null;
        }
        return pushRecentClosed(next, removed);
      },
      { collapseOnEmpty: true },
    );
  },

  closeOtherTabs(tabId) {
    commitTabState(set, (bucket, scope) => {
      const { state: next, removed } = closeOtherTabsFn(bucket.state, tabId, scope);
      if (removed.length === 0) {
        return null;
      }
      return next;
    });
  },

  closeAllTabs() {
    commitTabState(
      set,
      (bucket, scope) => {
        const { state: next, removed } = closeAllVisibleTabs(bucket.state, scope);
        if (removed.length === 0) {
          return null;
        }
        return next;
      },
      { collapseOnEmpty: true },
    );
  },

  reopenRecentClosed(tabId) {
    commitTabState(
      set,
      (bucket, scope) => {
        const { state: taken, tab } = takeRecentClosed(bucket.state, tabId, scope);
        if (!tab) {
          return null;
        }
        // 重开的 tab 归属重置为当前线程（spec 第 4 节），内容由渲染层按需重新加载。
        const reopened: SidePaneTab = { ...tab, threadId: normalizeOwner(scope.threadId) };
        return enforceFileTabLimit(openTabFn(taken, reopened, scope)).state;
      },
      { ensureOpen: true },
    );
  },

  pruneThread(threadId) {
    commitTabState(
      set,
      (bucket) => {
        const { state: next, removed } = pruneThreadTabs(bucket.state, threadId);
        if (removed.length === 0) {
          return null;
        }
        // 被删 tab 的激活记忆一并清掉，避免切回时指向不存在的 tab。
        for (const tab of removed) {
          for (const [key, value] of Object.entries(bucket.activeTabByThread)) {
            if (value === tab.id) {
              delete bucket.activeTabByThread[key];
            }
          }
        }
        return next;
      },
      { collapseOnEmpty: true },
    );
  },

  reorderTabs(activeTabId, overTabId) {
    commitTabState(set, (bucket) => reorderTabsFn(bucket.state, activeTabId, overTabId));
  },

  // 工作区 / 线程切换后的恢复。tab 数据留在各自桶里，这里只负责：
  // 1. 按记忆恢复该上下文的激活 tab；2. 恢复该线程显式设置过的开合偏好。
  // 没有 tab 时**不改变开合**——面板有文件树兜底视图，初始启动与切到
  // 无 tab 的线程都应保持用户当前的开合（强制收起会覆盖 localStorage
  // 恢复的偏好，也会让首屏文件树不可见）；「关末 tab 收起」由关闭类
  // 入口的 collapseOnEmpty 负责，两条路径各司其职。
  syncScope() {
    const scope = currentScope();
    const bucket = bucketFor(scope.workspaceId);
    const remembered = bucket.activeTabByThread[ownerKey(scope)];
    const exists = (id: string | null) => (id ? bucket.state.tabs.some((tab) => tab.id === id) : false);
    let activeTabId = bucket.state.activeTabId;
    if (remembered && exists(remembered)) {
      activeTabId = remembered;
    } else if (!exists(activeTabId)) {
      activeTabId = null;
    }
    bucket.state = { ...bucket.state, activeTabId };
    const preference = bucket.collapsedByThread[scope.threadId];
    set({
      tabs: bucket.state.tabs,
      activeTabId: visibleActiveTabId(bucket.state, scope),
      recentClosed: bucket.state.recentClosed,
      open: preference !== undefined ? !preference : get().open,
    });
  },
}));

/**
 * tab 写出口的统一编排骨架：拿到当前桶 → 用纯函数算出新状态 → 写回桶并
 * 记住激活项 → 把「当前上下文可见」的投影写给组件。
 *
 * mutate 返回 null 表示没有变化（如关闭不存在的 tab），整次提交作废。
 */
function commitTabState(
  set: (patch: Partial<PanelState>) => void,
  mutate: (bucket: PaneBucket, scope: SidePaneScope) => SidePaneState | null,
  options?: { collapseOnEmpty?: boolean; ensureOpen?: boolean },
): void {
  const scope = currentScope();
  const bucket = bucketFor(scope.workspaceId);
  const next = mutate(bucket, scope);
  if (!next) {
    return;
  }
  bucket.state = next;
  if (next.activeTabId) {
    bucket.activeTabByThread[ownerKey(scope)] = next.activeTabId;
  }
  const patch: Partial<PanelState> = {
    tabs: next.tabs,
    activeTabId: visibleActiveTabId(next, scope),
    recentClosed: next.recentClosed,
  };
  if (options?.ensureOpen) {
    patch.open = true;
  }
  if (options?.collapseOnEmpty && visibleTabs(next, scope).length === 0) {
    patch.open = false;
    patch.maximized = false;
  }
  set(patch);
}
