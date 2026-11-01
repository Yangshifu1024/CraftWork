import { useState } from "react";
import { Icon, type IconName } from "../../components/Icon";
import { copyText } from "../../lib/clipboard";
import type {
  ApprovalPartData,
  CalloutPartData,
  DraftPartData,
  ErrorPartData,
  NoticePartData,
  TodoPartData,
  ToolPartData,
} from "../../lib/types";
import { cn, formatBytes } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
};

function toolIcon(name: string): IconName {
  switch (name) {
    case "fs_read":
      return "file-text";
    case "fs_list":
      return "folder-open";
    case "fs_glob":
      return "file";
    case "fs_grep":
      return "search";
    case "fs_write":
      return "save";
    case "fs_edit":
      return "edit";
    case "shell_exec":
      return "terminal";
    case "git_status":
      return "git-branch";
    case "todo_write":
      return "list-todo";
    default:
      return "zap";
  }
}

function formatArg(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

const statusMap: Record<string, { cls: string; label: string; icon: IconName; spin?: boolean }> = {
  running: { cls: "running", label: "运行中", icon: "loader", spin: true },
  done: { cls: "done", label: "完成", icon: "check" },
  error: { cls: "error", label: "失败", icon: "x" },
};

export function ToolCard({ data }: { data: ToolPartData }) {
  const [open, setOpen] = useState(data.status === "error");
  const status = statusMap[data.status] ?? statusMap.done;
  const args = Object.entries(data.args ?? {}).slice(0, 6);

  return (
    <div className={cn("blk tool-card", `status-${status.cls}`, open && "is-open")}>
      <button type="button" className="tool-head" onClick={() => setOpen(!open)}>
        <span className="tool-icon">
          <Icon name={toolIcon(data.name)} />
        </span>
        <span className="tool-main">
          <span className="tool-name">{data.name}</span>
          {data.risk ? <span className={cn("risk-badge", `risk-${data.risk}`)}>{data.risk}</span> : null}
          <span className="tool-summary">{data.summary}</span>
        </span>
        <span className="tool-meta">
          {data.durationMs ? <span>{data.durationMs} ms</span> : null}
        </span>
        <span className={cn("tool-status", status.cls)}>
          <Icon name={status.icon} className={status.spin ? "spin" : undefined} />
          {status.label}
        </span>
        <span className="tool-chev">
          <Icon name="chevron-right" />
        </span>
      </button>
      <div className="tool-body">
        {args.length > 0 ? (
          <div className="tool-args">
            {args.map(([key, value]) => (
              <span key={key} className="tool-arg">
                <b>{key}</b> <span className="mono">{formatArg(value)}</span>
              </span>
            ))}
          </div>
        ) : null}
        <pre className="tool-output">{data.output ?? ""}</pre>
        <div className="tool-foot">
          <span>{data.isError ? "执行失败" : data.status === "running" ? "正在执行" : "执行完成"}</span>
          <span className="spacer" />
          <button type="button" onClick={() => void copyText(data.output ?? "")}>
            <Icon name="copy" />
            复制输出
          </button>
        </div>
      </div>
    </div>
  );
}

export function ReasoningBlock({ text }: { text?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className={cn("blk reasoning", open && "is-open")}>
      <button type="button" className="reasoning-head" onClick={() => setOpen(!open)}>
        <Icon name="chevron-right" className="chev" />
        <Icon name="sparkles" />
        <span>思考过程</span>
      </button>
      <div className="reasoning-body">{text ?? ""}</div>
    </div>
  );
}

export function TodoCard({ data }: { data: TodoPartData }) {
  const items = data.items ?? [];
  const done = items.filter((item) => item.status === "done").length;
  const pct = items.length ? Math.round((done / items.length) * 100) : 0;
  return (
    <div className="blk todo-card">
      <div className="todo-head">
        <Icon name="list-todo" />
        <span>任务清单</span>
        <span className="spacer" />
        <span className="todo-count">
          {done} / {items.length}
        </span>
        <span className="todo-progress">
          <i style={{ width: `${pct}%` }} />
        </span>
      </div>
      <div className="todo-list">
        {items.map((item, index) => (
          <div key={`${item.text}-${index}`} className={cn("todo-item", item.status === "done" && "done", item.status === "active" && "active")}>
            <span className="tbox">{item.status === "done" ? <Icon name="check" /> : null}</span>
            <span className="tlabel">{item.text}</span>
            {item.status === "active" ? <span className="tmeta">进行中</span> : null}
          </div>
        ))}
      </div>
    </div>
  );
}

function previewLines(oldText: string, newText: string) {
  const oldLines = oldText.split("\n").slice(0, 6);
  const newLines = newText.split("\n").slice(0, 6);
  return [
    ...oldLines.map((text) => ({ kind: "del" as const, text })),
    ...newLines.map((text) => ({ kind: "add" as const, text })),
  ];
}

export function ApprovalCard({ data }: { data: ApprovalPartData }) {
  const decide = useThreadStore((state) => state.decide);
  const level = useAppStore((state) => state.settings.approvalLevel);
  const openDiff = useWorkspaceStore((state) => state.openDiff);
  const waiting = data.status === "waiting";
  const approved = data.status === "approved";
  const denied = data.status === "denied" || data.status === "canceled";
  const permanentAllowed =
    Boolean(data.allowPermanent) &&
    (data.risk === "R1" || data.risk === "R2") &&
    level === "graded";

  const args = data.args ?? {};
  const oldText =
    typeof args.old_string === "string"
      ? args.old_string
      : typeof args.old_text === "string"
        ? args.old_text
        : "";
  const newText =
    typeof args.new_string === "string"
      ? args.new_string
      : typeof args.new_text === "string"
        ? args.new_text
        : "";
  const hasDiffPreview = (data.tool === "fs_edit" || data.tool === "doc_edit") && Boolean(oldText);
  const isCommand = data.tool === "shell_exec";
  const patchText = typeof args.patch === "string" ? args.patch : "";
  const isPatch = data.tool === "apply_patch" && Boolean(patchText);
  const previewLinesValue = hasDiffPreview ? previewLines(oldText, newText) : [];

  const note =
    level === "strict"
      ? "严格档位：所有写入与执行逐次询问，不提供永久允许"
      : level === "readonly"
        ? "只读档位：写入与执行会被阻止"
        : "默认选中“拒绝”；Esc 拒绝；R3 / R4 不提供永久允许";

  return (
    <div
      className={cn(
        "blk approval-card",
        waiting ? "is-waiting" : approved ? "is-approved" : denied ? "is-denied" : "",
      )}
      data-risk={data.risk}
    >
      <div className="approval-strip" />
      <div className="approval-head">
        <span className="approval-title">
          <Icon name="shield" />
          需要你的批准
          <span className={cn("risk-badge", `risk-${data.risk}`)}>{data.risk}</span>
          <span className="tool-name">{data.tool}</span>
          {level !== "graded" ? (
            <span className={cn("badge", level === "strict" ? "warn" : "neutral")} style={{ height: 16 }}>
              {levelLabel[level]}档位
            </span>
          ) : null}
        </span>
        <span className="approval-hint">{waiting ? "等待决定" : approved ? "已批准" : "已拒绝"}</span>
      </div>
      <div className="approval-body">
        <div className="approval-summary">{data.detail || data.title}</div>
        <div className="approval-target">
          <Icon name="file" />
          <span>{data.target}</span>
        </div>
        <div className="approval-facts">
          {(data.facts ?? []).map((fact) => (
            <span key={`${fact.label}-${fact.value}`} className="approval-fact">
              <Icon name="info" />
              {fact.label}：{fact.value}
            </span>
          ))}
        </div>
        {hasDiffPreview || isCommand || isPatch ? (
          <div className="approval-diff-preview">
            <div className="approval-diff-head">
              <Icon name="diff" />
              <span>{hasDiffPreview ? "文本级差异预览" : isPatch ? "补丁预览" : "命令预览"}</span>
              <span className="spacer" />
              {hasDiffPreview ? (
                <button type="button" onClick={() => void openDiff(data.target, oldText, newText)}>
                  查看完整差异
                </button>
              ) : null}
            </div>
            <div className="approval-diff-body">
              {hasDiffPreview ? (
                previewLinesValue.map((line, index) => (
                  <div key={index} className={cn("diff-line", line.kind)}>
                    <span className="sign">{line.kind === "add" ? "+" : "−"}</span>
                    <span className="ln" />
                    <span>{line.text}</span>
                  </div>
                ))
              ) : isPatch ? (
                patchText
                  .split("\n")
                  .slice(0, 60)
                  .map((line, index) => {
                    const kind = line.startsWith("@@")
                      ? "ctx"
                      : line.startsWith("+")
                        ? "add"
                        : line.startsWith("-")
                          ? "del"
                          : "ctx";
                    return (
                      <div key={index} className={cn("diff-line", kind)}>
                        <span className="sign">
                          {kind === "add" ? "+" : kind === "del" ? "−" : line.startsWith("@@") ? "@" : " "}
                        </span>
                        <span className="ln" />
                        <span>{line}</span>
                      </div>
                    );
                  })
              ) : (
                <div className="diff-line ctx">
                  <span className="sign">$</span>
                  <span className="ln" />
                  <span>{data.target}</span>
                </div>
              )}
            </div>
          </div>
        ) : null}
      </div>

      {waiting ? (
        <>
          <div className="approval-actions">
            <button type="button" className="btn danger" autoFocus onClick={() => void decide(data.approvalId, false, "once")}>
              拒绝
            </button>
            <span className="spacer" />
            <button type="button" className="btn" onClick={() => void decide(data.approvalId, true, "once")}>
              允许一次
            </button>
            {permanentAllowed ? (
              <>
                <button type="button" className="btn ghost" onClick={() => void decide(data.approvalId, true, "session")}>
                  本会话允许
                </button>
                <button type="button" className="btn ghost" onClick={() => void decide(data.approvalId, true, "workspace")}>
                  本工作区始终允许
                </button>
              </>
            ) : null}
          </div>
          <div className="approval-actions" style={{ borderTop: 0, paddingTop: 0 }}>
            <span className="approval-default-note">
              <Icon name="info" />
              {note}
            </span>
          </div>
        </>
      ) : null}

      {approved ? (
        <div className="approval-result approved">
          <Icon name="check-circle" />
          已允许
          {data.scope === "once" ? "一次" : data.scope === "session" ? "（本会话）" : "（本工作区）"}
          <span className="scope">· 已写入审计</span>
        </div>
      ) : denied ? (
        <div className="approval-result denied">
          <Icon name="x-circle" />
          {data.status === "canceled" ? "已取消" : "已拒绝"}
          <span className="scope">· 未执行任何修改</span>
        </div>
      ) : null}
    </div>
  );
}

const errorHints: Record<string, string> = {
  auth: "密钥无效或权限不足：请在设置 → 模型 → 提供商中重新保存 API Key，并确认该 Key 允许调用模型。",
  rate_limit: "请求被限流或额度不足：请稍后重试，或在设置中切换其他模型。",
  timeout: "模型响应超时：可以重试；长对话可先精简历史或新建线程。",
  context_overflow: "输入超出模型上下文窗口：请新建线程、精简历史，或切换到上下文更大的模型。",
  provider_unavailable: "模型服务暂时不可用：请稍后重试，或切换到其他提供商。",
  invalid_request: "请求参数不被模型接受：请检查模型名称、端点与工具配置。",
  interrupted: "上次运行在应用退出时中断，已保留已产生的消息；可以点击重试继续。",
  canceled: "本次生成已取消。",
};

export function ErrorCard({ data }: { data: ErrorPartData }) {
  const regenerate = useThreadStore((state) => state.regenerate);
  const toast = useUiStore((state) => state.toast);
  const hint = errorHints[data.code];
  return (
    <div className="blk error-card">
      <div className="error-head">
        <Icon name="alert" />
        <span>执行出错</span>
        <span className="spacer" />
        <span className="badge danger">{data.code || "error"}</span>
      </div>
      <div className="error-body">{data.message}</div>
      {hint ? <div className="error-hint">{hint}</div> : null}
      <div className="error-actions">
        <button type="button" className="btn sm" onClick={() => void regenerate()}>
          <Icon name="refresh" />
          重试
        </button>
        <button
          type="button"
          className="btn sm ghost"
          onClick={() => {
            void copyText(data.message);
            toast({ title: "已复制错误信息", tone: "ok" });
          }}
        >
          <Icon name="copy" />
          复制错误
        </button>
      </div>
    </div>
  );
}

export function DraftCard({ data }: { data: DraftPartData }) {
  const toast = useUiStore((state) => state.toast);
  return (
    <div className="blk draft-card">
      <div className="draft-head">
        <Icon name="mail" />
        <span className="draft-kind">草稿</span>
        <span className="spacer" />
        <span className="badge work">仅应用内显示</span>
      </div>
      <div className="draft-fields">
        <div className="draft-field">
          <b>收件人</b>
          <span>{data.to}</span>
        </div>
        <div className="draft-field">
          <b>主题</b>
          <span>{data.subject}</span>
        </div>
      </div>
      <div className="draft-body">{data.body}</div>
      <div className="draft-foot">
        <span>不会自动发送</span>
        <span className="spacer" />
        <button
          type="button"
          className="btn sm"
          onClick={() => {
            void copyText(`收件人：${data.to}\n主题：${data.subject}\n\n${data.body}`);
            toast({ title: "已复制草稿", tone: "ok" });
          }}
        >
          <Icon name="copy" />
          复制草稿
        </button>
      </div>
    </div>
  );
}

export function CalloutCard({ data }: { data: CalloutPartData }) {
  const variant = data.tone === "warn" ? "warn" : data.tone === "ok" ? "ok" : "";
  return (
    <div className={cn("blk callout", variant)}>
      <Icon name={variant === "warn" ? "alert" : variant === "ok" ? "check-circle" : "info"} />
      <div className="callout-body">
        <div>{data.text}</div>
      </div>
    </div>
  );
}

export function NoticeCard({ data }: { data: NoticePartData }) {
  return (
    <div className="blk sys-note">
      <Icon name="info" />
      <div className="sys-note-main">
        <span>{data.text}</span>
        {data.summary ? (
          <details className="sys-note-detail">
            <summary>查看摘要</summary>
            <div>{data.summary}</div>
          </details>
        ) : null}
      </div>
    </div>
  );
}

export function AttachmentChips({ attachments }: { attachments: string[] }) {
  if (!attachments?.length) {
    return null;
  }
  return (
    <div className="msg-attachments">
      {attachments.map((path) => (
        <span key={path} className="attach-chip">
          <Icon name="file" />
          {path.split("/").pop() ?? path}
        </span>
      ))}
    </div>
  );
}

export function FileFact({ path, size }: { path: string; size?: number }) {
  return (
    <span className="approval-fact">
      <Icon name="file" />
      {path}
      {size ? ` · ${formatBytes(size)}` : ""}
    </span>
  );
}
