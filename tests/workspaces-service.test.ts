// 工作区服务测试：项目重命名（改名契约 / 重名与空名拒绝 / 内置目录保护）与删除保护。
// 只测公共接缝：WorkspacesService.rename / remove。
import { beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB } from "../src/main/store/db";
import type { Bus } from "../src/main/core/bus";
import { Recorder } from "../src/main/core/record";
import { ensureConversationWorkspace, WorkspaceService } from "../src/main/services/workspaces";
import { upsertWorkspace, getWorkspace } from "../src/main/store/workspaces";
import { createThread, getThread } from "../src/main/store/threads";

let dir: string;
let db: DB;
let service: WorkspaceService;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "cw-ws-"));
  db = DB.open(join(dir, "test.db"));
  const recorder = new Recorder(db, { emit() {} } as unknown as Bus);
  service = new WorkspaceService(db, recorder, join(dir, "backups"));
});

function cleanup(): void {
  try {
    db.close();
  } catch {
    /* 忽略 */
  }
  rmSync(dir, { recursive: true, force: true });
}

function addProject(name: string): string {
  return upsertWorkspace(db, { name, path: `/tmp/ws-${name}`, trust: "restricted", purpose: "project" }).id;
}

let threadSeq = 0;

function addThread(workspaceId: string, title: string): { id: string } {
  return createThread(db, {
    id: `th_test_${threadSeq++}`,
    mode: "code",
    title,
    titleAuto: false,
    modelId: "",
    workspaceId,
    status: "idle",
    archived: false,
  });
}

describe("项目重命名", () => {
  it("改名生效并返回新视图", () => {
    const id = addProject("旧名");
    const view = service.rename(id, "新名");
    expect(view.name).toBe("新名");
    expect(getWorkspace(db, id).name).toBe("新名");
    cleanup();
  });

  it("名称两端空白被去除", () => {
    const id = addProject("旧名");
    const view = service.rename(id, "  新名  ");
    expect(view.name).toBe("新名");
    cleanup();
  });

  it("与其他项目重名时拒绝", () => {
    addProject("已占用");
    const id = addProject("原名");
    expect(() => service.rename(id, "已占用")).toThrow("已有同名项目");
    expect(getWorkspace(db, id).name).toBe("原名");
    cleanup();
  });

  it("与自身同名时允许（无操作语义）", () => {
    const id = addProject("同名");
    expect(service.rename(id, "同名").name).toBe("同名");
    cleanup();
  });

  it("空名拒绝", () => {
    const id = addProject("原名");
    expect(() => service.rename(id, "   ")).toThrow("项目名称不能为空");
    cleanup();
  });

  it("内置临时会话目录不可重命名", () => {
    const ws = ensureConversationWorkspace(db, dir);
    expect(() => service.rename(ws.id, "新名")).toThrow("内置临时会话目录不可重命名");
    cleanup();
  });

  it("不存在的工作区拒绝", () => {
    expect(() => service.rename("ws_missing", "新名")).toThrow();
    cleanup();
  });
});

describe("项目删除", () => {
  it("内置临时会话目录不可删除", () => {
    const ws = ensureConversationWorkspace(db, dir);
    expect(() => service.remove(ws.id)).toThrow("内置临时会话目录不可移除");
    cleanup();
  });

  it("普通项目可删除，其下会话级联删除", () => {
    const id = addProject("待删");
    const t1 = addThread(id, "会话一");
    const t2 = addThread(id, "会话二");
    service.remove(id);
    expect(() => getWorkspace(db, id)).toThrow();
    expect(() => getThread(db, t1.id)).toThrow();
    expect(() => getThread(db, t2.id)).toThrow();
    cleanup();
  });

  it("删除项目不影响其他项目的会话", () => {
    const keepId = addProject("保留项目");
    const id = addProject("待删项目");
    const kept = addThread(keepId, "保留会话");
    addThread(id, "删除会话");
    service.remove(id);
    expect(getThread(db, kept.id).title).toBe("保留会话");
    cleanup();
  });
});
