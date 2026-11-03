// 上下文压缩与用量估算测试。
import { describe, expect, it } from "vitest";
import { estimateHistoryTokens, planCompression, COMPRESSION_THRESHOLD } from "../src/main/agent/context";
import type { Message } from "../src/main/model/types";

function msg(content: string, toolCalls: Message["toolCalls"] = []): Message {
  return { role: "assistant", content, toolCalls, toolCallId: "", attachments: [] };
}

describe("token 估算", () => {
  it("按 rune 估且偏高", () => {
    const tokens = estimateHistoryTokens([msg("abcd")]);
    // 4 rune / 2 + 4 = 6
    expect(tokens).toBe(6);
  });

  it("工具调用计额外开销", () => {
    const withCall = msg("", [{ id: "c", name: "fs_read", args: { path: "a.txt" } }]);
    expect(estimateHistoryTokens([withCall])).toBeGreaterThan(8);
  });
});

describe("压缩切分", () => {
  it("低于阈值不触发", () => {
    const messages = [msg("短"), msg("历史")];
    expect(planCompression(messages, 100000)).toBeNull();
  });

  it("超过阈值切出早期部分且保留最少消息数", () => {
    const messages: Message[] = [];
    for (let i = 0; i < 40; i++) {
      messages.push(msg(`消息内容第 ${i} 条，这里是一些填充文本让 token 估算足够大。`.repeat(5)));
    }
    const plan = planCompression(messages, 1000); // 窗口很小，必然触发
    expect(plan).not.toBeNull();
    expect(plan!.cutIndex).toBeGreaterThan(0);
    expect(messages.length - plan!.cutIndex).toBeGreaterThanOrEqual(4);
    expect(plan!.cutIndex / messages.length).toBeGreaterThan(COMPRESSION_THRESHOLD * 0.3);
  });

  it("切分点不落在 tool 结果上", () => {
    const messages: Message[] = [];
    for (let i = 0; i < 20; i++) {
      messages.push(msg(`填充 ${i}。`.repeat(10)));
    }
    messages.push(msg("", [{ id: "c1", name: "shell_exec", args: { command: "ls" } }]));
    messages.push({ role: "tool", content: "结果".repeat(200), toolCalls: [], toolCallId: "c1", attachments: [] });
    const plan = planCompression(messages, 1000);
    expect(plan).not.toBeNull();
    const cut = plan!.cutIndex;
    // 切分点后的第一条不是 tool 结果（不拆散调用链）
    expect(messages[cut]!.role).not.toBe("tool");
    if (cut > 0) {
      expect(messages[cut - 1]!.toolCalls).toHaveLength(0);
    }
  });
});
