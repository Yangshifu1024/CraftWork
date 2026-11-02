/**
 * 右侧面板容器。
 *
 * 负责：渲染面板的 tab 条（多标签页）、按激活 tab 分派内容面板、
 * 通过拖拽分隔条调整面板宽度，以及在普通与全屏之间切换面板。
 *
 * tab 的状态与规则来自 panel store（分桶记忆 + 可见性过滤，见
 * docs/panel-tabs/spec.md）；文件内容、差异、文档、终端等业务数据仍在
 * workspace / terminal store，本组件只负责「显示哪个 tab」与「tab 内容
 * 与数据槽对齐」。没有激活 tab 时显示文件树兜底视图。
 *
 * 内容面板按需懒加载，避免编辑器、终端、文档预览拖慢首屏。
 */
import { lazy, Suspense, useEffect, useState } from "react";
import { Icon, type IconName } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import {
  type DocumentSidePaneTab,
  type FileSidePaneTab,
  isTabVisibleInScope,
  normalizeOwner,
  type SidePaneTab,
} from "../../lib/sidePane";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useWorkspaceStore } from "../../stores/workspace";
import { Editor } from "./Editor";

// 四个内容面板都按需加载：首次切换到对应 tab 时才拉取分包代码。
const FilesPanel = lazy(() => import("./FilesPanel").then((module) => ({ default: module.FilesPanel })));
const DiffPanel = lazy(() => import("./DiffPanel").then((module) => ({ default: module.DiffPanel })));
const TerminalPanel = lazy(() => import("./TerminalPanel").then((module) => ({ default: module.TerminalPanel })));
const DocumentPanel = lazy(() => import("./DocumentPanel").then((module) => ({ default: module.DocumentPanel })));

/** tab 类型对应的图标；不引用 FilesPanel，避免把分包拖进主包。 */
function tabIcon(tab: SidePaneTab): IconName {
  switch (tab.type) {
    case "file":
      return "file-code";
    case "diff":
      return "diff";
    case "terminal":
      return "terminal";
    case "document":
      return "file-text";
  }
}

/**
 * 关闭一个 tab，并做必要的数据槽清理：
 * 关闭 file tab 时同时清掉它在 workspace store 里的编辑器数据槽
 * （内容、草稿、定位行），槽与 tab 同生共死。
 */
function handleCloseTab(tab: SidePaneTab): void {
  if (tab.type === "file") {
    useWorkspaceStore.getState().closeFileSlot(tab.id);
  }
  usePanelStore.getState().closeTab(tab.id);
}

/**
 * 右侧面板根组件。
 *
 * 状态来自三个 store：
 * - useAppStore / useThreadStore 提供当前上下文（工作区 + 线程），决定
 *   哪些 tab 可见；
 * - usePanelStore 提供多 tab 状态、面板宽度与是否全屏。
 */
