/**
 * 右侧面板多标签页的纯函数状态库。
 *
 * 对应规格 `docs/panel-tabs/spec.md` 第 1–4 节。分层约定与 ZCode Side Pane
 * 一致：本文件只提供类型与不可变转移函数，不依赖 React 与 store；面板 store
 * 是这些函数的唯一编排者（写出口），组件层只读状态、从不直接修改 tabs。
 *
 * 核心设计：
 * 1. 结构化 id = 幂等打开。singleton 类型（diff / terminal）的 id 是固定字面量，
 *    内容类型（file / document）的 id 携带工作区、文件夹与路径，重复打开天然
 *    命中同一条 tab，无需额外的 singleton 标记位。
 * 2. 一份 tabs 数组 + 可见性过滤。所有线程与工作区共享同一个数组，归属
 *    （workspaceId + threadId）在打开时冻结，切换上下文只是过滤视图，隐藏
 *    的 tab 不销毁。
 * 3. 归属冻结后绝不漂移：盖戳函数只处理「未认领」的 tab，已带归属的
 *    永远不被覆盖。
 *
 * 将来新增会话派生类型（子代理、侧边聊天等）时，各自声明自己的过滤维度
 * 字段（如 rootThreadId / parentThreadId）并扩展可见性函数，不得把多种
 * 派生维度塞进同一个 threadId 字段——多种派生关系混用一个维度会互相误伤。
 */

/** 面板 tab 的类型集合。会话派生与浏览器类类型属于路线图，暂不进入本联合。 */
export type SidePaneTabType = "files" | "file" | "diff" | "terminal" | "document";

/** 差异 tab 的固定 id：singleton，重复打开只聚焦并刷新内容。 */
export const DIFF_TAB_ID = "diff";
/** 终端 tab 的固定 id：singleton，且是跨工作区、跨线程的公共 tab。 */
export const TERMINAL_TAB_ID = "terminal";
/**
 * 文件树 tab 的固定 id：singleton，工作区公共（跨线程可见、随工作区隔离）。
 * 文件树是导航器 tab：file tab 激活时编辑器占据内容区，用户经「文件」tab
 * 回到树再打开其他文件。
 */
export const FILES_TAB_ID = "files";

/** 草稿线程（尚未发送首条消息）的归属标识。 */
export const DRAFT_OWNER = "draft";

/** 所有 tab 类型的公共字段。 */
export interface SidePaneTabBase {
  /** 结构化 id，幂等键；singleton 类型为固定字面量。 */
  id: string;
  /** tab 类型。 */
  type: SidePaneTabType;
  /** tab 条上显示的标题。 */
  title: string;
  /**
   * 打开时冻结的所属工作区（分桶键）。
   * 空字符串表示全局公共（跨工作区可见），当前只有终端使用。
   */
  workspaceId: string;
  /**
   * 打开时冻结的归属线程；null 表示尚未认领（等待盖戳）。
   * 注意这与「草稿」不同：草稿是已归一化的归属值 DRAFT_OWNER。
   */
  threadId: string | null;
  /** 打开时间戳，用于展示与淘汰排序兜底。 */
  openedAt: number;
  /** 最近一次激活的时间戳；淘汰 file tab 时按它挑最久未看的。 */
  activatedAt?: number;
}

/** 文件树 tab：singleton，工作区公共的导航器（展示工作区文件树）。 */
export interface FilesSidePaneTab extends SidePaneTabBase {
  type: "files";
}

/** 文件 tab：在代码编辑器中打开一个工作区文件。 */
export interface FileSidePaneTab extends SidePaneTabBase {
  type: "file";
  /** 所属源文件夹；空字符串表示主要目录。 */
  folderId: string;
  /** 相对所属源文件夹的路径。 */
  path: string;
}

/** 差异 tab：singleton，展示最近一次打开的差异预览。 */
export interface DiffSidePaneTab extends SidePaneTabBase {
  type: "diff";
}

/** 终端 tab：singleton，全局公共——终端会话是全局资源，跨工作区共享。 */
export interface TerminalSidePaneTab extends SidePaneTabBase {
  type: "terminal";
}

