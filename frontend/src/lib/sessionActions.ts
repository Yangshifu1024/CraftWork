/**
 * 会话操作的共享流程：复制会话 Id、重命名、彻底删除。
 *
 * 侧栏会话条目的右键菜单与聊天顶部的更多操作菜单共用这里，
 * 保证确认文案、输入框初值与成功提示在两个入口完全一致；
 * 调用方只负责提供会话对象与关闭自己的浮层。
 */
import { useThreadStore } from "../stores/threads";
import { useUiStore } from "../stores/ui";
import { copyText } from "./clipboard";
import type { ThreadView } from "./types";

/**
 * 复制会话 Id 到剪贴板并提示。
 * Id 是后端生成的唯一标识，用于日志排查与问题反馈。
 */
export async function copySessionID(thread: ThreadView): Promise<void> {
  await copyText(thread.id);
  useUiStore.getState().toast({ title: "已复制会话 Id", description: thread.id, tone: "ok" });
}

/**
 * 弹出输入框重命名会话；用户取消或清空标题时不做修改。
 * 返回 true 表示标题已保存。
 */
export async function renameSessionViaPrompt(thread: ThreadView): Promise<boolean> {
  const next = await useUiStore.getState().askPrompt({
    title: "重命名会话",
    label: "新标题会立即保存。",
    value: thread.title,
    confirmText: "保存",
  });
  if (!next) {
    return false;
  }
  await useThreadStore.getState().rename(thread.id, next);
  return true;
}

/**
 * 彻底删除会话：先弹确认框，确认后连同消息、事件与审批记录一起删除。
 * 返回 true 表示已删除。
 */
export async function deleteSessionViaConfirm(thread: ThreadView): Promise<boolean> {
  const confirmed = await useUiStore.getState().askConfirm({
    title: "彻底删除会话？",
    description: "会话的消息、事件与审批记录都会被删除，无法恢复。",
    detail: thread.title,
    confirmText: "删除",
    danger: true,
  });
  if (!confirmed) {
    return false;
  }
  await useThreadStore.getState().remove(thread.id);
  useUiStore.getState().toast({ title: "会话已删除", tone: "warn" });
  return true;
}
