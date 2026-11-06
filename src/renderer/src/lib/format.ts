// 展示格式化工具：与历史 React 版 lib/utils.ts 的 formatTokens 同口径。

/** token 数紧凑展示：<1000 原样、<10000 一位小数 k、更大四舍五入 k（12.5k→13k、12.4k→12k）。 */
export function formatTokens(value: number): string {
  if (!value || value <= 0) return "0";
  if (value < 1000) return String(value);
  if (value < 10000) return `${(value / 1000).toFixed(1)}k`;
  return `${Math.round(value / 1000)}k`;
}
