// 模型层测试：错误分类、请求体构建、SSE 解析、费用估算。
import { describe, expect, it } from "vitest";
import { statusError, streamError, extractErrorMessage, retryable, transportError, ModelError } from "../src/main/model/errors";
import { buildOpenAIBody, OpenAICompatClient } from "../src/main/model/openai";
import { buildAnthropicBody, buildAnthropicMessages, joinAPIPath, AnthropicClient } from "../src/main/model/anthropic";
import { parseArguments, ToolCallBuilder } from "../src/main/model/sse";
import { estimateCostMicros } from "../src/main/agent/runner";
import type { Request, ToolSpec, Message, ModelInfo, Usage } from "../src/main/model/types";
import { emptyUsage } from "../src/main/model/types";

describe("错误分类", () => {
  it("状态码基础分类", () => {
    expect(statusError("p", 401, "{}").code).toBe("auth");
    expect(statusError("p", 429, "{}").code).toBe("rate_limit");
    expect(statusError("p", 503, "{}").code).toBe("provider_unavailable");
    expect(statusError("p", 400, "{}").code).toBe("invalid_request");
  });

  it("文案二次修正：400 里藏的上下文溢出", () => {
    const err = statusError("p", 400, JSON.stringify({ error: { message: "prompt is too long: 200000 tokens" } }));
    expect(err.code).toBe("context_overflow");
  });

  it("文案二次修正：401 里藏的限流", () => {
    const err = statusError("p", 401, JSON.stringify({ error: { message: "quota exceeded for project" } }));
    expect(err.code).toBe("rate_limit");
  });

  it("流内错误关键词匹配", () => {
    expect(streamError("p", "The context window is full", "").code).toBe("context_overflow");
    expect(streamError("p", "rate limit exceeded", "").code).toBe("rate_limit");
    expect(streamError("p", "", "overloaded_error").code).toBe("provider_unavailable");
  });

  it("错误消息提取：对象/字符串/顶层", () => {
    expect(extractErrorMessage(JSON.stringify({ error: { message: "boom", type: "x" } }))).toBe("boom (x)");
    expect(extractErrorMessage(JSON.stringify({ error: "plain" }))).toBe("plain");
    expect(extractErrorMessage(JSON.stringify({ message: "top" }))).toBe("top");
    expect(extractErrorMessage("not json")).toBe("");
  });

  it("可重试判定", () => {
    expect(retryable(statusError("p", 429, "{}"))).toBe(true);
    expect(retryable(statusError("p", 401, "{}"))).toBe(false);
    const cancelErr = transportError("p", Object.assign(new Error("x"), { name: "AbortError" }));
    expect(cancelErr.code).toBe("canceled");
  });

  it("空 Endpoint 在请求前抛可读配置错误（不可重试）", async () => {
    const consume = async (gen: AsyncIterable<unknown>): Promise<unknown> => {
      for await (const chunk of gen) return chunk;
      return undefined;
    };
    const openai = new OpenAICompatClient("", "k", "测试商");
    const err = await consume(openai.stream(makeRequest())).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ModelError);
    expect((err as ModelError).code).toBe("invalid_request");
    expect((err as ModelError).message).toMatch(/未配置服务地址/);
    expect(retryable(err)).toBe(false);
    const anthropic = new AnthropicClient("", "k", "测试商");
    await expect(consume(anthropic.stream(makeRequest()))).rejects.toMatchObject({ code: "invalid_request" });
  });
});

const TOOLS: ToolSpec[] = [{ name: "fs_read", description: "读取", schema: { type: "object", properties: { path: { type: "string" } } } }];

function makeRequest(): Request {
  const messages: Message[] = [
    { role: "user", content: "你好", toolCalls: [], toolCallId: "", attachments: [] },
  ];
  return { model: "gpt-x", mode: "code", system: [{ content: "sys", cacheControl: true }], messages, tools: TOOLS, maxTokens: 1000 };
}

describe("OpenAI 请求体", () => {
  it("系统块拼接为一条 system 消息；工具格式正确", () => {
    const body = buildOpenAIBody("gpt-x", makeRequest()) as Record<string, unknown>;
    const messages = body["messages"] as Array<Record<string, unknown>>;
    expect(messages[0]!.role).toBe("system");
    expect(messages[0]!.content).toBe("sys");
    expect(body["stream"]).toBe(true);
    expect((body["tools"] as Array<Record<string, unknown>>)[0]!.type).toBe("function");
    expect(body["max_tokens"]).toBe(1000);
  });

  it("assistant 工具调用与 tool 结果消息", () => {
    const req: Request = {
      ...makeRequest(),
      messages: [
        { role: "assistant", content: "", toolCalls: [{ id: "c1", name: "fs_read", args: { path: "a" } }], toolCallId: "", attachments: [] },
        { role: "tool", content: "内容", toolCalls: [], toolCallId: "c1", attachments: [] },
      ],
    };
    const body = buildOpenAIBody("gpt-x", req) as Record<string, unknown>;
    const messages = body["messages"] as Array<Record<string, unknown>>;
    // messages[0] 是 system；assistant 在 [1]，tool 结果在 [2]
    expect(messages[1]!["tool_calls"]).toBeTruthy();
    expect(messages[2]!["tool_call_id"]).toBe("c1");
  });
});

