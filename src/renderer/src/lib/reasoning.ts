// 思考档位显示名映射。档位值来自 models.dev reasoning_options 的归一化结果
// （effort 型原样 values；budget 型固定 off/low/high），未知值原样展示。

const REASONING_LABELS: Record<string, string> = {
  off: "关闭",
  none: "关闭",
  minimal: "极低",
  low: "低",
  medium: "中",
  high: "高",
  xhigh: "极高",
  max: "最高",
  on: "开启",
};

/** 档位值 → 中文显示名；未映射的值原样返回。 */
export function reasoningLabel(level: string): string {
  return REASONING_LABELS[level] ?? level;
}
