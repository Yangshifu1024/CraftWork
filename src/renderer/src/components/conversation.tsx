// 对话区：顶栏 + 消息流 + 输入区。
import { useEffect, useRef, useState } from "react";
import { useStickToBottom } from "use-stick-to-bottom";
import { ArrowUp, Check, ChevronDown, ChevronRight, Brain, AlertCircle, Copy, RefreshCw, Square, Plus, Paperclip, X } from "lucide-react";
import { Popover } from "radix-ui";
import { ModelPicker } from "./model-picker";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { api } from "../lib/api";
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

  return (
    <div id="content" className="flex min-h-0 min-w-[320px] flex-1 flex-col app-region-no-drag">
      <div className="relative flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-background shadow-xs">
        <WorkspaceHeader title={thread?.title ?? "新对话"} status={thread?.status ?? "idle"} mode={mode} />
        <Timeline activeId={activeId} chat={chat ? { messages: chat.messages, running: chat.running } : undefined} />
        <Composer activeId={activeId} chat={chat ? { running: chat.running } : { running: false }} />
      </div>
    </div>
  );
}

function WorkspaceHeader({ title, status, mode }: { title: string; status: string; mode: string }): React.JSX.Element {
  return (
    <div className="relative flex w-full shrink-0 border-b border-border/50 app-region-drag">
      <div className="flex h-12 min-w-0 flex-1 items-center justify-between gap-2 overflow-hidden p-2">
        <div className="flex min-w-0 items-center gap-2 pl-1">
          <span className="truncate text-ui-base font-medium text-foreground app-region-no-drag">{title}</span>
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

function Timeline({ activeId, chat }: { activeId: string; chat?: { messages: MessageSnapshot[]; running: boolean } }): React.JSX.Element {
  const { scrollRef, contentRef, isAtBottom, scrollToBottom } = useStickToBottom();
  const messages = chat?.messages ?? [];

  useEffect(() => {
    // 切换会话时贴底
    scrollToBottom();
  }, [activeId, scrollToBottom]);

  return (
    <div ref={scrollRef} className="relative min-h-0 flex-1 overflow-y-auto">
      <div ref={contentRef} className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-4 pb-8 pt-6">
        {messages.length === 0 && (
          <div className="flex flex-1 items-center justify-center py-24 text-ui-sm text-foreground-subtlest">
            开始你的第一条消息
          </div>
        )}
        {messages.map((message) => (
          <MessageRow key={message.id} message={message} chatRunning={chat?.running ?? false} />
        ))}
        <div />
      </div>
      {!isAtBottom && (
        <button
          className="sticky bottom-4 left-1/2 z-10 -translate-x-1/2 rounded-full border border-border bg-popover px-3 py-1.5 text-ui-sm text-foreground-subtle shadow-md hover:bg-menu-hover"
          onClick={() => scrollToBottom()}
        >
          <ChevronDown className="inline size-3.5" /> 回到底部
        </button>
      )}
    </div>
  );
}

function MessageRow({
  message,
  chatRunning,
}: {
  message: MessageSnapshot;
  chatRunning: boolean;
}): React.JSX.Element {
  const activeId = useApp((s) => s.activeThreadId[s.mode]);

  if (message.role === "user") {
    return (
      <div className="group/msg flex w-full flex-col items-end gap-1">
        <div className="flex max-w-full flex-col gap-2 rounded-xl rounded-tr-xs border border-border bg-user-message-bubble px-4 py-3 text-ui-base text-foreground sm:max-w-xl">
          {message.parts
            .filter((p) => p.kind === "text")
            .map((part) => (
              <div key={part.id} className="whitespace-pre-wrap break-words">
                {String(part.payload["text"] ?? "")}
              </div>
            ))}
        </div>
        {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动 */}
        <button
          className="mr-1 flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtlest opacity-0 transition-opacity duration-150 hover:bg-surface-hover group-hover/msg:opacity-100"
          onClick={() => {
            const text = message.parts
              .filter((p) => p.kind === "text")
              .map((p) => String(p.payload["text"] ?? ""))
              .join("\n");
            void api.clipboardWrite(text);
          }}
        >
          <Copy className="size-3" /> 复制
        </button>
      </div>
    );
  }

  // 助手消息：按「过程分组」切段渲染（工具/推理/审批收进折叠，最终正文在外）
  return (
    <div className="group/msg flex w-full flex-col gap-2">
      <TurnBody message={message} chatRunning={chatRunning} />
      {/* 操作条常驻占位：只切换透明度，避免 hover 时布局跳动；复制/重新生成由可见性控制 */}
      <div className="flex items-center gap-1 opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100">
        <button
          className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtlest hover:bg-surface-hover"
          onClick={() => {
            const text = message.parts
              .filter((p) => p.kind === "text")
              .map((p) => String(p.payload["text"] ?? ""))
              .join("\n");
            void api.clipboardWrite(text);
          }}
        >
          <Copy className="size-3" /> 复制
        </button>
        {!chatRunning && (
          <button
            className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtlest hover:bg-surface-hover"
            onClick={() => void api.chatRegenerate(activeId)}
          >
            <RefreshCw className="size-3" /> 重新生成
          </button>
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
      {hasWork && <WorkFold parts={parts} finalTextIdx={finalTextIdx} streaming={isStreaming} status={message.status} threadId={threadId} />}
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
            <div key={part.id} className="rounded-lg bg-surface px-3 py-2 text-ui-sm text-foreground-subtle">
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

/** 工作过程折叠块（推理 / 工具 / 审批 / todo）。 */
function WorkFold({
  parts,
  finalTextIdx,
  streaming,
  status,
  threadId,
}: {
  parts: PartSnapshot[];
  finalTextIdx: number;
  streaming: boolean;
  status: MessageSnapshot["status"];
  threadId: string;
}): React.JSX.Element {
  const work = parts.filter((p, i) => i !== finalTextIdx && p.kind !== "text");
  const hasRunning = work.some(
    (p) => (p.kind === "tool" && (p.payload as unknown as ToolCallPart).status === "running") || p.kind === "approval" && (p.payload as unknown as ApprovalPart).status === "waiting",
  );
  const [open, setOpen] = useState(false);
  const done = !streaming && status !== "streaming";

  const duration = done ? "" : "";
  return (
    <div className="flex w-full flex-col">
      <button
        className="group/reasoning inline-flex min-w-0 max-w-full items-center gap-2 self-start rounded-md px-1 py-0.5 text-ui-base text-foreground-subtlest hover:text-foreground-subtle"
        onClick={() => setOpen(!open)}
      >
        {done ? <Brain className="size-4 shrink-0" /> : <Brain className="size-4 shrink-0" />}
        <span className="shrink-0 whitespace-nowrap">
          {hasRunning ? <span className="animated-gradient-text">工作中</span> : done ? "已工作" : "正在思考"}
        </span>
        {duration && <span className="shrink-0 text-foreground-subtlest">{duration}</span>}
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
        <div className="whitespace-pre-wrap break-words text-ui-base text-foreground-subtlest">
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
            <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-surface px-3 py-2 font-mono text-ui-sm text-foreground-subtle">
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

  const decide = async (allow: boolean, scope: "once" | "session" | "workspace"): Promise<void> => {
    setBusy(true);
    try {
      await api.chatDecideApproval(part.approvalId, allow, scope);
    } catch (err) {
      // 决定失败保持卡片可重试
      console.warn(err);
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
      <p className="whitespace-pre-wrap break-words text-ui-sm text-foreground">{message}</p>
      {hints[code] && <p className="pt-1 text-ui-xs text-foreground-subtlest">{hints[code]}</p>}
    </div>
  );
}

function StreamCaret(): React.JSX.Element {
  return <span className="ml-0.5 inline-block h-3.5 w-[2px] translate-y-0.5 animate-pulse rounded-full bg-foreground" />;
}

// ---------- 输入区 ----------

/** 清洗 IPC 错误：剥掉「Error invoking remote method ...: Error: 」前缀，只留可读文案。 */
function cleanErrorMessage(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const marker = "Error:";
  const idx = raw.lastIndexOf(marker);
  return (idx >= 0 ? raw.slice(idx + marker.length) : raw).trim();
}

/** 模型 chip：显示会话模型（缺省用全局默认），点击在按钮上方弹出选择浮层。对齐 ZCode：模型名 + ChevronDown，未选时占位文案。
 *  作为 Popover.Trigger asChild 的 child，必须铺开透传 props（含 ref），否则 radix 的点击绑定落不到 DOM。 */
function ModelChip({ className, ...props }: React.ComponentProps<"button">): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const settings = useApp((s) => s.settings);
  const thread = threads.find((t) => t.id === activeId);
  const modelId = thread?.modelId || settings.defaultModel;
  const label = modelId
    ? thread?.modelName && thread.modelName !== "未选择模型" && thread.modelName !== "模型不可用"
      ? thread.modelName
      : (modelId.split("/").pop() ?? modelId)
    : "选择模型";
  return (
    <button
      type="button"
      {...props}
      className={`flex h-7 shrink-0 items-center gap-1 rounded-lg pr-1 pl-2 text-ui-sm hover:bg-surface-hover ${
        modelId ? "text-foreground" : "text-foreground-subtle hover:text-foreground-subtle"
      } ${className ?? ""}`}
      title={modelId || "先在设置中添加提供商并选择模型"}
    >
      <span className="max-w-40 truncate">{label}</span>
      <ChevronDown className="size-3.5 shrink-0 text-foreground-subtlest" />
    </button>
  );
}

/** 模式切换：Code / Work 两段 pill。 */
function ModeSwitch(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const setMode = useApp((s) => s.setMode);
  return (
    <div className="flex h-7 shrink-0 items-center gap-0.5 rounded-lg bg-surface p-0.5">
      {(["code", "work"] as const).map((m) => (
        <button
          key={m}
          className={`h-6 rounded-md px-2 text-ui-xs transition-colors ${
            mode === m ? "bg-card text-foreground shadow-xs" : "text-foreground-subtlest hover:text-foreground-subtle"
          }`}
          onClick={() => setMode(m)}
        >
          {m === "code" ? "Code" : "Work"}
        </button>
      ))}
    </div>
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
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState<string[]>([]);
  // composer 配置 picker 排他态收在 overlays store（模型 / 档位互斥，⌘M 可从快捷键打开）。
  const picker = useOverlays((s) => s.picker);
  const openPicker = useOverlays((s) => s.openPicker);
  const closePicker = useOverlays((s) => s.closePicker);
  const pushToast = useToasts((s) => s.push);
  const running = chat?.running ?? false;
  const taRef = useRef<HTMLTextAreaElement>(null);
  const armedRef = useRef<number>(0);

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
    if (Date.now() - armedRef.current < 800) {
      void api.chatCancel(activeId);
      armedRef.current = 0;
    } else {
      armedRef.current = Date.now();
      setTimeout(() => (armedRef.current = 0), 800);
    }
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
    if ((!clean && attachments.length === 0) || !activeId) return;
    const sendAttachments = attachments;
    setText("");
    setAttachments([]);
    const restore = (): void => {
      setText(clean); // 失败还原草稿与附件
      setAttachments(sendAttachments);
    };
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
            placeholder={running ? "任务执行中，输入补充指令（将并入当前任务）…" : "描述你的任务…"}
            data-composer-input
            className="max-h-[192px] min-h-[24px] w-full resize-none bg-transparent text-ui-base text-foreground placeholder:text-foreground-subtlest"
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
                  className="flex size-8 items-center justify-center rounded-full border border-border text-foreground-subtle hover:bg-surface-hover"
                  title="停止（Esc）"
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
