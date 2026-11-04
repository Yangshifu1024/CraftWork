// 主进程 API 与推送订阅的类型安全包装。
// api 经 Proxy 惰性转发到 window.craftwork：Electron 版由 preload 在页面脚本前注入；
// Tauri 版由渲染层入口（main.tsx）在模块加载后安装桥，顶层直接引用会拿到 undefined。
import type { Batch, Envelope } from "../../../shared/events";
import type { ToastPayload } from "../../../shared/channels";

export const api = new Proxy({} as typeof window.craftwork, {
  get: (_target, prop) => Reflect.get(window.craftwork, prop),
});

export function onEvents(callback: (batch: Batch) => void): () => void {
  return window.craftworkPush.onEvents((payload) => callback(payload as Batch));
}

export function onToast(callback: (payload: ToastPayload) => void): () => void {
  return window.craftworkPush.onToast((payload) => callback(payload as ToastPayload));
}

export type { Envelope };
