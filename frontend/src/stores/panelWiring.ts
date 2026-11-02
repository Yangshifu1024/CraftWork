/**
 * 面板与全局上下文的订阅接线。
 *
 * 单独成一个模块的原因：panel store 需要响应「当前工作区 / 工作模式 /
 * 活动线程」的变化，而 store 之间的模块级循环引用不能在加载期互相调用
 * （ESM 循环依赖里，先加载的一方拿到的绑定还没初始化）。把订阅放在
 * 这个第三方模块里，加载完成时三个 store 都已就绪，注册是安全的。
 *
 * 在应用入口（main.tsx）以副作用方式引入一次即可。
 */
import { normalizeOwner } from "../lib/sidePane";
import { useAppStore } from "./app";
import { usePanelStore } from "./panel";
import { useThreadStore } from "./threads";

// 工作区或工作模式变化 → 切换记忆桶并恢复该上下文的 tab 状态。
useAppStore.subscribe((state, prev) => {
  const workspaceChanged = (state.activeWorkspace?.id ?? "") !== (prev.activeWorkspace?.id ?? "");
  if (workspaceChanged || state.mode !== prev.mode) {
    usePanelStore.getState().syncScope();
  }
});

// 活动线程变化 → 恢复该线程的激活 tab 与开合偏好。
useThreadStore.subscribe((state, prev) => {
  const mode = useAppStore.getState().mode;
  if (normalizeOwner(state.activeID[mode]) !== normalizeOwner(prev.activeID[mode])) {
    usePanelStore.getState().syncScope();
  }
});
