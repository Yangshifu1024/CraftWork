// Recorder：事件写入与广播的唯一出口。
// 「写进历史」与「推给界面」的映射只存在这一份，不会漂移。
import { PART_EVENTS, type Bus } from "./bus";
import { appendEvent, type EventRecord } from "../store/events";
import { setThreadStatus as dbSetThreadStatus } from "../store/threads";
import type { DB } from "../store/db";
import { EventType } from "../../shared/events";

export class Recorder {
  constructor(
    private readonly db: DB,
    private readonly bus: Bus,
  ) {}

  /** 落库 + 广播；无 PART_EVENTS 映射的 kind 只入库不广播。 */
  append(rec: EventRecord): { id: string; seq: number } {
    const result = appendEvent(this.db, rec);
    const broadcastType = PART_EVENTS[rec.kind];
    if (broadcastType) {
      this.bus.emit(broadcastType, rec.threadId, rec.runId, rec.payload);
    }
    return result;
  }

  /** 运行中部件落库：失败只降级为纯广播，不中断运行。 */
  appendOrWarn(rec: EventRecord): void {
    try {
      this.append(rec);
    } catch {
      this.bus.emit(EventType.Notice, rec.threadId, rec.runId, rec.payload);
    }
  }

  /** 只广播（增量/生命周期事件，不入库）。 */
  emit(type: EventType, threadId: string | undefined, runId: string | undefined, payload: unknown): void {
    this.bus.emit(type, threadId, runId, payload);
  }

  /** 会话状态变更：先 DB 后广播 thread.updated。 */
  setThreadStatus(threadId: string, runId: string | undefined, status: string): void {
    dbSetThreadStatus(this.db, threadId, status);
    this.bus.emit(EventType.ThreadUpdated, threadId, runId, { threadId, status });
  }
}
