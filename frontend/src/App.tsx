/**
 * 前端根组件：负责应用启动，并把三栏界面交给 AppShell 渲染。
 *
 * 启动顺序（只在首次挂载时执行一次）：
 * 1. `useAppStore.boot()`：读取设置、当前工作区与模式，这是后续所有请求的前提；
 * 2. `useThreadStore.connect()` 与 `useTerminalStore.connect()`：先订阅后端事件，
 *    保证后续拉取数据期间产生的事件不会丢；
 * 3. `useThreadStore.boot()`：加载当前模式下的线程列表；
 * 4. `useWorkspaceStore.loadRoot()`：加载工作区根目录文件树与 Git 状态；
 * 5. `useAppStore.refreshProviders()`：刷新提供商与模型列表，状态栏据此显示上下文占用；
 * 6. `useTerminalStore.boot()`：恢复已有的终端会话。
 *
 * 启动完成后记录当时的模式。之后只要模式在 Code 与 Work 之间切换，
 * 就清空工作区缓存并重新加载线程与文件树，避免两个模式的数据互相串台。
 */
import { useEffect, useRef } from "react";
import { AppShell } from "./app/AppShell";
import { useAppStore } from "./stores/app";
import { useThreadStore } from "./stores/threads";
import { useTerminalStore } from "./stores/terminal";
import { useWorkspaceStore } from "./stores/workspace";

/**
 * 应用根组件。
 *
 * 关键状态：
 * - `bootedRef`：启动是否已执行过，用来扛住 StrictMode 的重复副作用；
 * - `bootedModeRef`：启动完成时的模式，作为「模式是否真的变了」的基准；
 *   为 null 表示启动还没结束，此时不响应模式变化。
 */
export default function App() {
  const bootedRef = useRef(false);
  const bootedModeRef = useRef<string | null>(null);
  const mode = useAppStore((state) => state.mode);

  // 首次挂载时按顺序跑完整套启动流程；StrictMode 下第二次调用会被 bootedRef 挡住。
  useEffect(() => {
    if (bootedRef.current) {
      return;
    }
    bootedRef.current = true;
    void (async () => {
      await useAppStore.getState().boot();
      // 事件订阅要早于数据加载，否则加载期间产生的事件会被漏掉。
      useThreadStore.getState().connect();
      useTerminalStore.getState().connect();
      await useThreadStore.getState().boot();
      await useWorkspaceStore.getState().loadRoot();
      await useAppStore.getState().refreshProviders();
      await useTerminalStore.getState().boot();
      // 记录启动完成时的模式，供第二个 effect 判断模式是否真的发生了变化。
      bootedModeRef.current = useAppStore.getState().mode;
    })();
  }, []);

  // 模式切换后的联动：旧模式的文件树、选中文件、草稿都属于旧工作区，必须整体重置，
  // 再按新模式重新拉取线程列表与文件树。启动尚未完成时直接跳过。
  useEffect(() => {
    if (!bootedModeRef.current) {
      return;
    }
    if (bootedModeRef.current === mode) {
      return;
    }
    bootedModeRef.current = mode;
    useWorkspaceStore.getState().reset();
    void useThreadStore.getState().boot();
    void useWorkspaceStore.getState().loadRoot();
  }, [mode]);

  return <AppShell />;
}
