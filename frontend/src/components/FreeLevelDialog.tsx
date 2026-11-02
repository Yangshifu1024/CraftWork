/**
 * 自由档位确认弹层：以 Portal 形式挂在 body 上，显示 `useUiStore.askFreeConfirm` 发起的确认请求。
 *
 * 与通用 ConfirmDialog 的区别：这里的内容（能力清单、风险提示、勾选门）是固定的，
 * 由 `lib/approval.ts` 的 `guardFreeLevel` 守卫触发；用户必须勾选「我已了解风险」
 * 后才能点「确认启用」，未勾选时确认按钮禁用、Enter 也不生效。
 */
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./Icon";
import { useUiStore } from "../stores/ui";

/** 启用自由档位后放行的三类能力，与截图约定的文案一致。 */
const freeCapabilities = [
  { icon: "folder", title: "文件和文件夹", desc: "读取、创建、修改和删除可访问的文件。" },
  { icon: "terminal", title: "终端命令", desc: "运行命令、安装软件或修改系统配置。" },
  { icon: "globe", title: "网络和应用", desc: "访问网络、发送数据，以及调用已连接的应用。" },
] as const;

/**
 * 自由档位确认弹层，不接收 props。
 *
 * 负责：渲染能力清单与风险提示，勾选确认门通过后把用户的两种选择
 * （确认 / 取消）交回 store。props：无。
 * 关键状态：`freeConfirmOpen` 为 false 时不渲染任何内容；`acked` 是勾选门，
 * 每次打开时重置，保证用户每次都要重新知悉。
 */
export function FreeLevelDialog() {
  const open = useUiStore((state) => state.freeConfirmOpen);
  const resolve = useUiStore((state) => state.resolveFreeConfirm);
  const [acked, setAcked] = useState(false);

  // 每次打开都清掉上次的勾选，确认门不允许「记住上一次」。
  useEffect(() => {
    if (open) {
      setAcked(false);
    }
  }, [open]);

  // 键盘支持：Esc 视为取消；Enter 仅在已勾选时确认，防止键盘绕过勾选门。
  // 使用捕获阶段并阻止冒泡，防止 Esc 继续触发 AppShell 的全局 Esc 逻辑。
  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        resolve(false);
      }
      if (event.key === "Enter" && acked) {
        event.preventDefault();
        resolve(true);
      }
    }
    document.addEventListener("keydown", handleKey, true);
    return () => document.removeEventListener("keydown", handleKey, true);
  }, [open, acked, resolve]);

  // 没有待处理请求时完全不渲染，避免空遮罩挡住页面。
  if (!open) {
    return null;
  }

  return createPortal(
    <div className="overlay-layer">
      {/* 点击遮罩等同于取消，是成本最低的退出方式 */}
      <div className="overlay-backdrop" onClick={() => resolve(false)} />
      <div className="free-dialog" role="dialog" aria-modal="true">
        <h3>
          <Icon name="alert" />
          要启用完全访问吗？
        </h3>
        <p className="free-dialog-lead">启用后，任务可以直接执行以下操作，无需逐次请求你确认。</p>
        <ul className="free-dialog-list">
          {freeCapabilities.map((item) => (
            <li key={item.icon}>
              <Icon name={item.icon} />
              <div>
                <div className="free-dialog-item-title">{item.title}</div>
                <div className="free-dialog-item-desc">{item.desc}</div>
              </div>
            </li>
          ))}
        </ul>
        <p className="free-dialog-note">
          这些操作可能造成数据丢失或泄露。请仅在信任任务内容时启用。你可以随时切回其他权限模式。
        </p>
        <label className="free-dialog-ack">
          <input type="checkbox" checked={acked} onChange={(event) => setAcked(event.target.checked)} />
          我已了解风险，并对自己的数据安全负责。
        </label>
        <div className="confirm-actions">
          <span className="spacer" />
          <button type="button" className="btn" onClick={() => resolve(false)}>
            取消
          </button>
          {/* 确认按钮受勾选门控制：未勾选时禁用，勾选后使用填充式危险样式 */}
          <button type="button" className="btn danger" disabled={!acked} onClick={() => resolve(true)}>
            确认启用
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
