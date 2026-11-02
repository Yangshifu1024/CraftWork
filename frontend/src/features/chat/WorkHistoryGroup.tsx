/**
 * 「已工作 N」折叠容器。
 *
 * 把一轮回复里最终总结之前的过程部件（推理、工具调用、审批、待办）
 * 收进一个可折叠区域，折叠头显示本轮运行的耗时与终止原因：
 * - 正常完成：已工作 1 分 28 秒
 * - 被取消：已工作 1 分 28 秒 · 已停止
 * - 运行失败：已工作 1 分 28 秒 · 出错
 *
 * 耗时来自后端的持久化事实（运行起止时间差），历史会话刷新后不会增长；
 * 运行中不使用本组件（过程保持平铺展开），因此没有「工作中」形态。
 */
import { type ReactNode, useState } from "react";
import { Icon } from "../../components/Icon";
import { cn } from "../../lib/utils";

/**
 * 把毫秒时长格式化为「最多两段非零单位」的中文写法。
 *
 * 规则：秒四舍五入取整、至少 1 秒；天 / 时 / 分 / 秒里只保留非零单位、
 * 最多两段（`1 小时 3 分` 不带秒）；数字与单位之间留空格。
 * 返回空字符串表示时长缺失或无效，调用方据此省略时长段落。
 */
export function formatWorkDuration(durationMs: number | undefined): string {
  if (!durationMs || durationMs <= 0) {
    return "";
  }
  const totalSeconds = Math.max(1, Math.round(durationMs / 1000));
  const days = Math.floor(totalSeconds / 86_400);
  const hours = Math.floor((totalSeconds % 86_400) / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  const parts: string[] = [];
  if (days > 0) parts.push(`${days} 天`);
  if (hours > 0) parts.push(`${hours} 小时`);
  if (minutes > 0) parts.push(`${minutes} 分`);
  if (seconds > 0 || parts.length === 0) parts.push(`${seconds} 秒`);
  return parts.slice(0, 2).join(" ");
}

/** 消息终态到折叠头后缀的映射；正常完成没有后缀。 */
function endTag(status: "complete" | "incomplete" | "error"): string | null {
  if (status === "incomplete") {
    return "已停止";
  }
  if (status === "error") {
    return "出错";
  }
  return null;
}

/**
 * 工作过程折叠容器。
 *
 * @param durationMs 本轮运行耗时（毫秒），来自后端运行记录；缺失时只显示「已工作」。
 * @param endStatus 消息终态，决定折叠头的终止后缀。
 * @param children 折叠区内渲染的过程部件子树。
 *
 * 关键状态：`open` 控制展开，默认收起；展开状态由本组件自持，
 * 消息数据更新（例如历史重新加载）不影响用户当前的展开选择。
 */
export function WorkHistoryGroup(options: {
  durationMs?: number;
  endStatus: "complete" | "incomplete" | "error";
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const duration = formatWorkDuration(options.durationMs);
  const tag = endTag(options.endStatus);
  return (
    <div className={cn("work-history", open && "is-open")}>
      <button type="button" className="work-history-head" onClick={() => setOpen((current) => !current)}>
        <span className="work-history-label">
          已工作{duration ? ` ${duration}` : ""}
          {tag ? (
            <>
              <span className="r-sep">·</span>
              <span className="work-history-tag">{tag}</span>
            </>
          ) : null}
        </span>
        <Icon name="chevron-right" className="chev" />
      </button>
      <div className="work-history-body">{options.children}</div>
    </div>
  );
}
