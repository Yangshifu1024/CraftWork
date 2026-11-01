/**
 * 聊天消息中的各类结构化卡片部件。
 *
 * 这些组件由 MessageView 的部件注册表按 `data-cw-*` 名称挂载：
 * 工具调用、审批请求、任务清单、错误、草稿、提示、系统通知与附件列表。
 *
 * 共同约定：
 * - 每个组件只接收一个 `data` prop，内容来自后端消息部件，不在本地保存业务状态。
 * - 纯展示类卡片不发起请求；需要写操作的卡片（审批决定、重试）通过 store 触发。
 * - 所有用户可见文案使用中文，风险等级、状态与档位都直接展示原始值。
 */
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

/** 审批档位到中文标签的映射，用于卡片上的档位徽标。 */
const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
  free: "自由",
};

/**
 * 后端工具名到图标的映射。
 * 未登记的工具统一用闪电图标，保证新增工具也能正常显示。
 */
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

/**
 * 把工具参数值转成可读字符串。
 * 字符串原样显示；对象与数组尝试 JSON 序列化；循环引用等异常时退回 String。
 */
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

/**
 * 工具执行状态的展示配置。
 * `spin` 表示图标需要旋转，只有运行中需要。
 */
const statusMap: Record<string, { cls: string; label: string; icon: IconName; spin?: boolean }> = {
  running: { cls: "running", label: "运行中", icon: "loader", spin: true },
  done: { cls: "done", label: "完成", icon: "check" },
  error: { cls: "error", label: "失败", icon: "x" },
};

/**
 * 工具调用卡片。
 *
 * @param data 工具部件数据：名称、摘要、风险等级、参数、输出、耗时与状态。
 *
 * 关键状态：`open` 控制详情展开。
 * 默认展开仅用于失败的工具，方便直接看到错误输出；成功的工具默认收起。
 * 参数最多展示前 6 项，避免长参数列表撑高消息。
 */
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

/**
 * 推理内容块。
 *
 * @param text 模型输出的思考文本，可能为空。
 *
 * 关键状态：`open` 控制展开，默认收起，避免推理内容挤占正文空间。
 */
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

/**
 * 任务清单卡片。
 *
 * @param data 清单项数组，每项包含文本与状态（待办 / 进行中 / 已完成）。
 *
 * 数量与进度条由已完成项数实时计算；没有任何项时进度为 0。
 */
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

/**
 * 生成审批卡片的差异预览行：旧文本前 6 行标为删除，新文本前 6 行标为新增。
 * 这里只是摘要，完整差异由“查看完整差异”按钮打开差异面板查看。
 */
function previewLines(oldText: string, newText: string) {
  const oldLines = oldText.split("\n").slice(0, 6);
  const newLines = newText.split("\n").slice(0, 6);
  return [
    ...oldLines.map((text) => ({ kind: "del" as const, text })),
    ...newLines.map((text) => ({ kind: "add" as const, text })),
  ];
}

/**
 * 审批卡片：展示一次工具调用的审批请求，并收集用户的决定。
 *
 * @param data 审批部件数据，包含审批 ID、工具名、风险等级、目标路径、
 *   参数、事实列表、当前状态、已授权范围与是否允许永久授权。
 *
 * 关键状态（全部由 `data.status` 驱动，组件本身不保存决定结果）：
 * - `waiting`：显示按钮组，等待用户决定。
 * - `approved`：显示已允许及授权范围，并标注已写入审计。
 * - `denied` / `canceled`：显示已拒绝或已取消，提示未执行任何修改。
 *
 * 永久允许的显示条件（与服务端校验一致）：
 * `allowPermanent` 为真、风险等级为 R1/R2、且当前是“分级”档位。
 * 受限工作区、R3/R4 或严格档位下不会出现“本会话允许 / 本工作区始终允许”。
 *
 * 预览规则：
 * - `fs_edit` / `doc_edit`：展示旧文本与新文本的文本级差异预览。
 * - `shell_exec`：展示将要执行的命令与目标。
 * - `apply_patch`：展示补丁内容前 60 行。
 *
 * 交互细节：拒绝按钮默认聚焦，按 Esc 或直接回车都按拒绝处理；
 * 所有决定都通过 `decide` 走跨 IPC 命令，由 Go 侧校验范围后落审计。
 */
