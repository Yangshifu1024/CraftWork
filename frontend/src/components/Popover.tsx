import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";

interface PopoverProps {
  anchor: HTMLElement | null;
  open: boolean;
  onClose: () => void;
  align?: "left" | "right";
  width?: number;
  children: ReactNode;
}

/** 锚定在元素旁的下拉浮层，对应原型的 .popover-layer / .popover。 */
export function Popover({ anchor, open, onClose, align = "left", width, children }: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<CSSProperties>({});

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
