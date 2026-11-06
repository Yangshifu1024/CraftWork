// 对话区：工作区标题栏 + 消息流（Timeline）+ 沉底输入区（Composer）。
// 滚动贴底与「回到底部」由 use-stick-to-bottom 托管；发送 / 引导（steer）/ 停止（armed 二次确认）编排收在 Composer。
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStickToBottom } from "use-stick-to-bottom";
import { ArrowUp, Check, ChevronDown, ChevronRight, Brain, AlertCircle, Copy, RefreshCw, Square, Plus, Paperclip, X, Folder, ListChecks, GitBranch } from "lucide-react";
import { Popover } from "radix-ui";
import { ModelPicker } from "./model-picker";
import { useApp, draftSlot, effectiveModelRef, effectiveReasoningLevel, EMPTY_DRAFT } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { api } from "../lib/api";
import { cleanErrorMessage } from "../lib/errors";
import { isImageName, pastedFileName, readFileAsBase64, selectPastedFiles } from "../lib/paste";
import { APPROVAL_LEVELS, APPROVAL_LEVEL_NAMES, APPROVAL_LEVEL_ICONS, useApprovalLevelAction } from "../lib/approval";
import { reasoningLabel } from "../lib/reasoning";
import { selectableModels } from "../lib/provider-list";
import { formatTokens } from "../lib/format";
import { isImeComposing } from "../lib/ime";
import type { ApprovalPart, ToolCallPart } from "../../../shared/events";
import type { MessageSnapshot, PartSnapshot, PlanInfo } from "../../../shared/types";
import { Markdown } from "./markdown";
import { TodoCard } from "./cards";

// 内容列宽度档位（行为约定见 docs/conversation-width/spec.md）：以对话卡片为命名容器
// `conversation` 做容器查询——与窗口/侧栏宽度解耦，右侧面板开合改变容器宽度时档位自动升降。
// 各档默认值都是可插值长度（max-w-full），断点跨越与草稿→会话切换对 max-width 平滑过渡不硬跳。
// 草稿档（会话无消息）固定收窄聚焦；会话档随容器宽度 896px → 1152px 渐进放宽。
const CONTENT_WIDTH_CLASS =
  "max-w-full transition-[max-width] duration-150 ease-out @min-[864px]/conversation:max-w-4xl @min-[1280px]/conversation:max-w-6xl";
const DRAFT_WIDTH_CLASS = "max-w-2xl transition-[max-width] duration-150 ease-out";

/** 对话区整体：标题取活动会话，草稿态（无活动会话）显示草稿槽归属的工作区名。 */
export function Conversation(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const chat = useApp((s) => s.chats[activeId]);
  const thread = useApp((s) => s.threads[mode].find((t) => t.id === activeId));
  const draftWsId = useApp((s) => s.draftWsId[mode]);
  const workspaces = useApp((s) => s.workspaces);
  // 草稿态（活动会话为空）标题显示槽归属：项目槽显示该工作区名（查不到回落「新对话」），临时槽显示「新对话」。
  const draftWs = activeId === "" && draftWsId ? workspaces.find((w) => w.id === draftWsId) : undefined;
  const title = activeId === "" ? (draftWs?.name ?? "新对话") : (thread?.title ?? "新对话");
  // 计划待批准：以计划数据为准（计划 run 正常完成后落盘回填 file_path，批准状态由数据管理）——
  // 中断/失败的计划没有文件，提示条不出现（不再用「最后消息带 planMode 标记」的启发式）。
  const plan = useApp((s) => s.plans[activeId]);
  const planAwaiting = !!plan && !plan.approved && !plan.ignored && plan.filePath !== "" && !(chat?.running ?? false);
  // 草稿档：会话尚无消息（草稿槽或新建会话）时内容列收窄聚焦，发出首条消息后放宽到会话档。
  const draftLayout = !chat || chat.messages.length === 0;

  return (
    <div className="flex min-h-0 min-w-[320px] flex-1 flex-col">
      {/* @container/conversation：内容列宽度档位的裁决容器（Timeline 与 Composer 都在其内）。 */}
      <div className="@container/conversation relative flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-background shadow-xs">
        <WorkspaceHeader title={title} status={thread?.status ?? "idle"} mode={mode} />
        {/* 输入区以 sticky 沉底浮在消息流上：滚动区贯穿到容器底，滚动条贴底=贴窗口底 */}
        <Timeline
          activeId={activeId}
          draftLayout={draftLayout}
          chat={chat ? { messages: chat.messages, running: chat.running } : undefined}
          footer={
            <Composer
              activeId={activeId}
              draftLayout={draftLayout}
              chat={chat ? { running: chat.running } : { running: false }}
              planAwaiting={planAwaiting}
              plan={plan}
            />
          }
        />
      </div>
    </div>
  );
}

/** 会话顶栏：标题 + 模式徽章 + 运行状态（运行中=文字扫光 / 等待审批 / 出错）。 */
function WorkspaceHeader({ title, status, mode }: { title: string; status: string; mode: string }): React.JSX.Element {
  // "deep"：子树内任意点击均可拖拽/双击最大化。裸属性只匹配直接命中的元素本身，
  // 而内层 div 铺满标题栏，点击永远落不到外层上；按钮等可点击元素会自动阻断，无需单独排除。
  // select-none：macOS 双击最大化时 drag.js 不拦截默认行为（以支持移动取消），需禁用选字。
  return (
    <div className="relative flex w-full shrink-0 select-none border-b border-border/50" data-tauri-drag-region="deep">
      <div className="flex h-12 min-w-0 flex-1 items-center justify-between gap-2 overflow-hidden p-2">
        <div className="flex min-w-0 items-center gap-2 pl-1">
          <span className="truncate text-ui-base font-medium text-foreground">{title}</span>
          {/* 模式徽章：浅色模式色底 + 对应文字色（与侧栏切换器同源 token） */}
          <span
            className={`shrink-0 rounded-full px-2 py-0.5 text-ui-xs ${
              mode === "work"
                ? "bg-mode-work-soft text-mode-work-soft-foreground"
                : "bg-mode-code-soft text-mode-code-soft-foreground"
            }`}
          >
            {mode === "work" ? "Work" : "Code"}
          </span>
          {status === "running" && <span className="animated-gradient-text shrink-0 text-ui-xs">运行中</span>}
          {status === "awaiting" && <span className="shrink-0 text-ui-xs text-warning">等待审批</span>}
          {status === "error" && <span className="shrink-0 text-ui-xs text-destructive">出错</span>}
        </div>
      </div>
    </div>
  );
}

// ---------- 消息流 ----------

