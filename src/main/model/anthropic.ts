// Anthropic 适配器。
// 关键点：三个 cache_control 断点（系统稳定块 / 系统动态块 / 历史最后一块）、
// 连续同角色合并、tool 结果以 user 身份回传。
import { missingEndpointError, streamError, transportError } from "./errors";
import { openSSE, sseData, ToolCallBuilder } from "./sse";
import { attachmentPromptText } from "./types";
import type { Chunk, Client, Request, Usage } from "./types";
import { emptyUsage } from "./types";

interface AnthropicEvent {
  type: string;
  message?: { usage?: { input_tokens?: number; cache_creation_input_tokens?: number; cache_read_input_tokens?: number } };
  content_block?: { type: string; id?: string; name?: string; input?: unknown };
  delta?: {
    type?: string;
    text?: string;
    thinking?: string;
    partial_json?: string;
    output_tokens?: number;
    input_tokens?: number;
  };
  error?: { message?: string; type?: string };
  usage?: { output_tokens?: number; input_tokens?: number };
  index?: number;
}

/** 端点拼接：base 已含 /v1 时跳过 path 的第一段，避免重复 /v1。 */
export function joinAPIPath(base: string, path: string): string {
  const cleanBase = base.replace(/\/+$/, "");
  const segments = path.replace(/^\/+/, "");
  const firstSeg = segments.split("/")[0] ?? "";
  if (firstSeg && cleanBase.endsWith(`/${firstSeg}`)) {
    return `${cleanBase}/${segments.slice(firstSeg.length + 1)}`;
  }
  return `${cleanBase}/${segments}`;
}

interface AnthropicContentBlock {
  type: string;
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  tool_use_id?: string;
  content?: string;
  source?: { type: string; media_type: string; data: string };
  cache_control?: { type: string };
}

interface AnthropicMessage {
  role: "user" | "assistant";
  content: AnthropicContentBlock[];
}

/** 消息序列转换：system 汇入顶层；连续同角色合并（满足 user/assistant 交替要求）。 */
export function buildAnthropicMessages(req: Request): { system: AnthropicContentBlock[]; messages: AnthropicMessage[] } {
  const system: AnthropicContentBlock[] = [];
  for (const block of req.system) {
    const entry: AnthropicContentBlock = { type: "text", text: block.content };
    if (block.cacheControl) entry["cache_control"] = { type: "ephemeral" };
    system.push(entry);
  }

  const messages: AnthropicMessage[] = [];
  const push = (role: "user" | "assistant", block: AnthropicContentBlock): void => {
    const last = messages[messages.length - 1];
    if (last && last.role === role) {
      last.content.push(block);
    } else {
      messages.push({ role, content: [block] });
    }
  };

  for (const msg of req.messages) {
    if (msg.role === "system") {
      system.push({ type: "text", text: msg.content });
    } else if (msg.role === "user") {
      const textParts: string[] = [msg.content];
      for (const att of msg.attachments) {
        if (att.skipped) continue;
        if (att.data) {
          push("user", {
            type: "image",
            source: { type: "base64", media_type: att.mime, data: att.data },
          });
        }
        if (att.text) textParts.push(attachmentPromptText(att));
      }
      const text = textParts.filter(Boolean).join("\n\n");
      if (text) push("user", { type: "text", text });
    } else if (msg.role === "assistant") {
      if (msg.content) push("assistant", { type: "text", text: msg.content });
      for (const call of msg.toolCalls) {
        push("assistant", { type: "tool_use", id: call.id, name: call.name, input: call.args ?? {} });
      }
    } else if (msg.role === "tool") {
      push("user", { type: "tool_result", tool_use_id: msg.toolCallId, content: msg.content });
    }
  }

  // 第三个缓存断点：历史最后一条消息的最后一个内容块（Agent 每轮尾部追加，下一轮从断点命中）
  const lastMsg = messages[messages.length - 1];
  const lastBlock = lastMsg?.content[lastMsg.content.length - 1];
  if (lastBlock) lastBlock["cache_control"] = { type: "ephemeral" };

  return { system, messages };
}

