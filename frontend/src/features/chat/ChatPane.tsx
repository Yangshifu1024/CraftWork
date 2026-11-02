/**
 * 聊天主区域。
 *
 * 职责：
 * - 用 `AssistantRuntimeProvider` 把 runtime 提供给整棵聊天组件树。
 * - 组装消息头、滚动视口、消息列表、输入框与拖拽附件遮罩。
 * - 会话为空时显示对应模式的引导文案与快捷提示句。
 * - 负责滚动相关交互：是否显示“回到底部”、点击后平滑滚动到末尾。
 * - 负责拖拽文件到聊天区添加附件。
 */

import { AssistantRuntimeProvider, ThreadPrimitive } from "@assistant-ui/react";
import { useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { ChatHeader } from "./ChatHeader";
import { Composer } from "./Composer";
import { MessageView } from "./MessageView";
import { useChatRuntime } from "./runtime";

/** 空会话时按模式展示的快捷提示句；点击后会直接作为一条用户消息发送。 */
const suggestions: Record<string, string[]> = {
  code: ["解释这个仓库的结构", "运行测试并修复失败用例", "为当前分支写一份变更摘要"],
  work: ["总结这份文档的要点", "把会议纪要整理成待办表", "草拟一封回复邮件"],
};

/**
 * 聊天主区域组件。
 *
 * props：无，模式与会话数据来自全局 store，运行时由 useChatRuntime 创建。
 *
 * 关键状态：
 * - `atBottom`：视口是否贴近底部（阈值 80 像素），用于决定是否显示“回到底部”按钮。
 * - `dragging`：是否有文件正拖到聊天区上方，用于显示投放遮罩。
 *
 * 拖拽投放的路径取自浏览器文件对象的 `path` 字段（桌面 WebView 提供），
 * 拿不到时退回文件名，交给后端按工作区相对路径解析。
 */
export function ChatPane() {
  const runtime = useChatRuntime();
  const mode = useAppStore((state) => state.mode);
  // 当前活动工作区是否为内置临时目录：空会话引导据此切换文案。
  const isTempWorkspace = useAppStore((state) => state.activeWorkspace?.purpose === "conversation");
  const messages = useThreadStore((state) => state.messages);
  const hasEarlier = useThreadStore((state) => state.hasEarlier);
  const send = useThreadStore((state) => state.send);
  const addAttachments = useThreadStore((state) => state.addAttachments);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  const [dragging, setDragging] = useState(false);

  /** 记录视口是否到底：滚动后距底部小于 80 像素就算“在底部”，避免抖动。 */
  function handleScroll(event: React.UIEvent<HTMLDivElement>) {
    const target = event.currentTarget;
    const distance = target.scrollHeight - target.scrollTop - target.clientHeight;
    setAtBottom(distance < 80);
  }

  /** 平滑滚动到视口底部，供“回到底部”按钮调用。 */
  function scrollToBottom() {
    const viewport = viewportRef.current;
    if (viewport) {
      viewport.scrollTo({ top: viewport.scrollHeight, behavior: "smooth" });
    }
  }

  /**
   * 处理文件投放：阻止浏览器默认的“打开文件”行为，收集路径后加入待发送附件。
   * 附件此时只进入输入框上方的附件条，真正发送仍由用户触发。
   */
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

        <ThreadPrimitive.Viewport ref={viewportRef as any} className="chat-scroll" onScroll={handleScroll} autoScroll>
          <div className="chat-stream">
            {/* 还有更早的消息时，在顶部提供分页加载入口。 */}
            {hasEarlier ? (
              <ThreadPrimitive.LoadEarlier className="load-earlier">
                <Icon name="chevron-up" className="ico" />
                <span>加载更早消息</span>
              </ThreadPrimitive.LoadEarlier>
            ) : null}

            {/* 空会话引导：标题、说明与按模式区分的快捷提示句。 */}
            {messages.length === 0 ? (
              <div className="chat-empty">
                <span className="empty-mark">
                  <Icon name={isTempWorkspace ? "zap" : mode === "work" ? "briefcase" : "code"} />
                </span>
                <h2>{isTempWorkspace ? "临时会话已就绪" : mode === "work" ? "Work 模式已就绪" : "Code 模式已就绪"}</h2>
                <p>
                  {isTempWorkspace
                    ? "无需选择工作区，直接提问即可；产生的文件会保存在内置临时目录，随时可在侧栏切回工作区会话。"
                    : mode === "work"
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

            {/* 消息条目由 MessageView 绘制，数据来自 assistant-ui 运行时。 */}
            <ThreadPrimitive.Messages>{() => <MessageView />}</ThreadPrimitive.Messages>
          </div>

          {/* 用户向上翻阅历史时出现，避免强制跟随新消息打断阅读。 */}
          {!atBottom && messages.length > 0 ? (
            <button type="button" className="jump-bottom" onClick={scrollToBottom}>
              <Icon name="arrow-down" className="ico" />
              <span>回到底部</span>
            </button>
          ) : null}
        </ThreadPrimitive.Viewport>

        <Composer />

        {/* 拖拽进入聊天区时显示投放遮罩，提示松开即可添加附件。 */}
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