/** 消息流滚动区：贴底状态由 use-stick-to-bottom 托管；footer（输入区）以 sticky 沉底、不挡消息点击。 */
function Timeline({
  activeId,
  draftLayout,
  chat,
  footer,
}: {
  activeId: string;
  draftLayout: boolean;
  chat?: { messages: MessageSnapshot[]; running: boolean };
  footer?: React.ReactNode;
}): React.JSX.Element {
  const { scrollRef, contentRef, isAtBottom, scrollToBottom, stopScroll } = useStickToBottom();
  const messages = chat?.messages ?? [];

  useEffect(() => {
    // 切换会话时贴底：instant 直达。默认弹簧动画下，内容高度骤变引发的 scrollTop 钳制
    // 会被库误判为「用户向上滚」（escape）并在下一帧取消动画，贴底会停在顶部附近。
    stopScroll();
    scrollToBottom("instant");
  }, [activeId, scrollToBottom, stopScroll]);

  // 用户发送新消息（user 消息数增加）时立即贴底：发送者必然想看到刚发出的消息，
  // 即使正翻在历史里也要拉回底部，否则会被误认为发送失败。
  // 助手流式回复不走这里——翻历史阅读时保持 use-stick-to-bottom 的「不打断」行为。
  const prevUserCount = useRef(0);
  useLayoutEffect(() => {
    const count = messages.reduce((n, m) => (m.role === "user" ? n + 1 : n), 0);
    if (count > prevUserCount.current) {
      stopScroll();
      scrollToBottom("instant");
    }
    prevUserCount.current = count;
  }, [messages, scrollToBottom, stopScroll]);

  return (
    <div ref={scrollRef} className="relative min-h-0 flex-1 overflow-y-auto">
      {/* min-h-full：消息少时也占满滚动视口，footer 始终沉底；footer 参与内容流，
          输入区增减高度会触发内容 resize，贴底状态由 use-stick-to-bottom 自动保持。
          宽度档位与 Composer 共用常量，保证消息列与输入卡各档始终对齐。 */}
      <div
        ref={contentRef}
        className={`mx-auto flex min-h-full w-full flex-col px-4 ${draftLayout ? DRAFT_WIDTH_CLASS : CONTENT_WIDTH_CLASS}`}
      >
        <div className="flex min-h-0 flex-1 flex-col gap-6 pb-4 pt-6">
          {messages.length === 0 && (
            <div className="flex flex-1 items-center justify-center py-24 text-ui-sm text-foreground-subtlest">
              开始你的第一条消息
            </div>
          )}
          {messages.map((message) => (
            <MessageRow key={message.id} message={message} chatRunning={chat?.running ?? false} />
          ))}
        </div>
        {footer && (
          // 外壳 pointer-events-none：全宽 sticky 层不挡消息（滚入输入区背后的消息仍可点选），
          // 只有输入列与按钮恢复命中——与 zcode ConversationTimeline 的 composer dock 同款处理。
          <div className="pointer-events-none sticky bottom-0 z-10 -mx-4 bg-gradient-to-t from-background via-background to-transparent pt-6">
            <div className="pointer-events-auto relative">
              {!isAtBottom && (
                <button
                  className="absolute -top-9 left-1/2 z-10 -translate-x-1/2 rounded-full border border-border bg-popover px-3 py-1.5 text-ui-sm text-foreground-subtle shadow-md hover:bg-menu-hover"
                  onClick={() => scrollToBottom()}
                >
                  <ChevronDown className="inline size-3.5" /> 回到底部
                </button>
              )}
              {footer}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

const pad2 = (n: number): string => String(n).padStart(2, "0");

/** 消息时间："yyyy-MM-dd HH:mm:ss"（本地时区）；ts 无效（0 = 未知，如旧数据）返回空串。 */
function fmtDateTime(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

/** 文件路径 → 展示名。 */
function attachmentLeaf(path: string): string {
  return path.split("/").pop() || path;
}

/** 用户消息附件：图片经 readAttachment 懒加载为缩略图（点击放大预览），其余渲染文件 chip。 */
function MessageAttachments({ attachments }: { attachments: unknown }): React.JSX.Element | null {
  const openPreview = useOverlays((s) => s.openImagePreview);
  const list = (Array.isArray(attachments) ? attachments : []).map(String).filter(Boolean);
  const [urls, setUrls] = useState<Record<string, string>>({});
  const requested = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const name of list) {
      if (!isImageName(name) || requested.current.has(name)) continue;
      requested.current.add(name);
      void api
        .readAttachment(name)
        .then((c) => setUrls((prev) => ({ ...prev, [name]: `data:${c.mime};base64,${c.base64}` })))
        .catch(() => setUrls((prev) => ({ ...prev, [name]: "" }))); // 读取失败降级为文件 chip
    }
  }, [list]);

  if (list.length === 0) return null;
  return (
    <div className="flex flex-wrap justify-end gap-1.5">
      {list.map((name) => {
        const url = urls[name];
        if (isImageName(name) && url) {
          return url ? (
            <button
              key={name}
              type="button"
              className="overflow-hidden rounded-lg border border-border"
              title={`放大查看 ${attachmentLeaf(name)}`}
              onClick={() => openPreview(url, attachmentLeaf(name))}
            >
              <img src={url} alt={attachmentLeaf(name)} className="max-h-40 max-w-56 object-cover" />
            </button>
          ) : null;
        }
        if (isImageName(name)) {
          return (
            <span
              key={name}
              className="flex h-16 w-24 items-center justify-center rounded-lg border border-border bg-surface text-ui-xs text-foreground-subtlest"
            >
              加载中…
            </span>
          );
        }
        return (
          <span
            key={name}
            className="flex h-7 items-center gap-1.5 rounded-lg border border-border bg-surface px-2 text-ui-xs text-foreground-subtle"
          >
            <Paperclip className="size-3" />
            <span className="max-w-48 truncate">{attachmentLeaf(name)}</span>
          </span>
        );
      })}
    </div>
  );
}

/** 消息复制按钮：写入成功后原处切换「已复制」反馈，短暂停留后恢复。 */
function CopyButton({ getText }: { getText: () => string }): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => clearTimeout(timer.current ?? undefined), []);
  return (
    <button
      className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtlest hover:bg-surface-hover"
      onClick={() => {
        void api.clipboardWrite(getText()).then(() => {
          setCopied(true);
          if (timer.current) clearTimeout(timer.current);
          timer.current = setTimeout(() => setCopied(false), 1600);
        });
      }}
    >
      {copied ? <Check className="size-3 text-success" /> : <Copy className="size-3" />}
      {copied ? "已复制" : "复制"}
    </button>
  );
}

/** 单条消息：用户=右对齐气泡（附件缩略图 + 待送达标记 + hover 操作条）；助手=工作折叠块 + 最终正文。 */
function MessageRow({
  message,
  chatRunning,
}: {
  message: MessageSnapshot;
  chatRunning: boolean;
}): React.JSX.Element {
  if (message.role === "user") {
    return (
      <div className="group/msg flex w-full flex-col items-end gap-1">
        <div className="select-text flex max-w-full flex-col gap-2 rounded-xl rounded-tr-xs border border-border bg-user-message-bubble px-4 py-3 text-ui-base text-foreground sm:max-w-xl">
          {message.parts.some((p) => p.payload["via"] === "steer") && (
            <span className="self-end rounded bg-warning-surface px-1.5 py-0.5 text-ui-xs text-warning-surface-foreground">
              待送达
            </span>
          )}
          {message.parts
            .filter((p) => p.kind === "text")
            .map((part) => (
              <div key={part.id} className="whitespace-pre-wrap break-words">
                {String(part.payload["text"] ?? "")}
              </div>
            ))}
          {message.parts
            .filter((p) => p.kind === "text" && Array.isArray(p.payload["attachments"]) && (p.payload["attachments"] as unknown[]).length > 0)
            .map((part) => (
              <MessageAttachments key={`att_${part.id}`} attachments={part.payload["attachments"]} />
            ))}
        </div>
        {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动 */}
        <div className="mr-1 flex items-center gap-1.5 opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100">
          {message.createdAt > 0 && (
            <span className="select-none text-ui-xs text-foreground-subtlest">{fmtDateTime(message.createdAt)}</span>
          )}
          <CopyButton
            getText={() =>
              message.parts
                .filter((p) => p.kind === "text")
                .map((p) => String(p.payload["text"] ?? ""))
                .join("\n")
            }
          />
        </div>
      </div>
    );
  }

  // 助手消息：按「过程分组」切段渲染（工具/推理/审批收进折叠，最终正文在外）
  return (
    <div className="group/msg flex w-full flex-col gap-2">
      <TurnBody message={message} chatRunning={chatRunning} />
      {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动。
          时间戳只在回复结束后显示，且取结束时刻（finishedAt）而非开始时刻 */}
      <div className="flex items-center gap-1.5 opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100">
        <CopyButton
          getText={() =>
            message.parts
              .filter((p) => p.kind === "text")
              .map((p) => String(p.payload["text"] ?? ""))
              .join("\n")
          }
        />
        {message.status !== "streaming" && (message.finishedAt ?? message.createdAt) > 0 && (
          <span className="select-none text-ui-xs text-foreground-subtlest">
            {fmtDateTime(message.finishedAt ?? message.createdAt)}
          </span>
        )}
      </div>
    </div>
  );
}

/** 单条助手消息的渲染（对齐 ZCode 单折叠区模型）：一个 run 只有一个过程折叠区——
 *  最终正文（最后一段非空 text）之前的全部内容（思考、工具、审批历史、**中间解说正文**）
 *  都收进折叠区，只留最终正文平铺在外；最终正文之后的过程行（运行中实时产出 / 中断残余）
 *  常驻平铺。run 结束后折叠区收起，只显示「已工作 · 持续 N 秒」+ 结论。
 *  等待中的审批卡常驻渲染在折叠区之外。 */
