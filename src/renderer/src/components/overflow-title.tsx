// 侧栏长标题：右侧 1.5rem 渐隐代替省略号；溢出时 hover 1 秒后走马灯滚动展示全文
// （40px/s，内容渲染两份、步长 = 单份宽 + 间隙，单向无缝循环）。对齐 ZCode TaskTitleOverflowText 的行为契约。
// 动画 keyframes 与 reduced-motion 降级集中在 styles.css。
import { useLayoutEffect, useRef, useState } from "react";

/** 相邻两份文本之间的空白（px）。 */
const MARQUEE_GAP = 24;
/** 滚动速度（px/s）。 */
const SPEED = 40;

/** 侧栏长标题：溢出时右侧渐隐，hover 走马灯单向滚动展示全文；速度/时长由 styles.css 的 keyframes 承担。 */
export function OverflowTitle({ text, className }: { text: string; className?: string }): React.JSX.Element {
  const boxRef = useRef<HTMLSpanElement>(null);
  const copyRef = useRef<HTMLSpanElement>(null);
  const [boxWidth, setBoxWidth] = useState(0);
  const [copyWidth, setCopyWidth] = useState(0);
  const [hovering, setHovering] = useState(false);

  // 单份文本宽与盒宽；容器宽度变化（侧栏拖拽、字体加载）时重测
  useLayoutEffect(() => {
    const box = boxRef.current;
    const copy = copyRef.current;
    if (!box || !copy) return;
    const measure = (): void => {
      setBoxWidth(box.clientWidth);
      setCopyWidth(copy.scrollWidth);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    return () => observer.disconnect();
  }, [text]);

  const overflow = copyWidth > boxWidth;
  const scrolling = hovering && overflow;
  // 一步位移 = 单份文本宽 + 份间空白：循环末第二份正好接上第一份起点，实现无缝单向滚动
  const travel = copyWidth + MARQUEE_GAP;
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
        className={`overflow-title-track inline-block whitespace-nowrap ${scrolling ? "is-scrolling" : ""}`}
        style={
          scrolling
            ? ({ "--marquee-distance": `-${travel}px`, "--marquee-duration": `${(travel / SPEED).toFixed(2)}s` } as React.CSSProperties)
            : undefined
        }
      >
        <span ref={copyRef} className="inline-block">
          {text}
        </span>
        {/* 第二份文本 + 份间空白，仅滚动期间挂载，供无缝循环接续 */}
        {scrolling && (
          <>
            <span aria-hidden="true" className="inline-block" style={{ width: MARQUEE_GAP }} />
            <span aria-hidden="true" className="inline-block">{text}</span>
          </>
        )}
      </span>
    </span>
  );
}
