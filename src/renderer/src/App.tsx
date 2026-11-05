// 根组件：布局外壳 + 事件接线。
// 布局：左侧会话侧栏（可收起）+ 拖拽分隔 + 主区行（对话列 + 右侧面板，含终端/文件/文档）。
import { useEffect, useRef } from "react";
import { useApp } from "./store/app";
import { onEvents, onToast } from "./lib/api";
import { Sidebar } from "./components/sidebar";
import { Conversation } from "./components/conversation";
import { SidePane } from "./components/panel";
import { useToasts } from "./store/toast";
import { CommandPalette } from "./overlays/palette";
import { SettingsOverlay } from "./overlays/settings";
import { ToastViewport } from "./overlays/toast";
import { Dialogs } from "./overlays/dialogs";
import { CreateProjectDialog } from "./overlays/create-project";
import { ImagePreview } from "./overlays/image-preview";
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
    // boot 只执行一次；订阅不设防——StrictMode 双执行会先 cleanup 再重跑 effect，
    // 若订阅也被防护跳过，事件通道将永久失效（dev 下界面收不到任何推送）。
    if (!started.current) {
      started.current = true;
      void boot();
    }
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
    <div className="relative flex h-full min-h-0 w-full overflow-hidden bg-background">
      <Sidebar />
      <div
        role="separator"
        aria-label="调整侧栏宽度"
        className="group/handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize bg-transparent after:absolute after:top-0 after:mx-auto after:h-full after:w-0.5 after:rounded-full after:bg-foreground-subtlest/50 after:opacity-0 hover:after:opacity-100"
      />
      <div id="content" className="flex min-w-[320px] flex-1 flex-col pb-1 pr-1">
        <div className="flex min-h-0 flex-1 flex-row">
          <Conversation />
          <SidePane />
        </div>
      </div>
      {overlay === "palette" && <CommandPalette />}
      {overlay === "settings" && <SettingsOverlay />}
      <Dialogs />
      <CreateProjectDialog />
      <ImagePreview />
      <ToastViewport />
    </div>
  );
}
