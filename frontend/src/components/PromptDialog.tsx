import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./Icon";
import { useUiStore } from "../stores/ui";

/** 单行输入对话框，用于重命名线程、输入命令前缀等。 */
export function PromptDialog() {
  const prompt = useUiStore((state) => state.prompt);
  const resolve = useUiStore((state) => state.resolvePrompt);
  const [value, setValue] = useState("");

  useEffect(() => {
    setValue(prompt?.value ?? "");
  }, [prompt]);

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

  if (!prompt) {
    return null;
  }

  function submit() {
    const trimmed = value.trim();
    if (!trimmed) {
      return;
    }
    resolve(trimmed);
  }

  return createPortal(
    <div className="overlay-layer">
      <div className="overlay-backdrop" onClick={() => resolve(null)} />
      <div className="confirm-dialog" role="dialog" aria-modal="true">
        <h3>
          <Icon name="edit" />
          {prompt.title}
        </h3>
        {prompt.label ? <p>{prompt.label}</p> : null}
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
