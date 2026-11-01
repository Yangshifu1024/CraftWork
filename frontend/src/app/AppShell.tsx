/**
 * 应用外壳：搭出「侧边栏 + 对话区 + 右侧面板 + 状态栏」的三栏布局，
 * 并统一挂载全局浮层（通知、命令面板、设置、数据面板、确认框、输入框）。
 *
 * 这里承担三类跨组件的职责：
 * 1. 把面板宽度写入 CSS 变量 `--panel-w`，让样式能跟着拖拽实时变化；
 * 2. 用媒体查询判断窄窗口，窄窗口下面板改为浮层，不再挤压对话区；
 * 3. 注册全局快捷键（新建对话、搜索、设置、命令面板、Esc 逐层退出）。
 */
import { lazy, Suspense, useEffect } from "react";
import { Sidebar } from "../features/sidebar/Sidebar";
import { Statusbar } from "../features/statusbar/Statusbar";
import { ChatPane } from "../features/chat/ChatPane";
import { useNotifications } from "../features/chat/useNotifications";
import { Panel } from "../features/panel/Panel";
import { Toasts } from "../components/Toasts";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { PromptDialog } from "../components/PromptDialog";
import { cn } from "../lib/utils";
import { usePanelStore } from "../stores/panel";
import { useThreadStore } from "../stores/threads";
import { useUiStore } from "../stores/ui";

// 命令面板、设置、数据面板体积较大且按需出现，用 lazy 拆成独立分包，
// 只有真正打开时才加载对应代码。
const CommandPalette = lazy(() =>
  import("../features/overlays/CommandPalette").then((module) => ({ default: module.CommandPalette })),
);
const SettingsModal = lazy(() =>
  import("../features/overlays/SettingsModal").then((module) => ({ default: module.SettingsModal })),
);
const DataModal = lazy(() =>
  import("../features/overlays/DataModal").then((module) => ({ default: module.DataModal })),
);

/**
 * 三栏布局与全局浮层的容器组件，不接收 props。
 *
 * 关键状态：
 * - 面板状态来自 `usePanelStore`：宽度、是否展开、是否最大化、是否处于窄窗口；
 * - 浮层开关来自 `useUiStore`：命令面板、设置、数据面板；
 * - 线程运行与待审批状态来自 `useThreadStore`，供 Esc 键逐层退出时使用。
 */
export function AppShell() {
  // 接入系统通知：线程在后台完成或出错时提醒用户。
  useNotifications();
  const width = usePanelStore((state) => state.width);
  const open = usePanelStore((state) => state.open);
  const maximized = usePanelStore((state) => state.maximized);
  const narrow = usePanelStore((state) => state.narrow);
  const setNarrow = usePanelStore((state) => state.setNarrow);
  const setMaximized = usePanelStore((state) => state.setMaximized);
  const paletteOpen = useUiStore((state) => state.paletteOpen);
  const settingsOpen = useUiStore((state) => state.settingsOpen);
  const dataOpen = useUiStore((state) => state.dataOpen);

  // 面板宽度由 store 管理，样式层通过 CSS 变量读取，宽度一变布局立刻跟随。
  useEffect(() => {
    document.documentElement.style.setProperty("--panel-w", `${width}px`);
  }, [width]);

  // 窄窗口判定：视口宽度小于 1180px 时面板改为浮层覆盖在对话区上方。
  // 监听 change 而不是只在挂载时判断，保证拖动窗口大小时能实时切换。
  useEffect(() => {
    const media = window.matchMedia("(max-width: 1179px)");
    const apply = () => setNarrow(media.matches);
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [setNarrow]);

  // 全局快捷键：
  // - Cmd/Ctrl + K 打开命令面板，Cmd/Ctrl + N 新建对话；
  // - Cmd/Ctrl + , 打开设置，Cmd/Ctrl + F 聚焦线程搜索；
  // - Esc 按「浮层 → 最大化面板 → 运行中的线程 → 待审批」的优先级逐层退出，
  //   一次只处理最上层的那件事，避免一次按键关掉所有东西。
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
        // 搜索框在侧边栏内部，用自定义 DOM 事件通知它聚焦，避免把 ref 层层透传。
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
          // 按 Esc 视为拒绝最后一条审批请求，范围固定为「仅本次」。
          void threadStore.decide(pending[pending.length - 1].approvalId, false, "once");
        }
      }
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [setMaximized]);

  // 面板的三种形态：常规停靠、窄窗口浮层、最大化铺满对话区。
  // 互斥关系由这里统一计算，样式层只按 class 渲染。
  const panelClass = cn(
    "panel",
    narrow && open && !maximized && "is-overlay",
    maximized && "is-maximized",
  );

  return (
    <>
      <div className="app">
        <div className="body">
          {/* 左侧：工作区与线程列表 */}
          <Sidebar />
          {/* 中间：对话主区域 */}
          <ChatPane />
          {/* 右侧：文件 / 文档 / 差异 / 终端面板，隐藏时仍保留 DOM 以维持内部状态 */}
          <aside className={panelClass} hidden={!open}>
            <Panel />
          </aside>
        </div>
        {/* 底部：模式、上下文占用、审批与安全状态 */}
        <Statusbar />
      </div>
      {/* 全局通知层 */}
      <Toasts />
      {/* 以下浮层按需渲染：未打开时不挂载，打开时才请求对应的懒加载分包 */}
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
      {/* 确认框与输入框常驻挂载，由 store 里的请求对象决定是否显示 */}
      <ConfirmDialog />
      <PromptDialog />
    </>
  );
}
