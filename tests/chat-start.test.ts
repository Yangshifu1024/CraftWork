// 「创建并首发」原子路径：服务层测试（真实 SQLite 临时目录 + 桩 Runner）。
// 只验证外部行为：落库结果、返回值与抛错；不窥探内部调用。
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB } from "../src/main/store/db";
import { getThread, listThreads } from "../src/main/store/threads";
import { listEvents } from "../src/main/store/events";
import { upsertWorkspace } from "../src/main/store/workspaces";
import { Bus } from "../src/main/core/bus";
import { Recorder } from "../src/main/core/record";
import { Grants } from "../src/main/agent/grants";
import { Approvals } from "../src/main/agent/approvals";
import type { Runner } from "../src/main/agent/runner";
import { Catalog, saveCustomProviders, type PersistedProvider } from "../src/main/model/catalog";
import { DEFAULT_CAPS } from "../src/main/model/types";
import { ChatService } from "../src/main/services/chat";

const MODEL_REF = "prov/model";

function makeProvider(): PersistedProvider {
  return {
    id: "prov",
    name: "测试提供商",
    kind: "openai-compatible",
    endpoint: "https://example.invalid/v1",
    keyRequired: false,
    note: "",
    source: "manual",
    catalogId: "",
    catalogVersion: "",
    envVars: [],
    enabled: true,
    models: [
      {
        id: "model",
        name: "测试模型",
        contextK: 8,
        maxTokens: 1024,
        caps: DEFAULT_CAPS,
        status: "",
        canonicalId: "",
        releaseDate: "",
        source: "",
        enabled: true,
      },
    ],
  };
}

let dir: string;
let db: DB;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "cw-chat-start-"));
  db = DB.open(join(dir, "test.db"));
});

function cleanup(): void {
  try {
    db.close();
  } catch {
    /* 忽略 */
  }
  rmSync(dir, { recursive: true, force: true });
}

/** 装配服务：模型已配置；runError 可注入 run 启动失败；广播事件收集进 pushed 供断言实时推送。 */
function assemble(runError?: Error): {
  chat: ChatService;
  runCalls: string[];
  pushed: Array<{ type: string; data: Record<string, unknown> }>;
} {
  const runCalls: string[] = [];
  const pushed: Array<{ type: string; data: Record<string, unknown> }> = [];
  const stub = {
    isRunning: () => false,
    run: (threadId: string) => {
      runCalls.push(threadId);
      if (runError) return { runId: "", error: runError };
      return { runId: "run_stub_1" };
    },
    trySteer: () => true,
    cancel: () => false,
  } as unknown as Runner;
  const bus = new Bus((batch) => {
    for (const event of batch.events) pushed.push({ type: event.type, data: event.data as Record<string, unknown> });
  });
  const recorder = new Recorder(db, bus);
  const approvals = new Approvals(db, recorder, new Grants(db));
  const catalog = new Catalog(db);
  const chat = new ChatService(db, recorder, stub, approvals, catalog);
  return { chat, runCalls, pushed };
}

function seededProvider(): void {
  saveCustomProviders(db, [makeProvider()]);
}

