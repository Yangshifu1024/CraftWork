// 对话区：顶栏 + 消息流 + 输入区。
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStickToBottom } from "use-stick-to-bottom";
import { ArrowUp, Check, ChevronDown, ChevronRight, Brain, AlertCircle, Copy, RefreshCw, Square, Plus, Paperclip, X } from "lucide-react";
import { Popover, ToggleGroup } from "radix-ui";
import { ModelPicker } from "./model-picker";
import { useApp, draftSlot, effectiveModelRef, EMPTY_DRAFT } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { api } from "../lib/api";
import { cleanErrorMessage } from "../lib/errors";
import { isImageName, pastedFileName, readFileAsBase64, selectPastedFiles } from "../lib/paste";
import { APPROVAL_LEVELS, APPROVAL_LEVEL_NAMES, APPROVAL_LEVEL_ICONS, useApprovalLevelAction } from "../lib/approval";
import type { ApprovalPart, ToolCallPart } from "../../../shared/events";
import type { MessageSnapshot, PartSnapshot } from "../../../shared/types";
import { Markdown } from "./markdown";
import { TodoCard } from "./cards";

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

  return (
    <div className="flex min-h-0 min-w-[320px] flex-1 flex-col">
      <div className="relative flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-background shadow-xs">
        <WorkspaceHeader title={title} status={thread?.status ?? "idle"} mode={mode} />
        {/* 输入区以 sticky 沉底浮在消息流上：滚动区贯穿到容器底，滚动条贴底=贴窗口底 */}
        <Timeline
          activeId={activeId}
          chat={chat ? { messages: chat.messages, running: chat.running } : undefined}
          footer={<Composer activeId={activeId} chat={chat ? { running: chat.running } : { running: false }} />}
        />
      </div>
    </div>
  );
}

