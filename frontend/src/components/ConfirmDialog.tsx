/**
 * 全局确认框：以 Portal 形式挂在 body 上，显示 `useUiStore.askConfirm` 发起的确认请求。
 *
 * 这是 Promise 式弹层的展示端之一：调用方 `await askConfirm(...)` 会挂起，
 * 用户点击确认、取消、遮罩或按 Esc / Enter 时，这里调用 `resolveConfirm` 落定 Promise。
 */
import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useUiStore } from "../stores/ui";
import { Icon } from "./Icon";

/**
 * 全局确认框，对应原型的 confirmDialog，不接收 props。
 *
 * 负责：渲染当前确认请求，并把用户的三种选择（确认 / 取消 / 关闭）交回 store。
 * props：无。
 * 关键状态：`confirm` 为 null 时不渲染任何内容；`resolve` 是结束流程的唯一出口。
 */
export function ConfirmDialog() {
  const confirm = useUiStore((state) => state.confirm);
  const resolve = useUiStore((state) => state.resolveConfirm);
  const confirmButtonRef = useRef<HTMLButtonElement>(null);

  // 确认按钮聚焦：等价于 autoFocus，但规避 lint 的 noAutofocus（对读屏与自动聚焦时机的兼容更好）。
  useEffect(() => {
    if (confirm) {
      confirmButtonRef.current?.focus();
    }
  }, [confirm]);

  // 键盘支持：Esc 视为取消，Enter 视为确认。
  // 使用捕获阶段并阻止冒泡，防止 Esc 继续触发 AppShell 的全局 Esc 逻辑。
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

  // 没有待处理请求时完全不渲染，避免空遮罩挡住页面。
  if (!confirm) {
    return null;
  }

  return createPortal(
    <div className="overlay-layer">
      {/* 点击遮罩等同于取消，是成本最低的退出方式 */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: 遮罩是纯装饰的便利点击目标，键盘用户走 Esc 与对话框内按钮 */}
      <div className="overlay-backdrop" role="presentation" onClick={() => resolve(false)} />
      <div className="confirm-dialog" role="dialog" aria-modal="true">
        <h3>
          {/* 危险操作使用警示图标，普通操作使用信息图标 */}
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
          {/* 确认按钮自动聚焦，键盘用户按 Enter 即可通过；危险操作改用危险样式 */}
          <button
            type="button"
            ref={confirmButtonRef}
            className={`btn ${confirm.danger ? "danger" : "primary"}`}
            onClick={() => resolve(true)}
          >
            {confirm.confirmText ?? "确认"}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
