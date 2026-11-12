// 侧栏收起后的展开条：36px 竖条，与右侧面板收起态的图标列同宽同构（对称的恢复入口）。
// 为什么需要它：⌘B 收起侧栏后界面上曾没有任何可见的恢复控件——只能靠记快捷键或开命令面板。
import { PanelLeftOpen } from "lucide-react";
import { useApp } from "../store/app";

/** 展开条：整条可点，点击即展开侧栏（与 ⌘B 同一条动作路径）。 */
export function SidebarExpandTab(): React.JSX.Element {
  const toggleSidebar = useApp((s) => s.toggleSidebar);
  return (
    <div className="flex w-9 shrink-0 flex-col items-center border-r border-border/40 py-2">
      <button
        className="flex size-7 items-center justify-center rounded-md text-foreground-subtlest hover:bg-surface-hover hover:text-foreground"
        title="展开侧栏（⌘B）"
        aria-label="展开侧栏"
        onClick={() => toggleSidebar()}
      >
        <PanelLeftOpen className="size-4" />
      </button>
    </div>
  );
}
