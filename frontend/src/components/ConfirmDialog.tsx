import { useEffect } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./Icon";
import { useUiStore } from "../stores/ui";

/** 全局确认框，对应原型的 confirmDialog。 */
export function ConfirmDialog() {
  const confirm = useUiStore((state) => state.confirm);
  const resolve = useUiStore((state) => state.resolveConfirm);

  useEffect(() => {
    if (!confirm) {
      return;
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        resolve(false);
      }
      if (event.key === "Enter") {
        event.preventDefault();
        resolve(true);
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => document.removeEventListener("keydown", handleKey, true);
  }, [confirm, resolve]);

  if (!confirm) {
    return null;
  }

  return createPortal(
    <div className="overlay-layer">
      <div className="overlay-backdrop" onClick={() => resolve(false)} />
      <div className="confirm-dialog" role="dialog" aria-modal="true">
        <h3>
          <Icon name={confirm.danger ? "alert" : "info"} />
          {confirm.title}
        </h3>
        {confirm.description ? <p>{confirm.description}</p> : null}
        {confirm.detail ? <div className="confirm-detail">{confirm.detail}</div> : null}
        <div className="confirm-actions">
          <span className="spacer" />
          <button type="button" className="btn" onClick={() => resolve(false)}>
            {confirm.cancelText ?? "取消"}
          </button>
          <button
            type="button"
            className={`btn ${confirm.danger ? "danger" : "primary"}`}
            onClick={() => resolve(true)}
            autoFocus
          >
            {confirm.confirmText ?? "确认"}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
