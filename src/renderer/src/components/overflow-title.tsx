// 侧栏长标题：右侧 1.5rem 渐隐代替省略号；溢出时 hover 1 秒后走马灯滚动展示全文
// （40px/s，去→尾端停顿→返回）。对齐 ZCode TaskTitleOverflowText 的行为契约。
// 动画 keyframes 与 reduced-motion 降级集中在 styles.css。
import { useLayoutEffect, useRef, useState } from "react";

/** 滚动尾部与起点之间的空白（px）。 */
const MARQUEE_GAP = 24;
/** 滚动速度（px/s）。 */
const SPEED = 40;

export function OverflowTitle({ text, className }: { text: string; className?: string }): React.JSX.Element {
  const boxRef = useRef<HTMLSpanElement>(null);
  const trackRef = useRef<HTMLSpanElement>(null);
  const [distance, setDistance] = useState(0);
  const [hovering, setHovering] = useState(false);

  // 溢出量 = 文本宽 - 盒宽；容器宽度变化（侧栏拖拽、字体加载）时重测
  useLayoutEffect(() => {
    const box = boxRef.current;
    const track = trackRef.current;
    if (!box || !track) return;
    const measure = (): void => setDistance(Math.max(0, track.scrollWidth - box.clientWidth));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    return () => observer.disconnect();
  }, [text]);

  const overflow = distance > 0;
  const scrolling = hovering && overflow;
  const travel = distance + MARQUEE_GAP;
  // mask 只在溢出时挂载：短标题的盒宽接近内容宽，恒开的 1.5rem 渐变区会吃掉尾部文本。
  // 只保留右侧尾部渐隐（左侧头部不渐隐：文字经过左缘时直接被裁切）。
  const mask = overflow ? "linear-gradient(to right, black calc(100% - 1.5rem), transparent)" : undefined;

  return (
    <span
      ref={boxRef}
      className={`block overflow-hidden ${className ?? ""}`}
      style={{ maskImage: mask, WebkitMaskImage: mask }}
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={() => setHovering(false)}
    >
      <span
        ref={trackRef}
        className={`overflow-title-track inline-block whitespace-nowrap ${scrolling ? "is-scrolling" : ""}`}
        style={
          scrolling
            ? ({ "--marquee-distance": `-${travel}px`, "--marquee-duration": `${(travel / SPEED).toFixed(2)}s` } as React.CSSProperties)
            : undefined
        }
      >
        {text}
        {/* 滚动尾部的接续空白，仅滚动期间参与宽度计算 */}
        {scrolling && <span aria-hidden="true" className="inline-block" style={{ width: MARQUEE_GAP }} />}
      </span>
    </span>
  );
}
