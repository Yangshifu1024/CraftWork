/**
 * 后台会话通知。
 *
 * 职责：订阅 `cw:events` 批量事件，在窗口失焦时用应用内 toast 提醒
 * 审批请求、运行完成与运行失败，并支持点击回到对应会话。
 *
 * 触发条件（全部满足才会提示）：
 * - 用户设置里开启了通知。
 * - 窗口不可见或没有焦点，避免用户正在看聊天时重复打扰。
 * - 事件携带会话 ID 且不属于 `terminal:` 前缀的终端会话。
 */
import { useEffect } from "react";
import { onBatch } from "../../lib/bus";
import { EVENT, type Mode } from "../../lib/types";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

/** 需要扫描的模式列表；会话 ID 只保证在各自模式内唯一。 */
const modes: Mode[] = ["code", "work"];

/**
 * 按会话 ID 找到它所属的模式与标题。
 * 会话列表按模式分开存放，因此需要逐个模式查找。
 */
function findThread(threadID: string): { mode: Mode; title: string } | null {
  const state = useThreadStore.getState();
  for (const mode of modes) {
    const thread = state.threads[mode].find((item) => item.id === threadID);
    if (thread) {
      return { mode, title: thread.title };
    }
  }
  return null;
}

/**
 * 点击通知后跳转到对应会话。
 * 会话在另一个模式时先切换模式，再选中会话；会话已被删除时直接忽略。
 */
async function openThread(threadID: string) {
  const info = findThread(threadID);
  if (!info) {
    return;
  }
  const app = useAppStore.getState();
  if (app.mode !== info.mode) {
    await app.setMode(info.mode);
  }
  await useThreadStore.getState().select(threadID);
}

/** 截断过长的通知正文，避免 toast 被大段错误信息撑开。 */
function truncate(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit)}…`;
}

/**
 * 监听后台会话事件，在窗口失焦时给出可点击的应用内通知。
 *
 * 真正的系统通知由 Go 侧发送（见 internal/notify）；这里的提示保证用户
 * 点击后能回到对应会话，也覆盖 WebView 不支持系统通知的场景。
 *
 * 处理的事件：
 * - `approval.requested`：有工具在等待审批，提示工具名。
 * - `run.completed`：本轮回复结束，提示会话标题。
 * - `run.failed`：运行失败，提示截断后的错误信息。
 *
 * 注意：只在挂载时订阅一次，卸载时退订；设置与会话状态在回调内实时读取。
 */
export function useNotifications() {
  useEffect(() => {
    const unsubscribe = onBatch((batch) => {
      // 通知开关必须实时读取，用户关闭后立即生效。
      const settings = useAppStore.getState().settings;
      if (!settings.notifications) {
        return;
      }
      // 窗口可见且有焦点时用户已经在看界面，不再重复提示。
      if (document.visibilityState === "visible" && document.hasFocus()) {
        return;
      }
      for (const event of batch.events ?? []) {
        const threadID = event.threadId ?? "";
        // 没有会话归属的事件与终端会话事件不属于聊天通知范围。
        if (!threadID || threadID.startsWith("terminal:")) {
          continue;
        }
        const info = findThread(threadID);
        // 审批请求优先级最高：用户不处理，run 会一直停在这里。
        if (event.type === EVENT.approvalRequested) {
          useUiStore.getState().toast({
            title: "需要审批",
            description: `${String(event.data?.tool ?? "工具")} 正在等待你的决定`,
            tone: "warn",
            onClick: () => void openThread(threadID),
          });
        } else if (event.type === EVENT.runCompleted) {
          // 完成通知用会话标题作为说明，找不到会话时给通用提示。
          useUiStore.getState().toast({
            title: "回复已完成",
            description: info?.title ?? "点击回到对应会话",
            tone: "ok",
            onClick: () => void openThread(threadID),
          });
        } else if (event.type === EVENT.runFailed) {
          // 失败信息可能很长，截断后完整内容留给会话里的错误卡片展示。
          useUiStore.getState().toast({
            title: "运行失败",
            description: truncate(String(event.data?.message ?? "请打开会话查看详情"), 120),
            tone: "danger",
            onClick: () => void openThread(threadID),
          });
        }
      }
    });
    return unsubscribe;
  }, []);
}
