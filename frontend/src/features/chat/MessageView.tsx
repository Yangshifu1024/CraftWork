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

function messageText(content: any[]): string {
  return content
    .filter((part) => part?.type === "text")
    .map((part) => String(part.text ?? ""))
    .join("\n\n");
}

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

        <MessagePrimitive.Parts components={partComponents as any} />

        {running ? <span className="stream-caret" /> : null}

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