function TurnBody({ message, chatRunning }: { message: MessageSnapshot; chatRunning: boolean }): React.JSX.Element {
  const threadId = useActiveThreadId();
  const parts = message.parts;
  const isStreaming = message.status === "streaming";

  // 最终正文 = 最后一个非空 text 部件；空 text（流式占位/旧数据）不算。
  // 等待决定的审批部件不进折叠区（由 openApprovals 常驻渲染）。
  const isWaitingApproval = (p: PartSnapshot): boolean =>
    p.kind === "approval" && (p.payload as unknown as ApprovalPart).status === "waiting";
  let finalIdx = -1;
  for (let i = parts.length - 1; i >= 0; i--) {
    const p = parts[i]!;
    if (p.kind === "text" && !isWaitingApproval(p) && String(p.payload["text"] ?? "").length > 0) {
      finalIdx = i;
      break;
    }
  }
  // 折叠区：最终正文之前的全部行（含中间解说正文——它们是过程的一部分）
  const historyParts = parts.slice(0, finalIdx < 0 ? parts.length : finalIdx).filter((p) => !isWaitingApproval(p));
  const finalText = finalIdx >= 0 ? parts[finalIdx]! : null;
  // 尾段：最终正文之后的过程行（运行中实时产出 / 中断残余），常驻平铺
  const tailParts = (finalIdx >= 0 ? parts.slice(finalIdx + 1) : []).filter((p) => !isWaitingApproval(p));
  // 流式中的思考段（消息流式中最后一个 reasoning part）：折叠行显示「思考中 + 最新一行摘要」
  const streamingReasoningId = isStreaming
    ? ([...parts].reverse().find((p) => p.kind === "reasoning")?.id ?? "")
    : "";
  // 等待决定的审批卡必须常驻可见：审批是 run 推进的关键路径，折叠进过程块会让用户
  // 找不到任何审批入口（只有标题栏小字「等待审批」），任务无声卡死。决定后（approved/
  // denied/canceled）不再命中 waiting，卡片自然回到折叠块作为历史记录。
  const openApprovals = parts.filter(isWaitingApproval);

  return (
    <div className="flex w-full flex-col gap-2">
      {historyParts.length > 0 && (
        <WorkFold
          parts={historyParts}
          streaming={isStreaming}
          status={message.status}
          threadId={threadId}
          streamingReasoningId={streamingReasoningId}
          durationMs={message.workDurationMs}
        />
      )}
      {finalText && (
        <div className="text-ui-base text-foreground">
          <Markdown text={String(finalText.payload["text"] ?? "")} streaming={isStreaming} />
        </div>
      )}
      {tailParts.length > 0 && (
        <WorkLines parts={tailParts} threadId={threadId} streamingReasoningId={streamingReasoningId} />
      )}
      {openApprovals.map((part) => (
        <ApprovalCard key={part.id} part={part.payload as unknown as ApprovalPart} threadId={threadId} />
      ))}
      {parts.map((part) => {
        if (part.kind === "error") {
          return <ErrorCard key={part.id} part={part} threadId={threadId} />;
        }
        if (part.kind === "notice") {
          return (
            <div key={part.id} className="select-text rounded-lg bg-surface px-3 py-2 text-ui-sm text-foreground-subtle">
              {String(part.payload["text"] ?? "")}
            </div>
          );
        }
        return null;
      })}
      {isStreaming && chatRunning && parts.every((p) => p === finalText || (p.kind === "text" && String(p.payload["text"] ?? "").length === 0)) && (
        <div className="text-ui-sm">
          <span className="animated-gradient-text">思考中</span>
        </div>
      )}
    </div>
  );
}

function useActiveThreadId(): string {
  return useApp((s) => s.activeThreadId[s.mode]);
}

/** 工作过程折叠块（一段正文章节前的过程行）。运行中自动展开让过程可见，结束后自动收起只留摘要行；
 * 用户手动切换过（manual）后完全尊重手动状态。展开内容无缩进平铺（对齐 ZCode 组容器），
 * 已完成时显示该轮工作的耗时。 */
function WorkFold({
  parts,
  streaming,
  status,
  threadId,
  durationMs,
  streamingReasoningId,
}: {
  parts: PartSnapshot[];
  streaming: boolean;
  status: MessageSnapshot["status"];
  threadId: string;
  durationMs?: number;
  streamingReasoningId?: string;
}): React.JSX.Element {
  const hasRunning = parts.some((p) => p.kind === "tool" && (p.payload as unknown as ToolCallPart).status === "running");
  const [open, setOpen] = useState(streaming);
  const [manual, setManual] = useState(false);
  // 流式运行中默认展开（思考文本与工具调用实时可见），run 结束后自动收起；
  // 用户点过折叠头之后不再自动干预（manual 恒为 true）
  useEffect(() => {
    if (!manual) setOpen(streaming);
  }, [streaming, manual]);
  const done = !streaming && status !== "streaming";

  // 完成态显示秒数（向上取整，与侧栏「思考过程 · 持续了 N 秒」口径一致）
  const duration = done && durationMs && durationMs > 0 ? `持续 ${Math.ceil(durationMs / 1000)} 秒` : "";
  return (
    <div className="flex w-full flex-col">
      <button
        className="group/reasoning inline-flex min-w-0 max-w-full items-center gap-2 self-start rounded-md px-1 py-0.5 text-ui-base text-foreground-subtlest hover:text-foreground-subtle"
        onClick={() => {
          setManual(true);
          setOpen(!open);
        }}
      >
        <Brain className="size-4 shrink-0" />
        <span className={`shrink-0 whitespace-nowrap ${hasRunning || !done ? "animated-gradient-text" : ""}`}>
          {hasRunning ? "工作中" : done ? "已工作" : "正在思考"}
        </span>
        {duration && <span className="shrink-0 whitespace-nowrap text-foreground-subtlest">{duration}</span>}
        <ChevronRight className={`size-4 shrink-0 transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="pt-2">
          <WorkLines parts={parts} threadId={threadId} streamingReasoningId={streamingReasoningId} />
        </div>
      )}
    </div>
  );
}

/** 过程行平铺容器：折叠块展开内容与尾段常驻区共用（对齐 ZCode 的组容器——无缩进、纵向 gap）。
 *  连续同名且已完成的工具行归并成组卡（× N 可展开）；思考行独立折叠，流式中的那段思考
 *  显示「思考中 + 最新一行摘要」。 */
function WorkLines({
  parts,
  threadId,
  streamingReasoningId,
}: {
  parts: PartSnapshot[];
  threadId: string;
  streamingReasoningId?: string;
}): React.JSX.Element {
  const rows = coalesceToolRows(parts);
  return (
    <div className="flex flex-col gap-3">
      {rows.map((row) =>
        row.kind === "group" ? (
          <ToolGroupCard key={row.key} name={row.name} tools={row.tools} />
        ) : (
          <WorkLine key={row.part.id} part={row.part} threadId={threadId} streamingReasoningId={streamingReasoningId} />
        ),
      )}
    </div>
  );
}

/** 单条过程行分发：思考折叠行 / 工具卡 / 审批卡 / 任务清单卡片，其余忽略。 */
function WorkLine({
  part,
  threadId,
  streamingReasoningId,
}: {
  part: PartSnapshot;
  threadId: string;
  streamingReasoningId?: string;
}): React.JSX.Element | null {
  switch (part.kind) {
    case "reasoning":
      return <ReasoningFold part={part} streaming={part.id === streamingReasoningId} />;
    case "text":
      // 折叠区内的中间解说正文（「边做边说」的过程叙述），弱化展示
      return (
        <div className="select-text whitespace-pre-wrap break-words text-ui-base text-foreground-subtle">
          {String(part.payload["text"] ?? "")}
        </div>
      );
    case "tool":
      return <ToolCard part={part.payload as unknown as ToolCallPart} />;
    case "approval":
      return <ApprovalCard part={part.payload as unknown as ApprovalPart} threadId={threadId} />;
    case "todo":
      return <TodoCard items={(part.payload["items"] as never) ?? []} />;
    default:
      return null;
  }
}

type CoalescedRow =
  | { kind: "group"; name: string; tools: ToolCallPart[]; key: string }
  | { kind: "part"; part: PartSnapshot };

/** 归并连续同名且已完成的工具行；running / error 工具保持独立（运行扫光与失败显眼优先），
 *  不同名工具穿插时各自分组。单元素组还原为普通工具卡。 */
function coalesceToolRows(parts: PartSnapshot[]): CoalescedRow[] {
  const out: CoalescedRow[] = [];
  let buf: { name: string; tools: ToolCallPart[]; key: string } | null = null;
  const flush = (): void => {
    if (!buf) return;
    if (buf.tools.length === 1) {
      out.push({ kind: "part", part: { id: buf.key, kind: "tool", seq: 0, payload: buf.tools[0] as unknown as Record<string, unknown> } });
    } else {
      out.push({ kind: "group", name: buf.name, tools: buf.tools, key: buf.key });
    }
    buf = null;
  };
  for (const part of parts) {
    if (part.kind === "tool") {
      const tc = part.payload as unknown as ToolCallPart;
      if (tc.status === "done") {
        if (buf && buf.name === tc.name) buf.tools.push(tc);
        else {
          flush();
          buf = { name: tc.name, tools: [tc], key: part.id };
        }
        continue;
      }
    }
    flush();
    out.push({ kind: "part", part });
  }
  flush();
  return out;
}

/** 工具归并组卡：一行摘要（工具名 + × N + 首个目标 + 总耗时），点击展开组内全部工具卡。 */
function ToolGroupCard({ name, tools }: { name: string; tools: ToolCallPart[] }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const totalMs = tools.reduce((n, p) => n + (p.durationMs ?? 0), 0);
  return (
    <div className="w-full">
      <button
        className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface-hover"
        onClick={() => setOpen(!open)}
      >
        <span className="h-4 w-0.5 shrink-0 rounded-full bg-border" />
        <span className="shrink-0 font-mono text-ui-sm text-foreground">{name}</span>
        <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">
          × {tools.length}
        </span>
        <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest">{tools[0]?.summary}</span>
        {totalMs > 0 && <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">{totalMs}ms</span>}
        <span className="shrink-0 text-ui-xs text-foreground-subtlest">完成</span>
        <ChevronRight className={`size-3.5 shrink-0 text-foreground-subtlest transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="flex flex-col gap-1 pt-1.5">
          {tools.map((p, i) => (
            <ToolCard key={`${i}_${p.toolCallId}`} part={p} />
          ))}
        </div>
      )}
    </div>
  );
}

