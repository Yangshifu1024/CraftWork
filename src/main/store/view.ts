// 视图投影：ThreadRow → ThreadView（含最近一次用量推导的上下文占用）。
import type { ThreadView } from "../../shared/types";
import type { DB } from "./db";
import type { ThreadRow } from "./threads";
import { normalizeModelRef } from "../model/catalog";

export interface LatestUsage {
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  durationMs: number;
}

/** 最近一次用量（上下文占用按会话取最新一条 usage 行）。 */
export function latestUsageByThread(db: DB, threadId: string): LatestUsage | null {
  const row = db
    .prepare(
      "SELECT input_tokens, output_tokens, cache_tokens, duration_ms FROM usage WHERE thread_id = ? ORDER BY ts DESC LIMIT 1",
    )
    .get(threadId) as Record<string, unknown> | undefined;
  if (!row) return null;
  return {
    inputTokens: Number(row["input_tokens"] ?? 0),
    outputTokens: Number(row["output_tokens"] ?? 0),
    cacheTokens: Number(row["cache_tokens"] ?? 0),
    durationMs: Number(row["duration_ms"] ?? 0),
  };
}

/** 不可用模型显示名。 */
export function modelNameFor(db: DB, modelId: string): string {
  if (!modelId) return "未选择模型";
  const resolved = normalizeModelRef(db, modelId);
  return resolved ? resolved.modelName : "模型不可用";
}

export function toThreadView(db: DB, row: ThreadRow): ThreadView {
  // ContextTokens 是「覆盖口径」：最近一次用量的 input+output，与累计 Tokens 不同
  const usage = latestUsageByThread(db, row.id);
  const contextTokens = usage ? usage.inputTokens + usage.outputTokens : 0;
  const speed = usage && usage.durationMs > 0 ? usage.outputTokens / (usage.durationMs / 1000) : 0;
  return {
    id: row.id,
    mode: row.mode,
    title: row.title,
    modelId: row.modelId,
    modelName: modelNameFor(db, row.modelId),
    workspaceId: row.workspaceId,
    status: row.status,
    archived: row.archived,
    tokens: row.tokens,
    contextTokens,
    contextCacheTokens: usage?.cacheTokens ?? 0,
    contextSpeed: speed,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  };
}
