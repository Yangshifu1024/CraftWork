// 渲染层全局类型：window.craftwork / window.craftworkPush（桥安装后可用的调用面与推送订阅面）。
import type { CraftworkApi } from "../../shared/channels";

/** 后端推送订阅面：事件批次、toast 与应用菜单动作；订阅返回同步解绑函数（组件 cleanup 用）。 */
export interface CraftworkPush {
  onEvents(callback: (batch: unknown) => void): () => void;
  onToast(callback: (payload: unknown) => void): () => void;
  onMenu(callback: (payload: unknown) => void): () => void;
}

declare global {
  interface Window {
    craftwork: CraftworkApi;
    craftworkPush: CraftworkPush;
  }
}

export {};
