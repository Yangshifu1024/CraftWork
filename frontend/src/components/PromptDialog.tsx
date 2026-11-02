/**
 * 全局单行输入框：显示 `useUiStore.askPrompt` 发起的输入请求，
 * 用于重命名会话、输入命令前缀等只需要一个字符串的场景。
 *
 * 与确认框同属 Promise 式弹层：调用方 `await askPrompt(...)` 挂起，
 * 用户提交时返回去掉首尾空白的文本，取消时返回 null。
 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useUiStore } from "../stores/ui";
import { Icon } from "./Icon";

/**
 * 单行输入对话框，不接收 props。
 *
 * 负责：渲染当前输入请求、维护输入框内容、把结果交回 store。
 * props：无。
 * 关键状态：`value` 是输入框的本地内容；每次出现新的请求时用请求里的初始值重置，
 * 避免上一次的输入残留到下一次。
 */
export function PromptDialog() {
  const prompt = useUiStore((state) => state.prompt);
  const resolve = useUiStore((state) => state.resolvePrompt);
  const [value, setValue] = useState("");

  // 请求变化（打开新的输入框）时同步初始值；请求为 null 时清空，避免残留。
  useEffect(() => {
    setValue(prompt?.value ?? "");
  }, [prompt]);

  // Esc 视为取消，返回 null。用捕获阶段并阻止冒泡，避免触发 AppShell 的全局 Esc 逻辑。
  useEffect(() => {
    if (!prompt) {
      return;
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        resolve(null);
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => document.removeEventListener("keydown", handleKey, true);
  }, [prompt, resolve]);

  // 没有待处理请求时完全不渲染。
  if (!prompt) {
    return null;
  }

  // 提交前去掉首尾空白；空内容不提交，保持对话框打开，让用户继续输入或按取消。
  function submit() {
    const trimmed = value.trim();
    if (!trimmed) {
      return;
    }
    resolve(trimmed);
  }

  return createPortal(
    <div className="overlay-layer">
      {/* 点击遮罩等同于取消 */}
      <div className="overlay-backdrop" onClick={() => resolve(null)} />
      <div className="confirm-dialog" role="dialog" aria-modal="true">
        <h3>
          <Icon name="edit" />
          {prompt.title}
        </h3>
        {prompt.label ? <p>{prompt.label}</p> : null}
        {/* 输入框自动聚焦；Enter 提交，避免用户还要移动到按钮上 */}
        <input
          className="text-input"
          style={{ width: "100%", marginTop: 8 }}
          value={value}
          placeholder={prompt.placeholder}
          autoFocus
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submit();
            }
          }}
        />
        <div className="confirm-actions">
          <span className="spacer" />
          <button type="button" className="btn" onClick={() => resolve(null)}>
            取消
          </button>
          <button type="button" className="btn primary" onClick={submit}>
            {prompt.confirmText ?? "确定"}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
