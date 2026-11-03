// 重试装饰器：指数退避，只对可重试错误生效。
// 只对「请求尚未建立流」阶段的可重试错误重试；流建立后的中断由 Runner 处理，不再发起请求。
import { retryable, transportError } from "./errors";
import type { Chunk, Client, Request } from "./types";

export class RetryClient implements Client {
  constructor(
    private readonly inner: Client,
    private readonly provider: string,
    private readonly attempts = 3,
    private readonly baseDelayMs = 700,
    private readonly maxDelayMs = 5000,
  ) {}

  async *stream(req: Request): AsyncIterable<Chunk> {
    let lastErr: unknown;
    for (let attempt = 1; attempt <= this.attempts; attempt++) {
      try {
        // 一旦开始产出片段就不再重试：重复内容会污染会话上下文。
        for await (const chunk of this.inner.stream(req)) {
          yield chunk;
        }
        return;
      } catch (err) {
        lastErr = err;
        if (attempt >= this.attempts || !retryable(err)) throw err;
        const delay = Math.min(this.baseDelayMs * 2 ** (attempt - 1), this.maxDelayMs);
        await sleep(delay);
      }
    }
    throw transportError(this.provider, lastErr);
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
