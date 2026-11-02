import { describe, expect, it } from "vitest";
import type { Batch, Envelope, ThreadView } from "../lib/types";
import { EVENT } from "../lib/types";
import { reduceThread, type ThreadSlice, updateThread } from "./threadReducer";

const thread: ThreadView = {
  id: "th",
  mode: "code",
  title: "测试会话",
  modelId: "openai/gpt-5.1",
  modelName: "GPT-5.1",
  workspaceId: "ws",
  status: "idle",
  archived: false,
  tokens: 0,
  contextTokens: 0,
  contextCacheTokens: 0,
  contextSpeed: 0,
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

function envelope(type: string, data: unknown, seq = 1, threadId = "th"): Envelope {
  return { seq, type, threadId, runId: "run_1", ts: 0, data };
}

function batch(...events: Envelope[]): Batch {
  return { v: 1, threadId: "th", events };
}

describe("reduceThread", () => {
  it("合并文本增量并在完成事件后保留完整文本", () => {
    const state = baseState();
    const started = reduceThread(state, batch(envelope(EVENT.runStarted, { messageId: "msg_1" })), "th");
    expect(started.running).toBe(true);
    expect(started.runningMessageID).toBe("msg_1");

    const delta1 = reduceThread(started, batch(envelope(EVENT.textDelta, { partId: "p_text", text: "你" }, 2)), "th");
    const delta2 = reduceThread(delta1, batch(envelope(EVENT.textDelta, { partId: "p_text", text: "好" }, 3)), "th");
    const completed = reduceThread(
      delta2,
      batch(envelope(EVENT.textCompleted, { partId: "p_text", text: "你好" }, 4)),
      "th",
    );
    const finished = reduceThread(completed, batch(envelope(EVENT.runCompleted, { status: "done" }, 5)), "th");

    expect(finished.messages[0]?.parts[0]?.data.text).toBe("你好");
    expect(finished.messages[0]?.status).toBe("complete");
    expect(finished.running).toBe(false);
    expect(finished.threads.code[0]?.status).toBe("idle");
    expect(finished.refreshNeeded).toBe(true);
  });

  it("审批请求进入待审批列表并切换会话状态", () => {
    const started = reduceThread(baseState(), batch(envelope(EVENT.runStarted, { messageId: "msg_1" })), "th");
    const requested = reduceThread(
      started,
      batch(
        envelope(
          EVENT.approvalRequested,
          {
            partId: "p_ap",
            approvalId: "ap_1",
            tool: "fs_write",
            risk: "R1",
            title: "写入文件",
            target: "note.txt",
            status: "waiting",
            allowPermanent: true,
          },
          2,
        ),
      ),
      "th",
    );

    expect(requested.pendingApprovals).toHaveLength(1);
    expect(requested.threads.code[0]?.status).toBe("awaiting");
    expect(requested.messages[0]?.parts[0]?.kind).toBe("approval");

    const resolved = reduceThread(
      requested,
      batch(
        envelope(
          EVENT.approvalResolved,
          {
            partId: "p_ap",
            approvalId: "ap_1",
            status: "approved",
            decision: "allow",
            scope: "session",
          },
          3,
        ),
      ),
      "th",
    );

    expect(resolved.pendingApprovals).toHaveLength(0);
    expect(resolved.messages[0]?.parts[0]?.data.status).toBe("approved");
    expect(resolved.threads.code[0]?.status).toBe("running");
  });

  it("usage.updated 累加会话 tokens", () => {
    const next = reduceThread(
      baseState(),
      batch(envelope(EVENT.usageUpdated, { inputTokens: 10, outputTokens: 5 })),
      "th",
    );
    expect(next.threads.code[0]?.tokens).toBe(15);
  });

  it("usage.updated 覆盖上下文占用与速度，缺字段时保持原值", () => {
    // 第一轮带上下文字段：覆盖为实测值。
    const first = reduceThread(
      baseState(),
      batch(
        envelope(EVENT.usageUpdated, {
          inputTokens: 100,
          outputTokens: 20,
          contextTokens: 120,
          cacheTokens: 90,
          tokensPerSecond: 42.5,
        }),
      ),
      "th",
    );
    expect(first.threads.code[0]?.contextTokens).toBe(120);
    expect(first.threads.code[0]?.contextCacheTokens).toBe(90);
    expect(first.threads.code[0]?.contextSpeed).toBe(42.5);

    // 第二轮是不带上下文字段的 sidecar 请求（如自动命名）：tokens 照常累加，
    // 但占用水位与速度不被 sidecar 小请求覆盖。
    const second = reduceThread(
      first,
      batch(envelope(EVENT.usageUpdated, { inputTokens: 30, outputTokens: 5 }, 2)),
      "th",
    );
    expect(second.threads.code[0]?.tokens).toBe(155);
    expect(second.threads.code[0]?.contextTokens).toBe(120);
    expect(second.threads.code[0]?.contextSpeed).toBe(42.5);

    // 第三轮主对话再次携带字段：覆盖为最新一轮。
    const third = reduceThread(
      second,
      batch(
        envelope(
          EVENT.usageUpdated,
          {
            inputTokens: 200,
            outputTokens: 50,
            contextTokens: 250,
            cacheTokens: 180,
            tokensPerSecond: 60,
          },
          3,
        ),
      ),
      "th",
    );
    expect(third.threads.code[0]?.contextTokens).toBe(250);
    expect(third.threads.code[0]?.contextCacheTokens).toBe(180);
    expect(third.threads.code[0]?.contextSpeed).toBe(60);
  });

  it("其他会话的 thread.updated 只标记刷新，不修改当前会话", () => {
    const result = reduceThread(
      baseState(),
      batch(envelope(EVENT.threadUpdated, { threadId: "other", status: "idle" }, 1, "other")),
      "th",
    );
    expect(result.refreshNeeded).toBe(true);
    expect(result.threads.code[0]?.status).toBe("idle");
  });

  it("终端与全局事件不修改会话状态", () => {
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

  it("user.message 插入待送达的引导消息并按 id 去重", () => {
    const started = reduceThread(
      baseState(),
      batch(envelope(EVENT.runStarted, { messageId: "msg_run" })),
      "th",
    );
    const steer = (seq: number) =>
      envelope(EVENT.userMessage, { messageId: "msg_steer", text: "补充指令", attachments: [], via: "steer" }, seq);

    const steered = reduceThread(started, batch(steer(2)), "th");
    const steerMessage = steered.messages.find((message) => message.id === "msg_steer");
    expect(steerMessage?.role).toBe("user");
    expect(steerMessage?.steerQueued).toBe(true);
    expect(steerMessage?.parts[0]?.data.text).toBe("补充指令");
    expect(steerMessage?.parts[0]?.data.attachments).toEqual([]);

    // 同一条引导重复推送（多窗口、事件重放）按消息 id 去重。
    const deduped = reduceThread(steered, batch(steer(3)), "th");
    expect(deduped.messages).toHaveLength(steered.messages.length);
  });

  it("待送达标记在流式与工具事件到达时清除，审批事件不影响", () => {
    const started = reduceThread(
      baseState(),
      batch(envelope(EVENT.runStarted, { messageId: "msg_run" })),
      "th",
    );
    const steered = reduceThread(
      started,
      batch(
        envelope(EVENT.userMessage, { messageId: "msg_steer", text: "补充指令", attachments: [], via: "steer" }, 2),
      ),
      "th",
    );

    // 审批对应的是引导之前已经发起的工具调用，不代表引导已投递。
    const approval = reduceThread(
      steered,
      batch(
        envelope(
          EVENT.approvalRequested,
          { partId: "p_ap", approvalId: "ap_1", tool: "fs_write", risk: "R1", title: "写入", target: "note.txt", status: "waiting", allowPermanent: true },
          3,
        ),
      ),
      "th",
    );
    expect(approval.messages.find((message) => message.id === "msg_steer")?.steerQueued).toBe(true);

    // 流式增量到达，说明引导已被并入模型请求。
    const delivered = reduceThread(
      approval,
      batch(envelope(EVENT.textDelta, { partId: "p_text", text: "收到" }, 4)),
      "th",
    );
    expect(delivered.messages.find((message) => message.id === "msg_steer")?.steerQueued).toBe(false);
  });

  it("工具开始与运行终态同样清除待送达标记", () => {
    const steer = (seq: number) =>
      envelope(EVENT.userMessage, { messageId: "msg_steer", text: "补充指令", attachments: [], via: "steer" }, seq);

    const started = reduceThread(baseState(), batch(envelope(EVENT.runStarted, { messageId: "msg_run" })), "th");
    const steered = reduceThread(started, batch(steer(2)), "th");
    const tool = reduceThread(
      steered,
      batch(
        envelope(EVENT.toolStarted, { partId: "p_tool", toolCallId: "c1", name: "fs_list", args: {}, risk: "R0", status: "running", summary: "." }, 3),
      ),
      "th",
    );
    expect(tool.messages.find((message) => message.id === "msg_steer")?.steerQueued).toBe(false);

    // 取消后未投递的引导留在会话里，但不再有「即将投递」语义。
    const steeredAgain = reduceThread(started, batch(steer(2)), "th");
    const canceled = reduceThread(
      steeredAgain,
      batch(envelope(EVENT.runCanceled, { status: "canceled" }, 3)),
      "th",
    );
    expect(canceled.messages.find((message) => message.id === "msg_steer")?.steerQueued).toBe(false);
  });

  it("未知事件类型被忽略", () => {
    const result = reduceThread(baseState(), batch(envelope("future.event", { anything: true })), "th");
    expect(result.messages).toHaveLength(0);
    expect(result.running).toBe(false);
  });
});

describe("updateThread", () => {
  it("只修改命中的会话", () => {
    const threads = updateThread({ code: [thread], work: [] }, "th", { title: "新标题" });
    expect(threads.code[0]?.title).toBe("新标题");
    expect(thread.title).toBe("测试会话");
  });
});
