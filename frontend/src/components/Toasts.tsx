/**
 * 全局通知层：把 ui store 里的通知列表渲染成右下角（或样式定义的角落）的浮层。
 *
 * 通知本身的生命周期由 store 负责（4.2 秒自动消失），这里只负责展示与手动关闭。
 * 带 `onClick` 的通知会把整条主体渲染成按钮，方便用户点击跳转。
 */
import { Icon, type IconName } from "./Icon";
import { useUiStore } from "../stores/ui";

/** 语气到图标的映射；语气缺失时按 info 处理。 */
const toneIcon: Record<string, IconName> = {
  ok: "check-circle",
  warn: "alert",
  danger: "x-circle",
  info: "info",
};

/**
 * 通知浮层组件，不接收 props。
 *
 * 负责：读取通知列表并逐条渲染，处理关闭按钮的点击。
 * props：无。
 * 关键状态：通知列表来自 `useUiStore`，组件本身不保存状态；
 * `aria-live="polite"` 让读屏在空闲时播报新通知，不打断用户当前操作。
 */
export function Toasts() {
  const toasts = useUiStore((state) => state.toasts);
  const dismiss = useUiStore((state) => state.dismissToast);

  return (
    <div className="toast-layer" aria-live="polite">
      {toasts.map((toast) => {
        // 未指定语气时统一按信息提示处理，保证样式类名始终有值。
        const tone = toast.tone ?? "info";
        return (
          // 危险语气在样式里对应 err 类名，其它语气直接复用语气名。
          <div key={toast.id} className={`toast ${tone === "danger" ? "err" : tone}${toast.onClick ? " is-action" : ""}`}>
            <span className="toast-ico">
              <Icon name={toneIcon[tone] ?? "info"} />
            </span>
            {/* 有回调时整块内容变成按钮，点击即执行回调；否则用普通容器展示。 */}
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
            {/* 关闭按钮要阻止冒泡，否则会顺带触发整条通知的点击回调。 */}
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
