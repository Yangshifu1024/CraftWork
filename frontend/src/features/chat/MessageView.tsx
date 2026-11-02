/**
 * 单条聊天消息的渲染与部件注册。
 *
 * 职责：
 * - 从 assistant-ui 的当前消息状态读取角色、时间、状态与内容。
 * - 维护“消息部件注册表”：把转换层产出的 `data-cw-*` 部件
 *   映射到具体的卡片组件（推理、工具、审批、任务清单、错误、草稿、提示、附件）。
 * - 助手消息用 `GroupedParts` 把最终总结之前的过程部件折叠为「已工作 N」：
 *   运行中保持平铺，结束后收拢，点击折叠头可展开回顾。
 * - 渲染消息内容、流式光标与末尾的元信息行（时间、生成中标记、复制与重新生成）。
 *
 * 说明：消息数据由 runtime 的 convertMessage 转换，这里只负责展示与复制/重新生成。
 */
import { MessagePrimitive, useAuiState } from "@assistant-ui/react";
import { Icon } from "../../components/Icon";
import { copyText } from "../../lib/clipboard";
import { cn, formatTime } from "../../lib/utils";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { MarkdownText } from "./Markdown";
import { partComponent, partComponentsByName, isWorkKind } from "./partRegistry";
import { WorkHistoryGroup } from "./WorkHistoryGroup";

/**
 * assistant-ui 的部件注册表。
 *
 * - `Text`：普通文本，使用 Markdown 渲染。
 * - `data.by_name`：从 partRegistry 单表派生，键名与 runtime 生成的
 *   `data-cw-<kind>` 去掉前缀后一致，例如 `cw-tool` 对应后端部件 `kind: "tool"`。
 */
const partComponents = {
  Text: MarkdownText,
  data: {
    by_name: partComponentsByName,
  },
};

/**
 * 判断一个部件是否属于「工作过程」。
 *
 * data 部件有两种形态，都需要兼容：
 * - 转换层产出（convertMessage 的 content）：type 直接是 `data-cw-<kind>`；
 * - 运行时内部（useAuiState / groupBy 拿到的 PartState）：type 归一为 `data`，
 *   具体种类在 `name` 上（assistant-ui 会剥掉 `data-` 前缀）。
 * 折叠归属由 partRegistry 的 work 字段单源决定。
 */
function isWorkPart(part: any): boolean {
  if (typeof part?.type !== "string") {
    return false;
  }
  if (part.type === "data") {
    return isWorkKind(part.name);
  }
  return isWorkKind(part.type.slice("data-".length));
}

/**
 * 把消息内容里的所有文本部件拼成一份纯文本，供“复制”按钮使用。
 * 用空行分隔，保持与原消息的段落结构接近。
 */
function messageText(content: any[]): string {
  return content
    .filter((part) => part?.type === "text")
    .map((part) => String(part.text ?? ""))
    .join("\n\n");
}

/**
 * 单条消息组件。
 *
 * props：无，所有数据来自 assistant-ui 的当前消息上下文。
 *
 * 关键状态：
 * - `role` 决定布局：用户消息靠右显示为强调色气泡，助手消息全宽靠左。
 * - `status.type === "running"` 时显示“生成中”标签与流式光标。
 * - 用户消息不提供“重新生成”按钮；助手消息的按钮会触发会话 store 的 regenerate。
 *
 * 复制操作只复制文本部件，附件与卡片内容不进入剪贴板。
 */
export function MessageView() {
  const role = useAuiState((state: any) => state.message.role as string);
  const createdAt = useAuiState((state: any) => state.message.createdAt as Date);
  const status = useAuiState((state: any) => state.message.status as { type: string; reason?: string });
  const content = useAuiState((state: any) => (state.message.content ?? []) as any[]);
  // 「已工作」工时从会话 store 直读：ChatMessage 上的持久化事实，
  // 快照加载与实时结束事件都会写入（metadata 通道在部分运行时路径下不透传）。
  const messageId = useAuiState((state: any) => state.message.id as string);
  const workDurationMs = useThreadStore(
    (state) => state.messages.find((message) => message.id === messageId)?.workDurationMs,
  );
  // 引导消息的「待送达」标记：已显示但还没并入下一次模型请求。
  const steerQueued = useThreadStore(
    (state) => state.messages.find((message) => message.id === messageId)?.steerQueued,
  );
  const regenerate = useThreadStore((state) => state.regenerate);
  const toast = useUiStore((state) => state.toast);

  const isUser = role === "user";
  const running = status?.type === "running";
  const text = messageText(content);

  // 过程折叠只对助手消息生效；没有过程部件（纯文本回复）时无需分组。
  const hasWork = !isUser && content.some(isWorkPart);

  // 结束后的折叠头按消息终态显示后缀：取消为「已停止」，失败为「出错」。
  const endStatus: "complete" | "incomplete" | "error" =
    status?.type === "running"
      ? "complete"
      : status?.type === "incomplete" && status?.reason === "error"
        ? "error"
        : status?.type === "incomplete"
          ? "incomplete"
          : "complete";

  return (
    <div className={cn("msg", isUser ? "user" : "assistant")}>
      <div className="msg-body">
        {isUser || !hasWork ? (
          /* 用户消息与纯文本回复没有过程可折叠，按平铺方式渲染。 */
          <MessagePrimitive.Parts components={partComponents as any} />
        ) : (
          /* 助手消息：相邻的过程部件合并成一组，结束后折叠为「已工作 N」。 */
          <MessagePrimitive.GroupedParts groupBy={(part: any) => (isWorkPart(part) ? ["group-work"] : null)}>
            {({ part, children }: any) => {
              if (part.type === "group-work") {
                // 组内仍在流式输出（含工具执行中）时保持平铺，结束后才折叠。
                if (part.status?.type === "running") {
                  return children;
                }
                return (
                  <WorkHistoryGroup durationMs={workDurationMs} endStatus={endStatus}>
                    {children}
                  </WorkHistoryGroup>
                );
              }
              if (part.type === "text") {
                return <MarkdownText />;
              }
              if (part.type === "indicator") {
                // 消息尚无部件时的流式光标，与生成中的视觉一致。
                return <span className="stream-caret" />;
              }
              if (part.type === "data") {
                // GroupedParts 没有 components 入参，leaf 返回 null 落回的
                // 注册表上下文由 MessagePrimitive.Parts 建立——这里没有渲染
                // Parts，所以按同一张注册表显式取组件渲染。
                const Component = partComponent(part.name);
                return Component ? <Component data={part.data} /> : null;
              }
              return null;
            }}
          </MessagePrimitive.GroupedParts>
        )}

        {/* 生成中时在末尾显示闪烁光标，表示内容还在追加。 */}
        {running ? <span className="stream-caret" /> : null}

        {/* 末尾元信息行：时间常显；操作按钮悬停消息时出现，只作用于本条消息。 */}
        <div className="msg-foot">
          <span className="msg-time">{formatTime(new Date(createdAt).getTime())}</span>
          {running ? <span className="msg-tag">生成中</span> : null}
          {isUser && steerQueued ? <span className="msg-tag is-steer">待送达</span> : null}
          <span className="foot-spacer" />
          <button
            type="button"
            onClick={() => {
              void copyText(text);
              toast({ title: "已复制", tone: "ok" });
            }}
          >
            <Icon name="copy" />
            复制
          </button>
          {isUser ? null : (
            // 重新生成会让后端丢弃最后一条回复并重新执行本轮 run。
            <button
              type="button"
              onClick={() => {
                void regenerate();
              }}
            >
              <Icon name="refresh" />
              重新生成
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
