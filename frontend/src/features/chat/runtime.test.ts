import { describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "../../lib/types";

vi.mock("../../lib/api", () => ({
  api: {
    threads: { list: vi.fn().mockResolvedValue([]), snapshotPage: vi.fn() },
    chat: { decideApproval: vi.fn(), send: vi.fn(), cancel: vi.fn(), regenerate: vi.fn() },
    providers: { list: vi.fn().mockResolvedValue([]), warnings: vi.fn().mockResolvedValue([]) },
  },
}));
vi.mock("../../lib/bus", () => ({
  onBatch: () => () => undefined,
  dispatch: vi.fn(),
  isDesktop: false,
  envelope: vi.fn(),
}));

const { convertMessage } = await import("./runtime");

function message(role: ChatMessage["role"], status: ChatMessage["status"], parts: ChatMessage["parts"]): ChatMessage {
  return { id: "m1", threadId: "th", role, status, createdAt: 1, parts };
}

describe("消息转换", () => {
  it("按模型能力隐藏推理内容", () => {
    const source = message("assistant", "streaming", [
      { id: "p1", kind: "text", seq: 1, data: { text: "回答" } },
      { id: "p2", kind: "reasoning", seq: 2, data: { text: "思考过程" } },
    ]);

    const hidden = convertMessage(source, false) as any;
    expect(hidden.content.map((part: any) => part.type)).toEqual(["text"]);

    const visible = convertMessage(source, true) as any;
    expect(visible.content.map((part: any) => part.type)).toEqual(["text", "data-cw-reasoning"]);
  });

  it("映射消息状态与数据部件", () => {
    const source = message("assistant", "error", [
      { id: "p1", kind: "tool", seq: 1, data: { partId: "p1", name: "fs_read", status: "done" } },
    ]);
    const converted = convertMessage(source, true) as any;
    expect(converted.status).toEqual({ type: "incomplete", reason: "error" });
    expect(converted.content[0].type).toBe("data-cw-tool");

    const canceled = convertMessage(message("assistant", "incomplete", []), true) as any;
    expect(canceled.status).toEqual({ type: "incomplete", reason: "cancelled" });
  });

  it("用户消息不携带 status，并渲染附件条", () => {
    const source = message("user", "complete", [
      {
        id: "p1",
        kind: "text",
        seq: 1,
        data: { text: "看看附件", attachments: ["/tmp/note.txt"] },
      },
    ]);
    const converted = convertMessage(source, true) as any;
    expect(converted.status).toBeUndefined();
    expect(converted.content[0].type).toBe("data-cw-attachments");
    expect(converted.content[1]).toMatchObject({ type: "text", text: "看看附件" });
  });

  it("推理部件携带流式标记与思考时长，仅最后一个部件在流式中标记为正在思考", () => {
    const source = message("assistant", "streaming", [
      { id: "p1", kind: "reasoning", seq: 1, data: { text: "前面的思考", durationMs: 5200 } },
      { id: "p2", kind: "reasoning", seq: 2, data: { text: "最新的思考" } },
    ]);
    const converted = convertMessage(source, true) as any;
    expect(converted.content[0].data).toMatchObject({ streaming: false, durationMs: 5200 });
    expect(converted.content[1].data).toMatchObject({ streaming: true });

    // 消息结束后流式标记消失。
    const done = convertMessage({ ...source, status: "complete" }, true) as any;
    expect(done.content[1].data.streaming).toBe(false);
  });

  it("推理部件携带流式标记与思考时长，仅最后一个部件在流式中标记为正在思考", () => {
    const source = message("assistant", "streaming", [
      { id: "p1", kind: "reasoning", seq: 1, data: { text: "前面的思考", durationMs: 5200 } },
      { id: "p2", kind: "reasoning", seq: 2, data: { text: "最新的思考" } },
    ]);
    const converted = convertMessage(source, true) as any;
    expect(converted.content[0].data).toMatchObject({ streaming: false, durationMs: 5200 });
    expect(converted.content[1].data).toMatchObject({ streaming: true });

    // 消息结束后流式标记消失。
    const done = convertMessage({ ...source, status: "complete" }, true) as any;
    expect(done.content[1].data.streaming).toBe(false);
  });
});
