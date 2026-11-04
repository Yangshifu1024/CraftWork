// 上下文窗口徽章格式化（spec §6 / 票 4）：Intl compact 紧凑文案 + 完整 token 数 title。
// 无 React / DOM 依赖，供模型列表行消费并由 vitest 直测（tests/format-context.test.ts）。

/** 紧凑格式：en-US 千分位 compact（1000 → "1K"、1048576 → "1M"，中段保留一位小数）。 */
const COMPACT = new Intl.NumberFormat("en-US", { notation: "compact" });

/** 完整数字：en-US 千分位（title 展示用）。 */
const FULL = new Intl.NumberFormat("en-US");

/** 徽章紧凑文案：输入 token 数（K 值 × 1000 后传入）。 */
export function formatContextK(tokens: number): string {
  return COMPACT.format(tokens);
}

/** 徽章 title：完整 token 数（如 "1,048,576 tokens"）。 */
export function formatContextTitle(tokens: number): string {
  return `${FULL.format(Math.round(tokens))} tokens`;
}
