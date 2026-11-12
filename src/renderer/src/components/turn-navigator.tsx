// 提问导航（Turn Navigator）：对话区左侧的竖轨，每条真实用户输入一根短横条——
// hover 弹出该次用户消息、点击滚到该条消息、视口所在那一轮高亮。行为规格见
// `src/renderer/src/lib/turn-nav.ts` 模块头；派生逻辑（过滤 / 活跃判定 / 山峰视觉）在 lib/turn-nav.ts。
//
// 结构：外壳负责宽度闸门与淡入，轨道是独立的小滚动
// 容器（行高固定 10px、超高时内部滚动、窗口化挂载），条目绝对定位在虚拟偏移上。
// 两条必须保留的约定：
//   1) 轨道只开纵向滚动——只声明 overflow-y 时浏览器把 overflow-x 也算成 auto，
//      横条 hover 放大后会出现横向滚动条；
//   2) 目录从短列表长成超长列表时，scrollToIndex 与虚拟化测量在同一 commit，
//      Chromium 可能合并 scroll 通知，需补发一次让轨道立即挂上活动项所在窗口。
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { HoverCard } from "radix-ui";
import {
  barVisualState,
  resolveActiveIndex,
  TURN_NAV_ROW_HEIGHT_PX,
  type TurnNavItem,
  type TurnNavRowPosition,
} from "../lib/turn-nav";

/** 轨道纵向留白：上下各让出 3rem，避开消息列的粘性输入区与顶栏。 */
const TRACK_MAX_HEIGHT = "calc(100% - 6rem)";

/** hover 卡里的时间："yyyy-MM-dd HH:mm"（本地时区）；时间段落时返回空串。 */
function fmtNavTime(ts: number): string {
  if (!ts) return "";
  const d = new Date(ts);
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}


/** 单根横条：视觉由「与焦点的距离」决定（焦点最长最亮，上下邻条依次收敛）。 */
const NavBar = memo(function NavBar({
  item,
  itemIndex,
  itemCount,
  offsetY,
  distance,
  active,
  running,
  onInteractionChange,
  onJump,
}: {
  item: TurnNavItem;
  itemIndex: number;
  itemCount: number;
  offsetY: number;
  distance: number | undefined;
  active: boolean;
  running: boolean;
  onInteractionChange: (index: number | undefined) => void;
  onJump: (item: TurnNavItem) => void;
}): React.JSX.Element {
  // 有交互焦点时按距离出山峰；没有焦点时活跃条也要比其它条亮一档（否则不知道自己在哪）。
  const visual = barVisualState(distance);
  const emphasizeActive = distance === undefined && active;
  const colorTone = distance === undefined && active ? "focus" : visual.colorTone;
  const opacity = emphasizeActive ? 0.9 : running ? Math.max(visual.opacity, 0.72) : visual.opacity;
  return (
    <div className="absolute left-0 top-0 h-2.5 w-9" style={{ transform: `translateY(${offsetY}px)` }}>
      <HoverCard.Root openDelay={120} closeDelay={80}>
        {/* 320px（w-80）固定宽度 + 120/80ms 延迟：鼠标沿轨道竖着掠过时，短延迟会让一串卡
            连闪；h-2.5 行高下横向移动的路径短，两个延迟配合后只在真正停住时展开。 */}
        <HoverCard.Trigger asChild>
          <button
            type="button"
            aria-current={active ? "location" : undefined}
            aria-label={`跳到第 ${itemIndex + 1} 条消息`}
            aria-posinset={itemIndex + 1}
            aria-setsize={itemCount}
            data-turn-nav-index={itemIndex}
            data-turn-nav-active={active ? "true" : "false"}
            onBlur={() => onInteractionChange(undefined)}
            onFocus={() => onInteractionChange(itemIndex)}
            onPointerEnter={() => onInteractionChange(itemIndex)}
            onPointerLeave={() => onInteractionChange(undefined)}
            onClick={() => onJump(item)}
            className="flex h-2.5 w-9 items-center justify-start rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-input-border-focused"
          >
            <span
              className={`block h-0.5 w-3 origin-left rounded-full transition-[height,opacity,transform,background-color] duration-150 ease-out motion-reduce:transition-none ${
                colorTone === "focus" ? "bg-foreground" : "bg-foreground-subtlest"
              }`}
              style={{ opacity, transform: `scaleX(${visual.scaleX})` }}
            />
          </button>
        </HoverCard.Trigger>
        <HoverCard.Portal>
          <HoverCard.Content
            side="right"
            align="start"
            sideOffset={8}
            className="z-[80] w-80 max-w-[calc(100vw-2rem)] rounded-xl border border-popover-border bg-popover p-3 text-popover-foreground shadow-md"
          >
            <p className="line-clamp-6 whitespace-pre-line text-ui-sm leading-5 text-foreground">{item.preview}</p>
            {item.createdAt > 0 && (
              <p className="pt-2 font-mono text-ui-xs text-foreground-subtlest">{fmtNavTime(item.createdAt)}</p>
            )}
          </HoverCard.Content>
        </HoverCard.Portal>
      </HoverCard.Root>
    </div>
  );
});

