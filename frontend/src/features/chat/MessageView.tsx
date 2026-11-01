/**
 * 单条聊天消息的渲染与部件注册。
 *
 * 职责：
 * - 从 assistant-ui 的当前消息状态读取角色、时间、状态与内容。
 * - 维护“消息部件注册表”：把转换层产出的 `data-cw-*` 部件
 *   映射到具体的卡片组件（工具、审批、任务清单、错误、草稿、提示、附件）。
 * - 渲染消息头部（头像、作者、时间、生成中标记）、消息部件、流式光标与底部操作。
 *
 * 说明：消息数据由 runtime 的 convertMessage 转换，这里只负责展示与复制/重新生成。
 */
import { MessagePrimitive, useAuiState } from "@assistant-ui/react";
import { BrandMark } from "../../components/BrandMark";
import { Icon } from "../../components/Icon";
import { copyText } from "../../lib/clipboard";
import { cn, formatTime } from "../../lib/utils";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { MarkdownText } from "./Markdown";
import {
  ApprovalCard,
  AttachmentChips,
  CalloutCard,
  DraftCard,
  ErrorCard,
  NoticeCard,
  ReasoningBlock,
  TodoCard,
  ToolCard,
} from "./Parts";

/**
 * assistant-ui 的部件注册表。
 *
 * - `Text`：普通文本，使用 Markdown 渲染。
 * - `Reasoning`：推理内容，使用可折叠的思考过程块。
 * - `data.by_name`：键名与 runtime 生成的 `data-cw-<kind>` 去掉前缀后一致，
 *   例如 `cw-tool` 对应后端部件 `kind: "tool"`。
 */
const partComponents = {
  Text: MarkdownText,
  Reasoning: ReasoningBlock as any,
  data: {
    by_name: {
      "cw-tool": ToolCard as any,
      "cw-approval": ApprovalCard as any,
      "cw-todo": TodoCard as any,
      "cw-error": ErrorCard as any,
      "cw-draft": DraftCard as any,
      "cw-callout": CalloutCard as any,
      "cw-notice": NoticeCard as any,
      "cw-attachments": AttachmentChips as any,
    },
  },
};

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
 * - `role` 决定气泡方向、头像与作者名（用户显示“你”，助手显示 CraftWork 与品牌图标）。
 * - `status.type === "running"` 时显示“生成中”标签与流式光标。
 * - 用户消息不提供“重新生成”按钮；助手消息的按钮会触发线程 store 的 regenerate。
 *
 * 复制操作只复制文本部件，附件与卡片内容不进入剪贴板。
 */
export function MessageView() {
  const role = useAuiState((state: any) => state.message.role as string);
  const createdAt = useAuiState((state: any) => state.message.createdAt as Date);
  const status = useAuiState((state: any) => state.message.status as { type: string });
  const content = useAuiState((state: any) => (state.message.content ?? []) as any[]);
  const regenerate = useThreadStore((state) => state.regenerate);
  const toast = useUiStore((state) => state.toast);

  const isUser = role === "user";
  const running = status?.type === "running";
  const text = messageText(content);

  return (
    <div className={cn("msg", isUser ? "user" : "assistant")}>
      <div className="msg-avatar">{isUser ? <Icon name="user" /> : <BrandMark size={16} />}</div>
      <div className="msg-body">
        <div className="msg-head">
          <span className="msg-author">{isUser ? "你" : "CraftWork"}</span>
          <span className="msg-time">{formatTime(new Date(createdAt).getTime())}</span>
          {running ? <span className="msg-tag">生成中</span> : null}
        </div>

        {/* 按注册表渲染消息部件：文本走 Markdown，其余走对应卡片组件。 */}
        <MessagePrimitive.Parts components={partComponents as any} />

        {/* 生成中时在末尾显示闪烁光标，表示内容还在追加。 */}
        {running ? <span className="stream-caret" /> : null}

        {/* 底部操作只作用于本条消息：复制文本；助手消息额外支持重新生成。 */}
        <div className="msg-actions">
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
