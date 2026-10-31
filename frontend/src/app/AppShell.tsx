import { lazy, Suspense, useEffect } from "react";
import { Sidebar } from "../features/sidebar/Sidebar";
import { Statusbar } from "../features/statusbar/Statusbar";
import { ChatPane } from "../features/chat/ChatPane";
import { Panel } from "../features/panel/Panel";
import { Toasts } from "../components/Toasts";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { PromptDialog } from "../components/PromptDialog";
import { cn } from "../lib/utils";
import { usePanelStore } from "../stores/panel";
import { useThreadStore } from "../stores/threads";
import { useUiStore } from "../stores/ui";

const CommandPalette = lazy(() =>
  import("../features/overlays/CommandPalette").then((module) => ({ default: module.CommandPalette })),
);
const SettingsModal = lazy(() =>
  import("../features/overlays/SettingsModal").then((module) => ({ default: module.SettingsModal })),
);
const DataModal = lazy(() =>
  import("../features/overlays/DataModal").then((module) => ({ default: module.DataModal })),
);

export function AppShell() {
  const width = usePanelStore((state) => state.width);
  const open = usePanelStore((state) => state.open);
  const maximized = usePanelStore((state) => state.maximized);
  const narrow = usePanelStore((state) => state.narrow);
  const setNarrow = usePanelStore((state) => state.setNarrow);
  const setMaximized = usePanelStore((state) => state.setMaximized);
  const paletteOpen = useUiStore((state) => state.paletteOpen);
  const settingsOpen = useUiStore((state) => state.settingsOpen);
  const dataOpen = useUiStore((state) => state.dataOpen);

  useEffect(() => {
    document.documentElement.style.setProperty("--panel-w", `${width}px`);
  }, [width]);

  useEffect(() => {
    const media = window.matchMedia("(max-width: 1179px)");
    const apply = () => setNarrow(media.matches);
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [setNarrow]);

  useEffect(() => {
    function handleKey(event: KeyboardEvent) {
      const meta = event.metaKey || event.ctrlKey;
      const key = event.key.toLowerCase();
      if (meta && key === "k") {
        event.preventDefault();
        useUiStore.getState().openPalette();
        return;
      }
      if (meta && key === "n") {
        event.preventDefault();
        void useThreadStore.getState().create();
        return;
      }
      if (meta && event.key === ",") {
        event.preventDefault();
        useUiStore.getState().openSettings();
        return;
      }
      if (meta && key === "f") {
        event.preventDefault();
        window.dispatchEvent(new Event("cw:focus-thread-search"));
        return;
      }
      if (event.key === "Escape") {
        const ui = useUiStore.getState();
        if (ui.paletteOpen) {
          ui.closePalette();
          return;
        }
        if (ui.settingsOpen) {
          ui.closeSettings();
          return;
        }
        if (ui.dataOpen) {
          ui.closeData();
          return;
        }
        if (usePanelStore.getState().maximized) {
          setMaximized(false);
          return;
        }
        const threadStore = useThreadStore.getState();
        if (threadStore.running) {
          void threadStore.cancel();
          return;
        }
        const pending = threadStore.pendingApprovals;
        if (pending.length > 0) {
          void threadStore.decide(pending[pending.length - 1].approvalId, false, "once");
        }
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [setMaximized]);

  const panelClass = cn(
    "panel",
    narrow && open && !maximized && "is-overlay",
    maximized && "is-maximized",
  );

  return (
    <>
      <div className="app">
        <div className="body">
          <Sidebar />
          <ChatPane />
          <aside className={panelClass} hidden={!open}>
            <Panel />
          </aside>
        </div>
        <Statusbar />
      </div>
      <Toasts />
      {paletteOpen ? (
        <Suspense fallback={null}>
          <CommandPalette />
        </Suspense>
      ) : null}
      {settingsOpen ? (
        <Suspense fallback={null}>
          <SettingsModal />
        </Suspense>
      ) : null}
      {dataOpen ? (
        <Suspense fallback={null}>
          <DataModal />
        </Suspense>
      ) : null}
      <ConfirmDialog />
      <PromptDialog />
    </>
  );
}
