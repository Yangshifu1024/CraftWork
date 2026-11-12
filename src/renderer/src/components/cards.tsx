// 任务清单卡片（todo 部件）：对话流卡片体系的一员，由消息流的 WorkPart 分发渲染。
// 只读展示：顶部完成进度条 + 分状态符号的条目列表，不承载任何操作。
// 子代理卡片（subagent 部件，决策 D15）同样只读：状态 + 耗时 + 结论折叠 + 「在右侧栏查看」入口。
import { useState } from "react";
import { Bot, ChevronRight, ListTodo } from "lucide-react";
import type { TodoItem } from "../../../shared/events";
import type { SubagentPart } from "../../../shared/types";

/** 任务清单卡片：完成进度条 + 条目列表（待办 ○ / 进行中 ◐ / 完成 ●，完成项划线弱化）。 */
export function TodoCard({ items }: { items: TodoItem[] }): React.JSX.Element {
  // 历史事件里模型可能输出别名 completed（新事件后端已归一化），渲染前统一映射
  const statusOf = (s: TodoItem["status"]) => (s === "completed" ? "done" : s);
  const done = items.filter((i) => statusOf(i.status) === "done").length;
  const mark = { pending: "○", active: "◐", done: "●" } as const;
  return (
    <div className="w-full overflow-hidden rounded-lg border border-border/60 bg-popover">
      <div className="flex items-center gap-2 px-3 py-2">
        <ListTodo className="size-3.5 shrink-0 text-foreground-subtlest" />
        <span className="text-ui-sm font-medium text-foreground">任务清单</span>
        <span className="font-mono text-ui-xs text-foreground-subtlest">
          {done} / {items.length}
        </span>
        <div className="h-[3px] w-16 overflow-hidden rounded-full bg-surface">
          <div
            className="h-full rounded-full bg-success transition-all"
            style={{ width: items.length > 0 ? `${(done / items.length) * 100}%` : "0%" }}
          />
        </div>
      </div>
      <ul className="border-t border-border/40 px-3 py-1.5">
        {items.map((item, i) => {
          const status = statusOf(item.status);
          return (
            <li key={i} className="flex items-center gap-2 py-0.5 text-ui-sm">
              <span className={`shrink-0 ${status === "done" ? "text-success" : status === "active" ? "text-icon-blue" : "text-foreground-subtlest"}`}>
                {mark[status]}
              </span>
              <span className={status === "done" ? "text-foreground-subtlest line-through" : "text-foreground"}>
                {item.text}
              </span>
              {status === "active" && <span className="ml-auto shrink-0 text-ui-xs text-icon-blue">进行中</span>}
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** 子代理状态 → 中文标签（运行中/完成/失败/已中断/已停止）。卡片与右侧面板的
 *  子代理列表 / 详情 tab 共用（四种终态各有准确文字）。 */
export const SUBAGENT_STATUS_LABEL: Record<SubagentPart["status"], string> = {
  running: "运行中",
  completed: "已完成",
  failed: "失败",
  interrupted: "已中断",
  closed: "已停止",
};

/** 子代理卡片（决策 D15）：父会话里的一张「派了谁、在做什么、结果如何」。
 *
 *  与工具卡的区别：工具卡描述一次调用的输入输出，子代理卡描述的是一整段独立的执行过程。
 *  因此这里给类型徽章、耗时与工具次数，并把「查看对话」作为进去看细节的入口（子会话只读）。
 *  运行态沿用全站统一的文字扫光，不用旋转图标。 */
export function SubagentCard({
  part,
  onOpen,
}: {
  part: SubagentPart;
  onOpen?: (agentId: string) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const running = part.status === "running";
  const failed = part.status === "failed" || part.status === "interrupted";
  const hasResult = part.result.trim().length > 0;
  return (
    <div className="w-full overflow-hidden rounded-lg border border-border/60 bg-popover">
      <div className="flex items-center gap-2 px-3 py-2">
        <Bot className="size-3.5 shrink-0 text-foreground-subtlest" />
        <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle">
          {part.agentTypeLabel || part.agentType}
        </span>
        <span className={`shrink-0 font-mono text-ui-sm ${running ? "animated-gradient-text" : "text-foreground"}`}>
          {part.name}
        </span>
        <span className="min-w-0 flex-1 truncate text-ui-sm text-foreground-subtlest">{part.description}</span>
        {part.durationMs ? (
          <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">
            {(part.durationMs / 1000).toFixed(1)}s
          </span>
        ) : null}
        {part.toolUses > 0 ? (
          <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">工具 {part.toolUses}</span>
        ) : null}
        <span className={`shrink-0 text-ui-xs ${failed ? "text-destructive" : "text-foreground-subtlest"}`}>
          {SUBAGENT_STATUS_LABEL[part.status]}
        </span>
        {onOpen ? (
          <button
            className="shrink-0 rounded-md px-1.5 py-0.5 text-ui-xs text-foreground-subtle hover:bg-surface-hover"
            onClick={() => onOpen(part.agentId)}
            title="在右侧面板打开这个子代理的详情（只读，不打断主对话）"
          >
            在右侧栏查看
          </button>
        ) : null}
        {hasResult ? (
          <button
            className="shrink-0 rounded-md p-1 text-foreground-subtlest hover:bg-surface-hover"
            onClick={() => setOpen(!open)}
            title={open ? "收起结论" : "展开结论"}
          >
            <ChevronRight className={`size-3.5 transition-transform ${open ? "rotate-90" : ""}`} />
          </button>
        ) : null}
      </div>
      {open && hasResult ? (
        <div className="border-t border-border/40 px-3 py-2">
          <pre className="select-text max-h-60 overflow-auto whitespace-pre-wrap break-words font-mono text-ui-sm text-foreground-subtle">
            {part.result}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
