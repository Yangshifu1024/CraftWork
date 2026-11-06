// 展示格式化工具：与历史 React 版 lib/utils.ts 的 formatTokens 同口径。

/** token 数紧凑展示：<1000 原样、<10000 一位小数 k、更大四舍五入 k（12.5k→13k、12.4k→12k）。 */
export function formatTokens(value: number): string {
  if (!value || value <= 0) return "0";
  if (value < 1000) return String(value);
  if (value < 10000) return `${(value / 1000).toFixed(1)}k`;
  return `${Math.round(value / 1000)}k`;
}

/**
 * 速度展示：<10 tok/s 保留一位小数（下限 0.1），≥10 取整。
 * 段内停顿（限流、慢段）会让实时速度衰减到 1 以下，整数取整把 0.x 显示成
 * 「0 tok/s」，造成明明在跑却显示 0 的假象；低速本来就该读作「很慢」而非「0」。
 */
export function formatSpeed(value: number): string {
  if (!value || value <= 0) return "0";
  if (value >= 10) return String(Math.round(value));
  return Math.max(0.1, value).toFixed(1);
}