/** 思考折叠行：单段思考独立开合（对齐 ZCode Reasoning）。默认折叠；
 * 流式中标题「思考中」扫光 + 右侧最新一行思考摘要（折叠着也能看到模型在想什么）；
 * 完成后显示「思考」（本会话内计时过的补「· N 秒」，快照恢复的历史无时长数据不显示）。
 * 手动展开后完全由用户控制。 */
function ReasoningFold({ part, streaming }: { part: PartSnapshot; streaming: boolean }): React.JSX.Element {
  const text = String(part.payload["text"] ?? "");
  // 流式摘要：思考文本最后一个非空行（正在写出的那行）
  const summary = streaming ? ([...text.split("\n")].reverse().find((l) => l.trim().length > 0)?.trim() ?? "") : "";
  const [open, setOpen] = useState(false);
  const [seconds, setSeconds] = useState<number | undefined>(undefined);
  // 流式计时：本会话内从该段开始流式起每秒推进；结束后固定。历史恢复（挂载即非流式）不计时。
  const startRef = useRef<number | null>(null);
  useEffect(() => {
    if (!streaming) {
      if (startRef.current !== null) {
        setSeconds(Math.max(1, Math.ceil((Date.now() - startRef.current) / 1000)));
        startRef.current = null;
      }
      return;
    }
    if (startRef.current === null) startRef.current = Date.now();
    const tick = () => {
      if (startRef.current !== null) setSeconds(Math.max(1, Math.ceil((Date.now() - startRef.current) / 1000)));
    };
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  }, [streaming]);

  return (
    <div className="flex w-full flex-col">
      <button
        className="inline-flex min-w-0 max-w-full items-center gap-2 self-start rounded-md px-1 py-0.5 text-left text-ui-base text-foreground-subtlest hover:text-foreground-subtle"
        onClick={() => setOpen(!open)}
      >
        <Brain className={`size-4 shrink-0 ${streaming ? "text-foreground-subtle" : ""}`} />
        {streaming ? (
          <>
            <span className="shrink-0 whitespace-nowrap animated-gradient-text">思考中</span>
            {summary && (
              <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest" title={summary}>
                {summary}
              </span>
            )}
          </>
        ) : (
          <span className="shrink-0 whitespace-nowrap">
            思考{seconds !== undefined ? ` · 持续了 ${seconds} 秒` : ""}
          </span>
        )}
        <ChevronRight className={`size-4 shrink-0 transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="select-text whitespace-pre-wrap break-words pt-1.5 text-ui-base text-foreground-subtlest">
          {text}
        </div>
      )}
    </div>
  );
}

// ---------- 卡片 ----------

/** 工具调用卡片：状态色条 + 工具名 + 摘要 + 耗时，点击展开输出；失败默认展开。 */
export function ToolCard({ part }: { part: ToolCallPart }): React.JSX.Element {
  const [open, setOpen] = useState(part.status === "error");
  const running = part.status === "running";
  return (
    <div className="w-full">
      <button
        className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-surface-hover"
        onClick={() => setOpen(!open)}
      >
        <span className={`h-4 w-0.5 rounded-full ${part.status === "error" ? "bg-destructive" : running ? "bg-icon-blue" : "bg-border"}`} />
        <span className={`shrink-0 font-mono text-ui-sm ${running ? "animated-gradient-text" : "text-foreground"}`}>
          {part.name}
        </span>
        <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest">{part.summary}</span>
        {part.durationMs ? <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">{part.durationMs}ms</span> : null}
        <span className={`shrink-0 text-ui-xs ${part.status === "error" ? "text-destructive" : "text-foreground-subtlest"}`}>
          {running ? "运行中" : part.status === "error" ? "失败" : "完成"}
        </span>
        <ChevronRight className={`size-3.5 shrink-0 text-foreground-subtlest transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="pt-1.5">
          {part.output ? (
            <pre className="select-text max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface px-3 py-2 font-mono text-ui-sm text-foreground-subtle">
              {part.output}
            </pre>
          ) : null}
        </div>
      )}
    </div>
  );
}

const RISK_LABELS: Record<string, string> = {
  R0: "只读",
  R1: "写入",
  R2: "执行",
  R3: "网络/越界",
  R4: "破坏性",
};

