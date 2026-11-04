// 确认框与输入框（全局单例：同类请求新覆盖旧，旧回调按取消收尾）。
// 独立于全屏浮层（overlay）叠加：可出现在设置页等浮层之上（如设置页删除模型确认），
// 不再顶掉底层浮层。Esc / 焦点圈定 / 焦点锁由 radix 承担；
// confirm 用 AlertDialog（点遮罩不关闭，防危险操作误触）。
import { useEffect, useRef, useState } from "react";
import { AlertDialog, Dialog } from "radix-ui";
import { useOverlays } from "../store/overlays";

export function Dialogs(): React.JSX.Element | null {
  const confirm = useOverlays((s) => s.confirm);
  const prompt = useOverlays((s) => s.prompt);
  const closeConfirm = useOverlays((s) => s.closeConfirm);
  const closePrompt = useOverlays((s) => s.closePrompt);
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (prompt) {
      setValue(prompt.value);
      setTimeout(() => inputRef.current?.select(), 30);
    }
  }, [prompt]);

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
