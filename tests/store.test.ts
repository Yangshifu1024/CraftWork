// 存储层测试：迁移、事件 seq、快照重建、审批并发保护、启动收尾。
import { describe, expect, it, beforeEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB, ErrNotFound } from "../src/main/store/db";
import { createThread, getThread, renameThread, autoTitleThread, deleteThread, listThreads } from "../src/main/store/threads";
import { appendEvent, snapshot, latestContextSummary } from "../src/main/store/events";
import { createApproval, resolveApproval, getApproval, interruptStaleRuns, createRun, finishRun } from "../src/main/store/runs";
import { upsertWorkspace, getWorkspace, replaceWorkspaceFolders } from "../src/main/store/workspaces";
import { setSetting, getSetting, addAudit, listAudit } from "../src/main/store/records";

let dir: string;
let db: DB;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "cw-store-"));
  db = DB.open(join(dir, "test.db"));
});

function cleanup(): void {
  try {
    db.close();
  } catch {
    // 忽略
  }
  rmSync(dir, { recursive: true, force: true });
}

describe("数据库", () => {
  it("迁移到当前版本且可重开", () => {
    const row = db.prepare("PRAGMA user_version").get() as { user_version: number };
    expect(row.user_version).toBe(6);
    db.close();
    const reopened = DB.open(join(dir, "test.db"));
    const again = reopened.prepare("PRAGMA user_version").get() as { user_version: number };
    expect(again.user_version).toBe(6);
    reopened.close();
    cleanup();
  });

  it("threads 基本读写与删除", () => {
    const t = createThread(db, { id: "th_1", mode: "code", title: "新对话", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    expect(getThread(db, t.id).title).toBe("新对话");
    renameThread(db, t.id, "改名");
    expect(getThread(db, t.id).titleAuto).toBe(false);
    deleteThread(db, t.id);
    expect(() => getThread(db, t.id)).toThrow(ErrNotFound);
    cleanup();
  });

  it("autoTitle 条件更新只在标题未被改动时生效", () => {
    createThread(db, { id: "th_2", mode: "code", title: "新对话", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    expect(autoTitleThread(db, "th_2", "新标题", "新对话")).toBe(true);
    expect(autoTitleThread(db, "th_2", "再改", "新对话")).toBe(false);
    cleanup();
  });

  it("事件 seq 单调递增且快照重建消息", () => {
    createThread(db, { id: "th_3", mode: "code", title: "t", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    const r1 = appendEvent(db, { threadId: "th_3", messageId: "m1", kind: "user_message", payload: { text: "你好", attachments: [] } });
    const r2 = appendEvent(db, { threadId: "th_3", messageId: "m2", kind: "text", payload: { partId: "p1", text: "回" } });
    expect(r2.seq).toBe(r1.seq + 1);
    const snap = snapshot(db, "th_3");
    // 不同 messageId 各自成一条消息：用户消息 + 助手回复
    expect(snap.messages).toHaveLength(2);
    expect(snap.messages[0]!.role).toBe("user");
    expect(snap.messages[1]!.role).toBe("assistant");
    expect(snap.messages[1]!.parts[0]!.payload["text"]).toBe("回");
    expect(snap.lastSeq).toBe(r2.seq);
    cleanup();
  });

  it("审批：并发保护与重复决定拒绝", () => {
    createThread(db, { id: "th_4", mode: "code", title: "t", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    const ap = createApproval(db, { threadId: "th_4", runId: "run_1", toolCallId: "call_1", tool: "fs_write", risk: "R1" });
    resolveApproval(db, ap.id, "allow", "session");
    expect(getApproval(db, ap.id).decision).toBe("allow");
    expect(() => resolveApproval(db, ap.id, "deny", "once")).toThrow(ErrNotFound);
    cleanup();
  });

  it("启动收尾：中断运行标记 interrupted 并写错误事件", () => {
    createThread(db, { id: "th_5", mode: "code", title: "t", titleAuto: true, modelId: "", workspaceId: "", status: "running", archived: false });
    const run = createRun(db, "th_5", "gpt-x");
    const count = interruptStaleRuns(db, (rec) => appendEvent(db, rec));
    expect(count).toBe(1);
    const stale = db.prepare("SELECT status, error_code FROM runs WHERE id = ?").get(run.id) as Record<string, unknown>;
    expect(stale["status"]).toBe("interrupted");
    const snap = snapshot(db, "th_5");
    // 中断消息标记 incomplete，且带有可重试的 error 部件
    const interruptedMsg = snap.messages.find((m) => m.parts.some((p) => p.kind === "error"));
    expect(interruptedMsg).toBeTruthy();
    expect(interruptedMsg!.status).toBe("incomplete");
    expect((interruptedMsg!.parts.find((p) => p.kind === "error")!.payload as Record<string, unknown>)["retryable"]).toBe(true);
    cleanup();
  });

  it("run 完成回填状态", () => {
    createThread(db, { id: "th_6", mode: "code", title: "t", titleAuto: true, modelId: "", workspaceId: "", status: "running", archived: false });
    const run = createRun(db, "th_6", "gpt-x");
    finishRun(db, run.id, "done", "", "");
    const row = db.prepare("SELECT status, finished_at FROM runs WHERE id = ?").get(run.id) as Record<string, unknown>;
    expect(row["status"]).toBe("done");
    expect(Number(row["finished_at"])).toBeGreaterThan(0);
    cleanup();
  });
});

describe("工作区", () => {
  it("按 path 幂等 upsert（沿用原 id）", () => {
    const a = upsertWorkspace(db, { name: "项目", path: "/tmp/ws-a", trust: "trusted", purpose: "project" });
    const b = upsertWorkspace(db, { name: "项目改名", path: "/tmp/ws-a", trust: "restricted", purpose: "conversation" });
    expect(a.id).toBe(b.id);
    // purpose 由首次创建决定
    expect(getWorkspace(db, a.id).purpose).toBe("project");
    expect(getWorkspace(db, a.id).trust).toBe("restricted");
    cleanup();
  });

  it("文件夹整体替换并同步主目录", () => {
    const ws = upsertWorkspace(db, { name: "项目", path: "/tmp/ws-b", trust: "restricted", purpose: "project" });
    replaceWorkspaceFolders(db, ws.id, ["/tmp/ws-b", "/tmp/ws-c"]);
    const loaded = getWorkspace(db, ws.id);
    expect(loaded.folders.map((f) => f.path)).toEqual(["/tmp/ws-b", "/tmp/ws-c"]);
    expect(loaded.path).toBe("/tmp/ws-b");
    cleanup();
  });
});

describe("设置与审计", () => {
  it("设置 upsert 与读取", () => {
    setSetting(db, "app", { theme: "dark" });
    expect(getSetting(db, "app", { theme: "system" })).toEqual({ theme: "dark" });
    setSetting(db, "app", { theme: "light" });
    expect(getSetting(db, "app", { theme: "system" })).toEqual({ theme: "light" });
    cleanup();
  });

  it("审计追加与倒序列表", () => {
    addAudit(db, { tool: "fs_write", risk: "R1", decision: "allow", scope: "user", detail: "a.txt" });
    addAudit(db, { tool: "shell_exec", risk: "R2", decision: "deny" });
    const list = listAudit(db, 10);
    expect(list).toHaveLength(2);
    expect(list[0]!.tool).toBe("shell_exec");
    cleanup();
  });
});

describe("会话列表", () => {
  it("按模式过滤与排序", () => {
    createThread(db, { id: "th_c1", mode: "code", title: "c1", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    createThread(db, { id: "th_w1", mode: "work", title: "w1", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    expect(listThreads(db, "code", false)).toHaveLength(1);
    expect(listThreads(db, "work", false)[0]!.id).toBe("th_w1");
    cleanup();
  });
});

describe("上下文摘要", () => {
  it("最新一条自然淘汰旧摘要", () => {
    createThread(db, { id: "th_7", mode: "code", title: "t", titleAuto: true, modelId: "", workspaceId: "", status: "idle", archived: false });
    appendEvent(db, { threadId: "th_7", messageId: "s0", kind: "context_summary", payload: { summary: "旧", untilSeq: 3 } });
    appendEvent(db, { threadId: "th_7", messageId: "s1", kind: "context_summary", payload: { summary: "新", untilSeq: 9 } });
    const summary = latestContextSummary(db, "th_7");
    expect(summary?.summary).toBe("新");
    expect(summary?.untilSeq).toBe(9);
    cleanup();
  });
});
