// 会话自动命名：首条消息可短名直用时直接用；否则调模型起标题（sidecar 调用计用量）。
import { singleSystem, type Attachment, type Client } from "../model/types";
import { autoTitleThread } from "../store/threads";
import { addUsage } from "../store/records";
import { addUsage as recordUsageRow } from "../store/records";
import type { DB } from "../store/db";

const TITLE_TIMEOUT_MS = 8000;
const TITLE_MAX_TOKENS = 128;
const TITLE_LIMIT_RUNES = 20;

const TITLE_SYSTEM_PROMPT =
  "为下面的对话生成一个简短的中文标题。只输出标题本身，不要引号、句号或任何解释，不超过 20 个字。";

/** 取首行、合并空白、剥前缀装饰符号与反引号、去两侧标点引号。 */
export function normalizeTitle(text: string): string {
  let s = text.split("\n")[0] ?? "";
  s = s.replace(/\s+/g, " ").trim();
  s = s.replace(/^[#>\-*_`]+/, "").trim();
  // 去两侧中英文标点引号
  s = s.replace(/^[\s\"'“”‘’「」『』《》【】()（）。，,、！!？?：:；;.\-—]+/, "");
  s = s.replace(/[\s\"'“”‘’「」『』《》【】()（）。，,、！!？?：:；;.\-—]+$/, "");
  return s.trim();
}

export function shortTitle(text: string): string {
  const normalized = normalizeTitle(text);
  if (!hasContent(normalized)) return "";
  return [...normalized].length <= TITLE_LIMIT_RUNES ? normalized : "";
}

/** 模型标题清洗：剥「标题：/Title:」前缀；须含字母或数字；超长截断加省略号。 */
export function sanitizeModelTitle(raw: string): string {
  let s = normalizeTitle(raw);
  s = s.replace(/^(标题|Title|title)\s*[:：]\s*/, "").trim();
  if (!hasContent(s)) return "";
  const runes = [...s];
  if (runes.length <= TITLE_LIMIT_RUNES) return s;
  let cut = runes.slice(0, TITLE_LIMIT_RUNES).join("");
  cut = cut.replace(/[\s"'“”‘’。，,、！!？?：:；;.\-—]+$/, "");
  return `${cut}…`;
}

function hasContent(s: string): boolean {
  return /[a-zA-Z0-9\u4e00-\u9fff]/.test(s);
}

/** 无模型兜底：首行有效文本；纯附件用文件名。 */
export function deriveTitle(text: string, attachments: string[]): string {
  const short = shortTitle(text);
  if (short) return short;
  if (attachments.length > 0) {
    const name = (attachments[0] ?? "").split("/").pop() ?? "";
    if (name) return [...name].slice(0, TITLE_LIMIT_RUNES).join("");
  }
  const normalized = normalizeTitle(text);
  if (!hasContent(normalized)) return "";
  const runes = [...normalized];
  if (runes.length <= TITLE_LIMIT_RUNES) return normalized;
  return `${runes.slice(0, TITLE_LIMIT_RUNES).join("").replace(/[\s，,。；;：:]+$/, "")}…`;
}

export interface AutoTitleInput {
  db: DB;
  client: Client;
  modelId: string;
  threadId: string;
  firstText: string;
  firstAttachments: Attachment[];
}

/** 生成并写入自动标题（并发防重复由调用方 pending 表保证；条件更新防覆盖手动改名）。 */
export async function autoTitle(input: AutoTitleInput): Promise<void> {
  const { db, client, modelId, threadId, firstText, firstAttachments } = input;
  const short = shortTitle(firstText);
  if (short) {
    autoTitleThread(db, threadId, short, "新对话");
    return;
  }
  const attachmentText = firstAttachments
    .filter((a) => !a.skipped && a.text)
    .map((a) => a.text)
    .join("\n");
  const prompt = `用户：${[...`${firstText} ${attachmentText}`.trim()].slice(0, 1500).join("")}`.trim();

  let generated = "";
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TITLE_TIMEOUT_MS);
  try {
    for await (const chunk of client.stream({
      model: modelId,
      mode: "code",
      system: singleSystem(TITLE_SYSTEM_PROMPT),
      messages: [{ role: "user", content: prompt, toolCalls: [], toolCallId: "", attachments: [] }],
      tools: [],
      maxTokens: TITLE_MAX_TOKENS,
    })) {
      if (chunk.err) throw chunk.err;
      if (chunk.text) generated += chunk.text;
    }
  } catch {
    generated = "";
  } finally {
    clearTimeout(timer);
  }

  // sidecar 用量记账（duration 记 0，不覆盖上下文水位）
  if (generated.trim()) {
    const title = sanitizeModelTitle(generated);
    if (title) {
      autoTitleThread(db, threadId, title, "新对话");
    }
  }
  void recordUsageRow;
  void addUsage;
}