/** 文档 tab：Word / Excel / PDF 等的预览。 */
export interface DocumentSidePaneTab extends SidePaneTabBase {
  type: "document";
  /** 所属源文件夹；空字符串表示主要目录。 */
  folderId: string;
  /** 相对所属源文件夹的路径。 */
  path: string;
  /** 文档类型（docx / xlsx / pdf / text），来自后端预览结果的 kind。 */
  kind: string;
}

/** 面板 tab 的联合类型。 */
export type SidePaneTab =
  | FilesSidePaneTab
  | FileSidePaneTab
  | DiffSidePaneTab
  | TerminalSidePaneTab
  | DocumentSidePaneTab;

/** 面板的多 tab 状态：一份共享数组 + 激活项 + 最近关闭栈。 */
export interface SidePaneState {
  /** 全部 tab（含当前线程不可见的隐藏 tab）。 */
  tabs: SidePaneTab[];
  /**
   * 激活 tab 的 id。允许指向当前线程不可见的 tab（切走时保留别人的激活项），
   * 渲染层用 visibleActiveTabId 兜底。null 表示没有激活 tab。
   */
  activeTabId: string | null;
  /** 最近关闭栈，新的在前，上限 RECENT_CLOSED_LIMIT。 */
  recentClosed: SidePaneTab[];
}

/** 可见性判定的工作区与线程上下文；threadId 应已归一化（草稿为 DRAFT_OWNER）。 */
export interface SidePaneScope {
  workspaceId: string;
  threadId: string;
}

/** 最近关闭栈的长度上限。 */
export const RECENT_CLOSED_LIMIT = 8;
/** file tab 的数量上限：超出时自动关闭最久未激活的（不入最近关闭栈）。 */
export const FILE_TAB_LIMIT = 8;

/** 跨线程可见的工作区公共 tab 类型集合；终端 additionally 跨工作区（workspaceId 为空）。 */
const PUBLIC_TAB_TYPES: ReadonlySet<SidePaneTabType> = new Set(["files", "terminal"]);

/** 把线程 id 归一化为归属值：空值归一为草稿。 */
export function normalizeOwner(threadId: string | null | undefined): string {
  const trimmed = threadId?.trim();
  return trimmed ? trimmed : DRAFT_OWNER;
}

/** 构造 file tab 的结构化 id：工作区 + 文件夹 + 路径共同保证唯一。 */
export function fileTabId(workspaceId: string, folderId: string, path: string): string {
  return `file:${workspaceId}:${folderId}:${path}`;
}

/** 构造 document tab 的结构化 id。 */
export function documentTabId(workspaceId: string, folderId: string, path: string): string {
  return `document:${workspaceId}:${folderId}:${path}`;
}

/** 构造一个 file tab；归属由调用方给出（未知的线程传 null 等待盖戳）。 */
export function makeFileTab(input: {
  workspaceId: string;
  threadId: string | null;
  folderId: string;
  path: string;
  title: string;
  openedAt?: number;
}): FileSidePaneTab {
  const openedAt = input.openedAt ?? Date.now();
  return {
    id: fileTabId(input.workspaceId, input.folderId, input.path),
    type: "file",
    title: input.title,
    workspaceId: input.workspaceId,
    threadId: input.threadId,
    openedAt,
    activatedAt: openedAt,
    folderId: input.folderId,
    path: input.path,
  };
}

/** 构造一个 document tab。 */
export function makeDocumentTab(input: {
  workspaceId: string;
  threadId: string | null;
  folderId: string;
  path: string;
  title: string;
  kind: string;
  openedAt?: number;
}): DocumentSidePaneTab {
  const openedAt = input.openedAt ?? Date.now();
  return {
    id: documentTabId(input.workspaceId, input.folderId, input.path),
    type: "document",
    title: input.title,
    workspaceId: input.workspaceId,
    threadId: input.threadId,
    openedAt,
    activatedAt: openedAt,
    folderId: input.folderId,
    path: input.path,
    kind: input.kind,
  };
}

/** 构造差异 singleton tab；workspaceId / threadId 记录打开时的上下文。 */
export function makeDiffTab(input: {
  workspaceId: string;
  threadId: string | null;
  title?: string;
  openedAt?: number;
}): DiffSidePaneTab {
  const openedAt = input.openedAt ?? Date.now();
  return {
    id: DIFF_TAB_ID,
    type: "diff",
    title: input.title ?? "差异",
    workspaceId: input.workspaceId,
    threadId: input.threadId,
    openedAt,
    activatedAt: openedAt,
  };
}

