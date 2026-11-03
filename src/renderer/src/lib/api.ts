// 主进程 API 与推送订阅的类型安全包装。
import type { Batch, Envelope } from "../../../shared/events";
import type { ToastPayload } from "../../../shared/channels";

export const api = window.craftwork;

export function onEvents(callback: (batch: Batch) => void): () => void {
  return window.craftworkPush.onEvents((payload) => callback(payload as Batch));
}

export function onTerminalData(callback: (sessionId: string, data: string) => void): () => void {
  return window.craftworkPush.onTerminalData((payload) => {
    const p = payload as { sessionId: string; data: string };
    callback(p.sessionId, p.data);
  });
}

export function onTerminalExit(callback: (sessionId: string, exitCode: number) => void): () => void {
  return window.craftworkPush.onTerminalExit((payload) => {
    const p = payload as { sessionId: string; exitCode: number };
    callback(p.sessionId, p.exitCode);
  });
}

export function onToast(callback: (payload: ToastPayload) => void): () => void {
  return window.craftworkPush.onToast((payload) => callback(payload as ToastPayload));
}

export type { Envelope };