/** 审批卡片：风险徽章 + 目标 + 事实 + 决定按钮组（拒绝 / 允许一次 / 本会话 / 本工作区，永久授权按 allowPermanent 与风险等级显隐）。 */
export function ApprovalCard({ part, threadId }: { part: ApprovalPart; threadId: string }): React.JSX.Element {
  const waiting = part.status === "waiting";
  const [busy, setBusy] = useState(false);
  const pushToast = useToasts((s) => s.push);

  const decide = async (allow: boolean, scope: "once" | "session" | "workspace"): Promise<void> => {
    setBusy(true);
    try {
      await api.chatDecideApproval(part.approvalId, allow, scope);
    } catch (err) {
      // 决定失败（如重启后等待已失效的僵尸卡）保持卡片可重试，但必须给出可读反馈
      pushToast("danger", "审批操作失败", String(err));
    } finally {
      setBusy(false);
    }
  };

  const permanentVisible = part.allowPermanent && (part.risk === "R1" || part.risk === "R2");

  return (
    <div
      className={`w-full overflow-hidden rounded-2xl border bg-popover shadow-xs ${
        waiting ? "border-warning/40" : part.status === "approved" ? "border-success/40" : "border-border opacity-90"
      }`}
    >
      <div className="flex flex-col gap-3 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-ui-base font-medium text-foreground-subtle">需要你的批准</span>
          <span className="rounded-md bg-warning-surface px-1.5 py-0.5 font-mono text-ui-xs text-warning-surface-foreground">
            {part.risk} {RISK_LABELS[part.risk] ?? ""}
          </span>
          <span className="rounded-md bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">{part.tool}</span>
          <span className="ml-auto shrink-0 text-ui-sm">
            {waiting ? (
              <span className="animated-gradient-text">等待决定</span>
            ) : part.status === "approved" ? (
              <span className="text-success">已允许{part.scope && part.scope !== "once" ? `（${part.scope === "session" ? "本会话" : "本工作区"}）` : "一次"}</span>
            ) : part.status === "denied" ? (
              <span className="text-destructive">已拒绝</span>
            ) : (
              <span className="text-foreground-subtlest">已取消</span>
            )}
          </span>
        </div>
        {part.detail && <p className="text-ui-base leading-5 text-foreground">{part.detail}</p>}
        {part.target && (
          <div className="truncate rounded-lg bg-surface px-2.5 py-1.5 font-mono text-ui-sm text-foreground-subtle">{part.target}</div>
        )}
        {part.facts.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {part.facts.map((f, i) => (
              <span key={i} className="rounded-md bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle">
                {f.label}：{f.value}
              </span>
            ))}
          </div>
        )}
        {waiting && (
          <div className="flex items-center gap-2 pt-1">
            <button
              disabled={busy}
              className="h-8 rounded-lg border border-destructive/50 px-3 text-ui-sm text-destructive hover:bg-destructive-surface disabled:opacity-50"
              onClick={() => void decide(false, "once")}
            >
              拒绝
            </button>
            <div className="flex-1" />
            <button
              disabled={busy}
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
              onClick={() => void decide(true, "once")}
            >
              允许一次
            </button>
            <button
              disabled={busy}
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
              onClick={() => void decide(true, "session")}
            >
              本会话允许
            </button>
            {permanentVisible && (
              <button
                disabled={busy}
                className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-50"
                onClick={() => void decide(true, "workspace")}
              >
                本工作区始终允许
              </button>
            )}
          </div>
        )}
      </div>
      <span className="hidden">{threadId}</span>
    </div>
  );
}

/** 错误卡片：错误码 + 原始信息 + 分类处置建议（hints 按 craft-model 错误码给出）+ 重试。 */
function ErrorCard({ part, threadId }: { part: PartSnapshot; threadId: string }): React.JSX.Element {
  const code = String(part.payload["code"] ?? "");
  const message = String(part.payload["message"] ?? "");
  const hints: Record<string, string> = {
    auth: "请检查提供商密钥是否有效（设置 → 模型与提供商）。",
    rate_limit: "请求过于频繁或额度不足，稍后重试。",
    timeout: "模型响应超时，可重试。",
    context_overflow: "会话内容超出模型上下文窗口，请新建会话或切换更大窗口的模型。",
    provider_unavailable: "服务暂时不可用，请检查端点与网络。",
    invalid_request: "请求参数不被接受，请检查模型配置。",
    interrupted: "上次运行被中断，可重试。",
    canceled: "已取消。",
    max_iterations: "已达本轮最大调用次数。",
    workspace_missing: "会话绑定的工作区不存在，请重新选择工作区。",
  };
  return (
    <div className="rounded-lg border border-destructive/30 bg-destructive-surface px-3 py-2.5">
      <div className="flex items-center gap-2 pb-1">
        <AlertCircle className="size-4 shrink-0 text-destructive" />
        <span className="text-ui-sm font-medium text-destructive">执行出错</span>
        <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">{code}</span>
        <div className="flex-1" />
        <button
          className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtle hover:bg-surface-hover"
          onClick={() => void api.chatRegenerate(threadId)}
        >
          <RefreshCw className="size-3" /> 重试
        </button>
      </div>
      <p className="select-text whitespace-pre-wrap break-words text-ui-sm text-foreground">{message}</p>
      {hints[code] && <p className="pt-1 text-ui-xs text-foreground-subtlest">{hints[code]}</p>}
    </div>
  );
}

// ---------- 输入区 ----------



/** 模型 chip：显示会话模型（草稿态显示当前草稿槽的模型引用，缺省回落全局默认），点击在按钮上方弹出选择浮层。
 *  对齐 ZCode：模型名 + ChevronDown，未选时占位文案。
 *  作为 Popover.Trigger asChild 的 child，必须铺开透传 props（含 ref），否则 radix 的点击绑定落不到 DOM。 */
function ModelChip({ className, disabled, ...props }: React.ComponentProps<"button">): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const settings = useApp((s) => s.settings);
  const draftModel = useApp((s) => effectiveModelRef(s, s.mode));
  const draftLevel = useApp((s) => effectiveReasoningLevel(s, s.mode));
  const draftSlotData = useApp((s) => draftSlot(s, s.mode));
  const thread = threads.find((t) => t.id === activeId);
  const modelId = activeId === "" ? draftModel : (thread?.modelId || settings.defaultModel);
  const level = activeId === "" ? draftLevel : (thread?.reasoningLevel || "");
  // 显示名解析：会话名 → 草稿槽随选存的名字 → 其他会话同模型的 name → ID 尾段兜底
  const validName = (n: string | undefined): n is string => !!n && n !== "未选择模型" && n !== "模型不可用";
  const slotName = draftSlotData?.modelRef && draftSlotData.modelRef === modelId ? draftSlotData.modelName : undefined;
  const borrowedName = threads.find((t) => t.modelId === modelId && validName(t.modelName))?.modelName;
  const name = validName(thread?.modelName)
    ? thread!.modelName
    : validName(slotName)
      ? slotName!
      : validName(borrowedName)
        ? borrowedName!
        : (modelId.split("/").pop() ?? modelId);
  const label = modelId ? name : "选择模型";
  return (
    <button
      type="button"
      disabled={disabled}
      {...props}
      className={`flex h-7 shrink-0 items-center gap-1 rounded-lg pr-1 pl-2 text-ui-sm hover:bg-surface-hover disabled:hover:bg-transparent disabled:cursor-default disabled:opacity-50 ${
        modelId ? "text-foreground" : "text-foreground-subtle hover:text-foreground-subtle"
      } ${className ?? ""}`}
      title={modelId || "先在设置中添加提供商并选择模型"}
    >
      <span className="max-w-60 truncate">
        {label}
        {modelId && level !== "" && ` · ${reasoningLabel(level)}`}
      </span>
      <ChevronDown className="size-3.5 shrink-0 text-foreground-subtlest" />
    </button>
  );
}

/** 审批档位 chip：点击在按钮上方弹出档位选择浮层；切到「自由」先弹危险确认。
 *  对齐 ZCode 模式切换：档位图标 + ChevronDown；「自由」为高权限警示态（text-warning 常驻）。 */