/** 构造终端 singleton tab；workspaceId 为空表示全局公共。 */
export function makeTerminalTab(input: { threadId: string | null; openedAt?: number }): TerminalSidePaneTab {
  const openedAt = input.openedAt ?? Date.now();
  return {
    id: TERMINAL_TAB_ID,
    type: "terminal",
    title: "终端",
    workspaceId: "",
    threadId: input.threadId,
    openedAt,
    activatedAt: openedAt,
  };
}

/** 构造文件树 singleton tab；工作区公共，归属当前工作区（跨线程可见）。 */
export function makeFilesTab(input: {
  workspaceId: string;
  threadId: string | null;
  openedAt?: number;
}): FilesSidePaneTab {
  const openedAt = input.openedAt ?? Date.now();
  return {
    id: FILES_TAB_ID,
    type: "files",
    title: "文件",
    workspaceId: input.workspaceId,
    threadId: input.threadId,
    openedAt,
    activatedAt: openedAt,
  };
}

/** 空的面板状态。 */
export function emptySidePaneState(): SidePaneState {
  return { tabs: [], activeTabId: null, recentClosed: [] };
}

/**
 * 判断 tab 在给定工作区与线程上下文下是否可见。
 *
 * 规则（spec 第 3 节）：
 * 1. workspaceId 非空时必须与当前工作区一致（空 = 全局公共）；
 * 2. 公共类型（terminal）直接可见；
 * 3. 其余类型要求归属线程与当前线程一致（草稿与草稿互相匹配）。
 */
export function isTabVisibleInScope(tab: SidePaneTab, scope: SidePaneScope): boolean {
  if (tab.workspaceId && tab.workspaceId !== scope.workspaceId) {
    return false;
  }
  if (PUBLIC_TAB_TYPES.has(tab.type)) {
    return true;
  }
  return normalizeOwner(tab.threadId) === normalizeOwner(scope.threadId);
}

/** 过滤出当前上下文可见的 tab 子集（保持原顺序）。 */
export function visibleTabs(state: SidePaneState, scope: SidePaneScope): SidePaneTab[] {
  return state.tabs.filter((tab) => isTabVisibleInScope(tab, scope));
}

/**
 * 解析当前上下文实际应显示的激活 tab。
 *
 * activeTabId 指向可见 tab 时直接使用；指向隐藏 tab（切走前的遗留）时
 * 回落到最后一个可见 tab；没有任何可见 tab 时为 null。
 */
export function visibleActiveTabId(state: SidePaneState, scope: SidePaneScope): string | null {
  if (state.activeTabId) {
    const active = state.tabs.find((tab) => tab.id === state.activeTabId);
    if (active && isTabVisibleInScope(active, scope)) {
      return active.id;
    }
  }
  const visible = visibleTabs(state, scope);
  return visible.length > 0 ? visible[visible.length - 1].id : null;
}

/**
 * 给未认领的 tab 盖上归属戳（打开时冻结工作区与线程）。
 *
 * 只处理 threadId 为 null 的 tab；已带归属的永远不覆盖——归属是 tab 的
 * 身份属性，随上下文漂移会让「切回去还看得到」的承诺失效。
 */
export function markTabsOwnership(state: SidePaneState, scope: SidePaneScope): SidePaneState {
  let changed = false;
  const tabs = state.tabs.map((tab) => {
    if (tab.threadId !== null) {
      return tab;
    }
    changed = true;
    return { ...tab, threadId: normalizeOwner(scope.threadId) };
  });
  return changed ? { ...state, tabs } : state;
}

/**
 * 打开一个 tab（幂等）。
 *
 * 已存在同 id 的 tab：激活并合并可刷新字段（标题等展示信息以新值为准），
 * tab 数组顺序不变；不存在：盖戳后追加到末尾并激活。
 * 两种情况都刷新激活时间，供 file tab 淘汰排序使用。
 */