export function ApprovalCard({ data }: { data: ApprovalPartData }) {
  const decide = useThreadStore((state) => state.decide);
  const level = useAppStore((state) => state.settings.approvalLevel);
  const openDiff = useWorkspaceStore((state) => state.openDiff);
  const waiting = data.status === "waiting";
  const approved = data.status === "approved";
  const denied = data.status === "denied" || data.status === "canceled";
  // 三个条件同时满足才显示永久授权按钮：后端允许、风险不超过 R2、档位为分级。
  const permanentAllowed =
    Boolean(data.allowPermanent) &&
    (data.risk === "R1" || data.risk === "R2") &&
    level === "graded";

  // 不同工具用不同的参数名承载旧/新文本，这里统一取出用于差异预览。
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

  // 档位说明跟随设置变化，告诉用户当前档位会如何影响这次决定。
  // 自由档位下不会再产生新的审批卡，这里只可能出现在看历史记录时。
  const note =
    level === "strict"
      ? "严格档位：所有写入与执行逐次询问，不提供永久允许"
      : level === "readonly"
        ? "只读档位：写入与执行会被阻止"
        : level === "free"
          ? "自由档位：所有操作自动放行，不再弹出审批"
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
            <span
              key={`${fact.label}-${fact.value}`}
              className="approval-fact"
              title={`${fact.label}：${fact.value}`}
            >
              <Icon name="info" />
              <span className="approval-fact-text">
                {fact.label}：{fact.value}
              </span>
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

      {/* 等待决定时显示按钮组；“拒绝”自动聚焦，回车与 Esc 都会走拒绝分支。 */}
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
            {/* 永久授权按钮只在允许条件下出现，范围为 session 或 workspace。 */}
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

      {/* 决定完成后用结果条替换按钮，保留授权范围与审计提示。 */}
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

/**
 * 模型错误码到处理建议的映射。
 * 后端统一返回这些错误码，前端根据码给出可操作的中文提示；
 * 未登记的错误码只显示原始信息，不猜测原因。
 */
const errorHints: Record<string, string> = {
  auth: "密钥无效或权限不足：请在设置 → 模型 → 提供商中重新保存 API Key，并确认该 Key 允许调用模型。",
  rate_limit: "请求被限流或额度不足：请稍后重试，或在设置中切换其他模型。",
  timeout: "模型响应超时：可以重试；长对话可先精简历史或新建会话。",
  context_overflow: "输入超出模型上下文窗口：请新建会话、精简历史，或切换到上下文更大的模型。",
  provider_unavailable: "模型服务暂时不可用：请稍后重试，或切换到其他提供商。",
  invalid_request: "请求参数不被模型接受：请检查模型名称、端点与工具配置。",
  interrupted: "上次运行在应用退出时中断，已保留已产生的消息；可以点击重试继续。",
  canceled: "本次生成已取消。",
};

/**
 * 错误卡片。
 *
 * @param data 错误部件数据：错误码与可读信息。
 *
 * 展示错误码、原始信息，并在错误码有对应建议时补充处理办法。
 * 提供“重试”（触发重新生成）与“复制错误”两个操作。
 */
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

/**
 * 邮件草稿卡片。
 *
 * @param data 草稿数据：收件人、主题与正文。
 *
 * 草稿只在应用内展示，不会自动发送；用户只能复制内容后自行处理。
 */
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

/**
 * 提示条卡片。
 *
 * @param data 文本与语气：`warn` 警告、`ok` 成功，其余按普通信息处理。
 *
 * 语气只影响图标与配色，不改变文案。
 */
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

/**
 * 系统通知卡片。
 *
 * @param data 主文本与可选摘要；摘要折叠在“查看摘要”里，默认不打扰阅读。
 *
 * 用于展示上下文压缩、模型切换等由系统产生的说明。
 */
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

/**
 * 用户消息的附件列表。
 *
 * @param attachments 附件路径数组；为空时组件不渲染任何内容。
 *
 * 只显示文件名（取路径最后一段），完整路径留在 title 里由上层按需展示。
 */
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

/**
 * 审批卡片里的一条“文件事实”，显示路径与可选大小。
 *
 * @param path 文件路径。
 * @param size 字节数；缺省时只显示路径。
 */
export function FileFact({ path, size }: { path: string; size?: number }) {
  return (
    <span className="approval-fact">
      <Icon name="file" />
      {path}
      {size ? ` · ${formatBytes(size)}` : ""}
    </span>
  );
}
