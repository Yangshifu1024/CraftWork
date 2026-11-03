// 根组件：布局外壳 + 事件接线。
// 布局：左侧会话侧栏（可收起）+ 拖拽分隔 + 主区（对话列，含顶栏/消息流/输入区/终端面板）。
import { useEffect, useRef } from "react";
import { useApp } from "./store/app";
import { onEvents, onToast } from "./lib/api";
import { Sidebar } from "./components/sidebar";
import { Conversation } from "./components/conversation";
import { SidePane } from "./components/panel";
import { useToasts } from "./store/toast";
import { CommandPalette } from "./overlays/palette";
import { SettingsOverlay } from "./overlays/settings";
import { DataOverlay } from "./overlays/data";
import { ToastViewport } from "./overlays/toast";
import { Dialogs } from "./overlays/dialogs";
import { useOverlays } from "./store/overlays";
import { useShortcuts } from "./hooks/shortcuts";

export default function App(): React.JSX.Element {
  const boot = useApp((s) => s.boot);
  const applyBatch = useApp((s) => s.applyBatch);
  const booted = useApp((s) => s.booted);
  const pushToast = useToasts((s) => s.push);
  const overlay = useOverlays((s) => s.overlay);
  useShortcuts();
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    void boot();
    const offEvents = onEvents((batch) => applyBatch(batch));
    const offToast = onToast((payload) => pushToast(payload.tone, payload.title, payload.message));
    return () => {
      offEvents();
      offToast();
    };
  }, [boot, applyBatch, pushToast]);

  if (!booted) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-foreground-subtle">正在启动…</div>
    );
  }

  return (
    <div className="relative flex h-full min-h-0 w-full overflow-hidden">
      <Sidebar />
      <Conversation />
      <SidePane />
      {overlay === "palette" && <CommandPalette />}
      {overlay === "settings" && <SettingsOverlay />}
      {overlay === "data" && <DataOverlay />}
      <Dialogs />
      <ToastViewport />
    </div>
  );
}
