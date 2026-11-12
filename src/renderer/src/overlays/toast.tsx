// Toast 视图：右下角堆叠（四语气），4.2s 自动消失（计时/悬停暂停/右滑关闭由 radix Toast 承担，
// 消失路径统一回调 store.dismiss 清数据）。
import { CheckCircle2, XCircle, Info, AlertTriangle } from "lucide-react";
import { Toast } from "radix-ui";
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

/** Toast 视口：右下角堆叠，图标与配色按语气（info / ok / warn / danger）映射语义 token。 */
export function ToastViewport(): React.JSX.Element {
  const items = useToasts((s) => s.items);
  const dismiss = useToasts((s) => s.dismiss);
  return (
    <Toast.Provider swipeDirection="right" duration={4200}>
      {items.map((toast) => {
        const Icon = ICONS[toast.tone];
        return (
          <Toast.Root
            key={toast.id}
            open
            onOpenChange={(o) => { if (!o) dismiss(toast.id); }}
            className="pointer-events-auto flex w-80 items-start gap-2.5 rounded-xl border border-border bg-toast p-3 shadow-md"
          >
            <Icon className={`mt-0.5 size-4 shrink-0 ${TONE_CLASS[toast.tone]}`} />
            <div className="min-w-0 flex-1">
              <Toast.Title className="text-ui-sm font-medium text-foreground">{toast.title}</Toast.Title>
              {toast.message && <Toast.Description className="text-ui-xs text-foreground-subtle">{toast.message}</Toast.Description>}
            </div>
            <Toast.Close className="shrink-0 rounded p-0.5 text-foreground-subtlest hover:bg-surface-hover">×</Toast.Close>
          </Toast.Root>
        );
      })}
      <Toast.Viewport className="pointer-events-none fixed bottom-6 right-4 z-[70] flex flex-col gap-2 outline-none" />
    </Toast.Provider>
  );
}
