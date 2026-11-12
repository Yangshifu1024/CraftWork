// 根组件：布局外壳 + 事件接线。
// 布局：左侧会话侧栏（可收起）+ 主区行（对话列 + 右侧面板：文件树/文档预览）；
// 全屏浮层（命令面板/设置）与独立对话框层（确认/输入/创建项目/图片预览）挂载在布局之上。
import { useEffect, useRef } from "react";
import { useApp } from "./store/app";
import { onEvents, onMenu, onToast } from "./lib/api";
import { Sidebar } from "./components/sidebar";
import { SidebarExpandTab } from "./components/sidebar-expand-tab";
import { ResizeHandle } from "./components/resize-handle";
import { SIDEBAR } from "./lib/resize";
import { Conversation } from "./components/conversation";
import { SidePane } from "./components/panel";
import { useToasts } from "./store/toast";
import { CommandPalette } from "./overlays/palette";
import { SettingsOverlay } from "./overlays/settings";
import { ToastViewport } from "./overlays/toast";
import { Dialogs } from "./overlays/dialogs";
import { CreateProjectDialog } from "./overlays/create-project";
import { EditProjectDialog } from "./overlays/edit-project";
import { ImagePreview } from "./overlays/image-preview";
import { isSettingsSection, useOverlays } from "./store/overlays";
import { useShortcuts } from "./hooks/shortcuts";
import { useNativeContextMenuGuard } from "./hooks/context-menu";

/** 应用根组件：一次性启动装配（boot），订阅事件/toast 推送并接入 applyBatch 与 toast 队列，组装布局与浮层。 */
export default function App(): React.JSX.Element {
  const boot = useApp((s) => s.boot);
  const applyBatch = useApp((s) => s.applyBatch);
  const booted = useApp((s) => s.booted);
  const pushToast = useToasts((s) => s.push);
  const overlay = useOverlays((s) => s.overlay);
  const sidebarVisible = useApp((s) => s.sidebarVisible);
  const sidebarWidth = useApp((s) => s.sidebarWidth);
  const setSidebarWidth = useApp((s) => s.setSidebarWidth);
  const toggleSidebar = useApp((s) => s.toggleSidebar);
  useShortcuts();
  useNativeContextMenuGuard();
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
    // 应用菜单动作（macOS 菜单栏「关于」等）：载荷来自后端，分节按白名单校验后再用。
    const offMenu = onMenu((payload) => {
      if (payload.action === "settings" && isSettingsSection(payload.section)) {
        useOverlays.getState().open("settings", payload.section);
      }
    });
    return () => {
      offEvents();
      offToast();
      offMenu();
    };
  }, [boot, applyBatch, pushToast]);

  if (!booted) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-foreground-subtle">正在启动…</div>
    );
  }

  // 布局主体：左侧会话侧栏（可收起、可拖宽）+ 主区（对话列 + 右侧面板：文件/子代理）。
  // 宽度行为见 D17：宽度只由 CSS 变量 --craft-sidebar-width /
  // --craft-pane-width 提供，拖拽条在拖动期直接改写变量，松手才提交 store（会话内姿态，不持久化）。
  // #content 是右侧面板最大化抽屉的定位上下文（relative），抽屉铺在对话列之上、浮层之下。
  //
  // Windows 自绘标题栏（D22）：窗口控件（window-controls.tsx）随面板状态内联迁移——
  // 面板关时在会话头右端，面板开时在面板 tab 条右端（面板卡片 mt-2 与对话卡片顶对齐）。
  return (
    // 根壳底色用窗口外围 token（bg-background-win-alt）：卡片之外露底的区域归窗口底色
    // （#2b2b2b 暗 / #ececee 亮）；对话/面板卡片仍用 bg-background——「窗口底色」与
    // 「卡片底色」两个语义分离，侧栏底色同走 bg-sidebar（D22 布局）
    <div className="relative flex h-full min-h-0 w-full overflow-hidden bg-background-win-alt">
      {/* 收起态：不渲染侧栏本体，改渲染 36px 展开条——宽度不再靠归零宽度类覆盖，
          收起就是真的不占位，界面上也有可见的恢复入口。 */}
      {sidebarVisible ? (
        <>
          <Sidebar />
          <ResizeHandle
            cssVar="--craft-sidebar-width"
            edge="left"
            value={sidebarWidth}
            min={SIDEBAR.min}
            max={SIDEBAR.max}
            initial={SIDEBAR.initial}
            capHalf
            label="调整侧栏宽度"
            onCommit={(w) => setSidebarWidth(w)}
            onCollapse={() => toggleSidebar()}
          />
        </>
      ) : (
        <SidebarExpandTab />
      )}
      {/* #content 统一 pb-2 下边距；上边距由 conversation（pt-2）与侧栏面板（mt-2）各自提供，
          两张卡片顶部对齐（D22：面板卡片边框/圆角/边距与对话卡片完全一致） */}
      <div id="content" className="relative flex min-w-[320px] flex-1 flex-col pb-2">
        <div className="flex min-h-0 flex-1 flex-row">
          <Conversation />
          <SidePane />
        </div>
      </div>
      {overlay === "palette" && <CommandPalette />}
      {overlay === "settings" && <SettingsOverlay />}
      <Dialogs />
      <CreateProjectDialog />
      <EditProjectDialog />
      <ImagePreview />
      <ToastViewport />
    </div>
  );
}
