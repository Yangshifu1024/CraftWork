// 窗口级快捷键：⌘K 命令面板 / ⌘N 新建 / ⌘B 侧栏 / ⌘, 设置 / ⌘D 数据 / ⌘M 模型菜单 /
// ⌘⇧M 审批档位循环 / Esc 退出链。浮层打开时 Esc 归浮层（各自监听）；空闲态运行中
// 会话双击 Esc 确认停止（与停止按钮共享 armed 态）；IME 组合中的 Esc 不拦截。
import { useEffect } from "react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { cycleApprovalLevel } from "../lib/approval";

/** 注册窗口级快捷键（keydown，组件卸载时解绑）；按键路由规则见文件头注释。 */
export function useShortcuts(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      const meta = e.metaKey || e.ctrlKey;
      const state = useOverlays.getState();
      const overlay = state.overlay;
      // 确认框 / 输入框 / 创建项目对话框（独立叠加层）打开时，窗口级浮层快捷键让位，避免静默丢弃未决操作。
      const dialogOpen = state.confirm !== null || state.prompt !== null || state.createProject || state.freeLevel !== null;
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
      // 必须先于 ⌘M 分支判定，否则带 Shift 的按键会先命中 ⌘M。
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
      if (e.key === "Escape" && overlay === "none" && !dialogOpen && state.picker === null) {
        // Esc 退出链（恢复 Gen1 语义）：浮层/对话框/picker 打开时归各自处理（上方让位），
        // 空闲态到达这里——运行中会话双击 Esc 确认停止（与停止按钮共享 armed 态，800ms）。
        // IME 组合中的 Esc 是取消候选，不拦截；右键菜单等组件层自收。
        if (e.isComposing || e.keyCode === 229) return;
        const app = useApp.getState();
        const activeId = app.activeThreadId[app.mode];
        if (!activeId || !app.chats[activeId]?.running) return;
        e.preventDefault();
        const overlays = useOverlays.getState();
        if (overlays.stopArmed) {
          overlays.setStopArmed(false);
          void api.chatCancel(activeId).catch(() => {});
        } else {
          overlays.setStopArmed(true);
          setTimeout(() => {
            const s = useOverlays.getState();
            if (s.stopArmed) s.setStopArmed(false);
          }, 800);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
