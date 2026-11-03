// 窗口级快捷键：⌘K 命令面板 / ⌘N 新建 / ⌘B 侧栏 / ⌘, 设置 / ⌘D 数据 / Esc 退出链。
// 输入框聚焦时 Esc 不拦截（走浏览器默认行为），浮层打开时 Esc 归浮层（各自监听）。
import { useEffect } from "react";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";

export function useShortcuts(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const meta = e.metaKey || e.ctrlKey;
      const overlay = useOverlays.getState().overlay;
      if (meta && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (overlay === "palette") useOverlays.getState().close();
        else if (overlay === "none") useOverlays.getState().open("palette");
        return;
      }
      if (meta && e.key.toLowerCase() === "n") {
        e.preventDefault();
        void useApp.getState().createThread();
        return;
      }
      if (meta && e.key.toLowerCase() === "b") {
        e.preventDefault();
        useApp.getState().toggleSidebar();
        return;
      }
      if (meta && e.key === ",") {
        e.preventDefault();
        if (overlay === "settings") useOverlays.getState().close();
        else useOverlays.getState().open("settings");
        return;
      }
      if (meta && e.key.toLowerCase() === "d") {
        e.preventDefault();
        useOverlays.getState().open("data");
        return;
      }
      if (e.key === "Escape" && overlay === "none") {
        // 空闲态 Esc：收起已打开的右键菜单等由组件层处理；这里收侧栏无意义，忽略
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
