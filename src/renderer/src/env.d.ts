// 渲染层全局类型：window.craftwork / window.craftworkPush。
import type { CraftworkApi } from "../../shared/channels";

export interface CraftworkPush {
  onEvents(callback: (batch: unknown) => void): () => void;
  onTerminalData(callback: (payload: unknown) => void): () => void;
  onTerminalExit(callback: (payload: unknown) => void): () => void;
  onToast(callback: (payload: unknown) => void): () => void;
}

declare global {
  interface Window {
    craftwork: CraftworkApi;
    craftworkPush: CraftworkPush;
  }
}

export {};
