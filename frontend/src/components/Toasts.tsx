import { Icon, type IconName } from "./Icon";
import { useUiStore } from "../stores/ui";

const toneIcon: Record<string, IconName> = {
  ok: "check-circle",
  warn: "alert",
  danger: "x-circle",
  info: "info",
};

export function Toasts() {
  const toasts = useUiStore((state) => state.toasts);
  const dismiss = useUiStore((state) => state.dismissToast);

  return (
    <div className="toast-layer" aria-live="polite">
      {toasts.map((toast) => {
        const tone = toast.tone ?? "info";
        return (
          <div key={toast.id} className={`toast ${tone === "danger" ? "err" : tone}${toast.onClick ? " is-action" : ""}`}>
            <span className="toast-ico">
              <Icon name={toneIcon[tone] ?? "info"} />
            </span>
            {toast.onClick ? (
              <button type="button" className="toast-main toast-action" onClick={toast.onClick}>
                <span className="toast-title">{toast.title}</span>
                {toast.description ? <span className="toast-desc">{toast.description}</span> : null}
              </button>
            ) : (
              <div className="toast-main">
                <div className="toast-title">{toast.title}</div>
                {toast.description ? <div className="toast-desc">{toast.description}</div> : null}
              </div>
            )}
            <button
              type="button"
              className="toast-close"
              aria-label="关闭通知"
              onClick={(event) => {
                event.stopPropagation();
                dismiss(toast.id);
              }}
            >
              <Icon name="x" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