export function buildAnthropicBody(modelId: string, req: Request): Record<string, unknown> {
  const { system, messages } = buildAnthropicMessages(req);
  const body: Record<string, unknown> = {
    model: modelId,
    max_tokens: req.maxTokens > 0 ? req.maxTokens : 4096,
    messages,
    stream: true,
  };
  if (system.length > 0) body["system"] = system;
  if (req.tools.length > 0) {
    body["tools"] = req.tools.map((t) => ({
      name: t.name,
      description: t.description,
      input_schema: t.schema && Object.keys(t.schema).length > 0 ? t.schema : { type: "object", properties: {} },
    }));
  }
  return body;
}

export class AnthropicClient implements Client {
  constructor(
    private readonly endpoint: string,
    private readonly apiKey: string,
    private readonly providerName: string,
  ) {}

  async *stream(req: Request): AsyncIterable<Chunk> {
    if (!this.endpoint) throw missingEndpointError(this.providerName);
    const controller = new AbortController();
    const lines = await openSSE(
      joinAPIPath(this.endpoint, "/v1/messages"),
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "text/event-stream",
          "x-api-key": this.apiKey,
          "anthropic-version": "2023-06-01",
        },
        body: JSON.stringify(buildAnthropicBody(req.model, req)),
      },
      this.providerName,
      controller.signal,
    );

    const usage: Usage = emptyUsage();
    let usageSeen = false;
    const toolCalls = new ToolCallBuilder();
    // index → 已见 input="{}" 的 tool_use（参数稍后增量给出）
    const pending = new Map<number, { id: string; name: string }>();

    try {
      for await (const payload of sseData(lines)) {
        if (payload === null) break;
        let event: AnthropicEvent;
        try {
          event = JSON.parse(payload) as AnthropicEvent;
        } catch {
          continue;
        }
        switch (event.type) {
          case "error":
            throw streamError(this.providerName, event.error?.message ?? "", event.error?.type ?? "");
          case "message_start": {
            const u = event.message?.usage;
            if (u) {
              usageSeen = true;
              usage.inputTokens = u.input_tokens ?? 0;
              usage.cacheTokens = (u.cache_creation_input_tokens ?? 0) + (u.cache_read_input_tokens ?? 0);
              usage.cacheWriteTokens = u.cache_creation_input_tokens ?? 0;
            }
            break;
          }
          case "content_block_start": {
            const block = event.content_block;
            if (block?.type === "tool_use" && block.id && block.name) {
              const index = event.index ?? 0;
              pending.set(index, { id: block.id, name: block.name });
              const raw = JSON.stringify(block.input ?? {});
              if (raw !== "{}") {
                toolCalls.add(index, block.id, block.name, raw === "{}" ? "" : raw);
              }
            }
            break;
          }
          case "content_block_delta": {
            const delta = event.delta;
            if (!delta) break;
            if (delta.type === "text_delta" && delta.text) {
              yield { text: delta.text };
            } else if (delta.type === "thinking_delta" && delta.thinking) {
              yield { reasoning: delta.thinking };
            } else if (delta.type === "input_json_delta" && delta.partial_json) {
              const index = event.index ?? 0;
              const info = pending.get(index);
              toolCalls.add(index, info?.id ?? "", info?.name ?? "", delta.partial_json);
            }
            break;
          }
          case "message_delta": {
            if (event.usage) {
              usageSeen = true;
              usage.outputTokens = event.usage.output_tokens ?? usage.outputTokens;
              if (event.usage.input_tokens) usage.inputTokens = event.usage.input_tokens;
            }
            break;
          }
          case "message_stop":
            // 结束由流关闭触发
            break;
          default:
            break;
        }
      }
    } catch (err) {
      controller.abort();
      if (err instanceof Error && err.name === "AbortError") throw transportError(this.providerName, err);
      throw err;
    } finally {
      controller.abort();
    }

    for (const call of toolCalls.flush()) {
      yield { toolCall: call };
    }
    yield { usage: usageSeen ? { ...usage } : emptyUsage() };
    yield { done: true };
  }
}
