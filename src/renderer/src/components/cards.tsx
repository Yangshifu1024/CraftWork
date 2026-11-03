// 任务清单卡片（todo 部件）。
import { ListTodo } from "lucide-react";
import type { TodoItem } from "../../../shared/events";

export function TodoCard({ items }: { items: TodoItem[] }): React.JSX.Element {
  const done = items.filter((i) => i.status === "done").length;
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
        {items.map((item, i) => (
          <li key={i} className="flex items-center gap-2 py-0.5 text-ui-sm">
            <span className={`shrink-0 ${item.status === "done" ? "text-success" : item.status === "active" ? "text-icon-blue" : "text-foreground-subtlest"}`}>
              {mark[item.status]}
            </span>
            <span className={item.status === "done" ? "text-foreground-subtlest line-through" : "text-foreground"}>
              {item.text}
            </span>
            {item.status === "active" && <span className="ml-auto shrink-0 text-ui-xs text-icon-blue">进行中</span>}
          </li>
        ))}
      </ul>
    </div>
  );
}
