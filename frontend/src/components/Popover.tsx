/**
 * 锚定在某个元素旁的下拉浮层，用于菜单、模型选择、审批范围选择等场景。
 *
 * 挂载方式：通过 Portal 挂到 `document.body`，避免被父级的 `overflow: hidden`
 * 或层叠上下文裁掉；位置在渲染前根据锚点的实际矩形计算。
 */
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

/**
 * 浮层属性。
 *
 * - `anchor`：锚点元素，浮层贴着它显示；为 null 时不渲染；
 * - `open`：是否打开；
 * - `onClose`：需要关闭时的回调，点击外部或按 Esc 都会触发；
 * - `align`：水平对齐方式，left 与锚点左边缘对齐，right 与右边缘对齐；
 * - `width`：固定宽度（像素），不传则由内容撑开；
 * - `children`：浮层内容。
 */
interface PopoverProps {
  /** 锚点元素。 */
  anchor: HTMLElement | null;
  /** 是否打开。 */
  open: boolean;
  /** 请求关闭时调用。 */
  onClose: () => void;
  /** 水平对齐方式，默认 left。 */
  align?: "left" | "right";
  /** 固定宽度（像素）。 */
  width?: number;
  /** 浮层内容。 */
  children: ReactNode;
}

/**
 * 锚定在元素旁的下拉浮层，对应原型的 .popover-layer / .popover。
 *
 * 负责：计算位置、渲染 Portal、处理点击外部与 Esc 关闭。
 * props 含义见 `PopoverProps`。
 * 关键状态：`style` 是根据锚点矩形算出的定位样式，锚点位置变化时会重新计算。
 */
export function Popover({ anchor, open, onClose, align = "left", width, children }: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<CSSProperties>({});

  // 位置计算放在 useLayoutEffect 里，保证浏览器绘制前就拿到最终位置，避免闪一下再跳。
  //
  // 垂直方向的策略：
  // - 先尝试放在锚点下方（留 6px 间距）；
  // - 用估算高度 280px 判断下方是否放得下；
  // - 放不下且上方空间更充足时改为贴着锚点上方显示，用 bottom 定位。
  // 水平方向：left 对齐取锚点左边缘，right 对齐取锚点右边缘到窗口右侧的距离，
  // 两种情况都至少留 8px 边距，避免贴边。
  useLayoutEffect(() => {
    if (!open || !anchor) {
      return;
    }
    const rect = anchor.getBoundingClientRect();
    const gap = 6;
    const next: CSSProperties = {};
    if (align === "right") {
      next.right = Math.max(8, window.innerWidth - rect.right);
    } else {
      next.left = Math.max(8, rect.left);
    }
    const estimatedHeight = 280;
    if (rect.bottom + gap + estimatedHeight > window.innerHeight && rect.top > estimatedHeight) {
      next.bottom = window.innerHeight - rect.top + gap;
    } else {
      next.top = rect.bottom + gap;
    }
    if (width) {
      next.width = width;
    }
    setStyle(next);
  }, [open, anchor, align, width]);

  // 关闭交互：
  // - 点击浮层与锚点以外的位置关闭；判断锚点是为了让锚点自身的点击只负责切换，不重复处理；
  // - Esc 关闭，并用捕获阶段 stopPropagation，避免同一个 Esc 继续触发外层快捷键（例如关闭面板）。
  useEffect(() => {
    if (!open) {
      return;
    }
    function handlePointer(event: MouseEvent) {
      const target = event.target as Node;
      if (ref.current?.contains(target) || anchor?.contains(target)) {
        return;
      }
      onClose();
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("mousedown", handlePointer);
    document.addEventListener("keydown", handleKey, true);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      document.removeEventListener("keydown", handleKey, true);
    };
  }, [open, anchor, onClose]);

  // 关闭时直接渲染空内容，不保留 Portal 节点。
  if (!open) {
    return null;
  }
  return createPortal(
    <div className="popover-layer">
      <div ref={ref} className="popover" style={style} role="menu">
        {children}
      </div>
    </div>,
    document.body,
  );
}
