// 模型错误归一化：错误码字符串直接传前端，文案中文可展示。
export const Code = {
  Auth: "auth",
  RateLimit: "rate_limit",
  Timeout: "timeout",
  ContextOverflow: "context_overflow",
  ProviderUnavailable: "provider_unavailable",
  InvalidRequest: "invalid_request",
  Canceled: "canceled",
  Unknown: "unknown",
} as const;

export type ErrorCode = (typeof Code)[keyof typeof Code];

/** 可重试错误码：仅这三个。 */
const RETRYABLE = new Set<string>([Code.RateLimit, Code.Timeout, Code.ProviderUnavailable]);

export class ModelError extends Error {
  constructor(
    readonly code: string,
    readonly provider: string,
    message: string,
    readonly status = 0,
    readonly causeErr?: unknown,
  ) {
    super(message);
    this.name = "ModelError";
  }
}

export function retryable(err: unknown): boolean {
  if (err instanceof ModelError) return RETRYABLE.has(err.code);
  return false;
}

export function codeOf(err: unknown): string {
  if (err instanceof ModelError) return err.code;
  if (isAbort(err)) return Code.Canceled;
  return Code.Unknown;
}

export function isAbort(err: unknown): boolean {
  return err instanceof Error && err.name === "AbortError";
}

function codeForStatus(status: number): string {
  if (status === 401 || status === 403) return Code.Auth;
  if (status === 408) return Code.Timeout;
  if (status === 429) return Code.RateLimit;
  if (status === 504) return Code.Timeout;
  if (status === 500 || status === 502 || status === 503 || status === 529) return Code.ProviderUnavailable;
  if (status === 400 || status === 404 || status === 422) return Code.InvalidRequest;
  if (status >= 500) return Code.ProviderUnavailable;
  return Code.InvalidRequest;
}

const OVERFLOW_KEYWORDS = [
  "context length",
  "context_length_exceeded",
  "maximum context",
  "max context",
  "context window",
  "too many tokens",
  "prompt is too long",
  "input is too long",
  "token limit",
  "reduce the length",
  "exceeds the maximum",
  "上下文",
];

const RATE_LIMIT_KEYWORDS = [
  "rate limit",
  "rate_limit",
  "too many requests",
  "quota",
  "insufficient_quota",
  "resource_exhausted",
];

const AUTH_KEYWORDS = [
  "authentication",
  "unauthorized",
  "invalid api key",
  "invalid_api_key",
  "permission",
  "api key",
  "unauthenticated",
  "credentials",
];

const TIMEOUT_KEYWORDS = ["timeout", "timed out", "deadline", "deadline_exceeded"];

const UNAVAILABLE_KEYWORDS = [
  "overloaded",
  "unavailable",
  "server error",
  "internal error",
  "connection",
  "bad gateway",
  "service_unavailable",
];

function containsAny(text: string, keywords: string[]): boolean {
  const lower = text.toLowerCase();
  return keywords.some((k) => lower.includes(k));
}

/**
 * HTTP 状态错误：先按状态码定基础码，再做文案二次修正
 * （很多提供商把上下文溢出/限流也用 400/401 返回）。
 */
export function statusError(provider: string, status: number, body: string): ModelError {
  let code = codeForStatus(status);
  const message = extractErrorMessage(body) || body.slice(0, 300);
  if (code === Code.InvalidRequest && containsAny(message, OVERFLOW_KEYWORDS)) code = Code.ContextOverflow;
  else if (code === Code.InvalidRequest && containsAny(message, RATE_LIMIT_KEYWORDS)) code = Code.RateLimit;
  else if (code === Code.Auth && containsAny(message, RATE_LIMIT_KEYWORDS)) code = Code.RateLimit;
  return new ModelError(code, provider, `${provider} 返回 HTTP ${status}：${message}`, status);
}

/** 流内错误（HTTP 已 200）：按 message + kind 做关键词匹配。 */
export function streamError(provider: string, message: string, kind: string): ModelError {
  const combined = `${message} ${kind}`;
  let code: string = Code.Unknown;
  if (containsAny(combined, OVERFLOW_KEYWORDS)) code = Code.ContextOverflow;
  else if (containsAny(combined, RATE_LIMIT_KEYWORDS)) code = Code.RateLimit;
  else if (containsAny(combined, AUTH_KEYWORDS)) code = Code.Auth;
  else if (containsAny(combined, TIMEOUT_KEYWORDS)) code = Code.Timeout;
  else if (containsAny(combined, UNAVAILABLE_KEYWORDS)) code = Code.ProviderUnavailable;
  return new ModelError(code, provider, message || kind || "模型返回错误");
}

/** 传输错误：取消 → canceled；超时 → timeout；其余 → provider_unavailable。 */
export function transportError(provider: string, err: unknown): ModelError {
  if (isAbort(err)) {
    return new ModelError(Code.Canceled, provider, "已取消", 0, err);
  }
  const msg = err instanceof Error ? err.message : String(err);
  if (err instanceof Error && (err.name === "TimeoutError" || /timeout|timed out/i.test(msg))) {
    return new ModelError(Code.Timeout, provider, `连接模型服务超时：${msg}`, 0, err);
  }
  return new ModelError(Code.ProviderUnavailable, provider, `无法连接模型服务：${msg}`, 0, err);
}

/** 提供商缺少服务地址（如从目录添加但目录未收录 Endpoint）：配置问题，不可重试。 */
export function missingEndpointError(provider: string): ModelError {
  return new ModelError(
    Code.InvalidRequest,
    provider,
    `${provider} 未配置服务地址，请在设置中编辑该提供商并补全 Endpoint`,
  );
}

/** 从错误响应体提取可读消息：兼容 error 对象 / error 字符串 / 顶层 message。 */
export function extractErrorMessage(body: string): string {
  try {
    const parsed = JSON.parse(body) as Record<string, unknown>;
    const errField = parsed["error"];
    if (typeof errField === "string") return errField;
    if (errField && typeof errField === "object") {
      const obj = errField as Record<string, unknown>;
      const msg = obj["message"];
      if (typeof msg === "string") {
        const extra = obj["type"] ?? obj["code"] ?? obj["status"];
        return extra ? `${msg} (${String(extra)})` : msg;
      }
    }
    const topMessage = parsed["message"];
    if (typeof topMessage === "string") return topMessage;
  } catch {
    // 非 JSON，退回原文
  }
  return "";
}