function LevelChip(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const chooseLevel = useApprovalLevelAction();
  const picker = useOverlays((s) => s.picker);
  const openPicker = useOverlays((s) => s.openPicker);
  const closePicker = useOverlays((s) => s.closePicker);
  const open = picker === "level";
  const current = APPROVAL_LEVEL_NAMES[settings.approvalLevel] ?? settings.approvalLevel;
  const Icon = APPROVAL_LEVEL_ICONS[settings.approvalLevel];
  const warning = settings.approvalLevel === "free";
  return (
    <Popover.Root open={open} onOpenChange={(o) => (o ? openPicker("level") : closePicker())}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className={`flex h-7 shrink-0 items-center gap-1 rounded-lg px-2 text-ui-sm hover:bg-surface-hover ${
            warning ? "text-warning hover:text-warning" : "text-foreground-subtle"
          }`}
          title={`审批档位：${current}`}
        >
          <Icon className="size-3.5 shrink-0" />
          <span className="max-w-16 truncate">{current}</span>
          <ChevronDown className="size-3.5 shrink-0 text-foreground-subtlest" />
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          aria-label="选择审批档位"
          onCloseAutoFocus={(e) => {
            // 对齐 ZCode：关闭后焦点回 composer 输入框。
            e.preventDefault();
            document.querySelector<HTMLTextAreaElement>("[data-composer-input]")?.focus();
          }}
          className="z-50 w-80 rounded-xl border border-popover-border bg-popover p-1.5 shadow-md"
        >
          {APPROVAL_LEVELS.map((l) => {
            const LevelIcon = APPROVAL_LEVEL_ICONS[l.id];
            const selected = settings.approvalLevel === l.id;
            return (
              <button
                key={l.id}
                className={`flex min-h-11 w-full cursor-pointer items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-menu-hover ${
                  l.id === "free" ? "text-warning" : ""
                }`}
                onClick={() => {
                  chooseLevel(l.id);
                  closePicker();
                }}
              >
                <LevelIcon className="mt-0.5 size-4 shrink-0" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5 text-ui-sm font-medium text-foreground">
                    {l.name}
                    {selected && <Check className="size-3.5 shrink-0 text-success" />}
                  </span>
                  <span className="block text-ui-xs text-foreground-subtlest">{l.desc}</span>
                </span>
              </button>
            );
          })}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

/** 上下文占用圆环（14px、2px 描边）：底环整圆 + 进度弧从 12 点方向顺时针（SVG dasharray + rotate -90），
 *  颜色三档与审批告警同源（<50% subtle、50–80% warning、≥80% destructive），超出窗口画满环。 */
function ContextRing({ percent, size = 14, strokeWidth = 2 }: { percent: number; size?: number; strokeWidth?: number }): React.JSX.Element {
  const r = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * r;
  const ratio = Math.max(0, Math.min(1, percent));
  const tone = percent >= 0.8 ? "stroke-destructive" : percent >= 0.5 ? "stroke-warning" : "stroke-foreground-subtle";
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="shrink-0 -rotate-90" aria-hidden>
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="stroke-border" strokeWidth={strokeWidth} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        className={tone}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeDasharray={`${ratio * circumference} ${circumference}`}
      />
    </svg>
  );
}

function ContextRow({ label, value, muted }: { label: string; value: string; muted?: boolean }): React.JSX.Element {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0 text-foreground-subtle">{label}</dt>
      <dd className={`text-right tabular-nums ${muted ? "text-foreground-subtlest" : "text-foreground"}`}>{value}</dd>
    </div>
  );
}

/** 上下文占用 chip：圆环 + 百分比 + 输出速度，点击弹出详情浮层（口径沿袭历史版 contextHint：
 *  占用实测优先（thread.contextTokens = 最近一轮输入+输出，覆盖语义），没有调用记录时按
 *  「会话累计 token + 全部正文 UTF-16 长度 ÷ 4」估算兜底；窗口取模型目录 contextK，缺省 32k；
 *  速度运行中用流式实时估算（字符/4 ÷ 首增量起秒数），空闲显示最近一轮后端实测值，从未产生则不渲染。
 *  空态收口：未选择模型时整块不渲染。正文长度用 UTF-16 length 近似码点数（中文等价，emoji 有微差，仅用于提示）。 */
