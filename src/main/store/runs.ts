// runs + approvals 表读写。
import { newId } from "../../shared/ids";
import type { DB } from "./db";
import { ErrNotFound, nowMillis } from "./db";
import { IdPrefix } from "../../shared/ids";

export interface RunRow {
  id: string;
  threadId: string;
  messageId: string;
  status: string;
  modelId: string;
  startedAt: number;
  finishedAt: number | null;
  errorCode: string | null;
  errorMessage: string | null;
}

export function createRun(db: DB, threadId: string, modelId: string): RunRow {
  const id = newId(IdPrefix.Run);
  const messageId = newId(IdPrefix.Message);
  const now = nowMillis();
  db.prepare(
    "INSERT INTO runs (id, thread_id, message_id, status, model_id, started_at) VALUES (?, ?, ?, 'running', ?, ?)",
  ).run(id, threadId, messageId, modelId, now);
  return { id, threadId, messageId, status: "running", modelId, startedAt: now, finishedAt: null, errorCode: null, errorMessage: null };
}

export function getRun(db: DB, id: string): RunRow {
  const row = db.prepare("SELECT * FROM runs WHERE id = ?").get(id) as Record<string, unknown> | undefined;
  if (!row) throw new ErrNotFound("运行不存在");
  return toRun(row);
}

function toRun(row: Record<string, unknown>): RunRow {
  return {
    id: String(row["id"] ?? ""),
    threadId: String(row["thread_id"] ?? ""),
    messageId: String(row["message_id"] ?? ""),
    status: String(row["status"] ?? ""),
    modelId: String(row["model_id"] ?? ""),
    startedAt: Number(row["started_at"] ?? 0),
    finishedAt: row["finished_at"] === null || row["finished_at"] === undefined ? null : Number(row["finished_at"]),
    errorCode: (row["error_code"] as string | null) ?? null,
    errorMessage: (row["error_message"] as string | null) ?? null,
  };
}

export function finishRun(db: DB, id: string, status: string, code: string, message: string): void {
  db.prepare("UPDATE runs SET status = ?, finished_at = ?, error_code = ?, error_message = ? WHERE id = ?").run(
    status,
    nowMillis(),
    code || null,
    message || null,
    id,
  );
}

/**
 * 启动收尾：把上次退出遗留的 running 运行标记为 interrupted、挂起审批标记 canceled、
 * 会话状态复位，并为每个中断 run 写一条可重试的 error 事件。返回中断数量。
 */
export function interruptStaleRuns(
  db: DB,
  appendEventFn: (rec: { threadId: string; messageId: string; runId?: string; kind: "error"; payload: unknown }) => void,
): number {
  const stale = db.prepare("SELECT id, thread_id, message_id FROM runs WHERE status = 'running'").all() as Record<
    string,
    unknown
  >[];
  if (stale.length === 0) return 0;
  const message = "应用退出导致运行中断，已保留已产生的消息，可以重试。";
  db.transaction(() => {
    for (const row of stale) {
      const runId = String(row["id"] ?? "");
      const threadId = String(row["thread_id"] ?? "");
      db.prepare("UPDATE runs SET status = 'interrupted', finished_at = ?, error_code = 'interrupted', error_message = ? WHERE id = ?").run(
        nowMillis(),
        message,
        runId,
      );
      db.prepare("UPDATE approvals SET decision = 'canceled', decided_at = ? WHERE thread_id = ? AND decision IS NULL").run(
        nowMillis(),
        threadId,
      );
      db.prepare("UPDATE threads SET status = 'error', updated_at = ? WHERE id = ? AND status IN ('running','awaiting')").run(
        nowMillis(),
        threadId,
      );
    }
  });
  // error 事件在事务外逐条写入（appendEvent 自带事务）
  for (const row of stale) {
    try {
      appendEventFn({
        threadId: String(row["thread_id"] ?? ""),
        messageId: String(row["message_id"] ?? ""),
        runId: String(row["id"] ?? ""),
        kind: "error",
        payload: {
          partId: newId(IdPrefix.Part),
          code: "interrupted",
          message,
          retryable: true,
        },
      });
    } catch {
      // 会话可能已被删除
    }
  }
  return stale.length;
}

// ---------- 审批 ----------

export interface ApprovalRow {
  id: string;
  threadId: string;
  runId: string;
  toolCallId: string;
  tool: string;
  risk: string;
  decision: string | null;
  scope: string | null;
  requestedAt: number;
}

export function createApproval(db: DB, input: { threadId: string; runId: string; toolCallId: string; tool: string; risk: string }): ApprovalRow {
  const id = newId(IdPrefix.Approval);
  db.prepare(
    "INSERT INTO approvals (id, thread_id, run_id, tool_call_id, tool, risk, requested_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
  ).run(id, input.threadId, input.runId, input.toolCallId, input.tool, input.risk, nowMillis());
  return { ...input, id, decision: null, scope: null, requestedAt: nowMillis() };
}

/** 并发保护：WHERE decision IS NULL——重复决定 RowsAffected=0 → ErrNotFound。 */
export function resolveApproval(db: DB, id: string, decision: string, scope: string): void {
  const res = db
    .prepare("UPDATE approvals SET decision = ?, scope = ?, decided_at = ? WHERE id = ? AND decision IS NULL")
    .run(decision, scope || null, nowMillis(), id);
  if (res.changes === 0) throw new ErrNotFound("审批不存在或已处理");
}

export function getApproval(db: DB, id: string): ApprovalRow {
  const row = db.prepare("SELECT * FROM approvals WHERE id = ?").get(id) as Record<string, unknown> | undefined;
  if (!row) throw new ErrNotFound("审批不存在");
  return {
    id: String(row["id"] ?? ""),
    threadId: String(row["thread_id"] ?? ""),
    runId: String(row["run_id"] ?? ""),
    toolCallId: String(row["tool_call_id"] ?? ""),
    tool: String(row["tool"] ?? ""),
    risk: String(row["risk"] ?? ""),
    decision: (row["decision"] as string | null) ?? null,
    scope: (row["scope"] as string | null) ?? null,
    requestedAt: Number(row["requested_at"] ?? 0),
  };
}

export function cancelThreadApprovals(db: DB, threadId: string): void {
  db.prepare("UPDATE approvals SET decision = 'canceled', decided_at = ? WHERE thread_id = ? AND decision IS NULL").run(
    nowMillis(),
    threadId,
  );
}
