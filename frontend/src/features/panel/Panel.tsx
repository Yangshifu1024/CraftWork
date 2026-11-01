/**
 * 右侧面板容器。
 *
 * 负责：切换面板标签（文件 / 差异 / 终端 / 文档）、渲染对应面板、
 * 通过拖拽分隔条调整面板宽度，以及在普通与全屏之间切换面板。
 *
 * 面板内容按标签懒加载，避免编辑器、终端、文档预览拖慢首屏。
 */
import { lazy, Suspense } from "react";
import { Icon, type IconName } from "../../components/Icon";
import type { Mode, PanelTab } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";

// 四个面板都按需加载：首次切换到对应标签时才拉取分包代码。
const FilesPanel = lazy(() => import("./FilesPanel").then((module) => ({ default: module.FilesPanel })));
const DiffPanel = lazy(() => import("./DiffPanel").then((module) => ({ default: module.DiffPanel })));
const TerminalPanel = lazy(() => import("./TerminalPanel").then((module) => ({ default: module.TerminalPanel })));
const DocumentPanel = lazy(() => import("./DocumentPanel").then((module) => ({ default: module.DocumentPanel })));

/**
 * 面板标签的展示描述。
 * id 对应 store 里记录当前标签的键，label 是按钮文字，icon 是图标名称。
 */
interface TabSpec {
  id: PanelTab;
  label: string;
  icon: IconName;
}

/** Code 模式下的标签顺序：文件、差异、终端。 */
const codeTabs: TabSpec[] = [
  { id: "files", label: "文件", icon: "folder-open" },
  { id: "diff", label: "差异", icon: "diff" },
  { id: "terminal", label: "终端", icon: "terminal" },
];

/** Work 模式下的标签顺序：文档优先，其次是差异与文件。 */
const workTabs: TabSpec[] = [
  { id: "document", label: "文档", icon: "file-text" },
  { id: "diff", label: "差异", icon: "diff" },
  { id: "files", label: "文件", icon: "folder-open" },
];

/**
 * 右侧面板根组件。
 *
 * 状态来自两个 store：
 * - useAppStore 提供当前模式（Code / Work），决定显示哪组标签；
 * - usePanelStore 提供每个模式各自的当前标签、面板宽度与是否全屏。
 */
export function Panel() {
  const mode = useAppStore((state) => state.mode);
  const tabs = usePanelStore((state) => state.tabs);
  const setTab = usePanelStore((state) => state.setTab);
  const width = usePanelStore((state) => state.width);
  const setWidth = usePanelStore((state) => state.setWidth);
  const maximized = usePanelStore((state) => state.maximized);
  const toggleMaximized = usePanelStore((state) => state.toggleMaximized);

  const tab = tabs[mode];
  const specs = mode === "work" ? workTabs : codeTabs;

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

  return (
    <>
      {/* 左侧的拖拽分隔条，用于调整面板宽度 */}
      <div className="panel-resize" role="separator" aria-orientation="vertical" aria-label="调整面板宽度" onPointerDown={startResize} />
      <div className="panel-tabs" role="tablist">
        {/* 当前模式下的标签按钮，点击后切换面板内容 */}
        {specs.map((spec) => (
          <button
            key={spec.id}
            type="button"
            className={cn("panel-tab", tab === spec.id && "is-active")}
            role="tab"
            aria-selected={tab === spec.id}
            onClick={() => setTab(mode, spec.id)}
          >
            <Icon name={spec.icon} className="ico" />
            {spec.label}
          </button>
        ))}
        <span className="panel-tab-spacer" />
        <div className="panel-head-actions">
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
          {/* 每次只渲染当前标签对应的面板，其余标签返回 null */}
          {tab === "files" ? <FilesPanel /> : null}
          {tab === "diff" ? <DiffPanel /> : null}
          {tab === "terminal" ? <TerminalPanel /> : null}
          {tab === "document" ? <DocumentPanel /> : null}
        </Suspense>
      </div>
    </>
  );
}

/**
 * 返回模式对应的面板标题文案。
 * 目前只区分 Code 与 Work 两种模式。
 */
export function panelLabel(mode: Mode): string {
  return mode === "work" ? "Work" : "Code";
}
