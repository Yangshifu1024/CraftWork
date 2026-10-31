import { useEffect } from "react";
import { onBatch } from "../../lib/bus";
import { EVENT, type Mode } from "../../lib/types";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

const modes: Mode[] = ["code", "work"];

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

function truncate(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit)}…`;
}

/**
 * 监听后台线程事件，在窗口失焦时给出可点击的应用内通知。
 *
 * 真正的系统通知由 Go 侧发送（见 internal/notify）；这里的提示保证用户
 * 点击后能回到对应线程，也覆盖 WebView 不支持系统通知的场景。
 */
export function useNotifications() {
  useEffect(() => {
    const unsubscribe = onBatch((batch) => {
      const settings = useAppStore.getState().settings;
      if (!settings.notifications) {
        return;
      }
      if (document.visibilityState === "visible" && document.hasFocus()) {
        return;
      }
      for (const event of batch.events ?? []) {
        const threadID = event.threadId ?? "";
        if (!threadID || threadID.startsWith("terminal:")) {
          continue;
        }
        const info = findThread(threadID);
        if (event.type === EVENT.approvalRequested) {
          useUiStore.getState().toast({
            title: "需要审批",
            description: `${String(event.data?.tool ?? "工具")} 正在等待你的决定`,
            tone: "warn",
            onClick: () => void openThread(threadID),
          });
        } else if (event.type === EVENT.runCompleted) {
          useUiStore.getState().toast({
            title: "回复已完成",
            description: info?.title ?? "点击回到对应线程",
            tone: "ok",
            onClick: () => void openThread(threadID),
          });
        } else if (event.type === EVENT.runFailed) {
          useUiStore.getState().toast({
            title: "运行失败",
            description: truncate(String(event.data?.message ?? "请打开线程查看详情"), 120),
            tone: "danger",
            onClick: () => void openThread(threadID),
          });
        }
      }
    });
    return unsubscribe;
  }, []);
}
