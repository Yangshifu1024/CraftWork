// 侧栏宽度算术：clamp、可用上限、吸附判定。纯函数、无 React 依赖——
// 拖动条（components/resize-handle.tsx）只负责取指针位置，裁决逻辑全在这里，
// 将来接 JS 测试基建时可直接单测（D17）。

/** 左侧会话侧栏的宽度区间（px）。上限同时受 CSS 的 max-w-[50%] 兜底约束。 */
export const SIDEBAR = { min: 200, max: 480, initial: 264 } as const;

/** 右侧面板展开态的宽度区间（px）。280 与面板自身的 min-w-[280px] 对齐。 */
export const PANE = { min: 280, max: 760, initial: 380 } as const;

/** 对话列保底宽度（px）：与 #content 的 min-w-[320px]、conversation.tsx 的 min-w-[320px] 同值。
 *  拖动时上限按它反算，到极限拖不动，而不是把对话列压没。 */
export const CONVERSATION_MIN = 320;

/** 拖过 min 之后还要再多拖多少才吸附收起——防「想窄一点，结果整栏消失」。 */
export const COLLAPSE_DEAD_ZONE = 24;

/** 键盘调整步长（←/→ 每按一次）。 */
export const KEYBOARD_STEP = 8;

/** 夹到区间内；非有限数（NaN / Infinity）回落 min，避免把 NaN 写进 CSS 变量。 */
export function clampWidth(raw: number, min: number, max: number): number {
  if (!Number.isFinite(raw)) return min;
  return Math.min(Math.max(raw, min), max);
}

/**
 * 视口允许的宽度上限：绝对上限、给对话列留出 CONVERSATION_MIN、以及（左栏）不超过容器一半。
 *
 * @param viewport     窗口内宽（px）
 * @param otherWidth   另一侧栏当前实占宽度（px；右栏收起时为 36px 图标列）
 * @param absoluteMax  该栏自己的区间上限
 * @param capHalf      是否再叠一道「不超过容器一半」（左栏与 CSS max-w-[50%] 同口径）
 *
 * 结果可能小于 min（窗口极窄时）：此时 clamp 会退化为 min，CSS 的 max-w-[50%] 再兜一层；
 * 两套口径的边界情形写在 D17「窗口极窄」一节。
 */
export function maxWidthByViewport(
  viewport: number,
  otherWidth: number,
  absoluteMax: number,
  capHalf = false,
): number {
  const byConversation = viewport - otherWidth - CONVERSATION_MIN;
  const byHalf = capHalf ? viewport / 2 : Number.POSITIVE_INFINITY;
  return Math.max(Math.min(absoluteMax, byConversation, byHalf), 0);
}

/** 松手时判定吸附收起：raw 越过 min 且超出死区。拖动中只做视觉 clamp，raw 单独记着，
 *  所以「视觉停在 min」与「是否该收起」是两件事。 */
export function shouldCollapseOnRelease(raw: number, min: number): boolean {
  return Number.isFinite(raw) && raw < min - COLLAPSE_DEAD_ZONE;
}
