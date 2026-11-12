// 模式切换：侧栏顶部「Craft Code / Craft Work」下拉（radix Select 原语组装；触发器样式
// 对齐「新建对话」行：h-8 圆角浅底、无边框，hover 提亮）。Craft 前缀为中性灰，模式名按
// 各自模式着色（Code 蓝 / Work 黄；浅色主题下取 soft-foreground 保证浅底可读），与全局
// 模式主色（--craft-brand 覆写，见 styles.css「模式主色命名空间」）同源。
// 面板对齐统一 Select 规格（bg-popover + rounded-xl 边框阴影 + 同宽 + z-90，层级纪律见
// select.tsx 的 SELECT_CONTENT_Z）；选项以模式色圆点标识模式，ItemText 为整段
// "Craft Code" / "Craft Work"，键盘 typeahead 不受内嵌分段影响。
// 切换仍经 store 的 setMode 唯一入口（侧栏 / ⌘K / 设置页共用），全局主色随模式整体切换。
import { Select as RadixSelect } from "radix-ui";
import { Check, ChevronDown } from "lucide-react";
import { SELECT_CONTENT_Z } from "./select";
import { useApp } from "../store/app";

const OPTIONS = [
  { value: "code", label: "Code", dotCls: "bg-mode-code", nameCls: "text-mode-code-soft-foreground" },
  { value: "work", label: "Work", dotCls: "bg-mode-work", nameCls: "text-mode-work-soft-foreground" },
] as const;

/** 模式切换器：整行下拉触发器 + 模式选项面板，占满侧栏导航区宽度。 */
export function ModeSwitch(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const setMode = useApp((s) => s.setMode);
  return (
    <RadixSelect.Root
      value={mode}
      onValueChange={(v) => {
        setMode(v as "code" | "work");
      }}
    >
      <RadixSelect.Trigger
        aria-label="切换工作模式"
        className="group flex h-8 w-full shrink-0 cursor-pointer items-center gap-1.5 rounded-lg bg-surface px-2.5 text-ui-xl outline-none transition-colors hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand data-[state=open]:bg-surface-hover"
      >
        <span className="min-w-0 truncate">
          <span className="font-medium text-foreground-subtle">Craft</span>{" "}
          <span className="font-semibold text-mode-code-soft-foreground data-[mode=work]:text-mode-work-soft-foreground" data-mode={mode}>
            {mode === "code" ? "Code" : "Work"}
          </span>
        </span>
        <RadixSelect.Icon className="ml-auto shrink-0 text-foreground-subtle">
          <ChevronDown className="size-3.5 transition-transform duration-150 group-data-[state=open]:rotate-180" />
        </RadixSelect.Icon>
      </RadixSelect.Trigger>
      <RadixSelect.Portal>
        <RadixSelect.Content
          position="popper"
          sideOffset={6}
          // 面板与触发器同宽（跟随 --radix-select-trigger-width），规格对齐统一 Select（select.tsx）
          className={`${SELECT_CONTENT_Z} max-h-72 min-w-[var(--radix-select-trigger-width)] overflow-y-auto rounded-xl border border-popover-border bg-popover p-1 shadow-md`}
        >
          <RadixSelect.Viewport>
            {OPTIONS.map((o) => (
              <RadixSelect.Item
                key={o.value}
                value={o.value}
                className="flex h-8 cursor-pointer select-none items-center gap-2 rounded-lg px-2.5 text-ui-lg text-foreground outline-none data-[highlighted]:bg-surface-hover data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
              >
                <span className={`size-2 shrink-0 rounded-full ${o.dotCls}`} aria-hidden />
                <RadixSelect.ItemText>
                  <span className="font-medium text-foreground-subtle">Craft</span>{" "}
                  <span className={`font-semibold ${o.nameCls}`}>{o.label}</span>
                </RadixSelect.ItemText>
                <RadixSelect.ItemIndicator className="ml-auto shrink-0">
                  <Check className="size-3.5 text-foreground" />
                </RadixSelect.ItemIndicator>
              </RadixSelect.Item>
            ))}
          </RadixSelect.Viewport>
        </RadixSelect.Content>
      </RadixSelect.Portal>
    </RadixSelect.Root>
  );
}
