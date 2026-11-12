// 拖拽条：两侧栏共用的调宽控件。拖动期**只写 CSS 变量、不进 React state**——
// 每帧 setState 会让 <App> 整棵重渲染，对话流（流式 markdown + 虚拟滚动 + 贴底）
// 是最重的子树，60fps 拖动会掉帧。松手才把最终宽度提交给 store。
//
// 交互契约（改动前先读这段，数值都在 lib/resize.ts 的常量里）：
// - **吸附收起**：松手时若宽度已越过 min 还多出 COLLAPSE_DEAD_ZONE(24px)，栏位吸附收起。
//   死区的意义是「想窄一点」与「想关掉」不是同一件事；判定只在松手时做，拖动中只做
//   视觉钳制，所以拖回 min 以上会自动取消。吸附保留吸附前的宽度，下次展开回到原值。
// - **双击复位**：双击拖拽条回到 initial（左 264 / 右 380）。
// - **键盘可达**：role="separator" + aria-valuenow/min/max，←/→ 每次 ±KEYBOARD_STEP(8)px，
//   Home 到 min、End 到上限——纯拖拽的调宽对键盘用户不可达。
import { useCallback, useEffect, useRef } from "react";
import { clampWidth, maxWidthByViewport, shouldCollapseOnRelease, KEYBOARD_STEP } from "../lib/resize";

interface ResizeHandleProps {
  /** 该条控制的 CSS 变量名（--craft-sidebar-width / --craft-pane-width）。 */
  cssVar: string;
  /** left = 侧栏在左、向右拖变宽；right = 侧栏在右、向左拖变宽。 */
  edge: "left" | "right";
  /** 当前宽度（store 值，用于初始化变量与键盘调整的起点）。 */
  value: number;
  min: number;
  max: number;
  /** 双击复位的目标宽度。 */
  initial: number;
  /** 是否再叠一道「不超过容器一半」（左栏传 true，与 CSS max-w-[50%] 同口径）。 */
  capHalf?: boolean;
  /** 可访问名称（「调整侧栏宽度」/「调整面板宽度」）。 */
  label: string;
  /** 松手或键盘调整后提交宽度。 */
  onCommit(width: number): void;
  /** 拖过 min 超出死区后吸附收起。 */
  onCollapse(): void;
}

/** 取当前视口宽度：拖动期间布局宽度不随栏宽变化，用内宽即够。 */
function viewportWidth(): number {
  return document.documentElement.clientWidth;
}

/** 取另一侧栏的实占宽度（用于反算对话列保底）。自身侧栏宽度不影响这个口径。 */
function measureOtherWidth(otherId: string): number {
  const other = document.getElementById(otherId);
  // 另一侧不在场（右栏收起时面板是 36px 图标列，仍有 id；左栏收起时只有展开条）
  return other ? other.getBoundingClientRect().width : 0;
}

/**
 * 拖拽条。视觉与命中区沿用 App.tsx 原先那条「假分隔条」的规格：
 * 4px 命中区、hover / 拖动中显 2px 竖线、cursor-ew-resize。
 */
export function ResizeHandle({
  cssVar,
  edge,
  value,
  min,
  max,
  initial,
  capHalf = false,
  label,
  onCommit,
  onCollapse,
}: ResizeHandleProps): React.JSX.Element {
  const ref = useRef<HTMLDivElement | null>(null);
  /** 拖动中的原始（未 clamp）宽度：用于松手时判吸附，与视觉值分开记。 */
  const rawRef = useRef<number | null>(null);
  /** 起点快照：拖动中不重算视口与另一侧宽度，否则拖动自身引起的布局变化会让手感漂。 */
  const startRef = useRef({ x: 0, width: 0, upper: max });

  // store 值变化（复位、键盘、吸附后重新展开）时同步 CSS 变量。
  // 拖动期不写 store，因此拖动中的值不受这条影响。
  useEffect(() => {
    document.documentElement.style.setProperty(cssVar, `${value}px`);
  }, [cssVar, value]);

  const upperBound = useCallback(() => {
    const otherId = edge === "left" ? "side-pane" : "sidebar";
    return maxWidthByViewport(viewportWidth(), measureOtherWidth(otherId), max, capHalf);
  }, [edge, max, capHalf]);

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>): void => {
    // 只认主键；右键 / 中键不参与拖动。
    if (e.button !== 0) return;
    // 不 preventDefault：那会连带阻止焦点落到本元素，点完拖拽条就不能用方向键继续微调。
    // 拖动期的文本选中已由 html[data-resizing] 的 user-select 规则禁掉。
    e.currentTarget.focus();
    const target = e.currentTarget;
    target.setPointerCapture(e.pointerId);
    rawRef.current = value;
    startRef.current = {
      x: e.clientX,
      width: value,
      upper: Math.max(upperBound(), min),
    };
    // 拖动期：全局光标 + 禁止选中 + 关宽度过渡（样式规则在 styles.css 的 html[data-resizing] 段）。
    document.documentElement.setAttribute("data-resizing", edge);
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>): void => {
    if (rawRef.current === null) return;
    const { x, width, upper } = startRef.current;
    // 左栏在左：指针右移 = 变宽；右栏在右：指针左移 = 变宽。两处都取反即统一。
    const delta = edge === "left" ? e.clientX - x : x - e.clientX;
    const raw = width + delta;
    rawRef.current = raw;
    const shown = clampWidth(raw, min, upper);
    document.documentElement.style.setProperty(cssVar, `${shown}px`);
  };

  const finish = (e: React.PointerEvent<HTMLDivElement>, commit: boolean): void => {
    if (rawRef.current === null) return;
    const raw = rawRef.current;
    rawRef.current = null;
    document.documentElement.removeAttribute("data-resizing");
    if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
    if (!commit) {
      // 取消（Esc / 指针被系统抢走）：视觉值回到 store 值，别留在半途的位置。
      document.documentElement.style.setProperty(cssVar, `${value}px`);
      return;
    }
    const { upper } = startRef.current;
    if (shouldCollapseOnRelease(raw, min)) {
      // 吸附：先恢复吸附前的宽度，再收起——下次展开回到用户最后设的值（spec 第 6 条）。
      document.documentElement.style.setProperty(cssVar, `${value}px`);
      onCollapse();
      return;
    }
    onCommit(clampWidth(raw, min, upper));
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>): void => {
    const upper = Math.max(upperBound(), min);
    let next: number | null = null;
    if (e.key === "ArrowLeft") next = value - KEYBOARD_STEP;
    else if (e.key === "ArrowRight") next = value + KEYBOARD_STEP;
    else if (e.key === "Home") next = min;
    else if (e.key === "End") next = upper;
    if (next === null) return;
    e.preventDefault();
    onCommit(clampWidth(next, min, upper));
  };

  return (
    <div
      ref={ref}
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      title="拖动调整宽度，双击复位"
      className="group/handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={(e) => finish(e, true)}
      onPointerCancel={(e) => finish(e, false)}
      onDoubleClick={() => onCommit(initial)}
      onKeyDown={onKeyDown}
    >
      {/* 2px 竖线：默认透明，hover / 键盘聚焦 / 拖动中显形。拖动中的显形走 styles.css 的
          [data-resizing] 规则，不依赖任意变体（v4 的任意变体对祖先属性选择器不可靠）。 */}
      <span className="resize-handle-line pointer-events-none absolute top-0 h-full w-0.5 rounded-full bg-foreground-subtlest/50 opacity-0 group-hover/handle:opacity-100 group-focus-visible/handle:opacity-100" />
    </div>
  );
}
