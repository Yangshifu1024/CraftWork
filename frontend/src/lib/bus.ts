/**
 * 前后端事件总线：判断当前是否运行在真实宿主里，并统一处理 Wails 事件订阅与分发。
 *
 * 数据流：Go 侧把同一批事件打包成 `Batch`，通过 `cw:events` 通道推送 →
 * 这里收到后交给所有订阅者（会话 store、终端 store 等）→ 各订阅者按事件类型更新状态。
 * Mock 预览没有真实后端，由 mock.ts 直接调用 `dispatch` 投递同样的批次。
 */
import { Events } from "@wailsio/runtime";
import type { Batch } from "./types";

/**
 * 是否为真实宿主（后端在 Go 进程里）。
 *
 * - 桌面窗口：Wails 的 wails:// 协议，Windows 下是 wails.localhost。
 * - server 模式：由 Go 通过 HTTP 提供页面（`task run:server`），地址是
 *   http(s)，绑定走 /wails/runtime、事件走 /wails/events，同样直连 Go。
 * - 只有 `npm run dev`（`vite --mode mock`）是不带后端的纯 UI 预览，
 *   由 Mock 接管，所以这里只把 mock 模式排除掉。
 */
export const isDesktop =
  typeof window !== "undefined" &&
  (window.location.protocol === "wails:" ||
    window.location.hostname === "wails.localhost" ||
    import.meta.env.MODE !== "mock");

/** 事件批次处理函数的签名。 */
type BatchHandler = (batch: Batch) => void;

/** 所有订阅者。用 Set 保证同一个处理函数不会被重复注册。 */
const handlers = new Set<BatchHandler>();
/** 是否已经向 Wails 注册过监听，避免重复订阅造成重复分发。 */
let connected = false;

/**
 * 订阅后端事件批次。返回取消订阅函数。
 *
 * 第一次订阅时会自动建立与 Wails 运行时的连接；重复订阅不会重复建立连接，
 * 只是把处理函数加入集合。组件卸载时调用返回的函数即可退订。
 */
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

/** 事件序号计数器，仅 Mock 使用，保证同一进程内递增。 */
let sequence = 0;

/**
 * 生成一个合法的事件信封，供 Mock 使用。
 *
 * `seq` 在 Mock 内部自增，`ts` 取当前时间；threadId、runId、data 由调用方给出。
 * 真实后端会用自己的序号，这里只需保证结构一致，前端处理逻辑才能共用。
 */
export function envelope(
  type: string,
  threadID: string,
  runID: string,
  data: any,
): Batch["events"][number] {
  sequence += 1;
  return { seq: sequence, type, threadId: threadID, runId: runID, ts: Date.now(), data };
}

/**
 * 建立与 Wails 运行时的事件连接（内部使用）。
 *
 * 纯 UI 预览没有后端，直接跳过；已经连接过也跳过。
 * 只处理 `cw:events` 这一个通道：载荷为空或没有事件的批次直接丢弃，
 * 避免空批次触发下游无意义的重新渲染。
 */
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
