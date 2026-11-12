// 后台任务轮询行的合并逻辑（`crates/craft-agent/src/shell_tasks.rs` 模块头）：
// 模型等待后台命令时会反复调用 bash_output，每次都是一次独立工具调用（新 partId）。
// 聊天区只保留一条「等待后台任务」行——同一任务号取首次出现的位置、载荷用最新一次查看。
// 纯函数，不碰 React：流式事件与快照重载两条路径共用同一套规则。
import type { ToolCallPart } from "../../../shared/events";
import type { PartSnapshot } from "../../../shared/types";

const BACKGROUND_POLL_TOOL = "bash_output";

function asTool(part: PartSnapshot): ToolCallPart | null {
  if (part.kind !== "tool") return null;
  const payload = part.payload as unknown as ToolCallPart;
  return payload && typeof payload.name === "string" ? payload : null;
}

/** 是否为后台命令任务的输出查看调用（同一任务可能被查看很多次）。 */
export function isBackgroundPollPart(part: PartSnapshot): boolean {
  return asTool(part)?.name === BACKGROUND_POLL_TOOL;
}

/** 轮询行对应的后台任务号：完成事件带 backgroundTaskId，流式中的 running 事件只有 args.task_id。 */
export function backgroundPollTaskId(part: PartSnapshot): string {
  const tool = asTool(part);
  if (!tool || tool.name !== BACKGROUND_POLL_TOOL) return "";
  const fromPayload = typeof tool.backgroundTaskId === "string" ? tool.backgroundTaskId.trim() : "";
  if (fromPayload) return fromPayload;
  const fromArgs = tool.args?.["task_id"];
  return typeof fromArgs === "string" ? fromArgs.trim() : "";
}

/**
 * 合并同一后台任务的多次查看：只保留首次出现的 part（位置与 React key 稳定），
 * 载荷按调用顺序浅合并（最新输出/状态覆盖），并沿用最后已知的 backgroundTaskState——
 * 最新一次调用的 running 事件还没有 meta，不能把已知的运行中/终态状态冲掉。
 * 旧事件没有 backgroundTaskId 时用 args.task_id 兜底，历史记录同样合并。
 */
export function mergeBackgroundPolls(parts: PartSnapshot[]): PartSnapshot[] {
  const firstIndex = new Map<string, number>();
  const latestPayload = new Map<string, Record<string, unknown>>();
  const latestState = new Map<string, NonNullable<ToolCallPart["backgroundTaskState"]>>();
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i]!;
    const taskId = backgroundPollTaskId(part);
    if (!taskId) continue;
    if (!firstIndex.has(taskId)) firstIndex.set(taskId, i);
    const tool = part.payload as unknown as ToolCallPart;
    if (tool.backgroundTaskState) latestState.set(taskId, tool.backgroundTaskState);
    latestPayload.set(taskId, { ...(latestPayload.get(taskId) ?? {}), ...part.payload });
  }
  for (const [taskId, state] of latestState) {
    latestPayload.set(taskId, { ...(latestPayload.get(taskId) ?? {}), backgroundTaskState: state });
  }
  return parts.flatMap((part, index) => {
    const taskId = backgroundPollTaskId(part);
    if (!taskId) return [part];
    if (firstIndex.get(taskId) !== index) return [];
    return [{ ...part, payload: latestPayload.get(taskId) ?? part.payload }];
  });
}
