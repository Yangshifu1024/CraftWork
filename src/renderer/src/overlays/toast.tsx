// Toast 视图：右下角堆叠（四语气），4.2s 自动消失。
import { CheckCircle2, XCircle, Info, AlertTriangle } from "lucide-react";
import { useToasts } from "../store/toast";

const ICONS = {
  info: Info,
  ok: CheckCircle2,
  warn: AlertTriangle,
  danger: XCircle,
} as const;

const TONE_CLASS = {
  info: "text-icon-blue",
  ok: "text-success",
  warn: "text-warning",
  danger: "text-destructive",
} as const;

export function ToastViewport(): React.JSX.Element {
  const items = useToasts((s) => s.items);
  const dismiss = useToasts((s) => s.dismiss);
  return (
    <div className="pointer-events-none fixed bottom-6 right-4 z-[70] flex flex-col gap-2">
      {items.map((toast) => {
        const Icon = ICONS[toast.tone];
        return (
          <div
            key={toast.id}
            className="pointer-events-auto flex w-80 items-start gap-2.5 rounded-xl border border-border bg-toast p-3 shadow-md"
          >
            <Icon className={`mt-0.5 size-4 shrink-0 ${TONE_CLASS[toast.tone]}`} />
            <div className="min-w-0 flex-1">
              <div className="text-ui-sm font-medium text-foreground">{toast.title}</div>
              {toast.message && <div className="text-ui-xs text-foreground-subtle">{toast.message}</div>}
            </div>
            <button
              className="shrink-0 rounded p-0.5 text-foreground-subtlest hover:bg-surface-hover"
              onClick={() => dismiss(toast.id)}
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}
