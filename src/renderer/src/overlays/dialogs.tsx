// 确认框与输入框（全局单例：同类请求新覆盖旧，旧回调按取消收尾）。
// 独立于全屏浮层（overlay）叠加：可出现在设置页等浮层之上（如设置页删除模型确认），
// 不再顶掉底层浮层。Esc / 焦点圈定 / 焦点锁由 radix 承担；
// confirm 用 AlertDialog（点遮罩不关闭，防危险操作误触）。
// 自由档确认门（FreeLevelDialog）是危险操作的专用形态：能力清单卡片 + 「我已了解
// 风险」勾选门——未勾选时确认禁用、Enter 不生效，勾选后按钮用填充式危险色。
import { useEffect, useRef, useState } from "react";
import { AlertDialog, Dialog } from "radix-ui";
import { Folder, Globe, ShieldAlert, Terminal } from "lucide-react";
import { useOverlays } from "../store/overlays";

/** 自由档放行的三类能力清单（自由档确认门的能力说明）。 */
const FREE_CAPABILITIES = [
  { icon: Folder, title: "文件和文件夹", desc: "读取与编辑任意位置的文件，不限于工作区" },
  {
    icon: Terminal,
    title: "终端命令",
    desc: "运行任意命令，包括删除文件等破坏性操作；计划模式下同样直接执行，不再拦截写操作",
  },
  { icon: Globe, title: "网络和应用", desc: "访问网络、打开应用与外部链接" },
] as const;

