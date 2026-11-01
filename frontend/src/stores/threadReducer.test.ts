import { describe, expect, it } from "vitest";
import type { Batch, Envelope, ThreadView } from "../lib/types";
import { EVENT } from "../lib/types";
import { reduceThread, updateThread, type ThreadSlice } from "./threadReducer";

const thread: ThreadView = {
  id: "th",
  mode: "code",
  title: "测试线程",
  modelId: "openai/gpt-5.1",
  modelName: "GPT-5.1",
  workspaceId: "ws",
  status: "idle",
  archived: false,
  tokens: 0,
  createdAt: 1,
  updatedAt: 1,
};

function baseState(): ThreadSlice {
  return {
    messages: [],
    running: false,
    runningMessageID: null,
    pendingApprovals: [],
    threads: { code: [thread], work: [] },
  };
}

function envelope(
  type: string,
  data: unknown,
  seq = 1,
  threadId = "th",
): Envelope {
  return { seq, type, threadId, runId: "run_1", ts: 0, data };
}

function batch(...events: Envelope[]): Batch {
  return { v: 1, threadId: "th", events };
}

describe("reduceThread", () => {
  it("合并文本增量并在完成事件后保留完整文本", () => {
    const state = baseState();
    const started = reduceThread(
      state,
      batch(envelope(EVENT.runStarted, { messageId: "msg_1" })),
      "th",
    );
    expect(started.running).toBe(true);
    expect(started.runningMessageID).toBe("msg_1");

    const delta1 = reduceThread(
      started,
      batch(envelope(EVENT.textDelta, { partId: "p_text", text: "你" }, 2)),
      "th",
    );
    const delta2 = reduceThread(
      delta1,
      batch(envelope(EVENT.textDelta, { partId: "p_text", text: "好" }, 3)),
      "th",
    );
    const completed = reduceThread(
      delta2,
      batch(envelope(EVENT.textCompleted, { partId: "p_text", text: "你好" }, 4)),
      "th",
    );
    const finished = reduceThread(
      completed,
      batch(envelope(EVENT.runCompleted, { status: "done" }, 5)),
      "th",
    );

    expect(finished.messages[0]?.parts[0]?.data.text).toBe("你好");
    expect(finished.messages[0]?.status).toBe("complete");
    expect(finished.running).toBe(false);
    expect(finished.threads.code[0]?.status).toBe("idle");
    expect(finished.refreshNeeded).toBe(true);
  });

  it("审批请求进入待审批列表并切换线程状态", () => {
    const started = reduceThread(
      baseState(),
      batch(envelope(EVENT.runStarted, { messageId: "msg_1" })),
      "th",
    );
    const requested = reduceThread(
      started,
      batch(
        envelope(EVENT.approvalRequested, {
          partId: "p_ap",
          approvalId: "ap_1",
          tool: "fs_write",
          risk: "R1",
          title: "写入文件",
          target: "note.txt",
          status: "waiting",
          allowPermanent: true,
        }, 2),
      ),
      "th",
    );

    expect(requested.pendingApprovals).toHaveLength(1);
    expect(requested.threads.code[0]?.status).toBe("awaiting");
    expect(requested.messages[0]?.parts[0]?.kind).toBe("approval");

    const resolved = reduceThread(
      requested,
      batch(
        envelope(EVENT.approvalResolved, {
          partId: "p_ap",
          approvalId: "ap_1",
          status: "approved",
          decision: "allow",
          scope: "session",
        }, 3),
      ),
      "th",
    );

    expect(resolved.pendingApprovals).toHaveLength(0);
    expect(resolved.messages[0]?.parts[0]?.data.status).toBe("approved");
    expect(resolved.threads.code[0]?.status).toBe("running");
  });

  it("usage.updated 累加线程 tokens", () => {
    const next = reduceThread(
      baseState(),
      batch(envelope(EVENT.usageUpdated, { inputTokens: 10, outputTokens: 5 })),
      "th",
    );
    expect(next.threads.code[0]?.tokens).toBe(15);
  });

  it("其他线程的 thread.updated 只标记刷新，不修改当前线程", () => {
    const result = reduceThread(
      baseState(),
      batch(envelope(EVENT.threadUpdated, { threadId: "other", status: "idle" }, 1, "other")),
      "th",
    );
    expect(result.refreshNeeded).toBe(true);
    expect(result.threads.code[0]?.status).toBe("idle");
  });

  it("终端与全局事件不修改线程状态", () => {
    const result = reduceThread(
      baseState(),
      batch(
        envelope(EVENT.terminalOutput, { sessionId: "s1", data: "x" }, 1, "terminal:s1"),
        envelope(EVENT.settingsUpdated, { theme: "light" }, 2),
        envelope(EVENT.workspaceUpdated, { id: "ws" }, 3),
        envelope(EVENT.contextCompressed, { summary: "摘要" }, 4),
      ),
      "th",
    );
    expect(result.messages).toHaveLength(0);
    expect(result.threads.code[0]?.tokens).toBe(0);
    expect(result.refreshNeeded).toBe(false);
  });

  it("未知事件类型被忽略", () => {
    const result = reduceThread(
      baseState(),
      batch(envelope("future.event", { anything: true })),
      "th",
    );
    expect(result.messages).toHaveLength(0);
    expect(result.running).toBe(false);
  });
});

describe("updateThread", () => {
  it("只修改命中的线程", () => {
    const threads = updateThread({ code: [thread], work: [] }, "th", { title: "新标题" });
    expect(threads.code[0]?.title).toBe("新标题");
    expect(thread.title).toBe("测试线程");
  });
});
