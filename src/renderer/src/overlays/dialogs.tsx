// 确认框与输入框（全局单例：同类请求新覆盖旧，旧回调按取消收尾）。
import { useEffect, useRef, useState } from "react";
import { useOverlays } from "../store/overlays";

export function Dialogs(): React.JSX.Element | null {
  const overlay = useOverlays((s) => s.overlay);
  const confirm = useOverlays((s) => s.confirm);
  const prompt = useOverlays((s) => s.prompt);
  const close = useOverlays((s) => s.close);
  const okRef = useRef<(() => void) | null>(null);
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (overlay === "prompt" && prompt) {
      setValue(prompt.value);
      setTimeout(() => inputRef.current?.select(), 30);
    }
  }, [overlay, prompt]);

  useEffect(() => {
    if (overlay !== "confirm" && overlay !== "prompt") return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close();
      }
      if (overlay === "prompt" && e.key === "Enter" && value.trim()) {
        e.preventDefault();
        okRef.current?.();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [overlay, close, value]);

  if (overlay === "confirm" && confirm) {
    okRef.current = confirm.onOk;
    return (
      <div className="fixed inset-0 z-[55] flex items-center justify-center bg-black/60" onClick={close}>
        <div
          className="w-[440px] max-w-[calc(100vw-40px)] rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <h3 className="text-ui-lg font-semibold">{confirm.title}</h3>
          <p className="pt-2 text-ui-sm text-foreground-subtle">{confirm.message}</p>
          <div className="flex justify-end gap-2 pt-5">
            <button className="h-9 rounded-lg border border-border px-4 text-ui-sm hover:bg-surface-hover" onClick={close}>
              取消
            </button>
            <button
              autoFocus
              className={`h-9 rounded-lg px-4 text-ui-sm ${
                confirm.danger ? "bg-destructive text-white" : "bg-brand text-foreground-inverse"
              }`}
              onClick={() => {
                const fn = okRef.current;
                close();
                fn?.();
              }}
            >
              确认
            </button>
          </div>
        </div>
      </div>
    );
  }

  if (overlay === "prompt" && prompt) {
    return (
      <div className="fixed inset-0 z-[55] flex items-center justify-center bg-black/60" onClick={close}>
        <div
          className="w-[440px] max-w-[calc(100vw-40px)] rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <h3 className="text-ui-lg font-semibold">{prompt.title}</h3>
          <p className="pt-1 text-ui-sm text-foreground-subtle">{prompt.message}</p>
          <input
            ref={inputRef}
            autoFocus
            value={value}
            onChange={(e) => setValue(e.target.value)}
            className="mt-3 h-9 w-full rounded-lg border border-border bg-input px-3 text-ui-base"
          />
          <div className="flex justify-end gap-2 pt-4">
            <button className="h-9 rounded-lg border border-border px-4 text-ui-sm hover:bg-surface-hover" onClick={close}>
              取消
            </button>
            <button
              className="h-9 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:opacity-40"
              disabled={!value.trim()}
              onClick={() => {
                const fn = prompt.onOk;
                const v = value.trim();
                close();
                fn?.(v);
              }}
            >
              保存
            </button>
          </div>
        </div>
      </div>
    );
  }
  return null;
}
