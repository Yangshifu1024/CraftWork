import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Batch, ChatMessage, ThreadSnapshot, ThreadView } from "../lib/types";
import { EVENT } from "../lib/types";

const mocks = vi.hoisted(() => ({
  decideApproval: vi.fn().mockResolvedValue(undefined),
  list: vi.fn().mockResolvedValue([]),
  snapshotPage: vi.fn(),
}));

vi.mock("../lib/api", () => ({
  api: {
    threads: {
      list: mocks.list,
      snapshotPage: mocks.snapshotPage,
    },
    chat: {
      decideApproval: mocks.decideApproval,
      send: vi.fn(),
      cancel: vi.fn(),
      regenerate: vi.fn(),
    },
    settings: { get: vi.fn(), save: vi.fn() },
    providers: { list: vi.fn().mockResolvedValue([]), warnings: vi.fn().mockResolvedValue([]) },
    app: { info: vi.fn() },
    workspace: { list: vi.fn().mockResolvedValue([]), active: vi.fn().mockResolvedValue(null) },
  },
}));

vi.mock("../lib/bus", () => ({
  onBatch: () => () => undefined,
  dispatch: vi.fn(),
  isDesktop: false,
  envelope: vi.fn(),
}));

const { useThreadStore } = await import("./threads");
const { useAppStore } = await import("./app");

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

function envelope(type: string, data: any, seq = 1) {
  return { seq, type, threadId: "th", runId: "run_1", ts: 0, data };
}

function batch(...events: ReturnType<typeof envelope>[]): Batch {
  return { v: 1, threadId: "th", events };
}

function message(id: string, role: ChatMessage["role"], parts: ChatMessage["parts"]): ChatMessage {
  return { id, threadId: "th", role, status: "complete", createdAt: 1, parts };
}

beforeEach(() => {
  mocks.decideApproval.mockClear();
  mocks.list.mockClear();
  mocks.snapshotPage.mockReset();
  useAppStore.setState({ mode: "code" });
  useThreadStore.setState({
    threads: { code: [thread], work: [] },
    activeID: { code: "th", work: null },
    messages: [],
    running: false,
    runningMessageID: null,
    loading: false,
    pendingApprovals: [],
    searchResults: null,
    attachments: [],
    connected: true,
    hasEarlier: false,
    loadingEarlier: false,
  });
});

