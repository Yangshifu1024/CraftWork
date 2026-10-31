import { useCallback } from "react";
import { useExternalStoreRuntime, type ThreadMessageLike } from "@assistant-ui/react";
import type { ChatMessage } from "../../lib/types";
import { useThreadStore } from "../../stores/threads";

function convertMessage(message: ChatMessage): ThreadMessageLike {
  const content: any[] = [];
  for (const part of message.parts) {
    if (part.kind === "text") {
      const attachments = Array.isArray(part.data?.attachments) ? part.data.attachments : [];
      if (message.role === "user" && attachments.length > 0) {
        content.push({
          type: "data-cw-attachments",
          data: { partId: `${part.id}_attachments`, attachments },
        });
      }
      content.push({ type: "text", text: String(part.data?.text ?? "") });
      continue;
    }
    if (part.kind === "reasoning") {
      content.push({ type: "reasoning", text: String(part.data?.text ?? "") });
      continue;
    }
    content.push({
      type: `data-cw-${part.kind}`,
      data: { ...part.data, partId: part.id },
    });
  }

  const status =
    message.status === "streaming"
      ? { type: "running" as const }
      : message.status === "error"
        ? { type: "incomplete" as const, reason: "error" as const }
        : message.status === "incomplete"
          ? { type: "incomplete" as const, reason: "cancelled" as const }
          : { type: "complete" as const, reason: "stop" as const };

  const base = {
    id: message.id,
    role: message.role,
    createdAt: new Date(message.createdAt),
    content,
  };

  // assistant-ui 只允许 assistant 消息携带 status，用户消息必须省略该字段。
  return message.role === "assistant" ? { ...base, status } : base;
}

/** 把线程 store 接入 assistant-ui 的外部存储运行时。 */
export function useChatRuntime() {
  const messages = useThreadStore((state) => state.messages);
  const running = useThreadStore((state) => state.running);
  const send = useThreadStore((state) => state.send);
  const cancel = useThreadStore((state) => state.cancel);
  const regenerate = useThreadStore((state) => state.regenerate);

  const handleNew = useCallback(
    async (message: any) => {
      const text = (message.content ?? [])
        .filter((part: any) => part?.type === "text")
        .map((part: any) => String(part.text ?? ""))
        .join("\n");
      const attachments = useThreadStore.getState().attachments.map((item) => item.path);
      await send(text, attachments);
    },
    [send],
  );

  const handleCancel = useCallback(async () => {
    await cancel();
  }, [cancel]);

  const handleReload = useCallback(async () => {
    await regenerate();
  }, [regenerate]);

  return useExternalStoreRuntime<ChatMessage>({
    messages,
    convertMessage,
    isRunning: running,
    onNew: handleNew,
    onCancel: handleCancel,
    onReload: handleReload,
  });
}