/** 全局对话框单例挂载点：自由档确认门 / 确认框 / 输入框三态互斥，按 overlays store 状态择一渲染（优先级从上到下）。 */
export function Dialogs(): React.JSX.Element | null {
  const confirm = useOverlays((s) => s.confirm);
  const prompt = useOverlays((s) => s.prompt);
  const freeLevel = useOverlays((s) => s.freeLevel);
  const closeConfirm = useOverlays((s) => s.closeConfirm);
  const closePrompt = useOverlays((s) => s.closePrompt);
  const closeFreeLevel = useOverlays((s) => s.closeFreeLevel);
  const [value, setValue] = useState("");
  const [riskAck, setRiskAck] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (prompt) {
      setValue(prompt.value);
      setTimeout(() => inputRef.current?.select(), 30);
    }
  }, [prompt]);

  // 每次打开自由档确认门都回到未勾选：确认必须是本次明确的选择
  useEffect(() => {
    if (freeLevel) setRiskAck(false);
  }, [freeLevel]);

  useEffect(() => {
    if (prompt) {
      setValue(prompt.value);
      setTimeout(() => inputRef.current?.select(), 30);
    }
  }, [prompt]);

  if (freeLevel) {
    return (
      <AlertDialog.Root
        open
        // 所有权守卫：onOk 链上可能已打开下一个框，只有状态仍归属本框才收尾。
        onOpenChange={(o) => {
          if (!o && useOverlays.getState().freeLevel === freeLevel) closeFreeLevel();
        }}
      >
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
          <AlertDialog.Content
            className="fixed left-1/2 top-1/2 z-[55] w-[440px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-warning/40 bg-popover p-5 shadow-2xl outline-none"
            onKeyDown={(e) => {
              // 未勾选「我已了解风险」时 Enter 不产生确认路径（信任必须是明确选择）
              if (e.key === "Enter" && !riskAck) e.preventDefault();
            }}
          >
            <AlertDialog.Title className="flex items-center gap-2 text-ui-lg font-semibold text-warning">
              <ShieldAlert className="size-5" /> 要启用完全访问吗？
            </AlertDialog.Title>
            <AlertDialog.Description className="pt-1 text-ui-sm text-foreground-subtle">
              自由档位会放行一切操作，安全只靠审计与备份兜底。建议仅在隔离或完全可信的环境中使用。
            </AlertDialog.Description>
            <div className="grid grid-cols-1 gap-2 pt-4">
              {FREE_CAPABILITIES.map(({ icon: Icon, title, desc }) => (
                <div key={title} className="flex items-start gap-3 rounded-xl border border-border/50 bg-panel px-3.5 py-3">
                  <Icon className="mt-0.5 size-4 shrink-0 text-foreground-subtle" />
                  <div className="min-w-0">
                    <div className="text-ui-sm font-medium text-foreground">{title}</div>
                    <div className="text-ui-xs text-foreground-subtle">{desc}</div>
                  </div>
                </div>
              ))}
            </div>
            <label className="mt-4 flex cursor-pointer items-center gap-2 text-ui-sm text-foreground">
              <input
                type="checkbox"
                checked={riskAck}
                onChange={(e) => setRiskAck(e.target.checked)}
                className="size-4 accent-destructive"
              />
              我已了解风险，确认启用完全访问
            </label>
            <div className="flex justify-end gap-2 pt-4">
              <AlertDialog.Cancel className="h-9 rounded-lg border border-border px-4 text-ui-sm hover:bg-surface-hover">
                取消
              </AlertDialog.Cancel>
              <AlertDialog.Action
                className="h-9 rounded-lg bg-destructive px-4 text-ui-sm text-white disabled:cursor-not-allowed disabled:opacity-40"
                disabled={!riskAck}
                onClick={() => {
                  const fn = freeLevel.onOk;
                  closeFreeLevel();
                  fn();
                }}
              >
                启用完全访问
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    );
  }

  if (confirm) {
    return (
      <AlertDialog.Root
        open
        // 所有权守卫：onOk 链上可能已打开下一个框，只有状态仍归属本框才收尾。
        onOpenChange={(o) => { if (!o && useOverlays.getState().confirm === confirm) closeConfirm(); }}
      >
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
          <AlertDialog.Content className="fixed left-1/2 top-1/2 z-[55] w-[440px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl outline-none">
            <AlertDialog.Title className="text-ui-lg font-semibold">{confirm.title}</AlertDialog.Title>
            <AlertDialog.Description className="pt-2 text-ui-sm text-foreground-subtle">{confirm.message}</AlertDialog.Description>
            <div className="flex justify-end gap-2 pt-5">
              <AlertDialog.Cancel className="h-9 rounded-lg border border-border px-4 text-ui-sm hover:bg-surface-hover">
                取消
              </AlertDialog.Cancel>
              <AlertDialog.Action
                className={`h-9 rounded-lg px-4 text-ui-sm ${
                  confirm.danger ? "bg-destructive text-white" : "bg-brand text-foreground-inverse"
                }`}
                onClick={() => {
                  const fn = confirm.onOk;
                  closeConfirm();
                  fn();
                }}
              >
                确认
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    );
  }

  if (prompt) {
    return (
      <Dialog.Root
        open
        // 所有权守卫：onOk 链上可能已打开下一个框，只有状态仍归属本框才收尾。
        onOpenChange={(o) => { if (!o && useOverlays.getState().prompt === prompt) closePrompt(); }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
          <Dialog.Content className="fixed left-1/2 top-1/2 z-[55] w-[440px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl outline-none">
            <Dialog.Title className="text-ui-lg font-semibold">{prompt.title}</Dialog.Title>
            <Dialog.Description className="pt-1 text-ui-sm text-foreground-subtle">{prompt.message}</Dialog.Description>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                if (!value.trim()) return;
                const fn = prompt.onOk;
                const v = value.trim();
                closePrompt();
                fn?.(v);
              }}
            >
              <input
                ref={inputRef}
                value={value}
                onChange={(e) => setValue(e.target.value)}
                className="mt-3 h-9 w-full rounded-lg border border-border bg-input px-3 text-ui-base"
              />
              <div className="flex justify-end gap-2 pt-4">
                <Dialog.Close asChild>
                  <button type="button" className="h-9 rounded-lg border border-border px-4 text-ui-sm hover:bg-surface-hover">
                    取消
                  </button>
                </Dialog.Close>
                <button
                  type="submit"
                  className="h-9 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:opacity-40"
                  disabled={!value.trim()}
                >
                  保存
                </button>
              </div>
            </form>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    );
  }
  return null;
}
