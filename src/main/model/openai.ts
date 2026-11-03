// OpenAI / OpenAI-compatible 适配器。
// 请求体拼接规则固定不增减空白——前缀缓存依赖字节稳定。
import { streamError, transportError } from "./errors";
import { openSSE, sseData, ToolCallBuilder } from "./sse";
import { attachmentPromptText } from "./types";
import type { Chunk, Client, Message, Request, SystemBlock, ToolSpec, Usage } from "./types";
import { emptyUsage } from "./types";

interface OpenAIStreamDelta {
  content?: string | null;
  reasoning_content?: string | null;
  reasoning?: string | null;
  tool_calls?: Array<{ index?: number; id?: string; function?: { name?: string; arguments?: string } }>;
}

interface OpenAIStreamChoice {
  delta?: OpenAIStreamDelta;
}

interface OpenAIStreamEvent {
  choices?: OpenAIStreamChoice[];
  usage?: {
    prompt_tokens?: number;
    completion_tokens?: number;
    prompt_tokens_details?: { cached_tokens?: number } | null;
    completion_tokens_details?: { reasoning_tokens?: number } | null;
  } | null;
  error?: { message?: string; type?: string; code?: string };
}

function joinURL(endpoint: string): string {
  return `${endpoint.replace(/\/+$/, "")}/chat/completions`;
}

/** 系统分块拼接为一条 system 消息（空行连接，字节稳定）。 */
function joinSystem(blocks: SystemBlock[]): string {
  return blocks.map((b) => b.content).join("\n\n");
}

interface ContentPart {
  type: string;
  text?: string;
  image_url?: { url: string };
}

function userContent(msg: Message): string | ContentPart[] {
  const hasImages = msg.attachments.some((a) => a.data && !a.skipped);
  if (!hasImages) {
    if (msg.attachments.length === 0) return msg.content;
    const texts = msg.attachments
      .filter((a) => a.text && !a.skipped)
      .map((a) => attachmentPromptText(a));
    return texts.length > 0 ? `${msg.content}\n\n${texts.join("\n\n")}` : msg.content;
  }
  const parts: ContentPart[] = [];
  let text = msg.content;
  for (const att of msg.attachments) {
    if (att.skipped) continue;
    if (att.text) text += `\n\n${attachmentPromptText(att)}`;
  }
  if (text) parts.push({ type: "text", text });
  for (const att of msg.attachments) {
    if (att.skipped || !att.data) continue;
    parts.push({ type: "image_url", image_url: { url: `data:${att.mime};base64,${att.data}` } });
  }
  return parts;
}

export function buildOpenAIBody(modelId: string, req: Request): Record<string, unknown> {
  const messages: Array<Record<string, unknown>> = [];
  if (req.system.length > 0) {
    messages.push({ role: "system", content: joinSystem(req.system) });
  }
  for (const msg of req.messages) {
    if (msg.role === "user") {
      messages.push({ role: "user", content: userContent(msg) });
    } else if (msg.role === "assistant") {
      const entry: Record<string, unknown> = { role: "assistant", content: msg.content };
      if (msg.toolCalls.length > 0) {
        entry["tool_calls"] = msg.toolCalls.map((call) => ({
          id: call.id,
          type: "function",
          function: { name: call.name, arguments: JSON.stringify(call.args ?? {}) },
        }));
      }
      messages.push(entry);
    } else if (msg.role === "tool") {
      messages.push({ role: "tool", tool_call_id: msg.toolCallId, content: msg.content });
    }
  }
  const body: Record<string, unknown> = {
    model: modelId,
    messages,
    stream: true,
    stream_options: { include_usage: true },
  };
  if (req.maxTokens > 0) body["max_tokens"] = req.maxTokens;
  if (req.tools.length > 0) {
    body["tools"] = req.tools.map(
      (t: ToolSpec) => ({ type: "function", function: { name: t.name, description: t.description, parameters: t.schema } }),
    );
  }
  return body;
}

export class OpenAICompatClient implements Client {
  constructor(
    private readonly endpoint: string,
    private readonly apiKey: string,
    private readonly providerName: string,
  ) {}

  async *stream(req: Request): AsyncIterable<Chunk> {
    const controller = new AbortController();
    const lines = await openSSE(
      joinURL(this.endpoint),
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Accept: "text/event-stream",
          ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
        },
        body: JSON.stringify(buildOpenAIBody(req.model, req)),
      },
      this.providerName,
      controller.signal,
    );

    let textSeen = false;
    let usageSeen = false;
    const usage: Usage = emptyUsage();
    const toolCalls = new ToolCallBuilder();

    try {
      for await (const payload of sseData(lines)) {
        if (payload === null) break; // [DONE]
        let event: OpenAIStreamEvent;
        try {
          event = JSON.parse(payload) as OpenAIStreamEvent;
        } catch {
          continue; // 心跳/非标注释直接忽略
        }
        if (event.error) {
          throw streamError(this.providerName, event.error.message ?? "", event.error.type ?? event.error.code ?? "");
        }
        const choice = event.choices?.[0];
        const delta = choice?.delta;
        if (delta) {
          const reasoning = delta.reasoning_content ?? delta.reasoning;
          if (reasoning) {
            textSeen = true;
            yield { reasoning };
          }
          if (delta.content) {
            textSeen = true;
            yield { text: delta.content };
          }
          if (delta.tool_calls) {
            for (const call of delta.tool_calls) {
              toolCalls.add(call.index ?? 0, call.id ?? "", call.function?.name ?? "", call.function?.arguments ?? "");
            }
          }
        }
        if (event.usage) {
          usageSeen = true;
          usage.inputTokens = event.usage.prompt_tokens ?? 0;
          usage.outputTokens = event.usage.completion_tokens ?? 0;
          usage.cacheTokens = event.usage.prompt_tokens_details?.cached_tokens ?? 0;
          usage.reasoningTokens = event.usage.completion_tokens_details?.reasoning_tokens ?? 0;
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
    void textSeen;
    yield { done: true };
  }
}