function ContextChip(): React.JSX.Element | null {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const settings = useApp((s) => s.settings);
  const thread = useApp((s) => s.threads[mode].find((t) => t.id === activeId));
  const draftModel = useApp((s) => effectiveModelRef(s, s.mode));
  const running = useApp((s) => s.chats[activeId]?.running ?? false);
  const modelId = activeId === "" ? draftModel : (thread?.modelId || settings.defaultModel);

  // 当前模型上下文窗口（千 token）：目录查不到（手动模型无目录信息）缺省 32k，保证界面仍有合理分母
  const [contextK, setContextK] = useState(32);
  useEffect(() => {
    if (!modelId) return;
    let alive = true;
    void api
      .providerList()
      .then((providers) => {
        const found = selectableModels(providers).flatMap((g) => g.models).find((m) => m.ref === modelId);
        // 查不到（手动模型无目录信息）回落缺省 32k，不保留上一个模型的窗口值
        if (alive) setContextK(found && found.contextK > 0 ? found.contextK : 32);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [modelId]);

  // 字符数派生（selector 返回数字：流式 delta 高频时仅在数值变化时触发重渲染）
  const textChars = useApp((s) => {
    let chars = 0;
    for (const m of s.chats[activeId]?.messages ?? []) {
      for (const p of m.parts) {
        if (p.kind === "text") chars += String(p.payload["text"] ?? "").length;
      }
    }
    return chars;
  });
  const streamChars = useApp((s) => {
    let chars = 0;
    for (const m of s.chats[activeId]?.messages ?? []) {
      if (m.role !== "assistant" || m.status !== "streaming") continue;
      for (const p of m.parts) {
        if (p.kind === "text") chars += String(p.payload["text"] ?? "").length;
      }
    }
    return chars;
  });
  // 实时速度的计时起点：首个流式增量出现时起表，流式结束清零
  const streamStartRef = useRef(0);
  if (streamChars > 0 && streamStartRef.current === 0) streamStartRef.current = Date.now();
  if (streamChars === 0) streamStartRef.current = 0;

  if (!modelId) return null;

  const reported = thread?.contextTokens ?? 0;
  const used = reported > 0 ? reported : Math.round((thread?.tokens ?? 0) + textChars / 4);
  const maxTokens = contextK * 1000;
  const percent = maxTokens > 0 ? Math.min(100, Math.round((used / maxTokens) * 100)) : 0;
  const elapsed = streamStartRef.current > 0 ? (Date.now() - streamStartRef.current) / 1000 : 0;
  const liveSpeed = streamChars > 0 && elapsed > 0 ? streamChars / 4 / elapsed : 0;
  const shownSpeed = running && liveSpeed > 0 ? liveSpeed : (thread?.contextSpeed ?? 0);
  const tone = percent >= 80 ? "text-destructive" : percent >= 50 ? "text-warning" : "text-foreground-subtle";

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          type="button"
          className={`flex h-7 shrink-0 items-center gap-1.5 rounded-lg px-2 text-ui-xs transition-colors hover:bg-surface-hover ${tone}`}
          title={`上下文占用 ${percent}%`}
        >
          <ContextRing percent={percent / 100} />
          <span className="tabular-nums">{percent}%</span>
          {shownSpeed > 0 && <span className="tabular-nums text-foreground-subtlest">{Math.round(shownSpeed)} tok/s</span>}
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          aria-label="上下文详情"
          onCloseAutoFocus={(e) => {
            // 关闭后焦点回 composer 输入框（与模型 / 档位浮层同约定）。
            e.preventDefault();
            document.querySelector<HTMLTextAreaElement>("[data-composer-input]")?.focus();
          }}
          className="z-50 w-80 rounded-xl border border-popover-border bg-popover p-3 shadow-md"
        >
          <p className="pb-2 text-ui-sm font-medium text-foreground">上下文</p>
          <dl className="flex flex-col gap-1.5 text-ui-xs">
            <ContextRow label="当前上下文" value={`${formatTokens(used)} / ${contextK}k（${percent}%）`} />
            <ContextRow label="本会话累计" value={formatTokens(thread?.tokens ?? 0)} />
            <ContextRow
              label="最近一轮缓存命中"
              value={(thread?.contextCacheTokens ?? 0) > 0 ? formatTokens(thread!.contextCacheTokens) : "—"}
              muted={(thread?.contextCacheTokens ?? 0) <= 0}
            />
            <ContextRow
              label="最近一轮速度"
              value={shownSpeed > 0 ? `${Math.round(shownSpeed)} tok/s` : "—"}
              muted={shownSpeed <= 0}
            />
          </dl>
          <p className="pt-2 text-ui-xs text-foreground-subtlest">占用为最近一轮模型请求的实测值；没有调用记录时按消息内容估算。</p>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

/** 输入区：草稿态读写当前草稿槽（内容随槽保留）、会话态用本地 state；运行中发送转为引导（steer），停止走 armed 二次确认（与双击 Esc 共享状态）。
 * 顶部信息行：左侧始终显示工作区名称（项目名 / 临时会话），右侧为计划模式与 worktree 两个发送选项开关。 */
function Composer({
  activeId,
  draftLayout,
  chat,
  planAwaiting,
  plan,
}: {
  activeId: string;
  draftLayout: boolean;
  chat: { running: boolean };
  planAwaiting: boolean;
  plan?: PlanInfo;
}): React.JSX.Element {
  // 输入态分流：草稿态（活动会话为空）读写当前草稿槽（按模式 × 工作区归属分槽，draftWsId 指向当前槽，
  // 内容随槽保留互不覆盖）；会话态回到组件本地 state（切会话丢输入的既有行为），打字不写草稿槽。
  const mode = useApp((s) => s.mode);
  const setDraft = useApp((s) => s.setDraft);
  const refreshThreads = useApp((s) => s.refreshThreads);
  const selectThread = useApp((s) => s.selectThread);
  const workspaces = useApp((s) => s.workspaces);
  const draftWsId = useApp((s) => s.draftWsId[s.mode]);
  const thread = useApp((s) => s.threads[s.mode].find((t) => t.id === activeId));
  const isDraft = activeId === "";
  const draft = useApp((s) => (isDraft ? (draftSlot(s, s.mode) ?? EMPTY_DRAFT) : EMPTY_DRAFT));
  const [localText, setLocalText] = useState("");
  const [localAttachments, setLocalAttachments] = useState<string[]>([]);
  const [localPlan, setLocalPlan] = useState(false);
  const [localWorktree, setLocalWorktree] = useState(false);
  const text = isDraft ? draft.text : localText;
  const attachments = isDraft ? draft.attachments : localAttachments;
  const setText = (value: string): void => {
    if (isDraft) setDraft(mode, { text: value });
    else setLocalText(value);
  };
  const setAttachments = (value: string[] | ((prev: string[]) => string[])): void => {
    if (isDraft) {
      const prev = draftSlot(useApp.getState(), mode)?.attachments ?? EMPTY_DRAFT.attachments;
      setDraft(mode, { attachments: typeof value === "function" ? value(prev) : value });
    } else {
      setLocalAttachments((prev) => (typeof value === "function" ? value(prev) : value));
    }
  };
  // 顶部信息行的工作区名称：草稿态取草稿槽归属，会话态取会话绑定的工作区；
  // 内置临时目录的名称就叫「临时会话」，查不到（孤儿会话）如实显示「未绑定工作区」。
  const metaWsId = isDraft ? draftWsId : (thread?.workspaceId ?? "");
  const metaWs = workspaces.find((w) => w.id === metaWsId);
  // 发送选项勾选态：草稿态随草稿槽保留；会话态用本地 state（与输入内容同生命周期）。
  // worktree 已绑定的会话恒为勾选且锁定（绑定不可解绑，本会话固定在工作树内执行）。
  const boundWorktree = thread?.worktreePath ?? "";
  const planOn = isDraft ? draft.planMode : localPlan;
  const worktreeOn = boundWorktree ? true : isDraft ? draft.useWorktree : localWorktree;
  const setPlanOn = (value: boolean): void => {
    if (isDraft) setDraft(mode, { planMode: value });
    else setLocalPlan(value);
  };
  const setWorktreeOn = (value: boolean): void => {
    if (isDraft) setDraft(mode, { useWorktree: value });
    else setLocalWorktree(value);
  };
  const togglePlan = (): void => setPlanOn(!planOn);
  /** worktree 勾选带预检：临时槽 / 非项目工作区 / Work 模式 / 非 git 仓库直接拒绝并说明，发送时后端还会兜底校验。 */
  const toggleWorktree = async (): Promise<void> => {
    if (boundWorktree || running) return;
    if (worktreeOn) {
      setWorktreeOn(false);
      return;
    }
    if (mode === "work") {
      pushToast("danger", "无法启用 worktree", "Work 模式没有 Git 能力，请切换到 Code 模式后重试");
      return;
    }
    if (!metaWs || metaWs.purpose !== "project") {
      pushToast("danger", "无法启用 worktree", "临时会话不绑定项目；请先添加项目，再从侧栏项目分组发起会话");
      return;
    }
    try {
      const statuses = await api.workspaceGitStatuses(metaWs.id);
      if (!statuses[metaWs.id]) {
        pushToast("danger", "无法启用 worktree", "当前项目不是 git 仓库，无法创建独立工作树");
        return;
      }
    } catch (err) {
      pushToast("danger", "worktree 预检失败", cleanErrorMessage(err));
      return;
    }
    setWorktreeOn(true);
  };
  // composer 配置 picker 排他态收在 overlays store（模型 / 档位互斥，⌘M 可从快捷键打开）。
  const picker = useOverlays((s) => s.picker);
  const openPicker = useOverlays((s) => s.openPicker);
  const closePicker = useOverlays((s) => s.closePicker);
  const pushToast = useToasts((s) => s.push);
  const running = chat?.running ?? false;
  const taRef = useRef<HTMLTextAreaElement>(null);
  // 停止「再按一次确认」armed 态：与全局 Esc 退出链共享（overlays store，800ms）
  const stopArmed = useOverlays((s) => s.stopArmed);
  const setStopArmed = useOverlays((s) => s.setStopArmed);

  const pickAndAttach = async (): Promise<void> => {
    try {
      const files = await api.pickFiles();
      if (files.length > 0) {
        setAttachments((prev) => [...prev, ...files.filter((f) => !prev.includes(f))].slice(0, 10));
      }
    } catch {
      // 用户取消
    }
  };

  const stop = (): void => {
    if (stopArmed) {
      setStopArmed(false);
      void api.chatCancel(activeId);
    } else {
      setStopArmed(true);
      setTimeout(() => {
        const s = useOverlays.getState();
        if (s.stopArmed) s.setStopArmed(false);
      }, 800);
    }
  };

  /** 粘贴上传：剪贴板文件经 savePastedAttachment 落附件目录后并入附件列表。 */
  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>): void => {
    const files = selectPastedFiles(e.clipboardData.files);
    if (files.length === 0) return;
    e.preventDefault();
    void (async () => {
      for (const file of files) {
        const base64 = await readFileAsBase64(file);
        const saved = await api.savePastedAttachment(pastedFileName(file), base64);
        setAttachments((prev) => (prev.includes(saved) ? prev : [...prev, saved].slice(0, 10)));
      }
    })().catch((err) => pushToast("danger", "粘贴上传失败", cleanErrorMessage(err)));
  };

  // 自适应高度（2→8 行）
  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 8 * 24)}px`;
  }, [text]);

  /** 计划确认执行：后端置批准状态（提示条消失）→ 取消勾选 → 发送固定确认消息（写操作回到审批档位管控）。 */
  const approvePlan = (): void => {
    void useApp
      .getState()
      .approvePlan(activeId)
      .then(() => {
        setPlanOn(false);
        return api.chatSend(activeId, "计划已确认，请按计划执行。", [], false, false);
      })
      .catch((err: unknown) => pushToast("danger", "发送失败", cleanErrorMessage(err)));
  };

  const submit = (): void => {
    const clean = text.trim();
    if (!clean && attachments.length === 0) return;
    const sendAttachments = attachments;
    const sendPlan = planOn;
    const sendWorktree = worktreeOn;
    setText("");
    setAttachments([]);
    const restore = (): void => {
      setText(clean); // 失败还原草稿与附件
      setAttachments(sendAttachments);
    };
    if (isDraft) {
      // 草稿首发：创建并首发原子路径。创建前失败（如未配置模型、worktree 创建失败）还原草稿；
      // 创建成功即视为发送成功——会话立即入栏并选中，Run 启动失败经 run.failed 在时间线展示，不还原草稿。
      // 工作区传当前草稿槽归属：临时槽空串（主进程回落 conversation 工作区），项目槽传项目 id（会话落入该项目分组）。
      // 模型取当前草稿槽引用（草稿态选择过），取不到回落全局默认；思考档位同理。
      const state = useApp.getState();
      const modelRef = effectiveModelRef(state, mode);
      const reasoningLevel = effectiveReasoningLevel(state, mode);
      void api
        .chatCreateAndStart(mode, state.draftWsId[mode], clean, sendAttachments, modelRef, reasoningLevel, sendPlan, sendWorktree)
        .then(async ({ threadId }) => {
          await refreshThreads(mode);
          // 首发飞行中切走模式时不抢选：新会话留在原模式侧栏
          if (useApp.getState().mode === mode) await selectThread(threadId);
        })
        .catch((err) => {
          restore();
          pushToast("danger", "发送失败", cleanErrorMessage(err));
        });
      return;
    }
    if (running) {
      // 运行中引导并入当前任务：仅计划标记生效（worktree 为会话级绑定，引导时不新增）
      void api
        .chatSteer(activeId, clean, sendAttachments, sendPlan)
        .catch((err) => {
          restore();
          pushToast("danger", "引导失败", cleanErrorMessage(err));
        });
    } else {
      void api
        .chatSend(activeId, clean, sendAttachments, sendPlan, sendWorktree)
        .catch((err) => {
          restore();
          pushToast("danger", "发送失败", cleanErrorMessage(err));
        });
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>): void => {
    // IME 组词中的 Enter 是候选确认（如中文输入法选词），交给输入法处理，不触发发送
    if (isImeComposing(e)) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="shrink-0 px-4 pb-4">
      {/* 宽度档位与消息列共用常量（docs/conversation-width/spec.md），草稿档收窄聚焦。 */}
      <div className={`mx-auto flex w-full flex-col gap-2 ${draftLayout ? DRAFT_WIDTH_CLASS : CONTENT_WIDTH_CLASS}`}>
          {/* 计划待批准条：计划 run 已结束且尚未确认；直接输入即继续调整，点按钮则确认执行。
              必须实底（bg-input，与输入卡同源）：本条浮在 sticky 渐变层的透明区上，bg-surface
              是半透明层次色（3%/5%），消息文字会穿透显示（docs/conversation-width/spec.md 第 8 条）。 */}
          {planAwaiting && !running && (
            <div className="flex items-center justify-end gap-3 rounded-xl border border-border bg-input px-3 py-2 shadow-xs" data-plan-approval>
            <button
              type="button"
              onClick={() => void api.planOpen(plan!.filePath).catch((err: unknown) => pushToast("danger", "打开计划失败", cleanErrorMessage(err)))}
              className="text-ui-xs text-foreground-subtle underline-offset-2 hover:underline"
              title="用系统默认应用打开计划文件"
            >
              查看计划
            </button>
            <button
              type="button"
              onClick={() => void useApp.getState().ignorePlan(activeId)}
              className="text-ui-xs text-foreground-subtlest underline-offset-2 hover:underline"
              title="暂不处理：提示条消失，之后可直接输入继续"
            >
              忽略
            </button>
            <span className="text-ui-xs text-foreground-subtle">计划已生成：直接输入可继续调整，或</span>
            <button
              type="button"
              onClick={approvePlan}
              className="flex items-center gap-1 rounded-full bg-brand px-3 py-1 text-ui-xs font-medium text-foreground-inverse transition-opacity hover:opacity-90"
              title="取消计划模式勾选，并让 Agent 按计划开始执行"
            >
              <Check className="size-3.5" /> 批准并执行
            </button>
          </div>
        )}
        <div
          className="relative flex flex-col gap-3 overflow-hidden rounded-2xl border-[0.5px] border-input-border bg-input p-3 shadow-(--shadow-composer-soft) transition-colors focus-within:border-input-border-focused"
          data-composer-layout="conversation"
        >
          {/* 顶部信息行：左 = 工作区名称（项目名 / 临时会话），右 = 发送选项开关（计划模式 / worktree） */}
          <div className="flex items-center justify-between gap-2" data-composer-meta-row>
            <span className="flex min-w-0 items-center gap-1.5 text-ui-xs text-foreground-subtle" title={metaWs?.path || undefined}>
              <Folder className="size-3 shrink-0" />
              <span className="truncate">{metaWs ? metaWs.name : "未绑定工作区"}</span>
            </span>
            <span className="flex shrink-0 items-center gap-1.5">
              <button
                type="button"
                aria-pressed={planOn}
                onClick={togglePlan}
                title="计划模式：本轮只做只读调研并输出计划，写类操作将被强制逐次审批，确认后再执行"
                className={`flex items-center gap-1 rounded-full border px-2 py-0.5 text-ui-xs transition-colors ${
                  planOn ? "border-brand/50 bg-brand/10 text-brand" : "border-border text-foreground-subtle hover:bg-surface-hover"
                }`}
              >
                <ListChecks className="size-3" /> 计划模式
              </button>
              <button
                type="button"
                aria-pressed={worktreeOn}
                disabled={running || !!boundWorktree}
                onClick={() => void toggleWorktree()}
                title={
                  boundWorktree
                    ? `已绑定独立工作树（不可解绑，删除会话时清理）：${boundWorktree}`
                    : "worktree：发送后创建独立 git 工作树并绑定本会话，任务改动与主目录隔离"
                }
                className={`flex items-center gap-1 rounded-full border px-2 py-0.5 text-ui-xs transition-colors ${
                  worktreeOn ? "border-brand/50 bg-brand/10 text-brand" : "border-border text-foreground-subtle hover:bg-surface-hover"
                } disabled:cursor-not-allowed disabled:opacity-60`}
              >
                <GitBranch className="size-3" /> {boundWorktree ? "worktree · 已绑定" : "worktree"}
              </button>
            </span>
          </div>
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {attachments.map((path) => (
                <span key={path} className="flex h-7 items-center gap-1.5 rounded-lg border border-border bg-surface px-2 text-ui-xs text-foreground-subtle">
                  <Paperclip className="size-3" />
                  <span className="max-w-48 truncate">{path.split("/").pop()}</span>
                  <button
                    className="rounded p-0.5 hover:bg-surface-hover"
                    onClick={() => setAttachments((prev) => prev.filter((p) => p !== path))}
                  >
                    <X className="size-3" />
                  </button>
                </span>
              ))}
            </div>
          )}
          <textarea
            ref={taRef}
            rows={2}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            placeholder={running ? "任务执行中，输入补充指令（将并入当前任务）…" : "描述你的任务…"}
            data-composer-input
            className="max-h-[192px] min-h-[48px] w-full resize-none overscroll-contain bg-transparent text-ui-base text-foreground placeholder:text-foreground-subtlest"
          />
          <div className="group/toolbar flex items-end gap-3">
            <div className="flex min-w-0 flex-1 items-center gap-0.5" data-composer-leading-actions>
              <button
                type="button"
                title="添加附件"
                className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover"
                onClick={() => void pickAndAttach()}
              >
                <Plus className="size-4" />
              </button>
              <LevelChip />
              <ContextChip />
            </div>
            <div className="ml-auto flex shrink-0 items-center justify-end gap-1.5" data-composer-trailing-actions>
              {/* 模型选择器贴发送簇左侧（对齐 ZCode：模型/思考簇紧挨停止/发送） */}
              <ModelPicker
                open={picker === "model"}
                onOpenChange={(o) => (o ? openPicker("model") : closePicker())}
                trigger={<ModelChip />}
              />
              {running ? (
                <button
                  type="button"
                  onClick={stop}
                  className={`flex size-8 items-center justify-center rounded-full border hover:bg-surface-hover ${
                    stopArmed ? "border-destructive text-destructive" : "border-border text-foreground-subtle"
                  }`}
                  title={stopArmed ? "再按一次确认停止（或按 Esc）" : "停止（再次点击或按 Esc）"}
                >
                  <Square className="size-3.5" />
                </button>
              ) : null}
              {/* 运行中且无待发内容时隐藏发送钮：禁用态圆钮与停止钮并排会被误读为两个发送键；
                  输入了补充指令后出现（此时点击=引导并入当前任务），空闲态恒显示 */}
              {(!running || text.trim() !== "" || attachments.length > 0) && (
                <button
                  type="button"
                  onClick={submit}
                  disabled={!text.trim()}
                  className="flex size-8 items-center justify-center gap-1 rounded-full bg-brand text-foreground-inverse transition-opacity disabled:opacity-30"
                  title="发送（Enter）"
                >
                  <ArrowUp className="size-4" />
                </button>
              )}
            </div>
          </div>
        </div>
        <div className="px-1 text-center text-ui-xs text-foreground-subtlest">
          Enter 发送 · Shift+Enter 换行 · 运行中输入为引导指令 · ⌘K 命令面板 · ⌘, 设置
        </div>
      </div>
    </div>
  );
}
