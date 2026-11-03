// 行级 diff：LCS 动态规划 + 前后缀修剪 + 上下文窗口合并。
// LCS 动态规划 + 前后缀修剪 + 3 行上下文窗口合并；n*m > 4,000,000 时退化为整段删+增。
import type { DiffLine, DiffResult } from "../../shared/types";

const MAX_LCS_CELLS = 4_000_000;

interface Op {
  kind: "ctx" | "add" | "del";
  text: string;
}

function splitLines(text: string): string[] {
  const normalized = text.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** 计算 diff 操作序列（剪公共前后缀 + LCS）。 */
function diffOps(oldLines: string[], newLines: string[]): Op[] | null {
  // 剪公共前缀
  let start = 0;
  while (start < oldLines.length && start < newLines.length && oldLines[start] === newLines[start]) start++;
  // 剪公共后缀
  let endOld = oldLines.length;
  let endNew = newLines.length;
  while (endOld > start && endNew > start && oldLines[endOld - 1] === newLines[endNew - 1]) {
    endOld--;
    endNew--;
  }
  const ops: Op[] = [];
  for (let i = 0; i < start; i++) ops.push({ kind: "ctx", text: oldLines[i] ?? "" });

  const midOld = oldLines.slice(start, endOld);
  const midNew = newLines.slice(start, endNew);
  const midOps = lcsOps(midOld, midNew);
  if (midOps === null) {
    // 退化为整段删 + 整段增
    for (const line of midOld) ops.push({ kind: "del", text: line });
    for (const line of midNew) ops.push({ kind: "add", text: line });
  } else {
    ops.push(...midOps);
  }

  for (let i = endOld; i < oldLines.length; i++) ops.push({ kind: "ctx", text: oldLines[i] ?? "" });
  return ops;
}

function lcsOps(a: string[], b: string[]): Op[] | null {
  const n = a.length;
  const m = b.length;
  if (n === 0) return b.map((text) => ({ kind: "add" as const, text }));
  if (m === 0) return a.map((text) => ({ kind: "del" as const, text }));
  if (n * m > MAX_LCS_CELLS) return null;

  // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度；相等时优先走删除，保证同输入同输出
  const dp: Uint32Array[] = [];
  for (let i = 0; i <= n; i++) dp.push(new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      const ai = a[i] ?? "";
      const bj = b[j] ?? "";
      dp[i]![j] = ai === bj ? (dp[i + 1]?.[j + 1] ?? 0) + 1 : Math.max(dp[i + 1]?.[j] ?? 0, dp[i]?.[j + 1] ?? 0);
    }
  }

  const ops: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    const ai = a[i] ?? "";
    const bj = b[j] ?? "";
    if (ai === bj) {
      ops.push({ kind: "ctx", text: ai });
      i++;
      j++;
    } else if ((dp[i + 1]?.[j] ?? 0) >= (dp[i]?.[j + 1] ?? 0)) {
      ops.push({ kind: "del", text: ai });
      i++;
    } else {
      ops.push({ kind: "add", text: bj });
      j++;
    }
  }
  while (i < n) {
    ops.push({ kind: "del", text: a[i] ?? "" });
    i++;
  }
  while (j < m) {
    ops.push({ kind: "add", text: b[j] ?? "" });
    j++;
  }
  return ops;
}

/** 由操作序列构建 hunks：每个变更行扩 3 行上下文，重叠窗口合并。 */
function buildHunks(ops: Op[]): { hunks: DiffResult["hunks"]; additions: number; deletions: number } {
  const CONTEXT = 3;
  const hunks: DiffResult["hunks"] = [];
  let additions = 0;
  let deletions = 0;

  // 变更行下标集合
  const changed: number[] = [];
  ops.forEach((op, idx) => {
    if (op.kind !== "ctx") changed.push(idx);
  });
  if (changed.length === 0) return { hunks, additions, deletions };

  // 合并窗口
  const windows: Array<[number, number]> = [];
  for (const idx of changed) {
    const lo = Math.max(0, idx - CONTEXT);
    const hi = Math.min(ops.length - 1, idx + CONTEXT);
    const last = windows[windows.length - 1];
    if (last && lo <= last[1] + 1) last[1] = hi;
    else windows.push([lo, hi]);
  }

  let oldPos = 1;
  let newPos = 1;
  const lineStarts: Array<{ old: number; new: number }> = [];
  for (const op of ops) {
    lineStarts.push({ old: oldPos, new: newPos });
    if (op.kind === "ctx") {
      oldPos++;
      newPos++;
    } else if (op.kind === "del") {
      oldPos++;
    } else {
      newPos++;
    }
  }

  for (const [lo, hi] of windows) {
    const lines: DiffLine[] = [];
    let hunkOldStart = 0;
    let hunkNewStart = 0;
    let hunkOldCount = 0;
    let hunkNewCount = 0;
    for (let idx = lo; idx <= hi; idx++) {
      const op = ops[idx]!;
      const pos = lineStarts[idx]!;
      if (op.kind === "ctx") {
        lines.push({ kind: "ctx", old: pos.old, new: pos.new, text: op.text });
        hunkOldCount++;
        hunkNewCount++;
        if (hunkOldStart === 0) hunkOldStart = pos.old;
        if (hunkNewStart === 0) hunkNewStart = pos.new;
      } else if (op.kind === "del") {
        lines.push({ kind: "del", old: pos.old, new: 0, text: op.text });
        hunkOldCount++;
        deletions++;
        if (hunkOldStart === 0) hunkOldStart = pos.old;
        if (hunkNewStart === 0) hunkNewStart = pos.new;
      } else {
        lines.push({ kind: "add", old: 0, new: pos.new, text: op.text });
        hunkNewCount++;
        additions++;
        if (hunkOldStart === 0) hunkOldStart = pos.old;
        if (hunkNewStart === 0) hunkNewStart = pos.new;
      }
    }
    // 纯新增/纯删除块的缺失侧起始写 0
    const hasOld = lines.some((l) => l.kind !== "add");
    const hasNew = lines.some((l) => l.kind !== "del");
    const oldStart = hasOld ? hunkOldStart : 0;
    const newStart = hasNew ? hunkNewStart : 0;
    hunks.push({
      header: `@@ -${oldStart},${hunkOldCount} +${newStart},${hunkNewCount} @@`,
      lines,
    });
  }
  return { hunks, additions, deletions };
}

const TRUNCATED_NOTE: DiffLine = { kind: "ctx", old: 0, new: 0, text: "…（差异过大已截断，仅显示统计）" };

/** 行级差异入口：任一侧超过 20000 行时返回截断标记。 */
export function diff(oldText: string, newText: string): DiffResult {
  const oldLines = splitLines(oldText);
  const newLines = splitLines(newText);
  if (oldLines.length > 20000 || newLines.length > 20000) {
    const additions = newLines.length;
    const deletions = oldLines.length;
    return { additions, deletions, hunks: [{ header: "@@ 截断 @@", lines: [TRUNCATED_NOTE] }], truncated: true };
  }
  const ops = diffOps(oldLines, newLines)!;
  const { hunks, additions, deletions } = buildHunks(ops);
  return { additions, deletions, hunks, truncated: false };
}