export function Panel() {
  const mode = useAppStore((state) => state.mode);
  const workspaceId = useAppStore((state) => state.activeWorkspace?.id ?? "");
  const threadId = useThreadStore((state) => normalizeOwner(state.activeID[mode]));
  const tabs = usePanelStore((state) => state.tabs);
  const activeTabId = usePanelStore((state) => state.activeTabId);
  const recentClosed = usePanelStore((state) => state.recentClosed);
  const activateTab = usePanelStore((state) => state.activateTab);
  const closeOtherTabs = usePanelStore((state) => state.closeOtherTabs);
  const closeAllTabs = usePanelStore((state) => state.closeAllTabs);
  const reopenRecentClosed = usePanelStore((state) => state.reopenRecentClosed);
  const openTerminalTab = usePanelStore((state) => state.openTerminalTab);
  const openDiffTab = usePanelStore((state) => state.openDiffTab);
  const width = usePanelStore((state) => state.width);
  const setWidth = usePanelStore((state) => state.setWidth);
  const maximized = usePanelStore((state) => state.maximized);
  const toggleMaximized = usePanelStore((state) => state.toggleMaximized);

  // tab 右键菜单：记录被右键的 tab 与锚点（tab 容器元素），供 Popover 定位。
  const [tabMenu, setTabMenu] = useState<{ tab: SidePaneTab; anchor: HTMLElement } | null>(null);

  // 当前上下文可见的 tab 子集（工作区匹配 + 线程归属或公共类型）。
  const scope = { workspaceId, threadId };
  const visible = tabs.filter((tab) => isTabVisibleInScope(tab, scope));
  const active = visible.find((tab) => tab.id === activeTabId) ?? null;
  // 激活 file tab 的路径：文件树的选中高亮跟随它（没有则为 null）。
  const activeFilePath = active?.type === "file" ? active.path : null;
  // 最近关闭栈里当前上下文可恢复的 tab（跨线程 / 跨工作区的不展示）。
  const reopenable = recentClosed.filter((tab) => isTabVisibleInScope(tab, scope));

  /**
   * tab 区域的鼠标按下处理：中键（button === 1）在按下阶段就阻止默认
   * 行为，取消浏览器的中键自动滚动；真正的关闭动作在 auxClick 里做，
   * 保证中键不会先激活 tab。
   */
  function handleTabMouseDown(event: React.MouseEvent<HTMLDivElement>) {
    if (event.button === 1) {
      event.preventDefault();
    }
  }

  /** tab 区域的辅助键点击：中键直接关闭被点的 tab，不先激活。 */
  function handleTabAuxClick(event: React.MouseEvent<HTMLDivElement>, tab: SidePaneTab) {
    if (event.button === 1) {
      event.preventDefault();
      handleCloseTab(tab);
    }
  }

  /**
   * 开始拖拽调整面板宽度。
   *
   * 面板贴在窗口右侧，所以指针向左移动（clientX 变小）时面板变宽，
   * 宽度增量用 startX 减去当前的 clientX 计算。
   * 监听挂在 window 上，保证指针移出分隔条后还能继续拖动；
   * 松开指针时移除监听并还原鼠标样式。
   */
  function startResize(event: React.PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = width;
    function move(moveEvent: PointerEvent) {
      setWidth(startWidth + (startX - moveEvent.clientX));
    }
    function up() {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.cursor = "";
    }
    document.body.style.cursor = "col-resize";
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  const hasTerminal = visible.some((tab) => tab.type === "terminal");
  const hasDiff = visible.some((tab) => tab.type === "diff");

  return (
    <>
      {/* 左侧的拖拽分隔条，用于调整面板宽度 */}
      {/* biome-ignore lint/a11y/useSemanticElements: 对应的 hr 是静态主题分隔，无法承载指针拖拽交互，此处是可交互的自定义分隔条 */}
      <div
        className="panel-resize"
        role="separator"
        aria-orientation="vertical"
        aria-label="调整面板宽度"
        onPointerDown={startResize}
      />
      <div className="panel-tabs" role="tablist">
        {/* 当前上下文可见的 tab；点击主区域切换，中键或关闭按钮关闭，右键弹菜单 */}
        {visible.map((tab) => (
          // biome-ignore lint/a11y/noStaticElementInteractions: 中键关闭与右键菜单需要覆盖 tab 的非交互区域（主按钮与关闭按钮之间的空隙），两个交互入口本身都有语义元素承载
          <div
            key={tab.id}
            className={cn("panel-tab", "is-tab", tab.id === activeTabId && "is-active")}
            onMouseDown={handleTabMouseDown}
            onAuxClick={(event) => handleTabAuxClick(event, tab)}
            onContextMenu={(event) => {
              event.preventDefault();
              setTabMenu({ tab, anchor: event.currentTarget });
            }}
          >
            <button
              type="button"
              className="panel-tab-main"
              role="tab"
              aria-selected={tab.id === activeTabId}
              title={tab.title}
              onClick={(event) => {
                // 中键不产生 click，这里兜底拦一下，确保只有左键切换 tab。
                if (event.button !== 0) {
                  return;
                }
                activateTab(tab.id);
              }}
            >
              <Icon name={tabIcon(tab)} className="ico" />
              <span className="panel-tab-title">{tab.title}</span>
            </button>
            <button
              type="button"
              className="panel-tab-close"
              aria-label={`关闭 ${tab.title}`}
              title="关闭"
              onClick={() => handleCloseTab(tab)}
            >
              <Icon name="x" className="ico" />
            </button>
          </div>
        ))}
        <span className="panel-tab-spacer" />
        <div className="panel-head-actions">
          {/* singleton tab 的快捷入口：还没有对应 tab 时才显示 */}
          {!hasTerminal ? (
            <button
              type="button"
              className="icon-btn sm"
              title="打开终端"
              aria-label="打开终端"
              onClick={openTerminalTab}
            >
              <Icon name="terminal" className="ico" />
            </button>
          ) : null}
          {!hasDiff ? (
            <button
              type="button"
              className="icon-btn sm"
              title="打开差异"
              aria-label="打开差异"
              onClick={() => openDiffTab("差异")}
            >
              <Icon name="diff" className="ico" />
            </button>
          ) : null}
          {/* 全屏 / 还原：全屏时面板占满中间区域，按钮图标同步变化 */}
          <button
            type="button"
            className="icon-btn sm"
            title={maximized ? "还原面板" : "全屏面板"}
            aria-label={maximized ? "还原面板" : "全屏面板"}
            onClick={toggleMaximized}
          >
            <Icon name={maximized ? "minimize" : "maximize"} className="ico" />
          </button>
        </div>
      </div>
      {/* tab 右键菜单：关闭类操作 + 当前上下文可恢复的最近关闭列表 */}
      {tabMenu ? (
        <Popover anchor={tabMenu.anchor} open onClose={() => setTabMenu(null)} align="left">
          <div className="ctx-menu">
            <button
              type="button"
              className="popover-item"
              onClick={() => {
                setTabMenu(null);
                handleCloseTab(tabMenu.tab);
              }}
            >
              <Icon name="x" />
              <span className="pi-main">
                <div className="pi-title">关闭</div>
              </span>
            </button>
            <button
              type="button"
              className="popover-item"
              onClick={() => {
                setTabMenu(null);
                closeOtherTabs(tabMenu.tab.id);
              }}
            >
              <Icon name="square-minus" />
              <span className="pi-main">
                <div className="pi-title">关闭其他</div>
              </span>
            </button>
            <button
              type="button"
              className="popover-item"
              onClick={() => {
                setTabMenu(null);
                closeAllTabs();
              }}
            >
              <Icon name="x-circle" />
              <span className="pi-main">
                <div className="pi-title">关闭全部</div>
              </span>
            </button>
            <div className="ctx-menu-sep" />
            <div className="popover-label">重新打开关闭的标签页</div>
            {reopenable.length === 0 ? (
              <div className="ctx-menu-empty">没有可恢复的标签页</div>
            ) : (
              reopenable.map((tab) => (
                <button
                  key={tab.id}
                  type="button"
                  className="popover-item"
                  title={tab.title}
                  onClick={() => {
                    setTabMenu(null);
                    reopenRecentClosed(tab.id);
                  }}
                >
                  <Icon name={tabIcon(tab)} />
                  <span className="pi-main">
                    <div className="pi-title">{tab.title}</div>
                  </span>
                </button>
              ))
            )}
          </div>
        </Popover>
      ) : null}
      <div className="panel-body">
        {/* 懒加载面板内容；分包尚未下载完成时显示加载占位 */}
        <Suspense
          fallback={
            <div className="panel-empty">
              <Icon name="loader" size={20} className="spin" />
              <div>正在加载面板…</div>
            </div>
          }
        >
          {/* 没有激活 tab 时显示文件树兜底视图；选中高亮跟随激活的 file tab */}
          {active === null ? <FilesPanel selectedPath={activeFilePath} /> : null}
          {active?.type === "file" ? <FileTabContent tab={active} /> : null}
          {active?.type === "diff" ? <DiffPanel /> : null}
          {active?.type === "terminal" ? <TerminalPanel /> : null}
          {active?.type === "document" ? <DocumentTabContent tab={active} /> : null}
        </Suspense>
      </div>
    </>
  );
}

/**
 * file tab 的内容：直接渲染编辑器。
 *
 * 编辑器数据槽（files[tab.id]）为空（tab 已建但内容未读，例如重开的 tab
 * 或模式切换 reset 后）时先补一次读取；读取完成渲染编辑器。
 * 每个文件的草稿、编辑模式与定位行都在各自的槽里，互不干扰。
 */
function FileTabContent({ tab }: { tab: FileSidePaneTab }) {
  const slot = useWorkspaceStore((state) => state.files[tab.id]);

  // 槽被清（重开 / reset）时重新读取内容；loadFileSlot 内部会写回槽。
  useEffect(() => {
    if (!slot) {
      void useWorkspaceStore.getState().loadFileSlot(tab.id, tab.folderId, tab.path);
    }
  }, [tab.id, tab.folderId, tab.path, slot]);

  if (!slot?.content) {
    return (
      <div className="panel-empty">
        <Icon name="loader" size={20} className="spin" />
        <div>正在打开 {tab.title}…</div>
      </div>
    );
  }
  return <Editor tab={tab} />;
}

/**
 * document tab 的内容。
 *
 * 文档预览数据（doc）在 workspace store 里是单槽，切换 document tab 时
 * 若预览的不是本 tab 的文件，就重新请求预览。
 */
function DocumentTabContent({ tab }: { tab: DocumentSidePaneTab }) {
  const docPath = useWorkspaceStore((state) => state.doc?.path ?? null);

  useEffect(() => {
    if (docPath !== tab.path) {
      void useWorkspaceStore.getState().openDocument(tab.folderId, tab.path);
    }
  }, [tab.folderId, tab.path, docPath]);

  if (docPath !== tab.path) {
    return (
      <div className="panel-empty">
        <Icon name="loader" size={20} className="spin" />
        <div>正在打开 {tab.title}…</div>
      </div>
    );
  }
  return <DocumentPanel />;
}
