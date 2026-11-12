// 提问导航（Turn Navigator）的纯派生逻辑：把消息流折算成左侧竖轨的条目、判定当前活跃条目、
// 给出「山峰」视觉状态。行为规格见 `src/renderer/src/lib/turn-nav.ts` 模块头。
//
// 这里不碰 DOM、不读 React：几何采集与窗口化在组件层（components/turn-navigator.tsx），
// 便于单独推演边界（过滤规则、空会话、贴底、超长文本）。
import type { MessageSnapshot } from "../../../shared/types";

/** 批准计划后自动发送的固定确认消息：不是用户手写输入，不进导航（同 `conversation.tsx` 的 Composer 注释 规则）。 */
export const PLAN_CONFIRM_TEXT = "计划已确认，请按计划执行。";

/** 单条 hover 卡的预览上限：2 段 / 220 字（超出部分截断）。 */
const MAX_PREVIEW_CHARS = 220;
const MAX_PREVIEW_PARAGRAPHS = 2;

/**
 * 不进导航的用户消息来源（`via` 载荷）：
 * - `plan_feedback`：计划审阅卡上的修改意见，渲染层已把它抽进卡片，不单独出现在消息流里；
 * - `subagent_report`：后台子代理回报时系统写入的唤醒锚点，不是用户输入。
 * `steer`（运行中引导）是用户真实输入，保留。
 */
const EXCLUDED_VIA = new Set(["plan_feedback", "subagent_report"]);

export interface TurnNavItem {
  /** 稳定标识：消息 id（虚拟化 key 与跳转定位共用）。 */
  id: string;
  /** hover 卡正文（已折叠空白并截断）。 */
  preview: string;
  /** 消息起始时刻（毫秒）；0 = 未知（旧事件流未携带），此时不显示时间戳。 */
  createdAt: number;
}

/** 消息里去空白的正文（多段 text 部件以换行拼接，与「复制」按钮同模式）。 */
function userTextOf(message: MessageSnapshot): string {
  return message.parts
    .filter((part) => part.kind === "text")
    .map((part) => String(part.payload["text"] ?? ""))
    .join("\n");
}

function hasAttachments(message: MessageSnapshot): boolean {
  return message.parts.some((part) => Array.isArray(part.payload["attachments"]) && (part.payload["attachments"] as unknown[]).length > 0);
}

/**
 * 预览文本：连续空白折叠为单空格、空行折叠为单换行，最多 MAX_PREVIEW_PARAGRAPHS 段、
 * 总长不超过 MAX_PREVIEW_CHARS（截断加省略号）。无正文但有附件时回退「（附件）」。
 */
export function previewText(text: string, hasAttachment: boolean): string {
  const paragraphs: string[] = [];
  let current: string[] = [];
  // 先按行切、再把行内连续空白折叠（与预览上限同一套折叠口径），
  // 但用整段 split 代替逐字符循环——导航条目最多几十条，不需要逐字符的微优化。
  for (const rawLine of text.split("\n")) {
    const line = rawLine.replace(/[ \t\f\v\u00a0]+/gu, " ").trim();
    if (line) {
      current.push(line);
      continue;
    }
    if (current.length > 0) {
      paragraphs.push(current.join(" "));
      current = [];
      if (paragraphs.length >= MAX_PREVIEW_PARAGRAPHS) break;
    }
  }
  if (current.length > 0 && paragraphs.length < MAX_PREVIEW_PARAGRAPHS) paragraphs.push(current.join(" "));

  const joined = paragraphs.join("\n");
  if (!joined) return hasAttachment ? "（附件）" : "";
  if (joined.length <= MAX_PREVIEW_CHARS) return joined;
  // 截断不切断代理对：按码点而不是 UTF-16 单元回退一位，避免留下半个 emoji。
  const sliced = joined.slice(0, MAX_PREVIEW_CHARS - 1);
  const safe = /[\ud800-\udbff]$/u.test(sliced) ? sliced.slice(0, -1) : sliced;
  return `${safe.trimEnd()}…`;
}

/**
 * 从（已按渲染规则处理过的）消息流里筛出导航条目：只有真实用户输入。
 * 排除：计划反馈、子代理回报锚点、批准计划的固定确认消息、空白且无附件的消息。
 */
export function buildTurnNavItems(messages: readonly MessageSnapshot[]): TurnNavItem[] {
  const items: TurnNavItem[] = [];
  for (const message of messages) {
    if (message.role !== "user") continue;
    const via = String(message.parts.find((part) => part.payload["via"] != null)?.payload["via"] ?? "");
    if (EXCLUDED_VIA.has(via)) continue;
    const text = userTextOf(message);
    if (text.trim() === PLAN_CONFIRM_TEXT) continue;
    const attachment = hasAttachments(message);
    const preview = previewText(text, attachment);
    if (!preview) continue;
    items.push({ id: message.id, preview, createdAt: message.createdAt });
  }
  return items;
}

/** 轨道行的固定高度（px）：与条目容器 h-2.5 一致，虚拟化按它估算偏移。 */
export const TURN_NAV_ROW_HEIGHT_PX = 10;
/** 轨道宽度闸门：容器宽度低于它不显示（与内容列宽度档同一断点）。 */
export const TURN_NAV_MIN_WIDTH_PX = 864;

/** 条目在某一行（滚动坐标系内）的上下边界。 */
export interface TurnNavRowPosition {
  index: number;
  start: number;
  end: number;
}

/**
 * 活跃条目：**已滚过视口顶的最后一条**（也就是用户正在看的那一轮的开头）；一条都还没滚过
 * 视口顶时取第一条。`positions` 按文档顺序（即 start 单调递增）。
 *
 * 贴底时直接取末条：流式期间内容每帧增高，一直算几何只会白烧预算——而贴底意味着
 * 用户看的就是最新一轮，这正是末条。
 */
export function resolveActiveIndex(params: {
  positions: readonly TurnNavRowPosition[];
  scrollTop: number;
  atBottom: boolean;
}): number {
  const { positions, scrollTop, atBottom } = params;
  if (positions.length === 0) return -1;
  if (atBottom) return positions[positions.length - 1]!.index;

  // 容忍 1px 的取整误差：条目的 start 与 scrollTop 都是整数像素，但缩放屏下可能差零点几。
  const viewportStart = Math.max(0, scrollTop) + 1;
  let low = 0;
  let high = positions.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (positions[middle]!.start <= viewportStart) low = middle + 1;
    else high = middle;
  }
  // low 是第一条 start > viewportStart 的下标；它前一条即「已滚过视口顶的最后一条」。
  return (low > 0 ? positions[low - 1]! : positions[0]!).index;
}

export interface TurnNavBarVisualState {  colorTone: "focus" | "muted";
  opacity: number;
  scaleX: number;
}

/**
 * 「山峰」视觉：焦点条最长最亮，上下邻条次之（焦点 / 近 / 中 / 静默四档）。
 * 距离为 undefined 表示没有交互焦点（既没 hover 也没离开视口判定），全部按 idle 处理。
 */
export function barVisualState(distance: number | undefined): TurnNavBarVisualState {
  if (distance === undefined) return { colorTone: "muted", opacity: 0.58, scaleX: 1 };
  if (distance <= 0) return { colorTone: "focus", opacity: 1, scaleX: 2.6 };
  if (distance === 1) return { colorTone: "muted", opacity: 0.86, scaleX: 1.7 };
  if (distance === 2) return { colorTone: "muted", opacity: 0.72, scaleX: 1.25 };
  return { colorTone: "muted", opacity: 0.58, scaleX: 1 };
}
