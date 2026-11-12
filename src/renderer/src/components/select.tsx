// 统一下拉选择：radix-ui Select 封装，替代散落的原生 <select>。
// 触发器对齐表单输入框样式（h-8 圆角边框 bg-input）；面板对齐项目浮层惯例
// （bg-popover + 圆角边框 + 阴影，见 DropdownMenu/Popover 既有用法）。
//
// 层级纪律（2026-11-10 修）：弹层是 Portal 到 body 的独立层，z 必须**高于宿主浮层**，
// 否则选项被宿主的遮罩盖住——「看得见、点不到」。修复前的 `z-50` 在模型编辑弹窗
// （Dialog Content/Overlay 为 z-[55]）里就出现了这个问题。当前全站层高：
//   设置页 / 命令面板 50 → 各类对话框 55 → 图片预览 60 → toast 70 →
//   设置内嵌 Dialog 78 → Popconfirm / 抽屉详情 80 → **选择器弹层 90** → 窗口按钮 95。
// 选择器是此刻的交互目标，压过确认框与 toast 是刻意的取舍：不打开下拉时两者互不重叠。
// 真要再加层，先读这条注释再决定放哪儿。
import { Select as RadixSelect } from "radix-ui";
import { Check, ChevronDown } from "lucide-react";

/** 下拉面板层级：见文件头的层级纪律。 */
const SELECT_CONTENT_Z = "z-[90]";

export interface SelectOption {
  value: string;
  label: string;
}

/** sm：设置页 / 对话框表单（h-8）；xs：侧栏窄条（h-7）。不用 className 覆盖尺寸，避免同类冲突。 */
type SelectSize = "sm" | "xs";

const SIZE_CLS: Record<SelectSize, { trigger: string; item: string }> = {
  sm: { trigger: "h-8 rounded-lg px-2 text-ui-sm", item: "px-2.5 py-1.5 text-ui-sm" },
  xs: { trigger: "h-7 rounded-md px-2 text-ui-xs", item: "px-2 py-1 text-ui-xs" },
};

export function Select({
  value,
  options,
  onChange,
  disabled,
  size = "sm",
  fullWidth,
  invalid,
  ariaLabel,
  placeholder,
  className,
}: {
  value: string;
  options: Array<SelectOption>;
  onChange: (value: string) => void;
  disabled?: boolean;
  size?: SelectSize;
  /** 表单字段场景铺满可用宽度（与 inputCls 的 w-full 对齐） */
  fullWidth?: boolean;
  /** 错误态边框（与输入框 border-destructive 一致） */
  invalid?: boolean;
  ariaLabel?: string;
  /** value 匹配不到任何选项时的占位文案 */
  placeholder?: string;
  /** 外部布局类（如外边距）；尺寸类请走 size，不在此覆盖 */
  className?: string;
}): React.JSX.Element {
  const current = options.find((o) => o.value === value);
  const sizeCls = SIZE_CLS[size];
  return (
    <RadixSelect.Root value={value} onValueChange={onChange} disabled={disabled}>
      <RadixSelect.Trigger
        aria-label={ariaLabel}
        className={`flex shrink-0 cursor-pointer items-center justify-between gap-1.5 border bg-input text-foreground outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand disabled:cursor-not-allowed disabled:opacity-50 ${
          invalid ? "border-destructive" : "border-border"
        } ${fullWidth ? "w-full" : "w-fit"} ${sizeCls.trigger} ${className ?? ""}`}
      >
        <span className="min-w-0 truncate">{current?.label ?? placeholder ?? ""}</span>
        <RadixSelect.Icon className="shrink-0 text-foreground-subtle">
          <ChevronDown className="size-3.5" />
        </RadixSelect.Icon>
      </RadixSelect.Trigger>
      <RadixSelect.Portal>
        <RadixSelect.Content
          position="popper"
          sideOffset={6}
          // 面板默认与触发器同宽（跟随 --radix-select-trigger-width），长选项文本再放宽
          className={`${SELECT_CONTENT_Z} max-h-72 min-w-[var(--radix-select-trigger-width)] overflow-y-auto rounded-xl border border-popover-border bg-popover p-1 shadow-md`}
        >
          <RadixSelect.Viewport>
            {options.map((o) => (
              <RadixSelect.Item
                key={o.value}
                value={o.value}
                className={`flex cursor-pointer select-none items-center justify-between gap-2 rounded-lg text-foreground outline-none data-[highlighted]:bg-surface-hover data-[disabled]:pointer-events-none data-[disabled]:opacity-50 ${sizeCls.item}`}
              >
                <RadixSelect.ItemText>{o.label}</RadixSelect.ItemText>
                <RadixSelect.ItemIndicator className="shrink-0">
                  <Check className="size-3.5" />
                </RadixSelect.ItemIndicator>
              </RadixSelect.Item>
            ))}
          </RadixSelect.Viewport>
        </RadixSelect.Content>
      </RadixSelect.Portal>
    </RadixSelect.Root>
  );
}
