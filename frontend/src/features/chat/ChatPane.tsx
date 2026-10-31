import { useRef, useState } from "react";
import { AssistantRuntimeProvider, ThreadPrimitive } from "@assistant-ui/react";
import { Icon } from "../../components/Icon";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { ChatHeader } from "./ChatHeader";
import { Composer } from "./Composer";
import { MessageView } from "./MessageView";
import { useChatRuntime } from "./runtime";

const suggestions: Record<string, string[]> = {
  code: ["解释这个仓库的结构", "运行测试并修复失败用例", "为当前分支写一份变更摘要"],
  work: ["总结这份文档的要点", "把会议纪要整理成待办表", "草拟一封回复邮件"],
};

export function ChatPane() {
  const runtime = useChatRuntime();
  const mode = useAppStore((state) => state.mode);
  const messages = useThreadStore((state) => state.messages);
  const hasEarlier = useThreadStore((state) => state.hasEarlier);
  const send = useThreadStore((state) => state.send);
  const addAttachments = useThreadStore((state) => state.addAttachments);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [dragging, setDragging] = useState(false);

  function handleScroll(event: React.UIEvent<HTMLDivElement>) {
    const target = event.currentTarget;
    const distance = target.scrollHeight - target.scrollTop - target.clientHeight;
    setAtBottom(distance < 80);
  }

  function scrollToBottom() {
    const viewport = viewportRef.current;
    if (viewport) {
      viewport.scrollTo({ top: viewport.scrollHeight, behavior: "smooth" });
    }
  }

  function handleDrop(event: React.DragEvent<HTMLElement>) {
    event.preventDefault();
    setDragging(false);
    const paths = Array.from(event.dataTransfer.files).map(
      (file) => (file as File & { path?: string }).path || file.name,
    );
    if (paths.length > 0) {
      addAttachments(paths);
    }
  }

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <ThreadPrimitive.Root
        className="chat-pane"
        onDragOver={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={handleDrop}
      >
        <ChatHeader />

        <ThreadPrimitive.Viewport
          ref={viewportRef as any}
          className="chat-scroll"
          onScroll={handleScroll}
          autoScroll
        >
          <div className="chat-stream">
            {hasEarlier ? (
              <ThreadPrimitive.LoadEarlier className="load-earlier">
                <Icon name="chevron-up" className="ico" />
                <span>加载更早消息</span>
              </ThreadPrimitive.LoadEarlier>
            ) : null}

            {messages.length === 0 ? (
              <div className="chat-empty">
                <span className="empty-mark">
                  <Icon name={mode === "work" ? "briefcase" : "code"} />
                </span>
                <h2>{mode === "work" ? "Work 模式已就绪" : "Code 模式已就绪"}</h2>
                <p>
                  {mode === "work"
                    ? "读取文档、修改 Word 与 Excel、汇总邮件与日历。所有文档写入都会在审批后执行并自动备份。"
                    : "在当前工作区里读代码、改文件、跑命令、看差异。所有写操作与命令执行都会先经过审批。"}
                </p>
                <div className="empty-suggestions">
                  {(suggestions[mode] ?? []).map((suggestion) => (
                    <button key={suggestion} type="button" onClick={() => void send(suggestion)}>
                      {suggestion}
                    </button>
                  ))}
                </div>
              </div>
            ) : null}

            <ThreadPrimitive.Messages>{() => <MessageView />}</ThreadPrimitive.Messages>
          </div>

          {!atBottom && messages.length > 0 ? (
            <button type="button" className="jump-bottom" onClick={scrollToBottom}>
              <Icon name="arrow-down" className="ico" />
              <span>回到底部</span>
            </button>
          ) : null}
        </ThreadPrimitive.Viewport>

        <Composer />

        {dragging ? (
          <div className="drop-overlay">
            <div className="drop-box">
              <Icon name="file-plus" size={24} />
              <span>松开以添加附件</span>
            </div>
          </div>
        ) : null}
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>
  );
}
