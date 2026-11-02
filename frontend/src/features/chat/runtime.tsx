/**
 * 聊天运行时适配层。
 *
 * 作用：把应用自己的会话 store（消息、发送、取消、重新生成、加载更早消息）
 * 适配成 assistant-ui 需要的外部存储运行时，并提供
 * `ChatMessage -> ThreadMessageLike` 的消息格式转换。
 *
 * 关键规则：
 * - 后端消息的 `parts[].kind` 决定前端渲染成什么部件：`text` 与 `reasoning`
 *   走 assistant-ui 内置部件，其余类型统一映射为 `data-cw-*` 自定义部件，
 *   再由 MessageView 里的部件注册表接管渲染。
 * - assistant-ui 只允许 assistant 消息携带 `status`，用户消息必须省略该字段。
 * - 当前模型的能力（是否支持推理、是否支持图片）会参与消息转换与发送前校验。
 */
import { useCallback, useMemo } from "react";
import { useExternalStoreRuntime, type ThreadMessageLike } from "@assistant-ui/react";
import type { ChatMessage } from "../../lib/types";
import { matchModelInfo } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";

/**
 * 把后端的一条消息转换成 assistant-ui 能识别的消息结构。
 *
 * @param message 后端存储的消息，包含角色、状态与按顺序排列的部件。
 * @param reasoningEnabled 当前模型是否支持推理；为 false 时丢弃推理部件，
 *   避免在不支持推理的模型上显示空的“思考过程”块。
 * @returns assistant-ui 的消息对象；用户消息不带 status 字段。
 *
 * 转换规则：
 * - 文本部件：用户消息如果带附件，会先插入一个 `data-cw-attachments` 部件，
 *   再插入正文；顺序与后端一致，保证附件显示在文字上方。
 * - 推理部件：仅在模型支持推理时保留。
 * - 其他部件（工具、审批、任务清单、错误等）：按 `data-cw-<kind>` 命名，
 *   由 MessageView 的 `partComponents.data.by_name` 注册表匹配渲染。
 * - 状态映射：streaming -> running；error -> incomplete(error)；
 *   incomplete -> incomplete(cancelled)；其余视为 complete(stop)。
 */
export function convertMessage(message: ChatMessage, reasoningEnabled: boolean): ThreadMessageLike {
  const content: any[] = [];
  // 流式中的判定依据：消息还在生成且该部件是消息的最后一个部件——
  // 最后一个部件还在增长说明它正在流式输出，「正在思考」状态由它驱动。
  const lastIndex = message.parts.length - 1;
  // 「最终总结」取最后一个正文部件；它之前的正文视为过程内容，转成
  // history-text 数据部件参与折叠。这样工具间隙的说明文字不会把
  // 相邻的过程部件切成多段折叠区，最终回复始终只有最后一段。
  let finalTextIndex = -1;
  if (message.role === "assistant") {
    for (let index = message.parts.length - 1; index >= 0; index -= 1) {
      if (message.parts[index].kind === "text") {
        finalTextIndex = index;
        break;
      }
    }
  }
  message.parts.forEach((part, index) => {
    if (part.kind === "text") {
      const attachments = Array.isArray(part.data?.attachments) ? part.data.attachments : [];
      if (message.role === "user" && attachments.length > 0) {
        content.push({
          type: "data-cw-attachments",
          data: { partId: `${part.id}_attachments`, attachments },
        });
      }
      if (message.role === "assistant" && index !== finalTextIndex) {
        content.push({
          type: "data-cw-history-text",
          data: { partId: part.id, text: String(part.data?.text ?? "") },
        });
        return;
      }
      content.push({ type: "text", text: String(part.data?.text ?? "") });
      return;
    }
    if (part.kind === "reasoning") {
      // 能力矩阵：当前模型不支持推理时不渲染推理块。
      if (!reasoningEnabled) {
        return;
      }
      content.push({
        // 推理块走自定义数据部件而不是 assistant-ui 内置的 reasoning 类型：
        // 需要携带思考时长与流式状态等扩展字段，data 部件的载荷不受类型约束。
        type: "data-cw-reasoning",
        data: {
          ...part.data,
          partId: part.id,
          streaming:
            message.role === "assistant" &&
            message.status === "streaming" &&
            index === lastIndex,
        },
      });
      return;
    }
    content.push({
      // 非文本部件统一转成自定义数据部件；partId 用于 React 列表的稳定标识。
      type: `data-cw-${part.kind}`,
      data: { ...part.data, partId: part.id },
    });
  });

  // assistant-ui 的状态模型只有 running / incomplete / complete 三类，
  // 这里把后端的四种状态折叠过去，并保留“出错”与“已取消”的区分。
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

/**
 * 把会话 store 接入 assistant-ui 的外部存储运行时。
 *
 * 负责：
 * - 提供消息列表与运行状态，assistant-ui 据此渲染消息、显示“生成中”与停止按钮。
 * - 把发送、取消、重新生成、加载更早消息转发给会话 store。
 * - 根据当前会话选中的模型读取能力标记，决定推理块是否渲染、是否允许图片附件。
 *
 * @returns assistant-ui 的运行时对象，需要放进 `AssistantRuntimeProvider`。
 */
export function useChatRuntime() {
  const messages = useThreadStore((state) => state.messages);
  const running = useThreadStore((state) => state.running);
  const submit = useThreadStore((state) => state.submit);
  const cancel = useThreadStore((state) => state.cancel);
  const regenerate = useThreadStore((state) => state.regenerate);
  const hasEarlier = useThreadStore((state) => state.hasEarlier);
  const loadEarlier = useThreadStore((state) => state.loadEarlier);

  const mode = useAppStore((state) => state.mode);
  const providers = useAppStore((state) => state.providers);
  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  // 当前会话选中的模型决定能力矩阵；找不到模型时按“不支持推理”处理。
  // 发送侧的能力门控由 threads.submit 负责，这里只服务消息渲染。
  const reasoningEnabled = useMemo(() => {
    const thread = threads.find((item) => item.id === activeID);
    return matchModelInfo(providers, thread?.modelId)?.caps.reasoning ?? false;
  }, [threads, activeID, providers]);

  // 转换函数依赖当前模型是否支持推理，模型切换后需要重新生成转换器。
  const convert = useCallback(
    (message: ChatMessage) => convertMessage(message, reasoningEnabled),
    [reasoningEnabled],
  );

  /**
   * 处理 assistant-ui 的“发送新消息”回调。
   *
   * 只负责从消息内容拼出纯文本并取走附件草稿；模型校验、能力门控、
   * 运行中转引导与自动建会话都由 threads.submit 的唯一策略点负责。
   */
  const handleNew = useCallback(
    async (message: any) => {
      const text = (message.content ?? [])
        .filter((part: any) => part?.type === "text")
        .map((part: any) => String(part.text ?? ""))
        .join("\n");
      const attachments = useThreadStore.getState().attachments.map((item) => item.path);
      await submit(text, attachments);
    },
    [submit],
  );

  // 取消当前生成：后端收到后会终止 run，并把消息标记为 incomplete。
  const handleCancel = useCallback(async () => {
    await cancel();
  }, [cancel]);

  // 重新生成：由会话 store 删除最后一条 assistant 消息并发起新的 run。
  const handleReload = useCallback(async () => {
    await regenerate();
  }, [regenerate]);

  // 字段映射：messages 是数据源，convertMessage 负责格式转换，
  // isRunning / hasEarlier 驱动界面状态，on* 是用户操作的出口。
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
