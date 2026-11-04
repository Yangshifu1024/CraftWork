// Tauri 环境下的 window.craftwork / window.craftworkPush 桥。
// Electron 版由 preload 的 contextBridge 提供同名 API；Tauri 版在渲染层入口安装本桥：
// 调用面（CraftworkApi）完全不变，组件零改动；事件订阅走 Tauri event 的 craftwork:* 通道。
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

export function installTauriBridge(): void {
  if (typeof window === "undefined" || window.craftwork) return;

  // 全部方法转发到 Rust 侧统一入口 craftwork_invoke（P2 起按方法名分发到领域服务）。
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
