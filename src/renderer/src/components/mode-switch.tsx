// 模式切换：侧栏顶部 Code / Work 两段 pill（radix ToggleGroup，方向键切换；single 模式防取消选中）。
// 选中段按各自模式着色（Code 蓝 / Work 亮黄），与全局模式主色（--zai-brand 覆写，见 styles.css
// 「模式主色命名空间」）同源；轨道与「新建对话」按钮同规格（h-8 圆角浅底）。
import { ToggleGroup } from "radix-ui";
import { useApp } from "../store/app";

/** 选中态按项自身模式取色，互不串色；hover 反馈只给未选中段，避免覆盖选中段着色。 */
const ITEM_ON_CLS = {
  code: "data-[state=on]:bg-mode-code data-[state=on]:text-mode-code-foreground",
  work: "data-[state=on]:bg-mode-work data-[state=on]:text-mode-work-foreground",
} as const;

/** 模式切换器：整行两段 pill，占满侧栏导航区宽度。 */
export function ModeSwitch(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const setMode = useApp((s) => s.setMode);
  return (
    <ToggleGroup.Root
      type="single"
      value={mode}
      onValueChange={(v) => {
        if (v) setMode(v as "code" | "work");
      }}
      className="flex h-8 w-full shrink-0 items-center gap-0.5 rounded-lg bg-surface p-0.5"
    >
      {(["code", "work"] as const).map((m) => (
        <ToggleGroup.Item
          key={m}
          value={m}
          className={`cursor-pointer h-7 flex-1 rounded-md px-2 text-ui-base transition-colors outline-none ${ITEM_ON_CLS[m]} data-[state=off]:text-foreground-subtlest data-[state=off]:hover:text-foreground-subtle`}
        >
          {m === "code" ? "Code" : "Work"}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  );
}