describe("Anthropic 请求体", () => {
  it("端点拼接避免重复 /v1", () => {
    expect(joinAPIPath("https://api.anthropic.com", "/v1/messages")).toBe("https://api.anthropic.com/v1/messages");
    expect(joinAPIPath("https://gw.example.com/v1", "/v1/messages")).toBe("https://gw.example.com/v1/messages");
  });

  it("连续同角色合并 + tool 结果转为 user", () => {
    const { messages } = buildAnthropicMessages({
      ...makeRequest(),
      messages: [
        { role: "user", content: "a", toolCalls: [], toolCallId: "", attachments: [] },
        { role: "assistant", content: "", toolCalls: [{ id: "c1", name: "fs_read", args: {} }], toolCallId: "", attachments: [] },
        { role: "tool", content: "r", toolCalls: [], toolCallId: "c1", attachments: [] },
        { role: "assistant", content: "done", toolCalls: [], toolCallId: "", attachments: [] },
      ],
    });
    // user(a) + assistant(tool_use) 是两种角色分开，tool_result 合并进后续？——实际序列：
    // user(a), assistant(tool_use), user(tool_result+assistant文本合并?)
    const roles = messages.map((m) => m.role);
    expect(roles[0]).toBe("user");
    expect(roles[1]).toBe("assistant");
    // tool_result(user) 与后续 assistant 文本不同角色，不合并
    expect(roles).toEqual(["user", "assistant", "user", "assistant"]);
  });

  it("缓存断点：system 块 + 历史最后一块", () => {
    const body = buildAnthropicBody("claude-x", makeRequest()) as Record<string, unknown>;
    const system = body["system"] as Array<Record<string, unknown>>;
    expect(system[0]!["cache_control"]).toEqual({ type: "ephemeral" });
    const messages = body["messages"] as Array<Array<Record<string, unknown>>>;
    const lastMsg = messages[messages.length - 1] as unknown as Array<{ content: Array<Record<string, unknown>> }>;
    void lastMsg;
    const lastBlock = (messages[messages.length - 1] as unknown as { content: Array<Record<string, unknown>> }).content[0];
    expect(lastBlock?.["cache_control"]).toEqual({ type: "ephemeral" });
  });

  it("max_tokens 缺省 4096", () => {
    const body = buildAnthropicBody("claude-x", { ...makeRequest(), maxTokens: 0 }) as Record<string, unknown>;
    expect(body["max_tokens"]).toBe(4096);
  });
});

describe("SSE 工具调用聚合", () => {
  it("按 index 累积并以稳定顺序交付", () => {
    const builder = new ToolCallBuilder();
    builder.add(1, "b", "tool_b", '{"x":');
    builder.add(0, "a", "tool_a", '{"y":1}');
    builder.add(1, "", "", "2}");
    const calls = [...builder.flush()];
    expect(calls.map((c) => c.name)).toEqual(["tool_a", "tool_b"]);
    expect(calls[1]!.args).toEqual({ x: 2 });
  });

  it("参数解析失败保留 _raw", () => {
    expect(parseArguments("{bad json")).toEqual({ _raw: "{bad json" });
    expect(parseArguments("")).toEqual({});
  });
});

describe("费用估算", () => {
  const info: ModelInfo = {
    id: "m",
    name: "m",
    provider: "p",
    contextK: 128,
    maxTokens: 4096,
    caps: { stream: true, tools: true, vision: false, reasoning: false, structured: false },
    cost: { inputPerMTok: 3, outputPerMTok: 15, cacheReadPerMTok: 0.3, cacheWritePerMTok: 3.75 },
    status: "",
    canonicalId: "",
    releaseDate: "",
    source: "",
    envVars: [],
    keyRequired: true,
  };

  it("OpenAI 口径：输入已含缓存 token（扣除缓存部分）", () => {
    const usage: Usage = { ...emptyUsage(), inputTokens: 1000, outputTokens: 100, cacheTokens: 400, cacheWriteTokens: 100 };
    // uncached = 600 * 3 + cacheRead 300 * 0.3 + cacheWrite 100 * 3.75 + 100 * 15 = 1800+90+375+1500 = 3765
    expect(estimateCostMicros(usage, info, "openai")).toBe(3765);
  });

  it("Anthropic 口径：输入不含缓存 token", () => {
    const usage: Usage = { ...emptyUsage(), inputTokens: 1000, outputTokens: 100, cacheTokens: 400, cacheWriteTokens: 100 };
    // uncached = 1000 * 3 + 90 + 375 + 1500 = 4965
    expect(estimateCostMicros(usage, info, "anthropic")).toBe(4965);
  });

  it("无价格返回 0", () => {
    expect(estimateCostMicros(emptyUsage(), { ...info, cost: undefined }, "openai")).toBe(0);
  });
});