export function openTab(state: SidePaneState, tab: SidePaneTab, scope: SidePaneScope): SidePaneState {
  const index = state.tabs.findIndex((existing) => existing.id === tab.id);
  const owned: SidePaneTab = tab.threadId === null ? { ...tab, threadId: normalizeOwner(scope.threadId) } : tab;
  const now = Date.now();
  if (index >= 0) {
    const tabs = state.tabs.slice();
    tabs[index] = { ...tabs[index], ...owned, openedAt: tabs[index].openedAt, activatedAt: now };
    // 已存在的 tab 若尚未认领（旧数据），借打开时机补盖归属。
    const stamped =
      tabs[index].threadId === null ? { ...tabs[index], threadId: normalizeOwner(scope.threadId) } : tabs[index];
    tabs[index] = stamped;
    return { ...state, tabs, activeTabId: tab.id };
  }
  return { ...state, tabs: [...state.tabs, owned], activeTabId: owned.id };
}

/** 激活指定 tab；id 不存在时返回原状态。激活会刷新该 tab 的激活时间。 */
export function activateTab(state: SidePaneState, tabId: string): SidePaneState {
  const index = state.tabs.findIndex((tab) => tab.id === tabId);
  if (index < 0) {
    return state;
  }
  const tabs = state.tabs.slice();
  tabs[index] = { ...tabs[index], activatedAt: Date.now() };
  return { ...state, tabs, activeTabId: tabId };
}

/** 从数组中删除一批 tab 的公共实现：返回删除的 tab 列表与新状态。 */
function removeTabsByIds(
  state: SidePaneState,
  ids: ReadonlySet<string>,
): { state: SidePaneState; removed: SidePaneTab[] } {
  if (ids.size === 0) {
    return { state, removed: [] };
  }
  const removed: SidePaneTab[] = [];
  const tabs = state.tabs.filter((tab) => {
    if (ids.has(tab.id)) {
      removed.push(tab);
      return false;
    }
    return true;
  });
  if (removed.length === 0) {
    return { state, removed: [] };
  }
  let activeTabId = state.activeTabId;
  if (activeTabId && ids.has(activeTabId)) {
    // 激活项被删除：激活它在旧数组中位置之后的第一个存活 tab（跨过同批被删的），
    // 后面没有存活 tab 则取前面最后一个；全部删光则为 null。
    const anchor = state.tabs.findIndex((tab) => tab.id === activeTabId);
    let next: SidePaneTab | undefined;
    for (let index = anchor + 1; index < state.tabs.length && !next; index += 1) {
      if (!ids.has(state.tabs[index].id)) {
        next = tabs.find((tab) => tab.id === state.tabs[index].id);
      }
    }
    if (!next) {
      next = tabs[tabs.length - 1];
    }
    activeTabId = next?.id ?? null;
  }
  return { state: { ...state, tabs, activeTabId }, removed };
}

/**
 * 关闭一个 tab。
 *
 * 关闭激活项时激活原位置右侧的第一个存活 tab（邻居选择），与主流编辑器
 * 的关闭行为一致；返回被删除的 tab 供调用方入最近关闭栈。
 */
export function closeTab(state: SidePaneState, tabId: string): { state: SidePaneState; removed: SidePaneTab[] } {
  return removeTabsByIds(state, new Set([tabId]));
}

/**
 * 关闭当前上下文可见 tab 中除 keepTabId 之外的全部（「关闭其他」）。
 *
 * 隐藏 tab（其他线程 / 其他工作区的）不属于当前上下文，不在关闭范围；
 * 完成后 keepTabId 若可见则被激活。
 */
export function closeOtherTabs(
  state: SidePaneState,
  keepTabId: string,
  scope: SidePaneScope,
): { state: SidePaneState; removed: SidePaneTab[] } {
  const keep = state.tabs.find((tab) => tab.id === keepTabId);
  if (!keep || !isTabVisibleInScope(keep, scope)) {
    return { state, removed: [] };
  }
  const ids = new Set(
    visibleTabs(state, scope)
      .filter((tab) => tab.id !== keepTabId)
      .map((tab) => tab.id),
  );
  const result = removeTabsByIds(state, ids);
  const nextState = result.state.activeTabId === keepTabId ? result.state : activateTab(result.state, keepTabId);
  return { state: nextState, removed: result.removed };
}

