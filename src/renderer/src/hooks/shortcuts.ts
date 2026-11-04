// 窗口级快捷键：⌘K 命令面板 / ⌘N 新建 / ⌘B 侧栏 / ⌘, 设置 / ⌘D 数据 / ⌘M 模型菜单 /
// ⌘⇧M 审批档位循环 / Esc 退出链。输入框聚焦时 Esc 不拦截（走浏览器默认行为），
// 浮层打开时 Esc 归浮层（各自监听）。
import { useEffect } from "react";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { cycleApprovalLevel } from "../lib/approval";

export function useShortcuts(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const meta = e.metaKey || e.ctrlKey;
      const state = useOverlays.getState();
      const overlay = state.overlay;
      // 确认框 / 输入框（独立叠加层）打开时，窗口级浮层快捷键让位，避免静默丢弃未决的确认操作。
      const dialogOpen = state.confirm !== null || state.prompt !== null;
      if (meta && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (dialogOpen) return;
        if (overlay === "palette") useOverlays.getState().close();
        else if (overlay === "none") useOverlays.getState().open("palette");
        return;
      }
      if (meta && e.key.toLowerCase() === "n") {
        e.preventDefault();
        useApp.getState().newDraft();
        return;
      }
      if (meta && e.key.toLowerCase() === "b") {
        e.preventDefault();
        useApp.getState().toggleSidebar();
        return;
      }
      if (meta && e.key === ",") {
        e.preventDefault();
        if (dialogOpen) return;
        if (overlay === "settings") useOverlays.getState().close();
        else useOverlays.getState().open("settings");
        return;
      }
      if (meta && e.key.toLowerCase() === "d") {
        e.preventDefault();
        // 用量/审计/备份已拆为设置页独立分区，⌘D 直达用量分区。
        if (dialogOpen) return;
        if (overlay === "settings") useOverlays.getState().close();
        else useOverlays.getState().open("settings", "usage");
        return;
      }
      // ⌘⇧M：循环审批档位（对齐 ZCode cycleSessionMode；切入「自由」走危险确认）。
      if (meta && e.shiftKey && e.key.toLowerCase() === "m") {
        e.preventDefault();
        if (overlay === "none" && !dialogOpen) cycleApprovalLevel();
        return;
      }
      // ⌘M：开/关模型菜单（composer picker 排他态在 overlays store）；草稿态同样可用（写入草稿槽）。
      if (meta && e.key.toLowerCase() === "m") {
        e.preventDefault();
        if (overlay !== "none" || dialogOpen) return;
        const { picker, openPicker, closePicker } = useOverlays.getState();
        if (picker === "model") closePicker();
        else openPicker("model");
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