function WorkspaceHeader({ title, status, mode }: { title: string; status: string; mode: string }): React.JSX.Element {
  // "deep"：子树内任意点击均可拖拽/双击最大化。裸属性只匹配直接命中的元素本身，
  // 而内层 div 铺满标题栏，点击永远落不到外层上；按钮等可点击元素会自动阻断，无需单独排除。
  // select-none：macOS 双击最大化时 drag.js 不拦截默认行为（以支持移动取消），需禁用选字。
  return (
    <div className="relative flex w-full shrink-0 select-none border-b border-border/50" data-tauri-drag-region="deep">
      <div className="flex h-12 min-w-0 flex-1 items-center justify-between gap-2 overflow-hidden p-2">
        <div className="flex min-w-0 items-center gap-2 pl-1">
          <span className="truncate text-ui-base font-medium text-foreground">{title}</span>
          <span className="shrink-0 rounded-full bg-surface px-2 py-0.5 text-ui-xs text-foreground-subtle">
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

function Timeline({
  activeId,
  chat,
  footer,
}: {
  activeId: string;
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
          输入区增减高度会触发内容 resize，贴底状态由 use-stick-to-bottom 自动保持。 */}
      <div ref={contentRef} className="mx-auto flex min-h-full w-full max-w-3xl flex-col px-4">
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
      {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动 */}
      <div className="flex items-center gap-1.5 opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100">
        <CopyButton
          getText={() =>
            message.parts
              .filter((p) => p.kind === "text")
              .map((p) => String(p.payload["text"] ?? ""))
              .join("\n")
          }
        />
        {message.createdAt > 0 && (
          <span className="select-none text-ui-xs text-foreground-subtlest">{fmtDateTime(message.createdAt)}</span>
        )}
      </div>
    </div>
  );
}

/** 单条助手消息的渲染：工作部件折叠 + 最终正文。 */
function TurnBody({ message, chatRunning }: { message: MessageSnapshot; chatRunning: boolean }): React.JSX.Element {
  const threadId = useActiveThreadId();
  const parts = message.parts;
  // 最终正文 = 最后一个非空 text 部件
  let finalTextIdx = -1;
  for (let i = parts.length - 1; i >= 0; i--) {
    const p = parts[i]!;
    if (p.kind === "text" && String(p.payload["text"] ?? "").length > 0) {
      finalTextIdx = i;
      break;
    }
  }
  const hasWork = parts.some((p, i) => i !== finalTextIdx && p.kind !== "text");
  const isStreaming = message.status === "streaming";

  return (
    <div className="flex w-full flex-col gap-2">
      {hasWork && (
        <WorkFold
          parts={parts}
          finalTextIdx={finalTextIdx}
          streaming={isStreaming}
          status={message.status}
          threadId={threadId}
          durationMs={message.workDurationMs}
        />
      )}
      {parts.map((part, idx) => {
        if (idx === finalTextIdx) {
          const text = String(part.payload["text"] ?? "");
          return (
            <div key={part.id} className="text-ui-base text-foreground">
              <Markdown text={text} streaming={isStreaming} />
              {isStreaming && <StreamCaret />}
            </div>
          );
        }
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
      {isStreaming && finalTextIdx < 0 && chatRunning && (
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

/** 工作过程折叠块（推理 / 工具 / 审批 / todo）。已完成时显示该轮工作的耗时。 */
function WorkFold({
  parts,
  finalTextIdx,
  streaming,
  status,
  threadId,
  durationMs,
}: {
  parts: PartSnapshot[];
  finalTextIdx: number;
  streaming: boolean;
  status: MessageSnapshot["status"];
  threadId: string;
  durationMs?: number;
}): React.JSX.Element {
  const work = parts.filter((p, i) => i !== finalTextIdx && p.kind !== "text");
  const hasRunning = work.some(
    (p) => (p.kind === "tool" && (p.payload as unknown as ToolCallPart).status === "running") || p.kind === "approval" && (p.payload as unknown as ApprovalPart).status === "waiting",
  );
  const [open, setOpen] = useState(false);
  const done = !streaming && status !== "streaming";

  // 完成态显示秒数（向上取整，与侧栏「思考过程 · 持续了 N 秒」口径一致）
  const duration = done && durationMs && durationMs > 0 ? `持续 ${Math.ceil(durationMs / 1000)} 秒` : "";
  return (
    <div className="flex w-full flex-col">
      <button
        className="group/reasoning inline-flex min-w-0 max-w-full items-center gap-2 self-start rounded-md px-1 py-0.5 text-ui-base text-foreground-subtlest hover:text-foreground-subtle"
        onClick={() => setOpen(!open)}
      >
        <Brain className="size-4 shrink-0" />
        <span className={`shrink-0 whitespace-nowrap ${hasRunning ? "animated-gradient-text" : ""}`}>
          {hasRunning ? "工作中" : done ? "已工作" : "正在思考"}
        </span>
        {duration && <span className="shrink-0 whitespace-nowrap text-foreground-subtlest">{duration}</span>}
        <ChevronRight className={`size-4 shrink-0 transition-transform ${open ? "rotate-90" : ""}`} />
      </button>
      {open && (
        <div className="ml-2 max-h-72 space-y-2 overflow-y-auto border-border border-l pl-3.5 pt-2">
          {work.map((part) => (
            <WorkPart key={part.id} part={part} threadId={threadId} />
          ))}
        </div>
      )}
    </div>
  );
}

function WorkPart({ part, threadId }: { part: PartSnapshot; threadId: string }): React.JSX.Element | null {
  switch (part.kind) {
    case "reasoning":
      return (
        <div className="select-text whitespace-pre-wrap break-words text-ui-base text-foreground-subtlest">
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

// ---------- 卡片 ----------

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

function StreamCaret(): React.JSX.Element {
  return <span className="ml-0.5 inline-block h-3.5 w-[2px] translate-y-0.5 animate-pulse rounded-full bg-foreground" />;
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
  const thread = threads.find((t) => t.id === activeId);
  const modelId = activeId === "" ? draftModel : (thread?.modelId || settings.defaultModel);
  const label = modelId
    ? thread?.modelName && thread.modelName !== "未选择模型" && thread.modelName !== "模型不可用"
      ? thread.modelName
      : (modelId.split("/").pop() ?? modelId)
    : "选择模型";
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
      <span className="max-w-40 truncate">{label}</span>
      <ChevronDown className="size-3.5 shrink-0 text-foreground-subtlest" />
    </button>
  );
}

/** 模式切换：Code / Work 两段 pill（radix ToggleGroup，方向键切换；single 模式防取消选中）。 */
function ModeSwitch(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const setMode = useApp((s) => s.setMode);
  return (
    <ToggleGroup.Root
      type="single"
      value={mode}
      onValueChange={(v) => { if (v) setMode(v as "code" | "work"); }}
      className="flex h-7 shrink-0 items-center gap-0.5 rounded-lg bg-surface p-0.5"
    >
      {(["code", "work"] as const).map((m) => (
        <ToggleGroup.Item
          key={m}
          value={m}
          className="h-6 rounded-md px-2 text-ui-xs transition-colors outline-none data-[state=on]:bg-card data-[state=on]:text-foreground data-[state=on]:shadow-xs data-[state=off]:text-foreground-subtlest hover:text-foreground-subtle"
        >
          {m === "code" ? "Code" : "Work"}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
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

function Composer({ activeId, chat }: { activeId: string; chat: { running: boolean } }): React.JSX.Element {
  // 输入态分流：草稿态（活动会话为空）读写当前草稿槽（按模式 × 工作区归属分槽，draftWsId 指向当前槽，
  // 内容随槽保留互不覆盖）；会话态回到组件本地 state（切会话丢输入的既有行为），打字不写草稿槽。
  const mode = useApp((s) => s.mode);
  const setDraft = useApp((s) => s.setDraft);
  const refreshThreads = useApp((s) => s.refreshThreads);
  const selectThread = useApp((s) => s.selectThread);
  const isDraft = activeId === "";
  const draft = useApp((s) => (isDraft ? (draftSlot(s, s.mode) ?? EMPTY_DRAFT) : EMPTY_DRAFT));
  const [localText, setLocalText] = useState("");
  const [localAttachments, setLocalAttachments] = useState<string[]>([]);
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

  // 自适应高度（1→8 行）
  useEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 8 * 24)}px`;
  }, [text]);

  const submit = (): void => {
    const clean = text.trim();
    if (!clean && attachments.length === 0) return;
    const sendAttachments = attachments;
    setText("");
    setAttachments([]);
    const restore = (): void => {
      setText(clean); // 失败还原草稿与附件
      setAttachments(sendAttachments);
    };
    if (isDraft) {
      // 草稿首发：创建并首发原子路径。创建前失败（如未配置模型）还原草稿；
      // 创建成功即视为发送成功——会话立即入栏并选中，Run 启动失败经 run.failed 在时间线展示，不还原草稿。
      // 工作区传当前草稿槽归属：临时槽空串（主进程回落 conversation 工作区），项目槽传项目 id（会话落入该项目分组）。
      // 模型取当前草稿槽引用（草稿态选择过），取不到回落全局默认。
      const state = useApp.getState();
      const modelRef = effectiveModelRef(state, mode);
      void api
        .chatCreateAndStart(mode, state.draftWsId[mode], clean, sendAttachments, modelRef)
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
      void api
        .chatSteer(activeId, clean, sendAttachments)
        .catch((err) => {
          restore();
          pushToast("danger", "引导失败", cleanErrorMessage(err));
        });
    } else {
      void api
        .chatSend(activeId, clean, sendAttachments)
        .catch((err) => {
          restore();
          pushToast("danger", "发送失败", cleanErrorMessage(err));
        });
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>): void => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="shrink-0 px-4 pb-4">
      <div className="mx-auto flex max-w-3xl flex-col gap-2">
        <div
          className="relative flex flex-col gap-3 overflow-hidden rounded-2xl border-[0.5px] border-input-border bg-input p-3 shadow-(--shadow-composer-soft) transition-colors focus-within:border-input-border-focused"
          data-composer-layout="conversation"
        >
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
            rows={1}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            placeholder={running ? "任务执行中，输入补充指令（将并入当前任务）…" : "描述你的任务…"}
            data-composer-input
            className="max-h-[192px] min-h-[24px] w-full resize-none overscroll-contain bg-transparent text-ui-base text-foreground placeholder:text-foreground-subtlest"
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
              <ModeSwitch />
              <ModelPicker
                open={picker === "model"}
                onOpenChange={(o) => (o ? openPicker("model") : closePicker())}
                trigger={<ModelChip />}
              />
              <LevelChip />
            </div>
            <div className="ml-auto flex shrink-0 items-center justify-end gap-1.5" data-composer-trailing-actions>
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
              <button
                type="button"
                onClick={submit}
                disabled={!text.trim()}
                className="flex size-8 items-center justify-center gap-1 rounded-full bg-brand text-foreground-inverse transition-opacity disabled:opacity-30"
                title="发送（Enter）"
              >
                <ArrowUp className="size-4" />
              </button>
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
