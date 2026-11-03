// SSE 公共骨架：POST JSON → 状态码校验 → data: 行解析 → 适配器解码。
// POST JSON → 校验状态码 → 逐行解析 data: 事件 → 由适配器解码为 Chunk。
import { transportError, statusError, ModelError } from "./errors";

const MAX_SSE_LINE_BYTES = 4 * 1024 * 1024;

export interface SSEHandle {
  /** 建立流（非 2xx 抛 ModelError）。 */
  start(): Promise<AsyncIterable<string>>;
}

/**
 * 建立 SSE 流。返回异步行迭代器；结束/错误语义：
 * - 网络中断 → 迭代器抛 TransportError
 * - 状态码非 2xx → start() 抛 StatusError（body 前 8192 字节）
 */
export async function openSSE(
  url: string,
  init: RequestInit,
  provider: string,
  signal: AbortSignal,
): Promise<AsyncIterable<string>> {
  let resp: Response;
  try {
    resp = await fetch(url, { ...init, signal });
  } catch (err) {
    throw transportError(provider, err);
  }
  if (!resp.ok) {
    const body = (await resp.text().catch(() => "")).slice(0, 8192);
    throw statusError(provider, resp.status, body);
  }
  if (!resp.body) {
    throw new ModelError("provider_unavailable", provider, "模型服务返回空响应体");
  }
  return readLines(resp.body, signal);
}

/** 按 SSE 帧拆行（容忍超长行截断；跨 chunk 缓冲拼接）。 */
async function* readLines(body: ReadableStream<Uint8Array>, signal: AbortSignal): AsyncIterable<string> {
  const reader = body.getReader();
  const decoder = new TextDecoder("utf8");
  let buffer = "";
  try {
    for (;;) {
      if (signal.aborted) throw new DOMException("Aborted", "AbortError");
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      // 超长行保护
      if (buffer.length > MAX_SSE_LINE_BYTES) {
        buffer = buffer.slice(-1024);
      }
      let idx: number;
      while ((idx = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, idx).replace(/\r$/, "");
        buffer = buffer.slice(idx + 1);
        yield line;
      }
    }
    if (buffer.length > 0) yield buffer.replace(/\r$/, "");
  } finally {
    try {
      reader.releaseLock();
    } catch {
      // 流已关闭
    }
  }
}

/** 从 SSE 行流中提取 data: 载荷（忽略注释/心跳与空行），[DONE] 返回 null。 */
export async function* sseData(lines: AsyncIterable<string>): AsyncIterable<string | null> {
  for await (const line of lines) {
    if (!line.startsWith("data:")) continue;
    const payload = line.slice(5).trim();
    if (payload === "[DONE]") {
      yield null;
      return;
    }
    if (payload.length > 0) yield payload;
  }
}

/** 解析工具调用参数 JSON；失败保留原文到 _raw（避免丢弃模型输出）。 */
export function parseArguments(raw: string): Record<string, unknown> {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
    return { _raw: parsed };
  } catch {
    return { _raw: raw };
  }
}

/** 工具调用增量聚合：按 index 累积 {id, name, args 串接}，结束时按 index 升序交付。 */
export class ToolCallBuilder {
  private items = new Map<number, { id: string; name: string; args: string }>();

  add(index: number, id: string, name: string, argsDelta: string): void {
    const item = this.items.get(index) ?? { id, name, args: "" };
    if (id) item.id = id;
    if (name) item.name = name;
    if (argsDelta) item.args += argsDelta;
    this.items.set(index, item);
  }

  /** 交付全部调用；缺 ID 生成占位（保证顺序稳定）。 */
  *flush(): Generator<{ id: string; name: string; args: Record<string, unknown> }> {
    const indices = [...this.items.keys()].sort((a, b) => a - b);
    for (const idx of indices) {
      const item = this.items.get(idx)!;
      const id = item.id || `call_${idx.toString(16).padStart(4, "0")}`;
      yield { id, name: item.name, args: parseArguments(item.args) };
    }
  }
}
