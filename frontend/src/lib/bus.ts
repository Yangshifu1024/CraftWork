import { Events } from "@wailsio/runtime";
import type { Batch } from "./types";

/** 桌面宿主：Wails 的 wails:// 协议或 Windows 的 wails.localhost。 */
export const isDesktop =
  typeof window !== "undefined" &&
  (window.location.protocol === "wails:" || window.location.hostname === "wails.localhost");

type BatchHandler = (batch: Batch) => void;

const handlers = new Set<BatchHandler>();
let connected = false;

/** 订阅后端事件批次。返回取消订阅函数。 */
export function onBatch(handler: BatchHandler): () => void {
  handlers.add(handler);
  connect();
  return () => {
    handlers.delete(handler);
  };
}

/** 由 Mock 后端直接投递一个事件批次。 */
export function dispatch(batch: Batch): void {
  handlers.forEach((handler) => handler(batch));
}

let sequence = 0;

/** 生成一个合法的事件信封，供 Mock 使用。 */
export function envelope(
  type: string,
  threadID: string,
  runID: string,
  data: any,
): Batch["events"][number] {
  sequence += 1;
  return { seq: sequence, type, threadId: threadID, runId: runID, ts: Date.now(), data };
}

function connect(): void {
  if (!isDesktop || connected) {
    return;
  }
  connected = true;
  Events.On("cw:events", (event: any) => {
    const batch = event?.data as Batch | undefined;
    if (batch?.events?.length) {
      dispatch(batch);
    }
  });
}
