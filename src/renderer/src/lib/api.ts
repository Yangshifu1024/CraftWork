// window.craftwork（通道表）的类型安全访问口与推送订阅。
// api 经 Proxy 惰性转发：本模块在桥安装（main.tsx 的 installTauriBridge）之前就被各 store
// 顶层 import，模块求值时直接引用 window.craftwork 会拿到 undefined，故每次调用时再取属性。
import type { Batch, Envelope } from "../../../shared/events";
import type { MenuActionPayload, ToastPayload } from "../../../shared/channels";

export const api = new Proxy({} as typeof window.craftwork, {
  get: (_target, prop) => Reflect.get(window.craftwork, prop),
});

/** 订阅后端事件批次推送（applyBatch 的数据源）；返回解绑函数（组件 cleanup 用）。 */
export function onEvents(callback: (batch: Batch) => void): () => void {
  return window.craftworkPush.onEvents((payload) => callback(payload as Batch));
}

/** 订阅后端 toast 推送；返回解绑函数（组件 cleanup 用）。 */
export function onToast(callback: (payload: ToastPayload) => void): () => void {
  return window.craftworkPush.onToast((payload) => callback(payload as ToastPayload));
}

/** 订阅应用菜单动作（macOS 菜单栏点击）；返回解绑函数（组件 cleanup 用）。 */
export function onMenu(callback: (payload: MenuActionPayload) => void): () => void {
  return window.craftworkPush.onMenu((payload) => callback(payload as MenuActionPayload));
}

export type { Envelope };
