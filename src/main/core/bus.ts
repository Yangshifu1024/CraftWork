// 内部事件总线：按会话分配内存内单调 seq（与 DB 的 seq 是两套，互不相干）；
// 推送侧做 30ms 微批合并，界面层按帧渲染即可。
import type { Batch, Envelope, EventType } from "../../shared/events";
import { newId } from "../../shared/ids";

const FLUSH_MS = 30;

export class Bus {
  private seqs = new Map<string, number>();
  private pending: Envelope[] = [];
  private timer: NodeJS.Timeout | null = null;
  private closed = false;

  constructor(private readonly push: (batch: Batch) => void) {}

  /** 广播一个事件（立即进入合并窗口）。 */
  emit(type: EventType, threadId: string | undefined, runId: string | undefined, data: unknown): void {
    if (this.closed) return;
    const seq = threadId ? (this.seqs.get(threadId) ?? 0) + 1 : 0;
    if (threadId) this.seqs.set(threadId, seq);
    this.pending.push({ seq, type, threadId, runId, ts: Date.now(), data });
    this.schedule();
  }

  private schedule(): void {
    if (this.timer !== null || this.closed) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      if (this.pending.length === 0) return;
      const events = this.pending;
      this.pending = [];
      this.push({ v: 1, threadId: events[0]?.threadId, events });
    }, FLUSH_MS);
  }

  close(): void {
    this.closed = true;
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    // 关闭前冲刷剩余事件（尽力而为）
    if (this.pending.length > 0) {
      const events = this.pending;
      this.pending = [];
      this.push({ v: 1, threadId: events[0]?.threadId, events });
    }
  }
}

/** 部件落库后广播哪个完成事件（原 record.partEvents，契约只存在一份）。 */
export const PART_EVENTS: Record<string, EventType> = {
  text: "text.completed",
  reasoning: "reasoning.completed",
  tool_call: "tool.completed",
  approval: "approval.requested",
  todo: "todo.updated",
  error: "error",
  notice: "notice",
};

export { newId };
