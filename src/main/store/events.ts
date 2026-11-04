// events 表读写与快照重建。
// 事件表是唯一事实源：消息与部件从事件行重建，不另存副本。
// seq 分配与插入必须同事务（并发安全）；写入后顺带推进 threads.updated_at。
import { newId } from "../../shared/ids";
import type { ApprovalPart, EventKind } from "../../shared/events";
import type { MessageSnapshot, PartKind, PartSnapshot, Snapshot, ThreadView } from "../../shared/types";
import type { DB } from "./db";
import { nowMillis } from "./db";
import { IdPrefix } from "../../shared/ids";
import { getThread } from "./threads";
import { toThreadView } from "./view";

export interface EventRow {
  id: string;
  threadId: string;
  messageId: string;
  runId: string | null;
  seq: number;
  kind: EventKind;
  payload: string;
  createdAt: number;
}

export interface EventRecord {
  threadId: string;
  messageId: string;
  runId?: string;
  kind: EventKind;
  payload: unknown;
}

export function appendEvent(db: DB, rec: EventRecord): { id: string; seq: number } {
  return db.transaction(() => {
    const row = db
      .prepare("SELECT COALESCE(MAX(seq), 0) + 1 AS next FROM events WHERE thread_id = ?")
      .get(rec.threadId) as { next?: number } | undefined;
    const seq = row?.next ?? 1;
    const id = newId(IdPrefix.Part);
    db.prepare(
      "INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    ).run(id, rec.threadId, rec.messageId, rec.runId ?? null, seq, rec.kind, JSON.stringify(rec.payload ?? {}), nowMillis());
    db.prepare("UPDATE threads SET updated_at = ? WHERE id = ?").run(nowMillis(), rec.threadId);
    return { id, seq };
  });
}

export function listEvents(db: DB, threadId: string): EventRow[] {
  const rows = db
    .prepare("SELECT * FROM events WHERE thread_id = ? ORDER BY seq ASC")
    .all(threadId) as Record<string, unknown>[];
  return rows.map((row) => ({
    id: String(row["id"] ?? ""),
    threadId: String(row["thread_id"] ?? ""),
    messageId: String(row["message_id"] ?? ""),
    runId: (row["run_id"] as string | null) ?? null,
    seq: Number(row["seq"] ?? 0),
    kind: String(row["kind"] ?? "") as EventKind,
    payload: String(row["payload"] ?? "{}"),
    createdAt: Number(row["created_at"] ?? 0),
  }));
}

export function latestContextSummary(db: DB, threadId: string): { summary: string; untilSeq: number } | null {
  const rows = db
    .prepare("SELECT payload, seq FROM events WHERE thread_id = ? AND kind = 'context_summary' ORDER BY seq DESC LIMIT 1")
    .all(threadId) as Array<Record<string, unknown>>;
  const row = rows[0];
  if (!row) return null;
  try {
    const payload = JSON.parse(String(row["payload"] ?? "{}")) as { summary?: string; untilSeq?: number };
    if (!payload.summary) return null;
    return { summary: payload.summary, untilSeq: payload.untilSeq ?? Number(row["seq"] ?? 0) };
  } catch {
    return null;
  }
}

// ---------- 快照重建 ----------

const SKIPPED_KINDS = new Set(["run_started", "run_finished", "context_summary"]);

/** DB kind → 快照部件 kind（approval_resolved 不产生部件，回填用）。 */
function snapshotPartKind(kind: EventKind): PartKind | null {
  switch (kind) {
    case "user_message":
      return "text";
    case "text":
    case "reasoning":
    case "approval":
    case "todo":
    case "error":
    case "notice":
    case "draft":
    case "callout":
      return kind;
    case "tool_call":
      return "tool";
    default:
      return null;
  }
}

interface RunInfo {
  id: string;
  messageId: string;
  status: string;
  startedAt: number;
  finishedAt: number | null;
}

function listRuns(db: DB, threadId: string): RunInfo[] {
  const rows = db
    .prepare("SELECT id, message_id, status, started_at, finished_at FROM runs WHERE thread_id = ? ORDER BY started_at ASC")
    .all(threadId) as Record<string, unknown>[];
  return rows.map((row) => ({
    id: String(row["id"] ?? ""),
    messageId: String(row["message_id"] ?? ""),
    status: String(row["status"] ?? ""),
    startedAt: Number(row["started_at"] ?? 0),
    finishedAt: row["finished_at"] === null ? null : Number(row["finished_at"]),
  }));
}

function runMessageStatus(runStatus: string): MessageSnapshot["status"] {
  switch (runStatus) {
    case "running":
      return "streaming";
    case "error":
      return "error";
    case "canceled":
    case "interrupted":
      return "incomplete";
    default:
      return "complete";
  }
}

function pendingApprovals(db: DB, threadId: string): ApprovalPart[] {
  const rows = db
    .prepare(
      "SELECT id, tool_call_id, tool, risk, requested_at FROM approvals WHERE thread_id = ? AND decision IS NULL ORDER BY requested_at ASC",
    )
    .all(threadId) as Record<string, unknown>[];
  return rows.map((row) => ({
    partId: "",
    approvalId: String(row["id"] ?? ""),
    toolCallId: String(row["tool_call_id"] ?? ""),
    tool: String(row["tool"] ?? ""),
    risk: String(row["risk"] ?? ""),
    title: "需要你的批准",
    target: "",
    detail: "",
    facts: [],
    args: {},
    allowPermanent: false,
    status: "waiting" as const,
  }));
}

/**
 * 快照重建：从事件行还原消息列表。
 * beforeSeq > 0 时只保留「最后部件 seq < beforeSeq」的消息（向前翻页）。
 */
export function snapshot(db: DB, threadId: string, limit = 0, beforeSeq = 0): Snapshot {
  const threadRow = getThread(db, threadId);
  const thread: ThreadView = toThreadView(db, threadRow);
  const events = listEvents(db, threadId);
  const runs = listRuns(db, threadId);

  // 按 messageId 归组
  const messages = new Map<string, MessageSnapshot>();
  const approvalByPart = new Map<string, PartSnapshot>();

  const ensureMessage = (messageId: string, role: MessageSnapshot["role"]): MessageSnapshot => {
    let msg = messages.get(messageId);
    if (!msg) {
      msg = { id: messageId, role, parts: [], status: "complete", seq: 0 };
      messages.set(messageId, msg);
    }
    return msg;
  };

  for (const ev of events) {
    if (SKIPPED_KINDS.has(ev.kind)) continue;

    if (ev.kind === "approval_resolved") {
      // 回填对应 approval 部件的状态
      let payload: Record<string, unknown> = {};
      try {
        payload = JSON.parse(ev.payload) as Record<string, unknown>;
      } catch {
        payload = {};
      }
      const partId = String(payload["partId"] ?? "");
      const part = approvalByPart.get(partId);
      if (part) {
        const p = part.payload as unknown as ApprovalPart;
        p.status = (String(payload["status"] ?? "canceled") as ApprovalPart["status"]) ?? "canceled";
        p.decision = String(payload["decision"] ?? "");
        p.scope = String(payload["scope"] ?? "");
      }
      continue;
    }

    const kind = snapshotPartKind(ev.kind);
    if (kind === null) continue;

    let payload: Record<string, unknown>;
    try {
      payload = JSON.parse(ev.payload) as Record<string, unknown>;
    } catch {
      payload = {};
    }
    if (ev.kind === "user_message") {
      // user_message 只保留 text 与 attachments
      payload = { text: payload["text"] ?? "", attachments: payload["attachments"] ?? [] };
    }

    const role: MessageSnapshot["role"] = ev.kind === "user_message" ? "user" : "assistant";
    const msg = ensureMessage(ev.messageId, role);
    const part: PartSnapshot = { id: ev.id, kind, seq: ev.seq, payload };
    msg.parts.push(part);
    if (ev.seq > msg.seq) msg.seq = ev.seq;
    if (kind === "approval") {
      const ap = payload as unknown as ApprovalPart;
      if (ap.partId) approvalByPart.set(ap.partId, part);
    }
  }

  // run 状态覆盖消息状态；没有事件的 run 补一条 assistant 消息
  for (const run of runs) {
    if (run.messageId && messages.has(run.messageId)) {
      const msg = messages.get(run.messageId)!;
      msg.status = runMessageStatus(run.status);
      if (run.finishedAt !== null && run.finishedAt > run.startedAt) {
        msg.workDurationMs = run.finishedAt - run.startedAt;
      }
    } else if (run.messageId && (run.status === "running" || run.status === "error")) {
      // 补消息的 seq 取该 run 名下最早事件的 seq：失败 run 往往没有任何部件，
      // 若落 seq=0 会排到触发它的用户消息（seq 更大）之前，时间线顺序颠倒。
      const runFirstSeq = events
        .filter((e) => e.runId === run.id)
        .reduce((min, e) => (min === 0 || (e.seq > 0 && e.seq < min) ? e.seq : min), 0);
      messages.set(run.messageId, {
        id: run.messageId,
        role: "assistant",
        parts: [],
        status: runMessageStatus(run.status),
        seq: runFirstSeq,
        workDurationMs: run.finishedAt !== null && run.finishedAt > run.startedAt ? run.finishedAt - run.startedAt : undefined,
      });
    }
  }

  let list = [...messages.values()].filter((m) => m.parts.length > 0 || m.status !== "complete");
  list.sort((a, b) => a.seq - b.seq || (a.id < b.id ? -1 : 1));

  let hasMore = false;
  if (beforeSeq > 0) {
    list = list.filter((m) => m.seq > 0 && m.seq < beforeSeq);
  }
  if (limit > 0 && list.length > limit) {
    hasMore = true;
    list = list.slice(list.length - limit);
  }

  const lastSeq = events.length > 0 ? (events[events.length - 1]?.seq ?? 0) : 0;
  const firstWithParts = list.find((m) => m.parts.length > 0);
  const oldestSeq = firstWithParts && firstWithParts.parts.length > 0 ? (firstWithParts.parts[0]?.seq ?? 0) : 0;

  return {
    thread,
    messages: list,
    pendingApprovals: pendingApprovals(db, threadId),
    lastSeq,
    oldestSeq,
    hasMore,
  };
}
