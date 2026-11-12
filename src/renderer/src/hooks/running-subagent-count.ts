// 当前会话的运行中子代理数量（跨会话索引按 parentThreadId 过滤）。
// 消费方：会话头的「展开侧栏」开关徽标（面板关时不错过并行进度）、面板 tab 条的
// 「子代理」tab 徽标——两处共用一份计数，随时回答「正在跑几个」。
import { useApp } from "../store/app";

export function useRunningSubagentCount(): number {
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  return useApp((s) =>
    Object.values(s.subagentIndex).filter((v) => v.parentThreadId === activeThreadId && v.status === "running").length,
  );
}
