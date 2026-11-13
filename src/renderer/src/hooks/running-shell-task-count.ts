// 当前会话仍在运行的后台任务（右栏「后台任务」tab 的徽标计数与列表数据源）。
//
// 与 `running-subagent-count.ts` 的差别：子代理有跨会话索引（`subagentIndex`），计数可以
// 直接读 store、零 IPC；后台任务只有进程内注册表（`craft-tools::shell_tasks`），既没有
// 索引也没有终态推送事件（`EventKind` 里没有 shell 任务类型），因此只能轮询。
//
// 轮询边界（仿 `quota-chip.tsx` 的 generation 守卫）：
// - 面板关闭立即停（`enabled=false` 清空并清 timer），不常驻占 IPC；
// - 切会话时 effect 重建，迟到的旧响应由 cancelled 标记丢弃，避免把上一个会话的任务画到当前会话。
//
// 已知边界：任务结束后下一次轮询即从列表消失（方案 A：只列 Running）。这与「结束即消失」
// 的取舍一致——用户主动点「终止」后行也一并消失，不在界面上留退出码。要回看某个任务的
// 结果与输出，走聊天里的 `bash_output` 工具卡（那里有完整的退出码与输出尾部）。
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { ShellTaskView } from "../../../shared/types";
import { useApp } from "../store/app";

/** 轮询间隔：2s。后台任务是低频操作，且只在面板打开时轮询。 */
const POLL_MS = 2000;

/** 运行中后台任务清单；`enabled=false` 时立即停轮询并清空（面板关闭的语义）。
 *
 *  `enabled` 由调用方传面板开合状态，**不是**「是否停在该 tab」——徽标的意义正是
 *  「用户不在这个 tab 时也知道有几个在跑」，按 tab 门控会让徽标在切走后停更。 */
export function useShellTasks(enabled: boolean): ShellTaskView[] {
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const [tasks, setTasks] = useState<ShellTaskView[]>([]);

  useEffect(() => {
    // 面板关闭 / tab 切走 / 无活动会话：清空并停止，不保留上一个会话的陈旧列表
    if (!enabled || !activeThreadId) {
      setTasks([]);
      return;
    }
    let cancelled = false;
    let timer: number | null = null;
    const threadId = activeThreadId;

    const load = async (): Promise<void> => {
      try {
        const list = await api.chatShellTasks(threadId);
        if (cancelled) return;
        setTasks(list);
      } catch {
        // 后台任务查询失败不该打断界面（会话可能刚被删除）：保持上一次列表，等下一轮
        if (!cancelled) setTasks([]);
      }
    };

    void load();
    timer = window.setInterval(() => void load(), POLL_MS);
    return () => {
      cancelled = true;
      if (timer !== null) window.clearInterval(timer);
    };
  }, [enabled, activeThreadId]);

  return tasks;
}
