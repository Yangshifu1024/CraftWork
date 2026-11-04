// 草稿槽的本地持久化：整表按 { code, work } 两模式 × 工作区 id 分槽存取 localStorage。
// key 与 theme.ts 的 craftwork-theme 同风格；空串 "" 表示内置临时会话槽。
// 附件只存路径字符串，恢复时不校验存在性（发送时走既有附件加载降级）。
import type { DraftState, Mode } from "../store/app";

const DRAFTS_KEY = "craftwork-drafts";

/** 草稿整表：模式 → 槽 key（工作区 id，"" 为临时槽）→ 槽内容。 */
export type DraftTable = Record<Mode, Record<string, DraftState>>;

/** 空整表。 */
export function emptyDraftTable(): DraftTable {
  return { code: {}, work: {} };
}

/** 读整表；key 缺失或数据损坏时逐项容错，整体不可用回落空表。 */
export function loadDrafts(): DraftTable {
  try {
    const raw = localStorage.getItem(DRAFTS_KEY);
    if (!raw) return emptyDraftTable();
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return emptyDraftTable();
    const source = parsed as Record<string, unknown>;
    const out = emptyDraftTable();
    for (const mode of ["code", "work"] as const) {
      const slots = source[mode];
      if (typeof slots !== "object" || slots === null) continue;
      for (const [wsId, slot] of Object.entries(slots as Record<string, unknown>)) {
        if (typeof slot !== "object" || slot === null) continue;
        const record = slot as Record<string, unknown>;
        const attachments = Array.isArray(record["attachments"]) ? record["attachments"] : [];
        out[mode][wsId] = {
          text: typeof record["text"] === "string" ? record["text"] : "",
          attachments: attachments.filter((a): a is string => typeof a === "string"),
          // 旧版本槽无此字段：缺失/非字符串回落空串（空 = 首发用全局默认模型）
          modelRef: typeof record["modelRef"] === "string" ? record["modelRef"] : "",
        };
      }
    }
    return out;
  } catch {
    return emptyDraftTable();
  }
}

/** 写整表；存储不可用时静默（草稿持久化是尽力而为）。 */
export function saveDrafts(drafts: DraftTable): void {
  try {
    localStorage.setItem(DRAFTS_KEY, JSON.stringify(drafts));
  } catch {
    // 忽略写入失败
  }
}