describe("创建并首发", () => {
  it("成功：短标题命名、titleAuto=false、用户消息事件存在、Run 启动、返回 threadId 与 runId", () => {
    seededProvider();
    const wsId = upsertWorkspace(db, {
      name: "临时会话",
      path: join(dir, "workspace"),
      trust: "trusted",
      purpose: "conversation",
    }).id;
    const { chat, runCalls, pushed } = assemble();
    // Bus 广播走 30ms 微批：fake timers 推进 flush 窗口后再断言
    vi.useFakeTimers();

    const result = chat.createAndStart("code", "", "帮我写个快排", [], MODEL_REF);
    vi.advanceTimersByTime(31);
    vi.useRealTimers();

    const thread = getThread(db, result.threadId);
    expect(thread.title).toBe("帮我写个快排");
    expect(thread.titleAuto).toBe(false);
    expect(thread.modelId).toBe(MODEL_REF);
    expect(thread.workspaceId).toBe(wsId);
    const events = listEvents(db, result.threadId);
    const userMessage = events.find((e) => e.kind === "user_message");
    expect(userMessage).toBeDefined();
    expect(JSON.parse(userMessage?.payload ?? "{}")).toMatchObject({ text: "帮我写个快排", attachments: [] });
    // 落库即广播 user.message（载荷带 messageId），且先于 thread.updated
    const umIdx = pushed.findIndex((p) => p.type === "user.message");
    const tuIdx = pushed.findIndex((p) => p.type === "thread.updated");
    expect(umIdx).toBeGreaterThanOrEqual(0);
    expect(pushed[umIdx]?.data["messageId"]).toBe(userMessage?.messageId);
    expect(tuIdx).toBeGreaterThan(umIdx);
    expect(runCalls).toEqual([result.threadId]);
    expect(result.runId).toBe("run_stub_1");
    cleanup();
  });

  it("modelRef 空串：抛「尚未选择模型」且零落库", () => {
    seededProvider();
    const { chat, runCalls } = assemble();

    expect(() => chat.createAndStart("code", "", "帮我写个快排", [], "")).toThrowError(
      "尚未选择模型，请先在设置中添加提供商并选择模型",
    );
    expect(listThreads(db, "code", true)).toHaveLength(0);
    expect(runCalls).toHaveLength(0);
    cleanup();
  });

  it("模型未配置（目录里无该提供商）：抛「当前模型不可用」且零落库", () => {
    // 不 seed 任何 provider
    const { chat, runCalls } = assemble();

    expect(() => chat.createAndStart("code", "", "帮我写个快排", [], MODEL_REF)).toThrowError(
      "当前模型不可用，请重新选择模型",
    );
    expect(listThreads(db, "code", true)).toHaveLength(0);
    expect(runCalls).toHaveLength(0);
    cleanup();
  });

  it("模型不可用（provider 存在但模型不存在）：抛「当前模型不可用」且零落库", () => {
    seededProvider();
    const { chat, runCalls } = assemble();

    expect(() => chat.createAndStart("code", "", "帮我写个快排", [], "prov/missing")).toThrowError(
      "当前模型不可用，请重新选择模型",
    );
    expect(listThreads(db, "code", true)).toHaveLength(0);
    expect(runCalls).toHaveLength(0);
    cleanup();
  });

  it("工作区不存在：抛可读错误且零落库", () => {
    seededProvider();
    const { chat, runCalls } = assemble();

    expect(() => chat.createAndStart("code", "ws_missing", "帮我写个快排", [], MODEL_REF)).toThrowError(
      "工作区不存在，请重新选择工作区后再新建会话",
    );
    expect(listThreads(db, "code", true)).toHaveLength(0);
    expect(runCalls).toHaveLength(0);
    cleanup();
  });

  it("短标题取不到（纯附件）：回落「新对话」且 titleAuto=true，事件携带附件", () => {
    seededProvider();
    const { chat, runCalls } = assemble();

    const result = chat.createAndStart("work", "", "", ["/tmp/demo.png"], MODEL_REF);

    const thread = getThread(db, result.threadId);
    expect(thread.title).toBe("新对话");
    expect(thread.titleAuto).toBe(true);
    expect(thread.mode).toBe("work");
    const userMessage = listEvents(db, result.threadId).find((e) => e.kind === "user_message");
    expect(JSON.parse(userMessage?.payload ?? "{}")).toMatchObject({ text: "", attachments: ["/tmp/demo.png"] });
    expect(runCalls).toEqual([result.threadId]);
    cleanup();
  });

  it("空消息且无附件：抛「消息不能为空」且零落库", () => {
    seededProvider();
    const { chat, runCalls } = assemble();

    expect(() => chat.createAndStart("code", "", "   ", [], MODEL_REF)).toThrowError("消息不能为空");
    expect(listThreads(db, "code", true)).toHaveLength(0);
    expect(runCalls).toHaveLength(0);
    cleanup();
  });

  it("Run 启动失败：不抛错，返回 runId 为空串，线程与用户消息事件保留", () => {
    seededProvider();
    const { chat, runCalls } = assemble(new Error("模型连接失败"));

    const result = chat.createAndStart("code", "", "帮我写个快排", [], MODEL_REF);

    expect(result.runId).toBe("");
    const thread = getThread(db, result.threadId);
    expect(thread.title).toBe("帮我写个快排");
    expect(listEvents(db, result.threadId).some((e) => e.kind === "user_message")).toBe(true);
    expect(runCalls).toEqual([result.threadId]);
    cleanup();
  });
});
