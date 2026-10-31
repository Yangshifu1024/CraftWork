import { useCallback, useMemo } from "react";
import { useExternalStoreRuntime, type ThreadMessageLike } from "@assistant-ui/react";
import type { ChatMessage } from "../../lib/types";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

const imagePattern = /\.(png|jpe?g|gif|webp|bmp|heic|heif|svg)$/i;

function convertMessage(message: ChatMessage, reasoningEnabled: boolean): ThreadMessageLike {
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
      // 能力矩阵：当前模型不支持推理时不渲染推理块。
      if (!reasoningEnabled) {
        continue;
      }
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
  const hasEarlier = useThreadStore((state) => state.hasEarlier);
  const loadEarlier = useThreadStore((state) => state.loadEarlier);

  const mode = useAppStore((state) => state.mode);
  const providers = useAppStore((state) => state.providers);
  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const currentModel = useMemo(() => {
    const thread = threads.find((item) => item.id === activeID);
    if (!thread) {
      return undefined;
    }
    return providers.flatMap((provider) => provider.models).find((model) => model.id === thread.modelId);
  }, [threads, activeID, providers]);

  const reasoningEnabled = currentModel?.caps.reasoning ?? false;
  const visionEnabled = currentModel?.caps.vision ?? false;

  const convert = useCallback(
    (message: ChatMessage) => convertMessage(message, reasoningEnabled),
    [reasoningEnabled],
  );

  const handleNew = useCallback(
    async (message: any) => {
      const text = (message.content ?? [])
        .filter((part: any) => part?.type === "text")
        .map((part: any) => String(part.text ?? ""))
        .join("\n");
      const attachments = useThreadStore.getState().attachments.map((item) => item.path);
      if (!visionEnabled && attachments.some((path) => imagePattern.test(path))) {
        useUiStore.getState().toast({
          title: "当前模型不支持图片输入",
          description: "请切换支持图片的模型，或移除图片附件后再发送。",
          tone: "warn",
        });
        return;
      }
      await send(text, attachments);
    },
    [send, visionEnabled],
  );

  const handleCancel = useCallback(async () => {
    await cancel();
  }, [cancel]);

  const handleReload = useCallback(async () => {
    await regenerate();
  }, [regenerate]);

  return useExternalStoreRuntime<ChatMessage>({
    messages,
    convertMessage: convert,
    isRunning: running,
    hasEarlier,
    onLoadEarlier: loadEarlier,
    onNew: handleNew,
    onCancel: handleCancel,
    onReload: handleReload,
  });
}