describe("会话事件归约", () => {
  it("合并文本增量并在完成事件后写入完整文本", () => {
    const store = useThreadStore.getState();
    store.applyBatch(batch(envelope(EVENT.runStarted, { messageId: "msg_1" })));
    expect(useThreadStore.getState().running).toBe(true);
    expect(useThreadStore.getState().messages[0]?.id).toBe("msg_1");

    store.applyBatch(batch(envelope(EVENT.textDelta, { partId: "p_text", text: "你" }, 2)));
    store.applyBatch(batch(envelope(EVENT.textDelta, { partId: "p_text", text: "好" }, 3)));
    expect(useThreadStore.getState().messages[0]?.parts[0]?.data.text).toBe("你好");

    store.applyBatch(batch(envelope(EVENT.textCompleted, { partId: "p_text", text: "你好" }, 4)));
    store.applyBatch(batch(envelope(EVENT.runCompleted, { status: "done" }, 5)));
    expect(useThreadStore.getState().running).toBe(false);
    expect(useThreadStore.getState().messages[0]?.status).toBe("complete");
  });

  it("维护审批等待、批准与待审批列表", async () => {
    const store = useThreadStore.getState();
    store.applyBatch(batch(envelope(EVENT.runStarted, { messageId: "msg_1" })));
    store.applyBatch(
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
    );
    expect(useThreadStore.getState().pendingApprovals).toHaveLength(1);
    expect(useThreadStore.getState().threads.code[0]?.status).toBe("awaiting");

    await useThreadStore.getState().decide("ap_1", true, "session");
    expect(mocks.decideApproval).toHaveBeenCalledWith("ap_1", true, "session");
    const approved = useThreadStore.getState().messages[0]?.parts[0];
    expect(approved?.data.status).toBe("approved");

    store.applyBatch(
      batch(
        envelope(EVENT.approvalResolved, {
          partId: "p_ap",
          approvalId: "ap_1",
          status: "approved",
          decision: "allow",
          scope: "session",
        }, 3),
      ),
    );
    expect(useThreadStore.getState().pendingApprovals).toHaveLength(0);
  });

  it("处理工具、提示、错误与运行失败", () => {
    const store = useThreadStore.getState();
    store.applyBatch(batch(envelope(EVENT.runStarted, { messageId: "msg_1" })));
    store.applyBatch(batch(envelope(EVENT.toolStarted, {
      partId: "p_tool",
      toolCallId: "call_1",
      name: "fs_read",
      args: { path: "a.txt" },
      risk: "R0",
      status: "running",
      summary: "a.txt",
    }, 2)));
    store.applyBatch(batch(envelope(EVENT.toolCompleted, {
      partId: "p_tool",
      toolCallId: "call_1",
      name: "fs_read",
      args: { path: "a.txt" },
      risk: "R0",
      status: "done",
      output: "hello",
      isError: false,
    }, 3)));
    store.applyBatch(batch(envelope(EVENT.notice, { partId: "p_notice", text: "上下文已压缩" }, 4)));
    store.applyBatch(batch(envelope(EVENT.error, { partId: "p_error", code: "rate_limit", message: "限流" }, 5)));

    const parts = useThreadStore.getState().messages[0]?.parts ?? [];
    expect(parts.find((part) => part.id === "p_tool")?.kind).toBe("tool");
    expect(parts.find((part) => part.id === "p_tool")?.data.status).toBe("done");
    expect(parts.find((part) => part.id === "p_notice")?.kind).toBe("notice");
    expect(parts.find((part) => part.id === "p_error")?.kind).toBe("error");
    expect(parts.find((part) => part.id === "p_error")?.data.code).toBe("rate_limit");

    store.applyBatch(batch(envelope(EVENT.runFailed, { status: "error", code: "rate_limit" }, 6)));
    expect(useThreadStore.getState().running).toBe(false);
    expect(useThreadStore.getState().messages[0]?.status).toBe("error");
  });
});

describe("会话分页", () => {
  it("选择会话时加载最新一页并记录 hasEarlier", async () => {
    const snapshot: ThreadSnapshot = {
      thread: { ...thread, mode: "code" as const },
      messages: [message("m_latest", "user", [{ id: "p1", kind: "text", seq: 10, data: { text: "最新" } }])],
      lastSeq: 10,
      approvals: [],
      hasMore: true,
      oldestSeq: 10,
    };
    mocks.snapshotPage.mockResolvedValue(snapshot);

    await useThreadStore.getState().select("th");
    expect(mocks.snapshotPage).toHaveBeenCalledWith("th", 80, 0);
    expect(useThreadStore.getState().hasEarlier).toBe(true);
    expect(useThreadStore.getState().messages).toHaveLength(1);
  });

  it("向前加载时按最旧序号请求并去重", async () => {
    useThreadStore.setState({
      messages: [message("m_latest", "user", [{ id: "p1", kind: "text", seq: 100, data: { text: "最新" } }])],
      hasEarlier: true,
    });
    mocks.snapshotPage.mockResolvedValue({
      thread: { ...thread, mode: "code" as const },
      messages: [message("m_older", "user", [{ id: "p0", kind: "text", seq: 50, data: { text: "更早" } }])],
      lastSeq: 100,
      approvals: [],
      hasMore: false,
      oldestSeq: 50,
    });

    await useThreadStore.getState().loadEarlier();
    expect(mocks.snapshotPage).toHaveBeenCalledWith("th", 80, 100);
    expect(useThreadStore.getState().messages.map((item) => item.id)).toEqual(["m_older", "m_latest"]);
    expect(useThreadStore.getState().hasEarlier).toBe(false);
  });
});
