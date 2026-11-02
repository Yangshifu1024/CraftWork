/**
 * 上下文占用圆环。
 *
 * 这是数据可视化组件而不是图标：用 SVG circle 的 strokeDasharray 按
 * used / max 比例描边，展示当前上下文占模型窗口的比例。AGENTS.md 的
 * 「不手写 SVG path」约束针对 lucide 图标；这里只使用 circle 与描边
 * 属性，不涉及 path，且视觉效果无法用任何 lucide 图标表达。
 *
 * 颜色按占比三档：<50% 常规灰、50%–80% 提醒黄、≥80% 危险红，
 * 与审批风险的告警色一致，让「快满」的状态一眼可见。
 */
import { cn } from "../lib/utils";

/** ContextRing 的属性。 */
export function ContextRing({
  used,
  max,
  size = 14,
  strokeWidth = 2,
  className,
}: {
  /** 当前占用的 token 数。 */
  used: number;
  /** 模型上下文窗口大小（token 数），小于等于 0 时画空环。 */
  max: number;
  /** 圆环直径（像素），默认 14。 */
  size?: number;
  /** 描边宽度（像素），默认 2。 */
  strokeWidth?: number;
  /** 附加 className。 */
  className?: string;
}) {
  // 描边沿圆心外侧偏移，半径留出线宽避免被视口裁掉。
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  // 比例钳制在 [0,1]；超出窗口时画满环（红色档自然生效）。
  const ratio = max > 0 && used > 0 ? Math.min(used / max, 1) : 0;
  const color = ratio >= 0.8 ? "var(--danger)" : ratio >= 0.5 ? "var(--warn)" : "var(--text-3)";

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className={cn("context-ring", className)}
      aria-hidden="true"
    >
      {/* 底环：始终完整画出，作为进度参照 */}
      <circle cx={size / 2} cy={size / 2} r={radius} fill="none" stroke="var(--border)" strokeWidth={strokeWidth} />
      {/* 进度环：rotate(-90°) 让起点在 12 点方向，与常见进度指示一致 */}
      <circle
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeDasharray={`${circumference * ratio} ${circumference}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
        style={{ transition: "stroke-dasharray 300ms var(--ease)" }}
      />
    </svg>
  );
}
