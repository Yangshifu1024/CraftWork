// 桥（tauri-bridge）：把通道表（shared/channels 的 CraftworkApi）实现为 Tauri invoke，
// 由渲染层入口（main.tsx）安装为 window.craftwork / window.craftworkPush 调用面，不含业务逻辑。
// 调用面签名与 Electron 时代一致，组件零改动；事件订阅走 Tauri event 的 craftwork:* 通道。
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CraftworkApi } from "../../../shared/channels";

/** Tauri event 的 listen 是异步的；本适配器保证返回同步清理函数（组件 cleanup 语义与 Electron 版一致）。 */
function listenAdapter(channel: string, callback: (payload: unknown) => void): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void listen(channel, (event) => callback(event.payload)).then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** 安装 window.craftwork / window.craftworkPush；已存在则跳过（幂等，热重载安全）。 */
export function installTauriBridge(): void {
  if (typeof window === "undefined" || window.craftwork) return;

  // 全部方法转发到 Rust 侧统一分发入口 craftwork_invoke（src-tauri 按方法名分发到服务层）。
  const api = new Proxy(
    {},
    {
      get: (_target, method: string) =>
        (...args: unknown[]) =>
          invoke("craftwork_invoke", { method, args }),
    },
  );
  window.craftwork = api as unknown as CraftworkApi;

  window.craftworkPush = {
    onEvents: (callback) => listenAdapter("craftwork:events", callback),
    onToast: (callback) => listenAdapter("craftwork:toast", callback),
  };
}