/** 关闭当前上下文可见的全部 tab（「关闭全部」）；隐藏 tab 不受影响。 */
export function closeAllVisibleTabs(
  state: SidePaneState,
  scope: SidePaneScope,
): { state: SidePaneState; removed: SidePaneTab[] } {
  const ids = new Set(visibleTabs(state, scope).map((tab) => tab.id));
  return removeTabsByIds(state, ids);
}

/**
 * 把关闭的 tab 压入最近关闭栈。
 *
 * 新的在前；同 id 的旧记录先移除（重开再关不产生重复项）；超过上限从
 * 栈尾淘汰。singleton 与普通 tab 一视同仁。
 */
export function pushRecentClosed(state: SidePaneState, closed: readonly SidePaneTab[]): SidePaneState {
  if (closed.length === 0) {
    return state;
  }
  const closedIds = new Set(closed.map((tab) => tab.id));
  const merged = [...closed, ...state.recentClosed.filter((tab) => !closedIds.has(tab.id))];
  return { ...state, recentClosed: merged.slice(0, RECENT_CLOSED_LIMIT) };
}

/**
 * 从最近关闭栈取出一条记录准备重开。
 *
 * 校验该记录在当前上下文可见（防止跨工作区、跨线程泄漏别的线程的 tab）；
 * 校验通过则从栈中移除并返回该 tab，由调用方按打开流程重建内容；不可见
 * 或不存在时返回 null 且状态不变。
 */
export function takeRecentClosed(
  state: SidePaneState,
  tabId: string,
  scope: SidePaneScope,
): { state: SidePaneState; tab: SidePaneTab | null } {
  const tab = state.recentClosed.find((item) => item.id === tabId);
  if (!tab || !isTabVisibleInScope(tab, scope)) {
    return { state, tab: null };
  }
  return {
    state: { ...state, recentClosed: state.recentClosed.filter((item) => item.id !== tabId) },
    tab,
  };
}

/** 删除某个线程名下的全部 tab（线程删除联动）；公共类型保留，不入最近关闭栈。 */
export function pruneThreadTabs(
  state: SidePaneState,
  threadId: string,
): { state: SidePaneState; removed: SidePaneTab[] } {
  const ids = new Set(
    state.tabs
      .filter((tab) => !PUBLIC_TAB_TYPES.has(tab.type) && normalizeOwner(tab.threadId) === normalizeOwner(threadId))
      .map((tab) => tab.id),
  );
  return removeTabsByIds(state, ids);
}

/**
 * 拖拽排序：把 activeTabId 移动到 overTabId 的位置。
 *
 * 只改变数组顺序，不改变激活项（拖拽结束留在原激活 tab 上，与 ZCode 一致）；
 * 任一 id 不存在或相同位置时返回原状态。
 */
export function reorderTabs(state: SidePaneState, activeTabId: string, overTabId: string): SidePaneState {
  if (activeTabId === overTabId) {
    return state;
  }
  const from = state.tabs.findIndex((tab) => tab.id === activeTabId);
  const to = state.tabs.findIndex((tab) => tab.id === overTabId);
  if (from < 0 || to < 0) {
    return state;
  }
  const tabs = state.tabs.slice();
  const [moved] = tabs.splice(from, 1);
  tabs.splice(to, 0, moved);
  return { ...state, tabs };
}

/**
 * 执行 file tab 数量上限（spec 第 6 节）：超出时淘汰最久未激活的 file tab。
 *
 * 淘汰走真正的删除（返回 evicted 供调用方清理数据槽），但按约定**不入**
 * 最近关闭栈——自动清理不是用户动作，把被清理的文件塞进恢复列表反而
 * 让人困惑。其他类型不受上限约束。
 */
export function enforceFileTabLimit(
  state: SidePaneState,
  limit = FILE_TAB_LIMIT,
): { state: SidePaneState; evicted: SidePaneTab[] } {
  const files = state.tabs.filter((tab): tab is FileSidePaneTab => tab.type === "file");
  if (files.length <= limit) {
    return { state, evicted: [] };
  }
  const overflow = files.length - limit;
  const sorted = files.slice().sort((a, b) => (a.activatedAt ?? a.openedAt) - (b.activatedAt ?? b.openedAt));
  const evicted = sorted.slice(0, overflow);
  const result = removeTabsByIds(state, new Set(evicted.map((tab) => tab.id)));
  return { state: result.state, evicted };
}
