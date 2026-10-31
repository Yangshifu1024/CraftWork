import { lazy, Suspense } from "react";
import { Icon, type IconName } from "../../components/Icon";
import type { Mode, PanelTab } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";

const FilesPanel = lazy(() => import("./FilesPanel").then((module) => ({ default: module.FilesPanel })));
const DiffPanel = lazy(() => import("./DiffPanel").then((module) => ({ default: module.DiffPanel })));
const TerminalPanel = lazy(() => import("./TerminalPanel").then((module) => ({ default: module.TerminalPanel })));
const DocumentPanel = lazy(() => import("./DocumentPanel").then((module) => ({ default: module.DocumentPanel })));

interface TabSpec {
  id: PanelTab;
  label: string;
  icon: IconName;
}

const codeTabs: TabSpec[] = [
  { id: "files", label: "文件", icon: "folder-open" },
  { id: "diff", label: "差异", icon: "diff" },
  { id: "terminal", label: "终端", icon: "terminal" },
];

const workTabs: TabSpec[] = [
  { id: "document", label: "文档", icon: "file-text" },
  { id: "diff", label: "差异", icon: "diff" },
  { id: "files", label: "文件", icon: "folder-open" },
];

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
      <div className="panel-resize" role="separator" aria-orientation="vertical" aria-label="调整面板宽度" onPointerDown={startResize} />
      <div className="panel-tabs" role="tablist">
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
        <Suspense
          fallback={
            <div className="panel-empty">
              <Icon name="loader" size={20} className="spin" />
              <div>正在加载面板…</div>
            </div>
          }
        >
          {tab === "files" ? <FilesPanel /> : null}
          {tab === "diff" ? <DiffPanel /> : null}
          {tab === "terminal" ? <TerminalPanel /> : null}
          {tab === "document" ? <DocumentPanel /> : null}
        </Suspense>
      </div>
    </>
  );
}

export function panelLabel(mode: Mode): string {
  return mode === "work" ? "Work" : "Code";
}