/**
 * 左侧竖轨。`wide` 由父级（Timeline）按对话区根节点宽度裁决后传入；`scrollRef` 是消息流
 * 滚动容器（活跃判定与跳转共用）；跳转动作本身由调用方实现（它持有贴底状态）。
 */
export function TurnNavigator({
  items,
  wide,
  scrollRef,
  running,
  onJump,
}: {
  items: readonly TurnNavItem[];
  /** 容器宽度是否够宽（≥ TURN_NAV_MIN_WIDTH_PX）：不够时整条轨道淡出且不启用虚拟化。 */
  wide: boolean;
  /** 消息流滚动容器；use-stick-to-bottom 给出的是 HTMLElement ref。 */
  scrollRef: React.RefObject<HTMLElement | null>;
  running: boolean;
  onJump: (item: TurnNavItem) => void;
}): React.JSX.Element | null {
  const [activeIndex, setActiveIndex] = useState(-1);
  const [interactionIndex, setInteractionIndex] = useState<number | undefined>(undefined);
  const railRef = useRef<HTMLDivElement>(null);

  const enabled = wide && items.length >= 2;

  const virtualizer = useVirtualizer({
    enabled,
    count: items.length,
    // 行高固定：条目容器 h-2.5（10px），不需要测量路径。
    estimateSize: () => TURN_NAV_ROW_HEIGHT_PX,
    getItemKey: (index) => items[index]?.id ?? index,
    getScrollElement: () => railRef.current,
    overscan: 6,
  });
  const virtualRows = virtualizer.getVirtualItems();

  // 活跃判定：消息流滚动 / 尺寸变化时重算，rAF 合并到每帧一次。
  // 行几何从已挂载的 user 行读实时 getBoundingClientRect——流式增高、折叠展开都自然跟随。
  // 轨道不可见时（宽度不足或条目太少）直接跳过：此时既没人看，也不该为它读布局。
  // items / enabled 走 ref 而不是闭包：rAF 在下一帧才跑，闭包里的值可能已经过期；
  // 同时让 measure 的依赖恒为 scrollRef，滚动监听器不必因条目变化重建。
  const frameRef = useRef<number | null>(null);
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const measure = useCallback(() => {
    frameRef.current = null;
    const scroller = scrollRef.current;
    if (!scroller || !enabledRef.current) return;
    const scrollerRect = scroller.getBoundingClientRect();
    const positions: TurnNavRowPosition[] = [];
    itemsRef.current.forEach((item, index) => {
      const row = scroller.querySelector<HTMLElement>(`[data-message-id="${item.id}"]`);
      if (!row) return;
      const rect = row.getBoundingClientRect();
      const start = scroller.scrollTop + rect.top - scrollerRect.top;
      positions.push({ index, start, end: start + rect.height });
    });
    const atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 2;
    setActiveIndex(resolveActiveIndex({ positions, scrollTop: scroller.scrollTop, atBottom }));
  }, [scrollRef]);

  const schedule = useCallback(() => {
    if (frameRef.current !== null) return;
    frameRef.current = window.requestAnimationFrame(measure);
  }, [measure]);

  // 监听器只绑一次（不依赖 items）：条目变化改用下面的 effect 主动 schedule，
  // 否则每来一条新消息都要解绑重绑，滚动中的监听还会丢帧。
  useEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    schedule();
    scroller.addEventListener("scroll", schedule, { passive: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    observer?.observe(scroller);
    return () => {
      scroller.removeEventListener("scroll", schedule);
      observer?.disconnect();
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [schedule, scrollRef]);

  // 条目集合变化（新消息、切会话）、轨道刚变可见、运行状态切换后重算一次；
  // 挂载顺序晚于消息行，放到下一帧。
  useEffect(() => {
    schedule();
  }, [items, enabled, running, schedule]);

  // 活跃条跟随：滚出轨道可视区时把它带回视野（align "auto" 只在越界时移动）。
  useEffect(() => {
    if (!enabled || activeIndex < 0) return;
    virtualizer.scrollToIndex(activeIndex, { align: "auto" });
    const element = railRef.current;
    if (!element) return;
    // 目录从短列表扩成超长列表时，scrollToIndex 与虚拟化测量在同一 commit，
    // Chromium 可能合并 scroll 通知；补发一次确保活动项所在窗口立即挂载。
    window.queueMicrotask(() => {
      if (railRef.current === element) element.dispatchEvent(new Event("scroll"));
    });
  }, [activeIndex, enabled, items.length, virtualizer]);

  const visualFocusIndex = interactionIndex;
  const itemCount = items.length;
  const runIndex = running ? items.length - 1 : -1;

  const handleJump = useCallback(
    (item: TurnNavItem) => {
      setInteractionIndex(undefined);
      onJump(item);
    },
    [onJump],
  );

  const rows = useMemo(
    () =>
      virtualRows.map((row) => {
        const item = items[row.index];
        if (!item) return null;
        const distance = visualFocusIndex === undefined ? undefined : Math.min(3, Math.abs(row.index - visualFocusIndex));
        return (
          <NavBar
            key={item.id}
            item={item}
            itemIndex={row.index}
            itemCount={itemCount}
            offsetY={row.start}
            distance={distance}
            active={row.index === activeIndex}
            running={row.index === runIndex}
            onInteractionChange={setInteractionIndex}
            onJump={handleJump}
          />
        );
      }),
    [activeIndex, handleJump, itemCount, items, runIndex, visualFocusIndex, virtualRows],
  );

  // 不足两条真实输入时没有导航意义；宽度不够时挂外壳也不渲染（enabled 已挡住虚拟化）。
  if (items.length < 2) return null;

  return (
    <nav
      aria-label="提问导航"
      className={`pointer-events-none absolute inset-y-0 left-0 z-10 w-12 transition-[opacity,transform,visibility] duration-150 ease-out motion-reduce:transition-none ${
        enabled ? "visible translate-x-0 opacity-100" : "invisible -translate-x-2 opacity-0"
      }`}
    >
      <div
        ref={railRef}
        className="turn-nav-rail pointer-events-auto absolute left-3 top-1/2 w-9 -translate-y-1/2 overflow-x-hidden overflow-y-auto py-1"
        style={{ maxHeight: TRACK_MAX_HEIGHT }}
        onPointerLeave={() => setInteractionIndex(undefined)}
        onScroll={() => setInteractionIndex(undefined)}
      >
        <div className="relative w-9" style={{ height: `${virtualizer.getTotalSize()}px` }}>
          {rows}
        </div>
      </div>
    </nav>
  );
}
