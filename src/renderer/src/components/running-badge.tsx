// 运行中数量徽标：tab 条上内联展示（inset），开关按钮上绝对定位到右上角（floating）。
// 消费方：面板 tab 条的「子代理」tab、会话头的「展开侧栏」开关——两处共用一份样式。
export function RunningBadge({ count, floating = false }: { count: number; floating?: boolean }): React.JSX.Element {
  return (
    <span
      className={
        floating
          ? "absolute -right-0.5 -top-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-brand px-1 text-[10px] font-medium leading-none text-background"
          : "flex h-4 min-w-4 items-center justify-center rounded-full bg-brand px-1 text-[10px] font-medium leading-none text-background"
      }
    >
      {count}
    </span>
  );
}
