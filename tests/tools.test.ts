// 工具层测试：路径越界、fs 读写、补丁、授权前缀。
import { describe, expect, it, beforeEach, afterAll } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DB } from "../src/main/store/db";
import { Registry } from "../src/main/tools/registry";
import { fsList, fsRead, fsWrite, fsEdit, applyPatch, fsGlob } from "../src/main/tools/fs";
import { ErrOutsideWorkspace, resolveAt } from "../src/main/policy/resolve";
import { commandPrefix, Grants } from "../src/main/agent/grants";

let dir: string;
let db: DB;
let registry: Registry;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "cw-tools-"));
  mkdirSync(join(dir, "sub"), { recursive: true });
  writeFileSync(join(dir, "hello.txt"), "line1\nline2\nline3\n");
  writeFileSync(join(dir, "sub", "a.ts"), "export const a = 1;\n");
  db = DB.open(join(dir, "tools.db"));
  registry = new Registry({ backupDir: join(dir, "backups"), db });
  registry.register(fsList);
  registry.register(fsRead);
  registry.register(fsWrite);
  registry.register(fsEdit);
  registry.register(fsGlob);
  registry.register(applyPatch);
});

afterAll(() => {
  try {
    db.close();
  } catch {
    // 忽略
  }
  rmSync(dir, { recursive: true, force: true });
});

describe("路径安全", () => {
  it("工作区内路径解析", () => {
    const resolved = resolveAt(dir, "hello.txt");
    expect(resolved.abs.endsWith("hello.txt")).toBe(true);
  });

  it("拒绝 ../ 越界（即使目标不存在）", () => {
    expect(() => resolveAt(dir, "../outside.txt")).toThrow(ErrOutsideWorkspace);
    expect(() => resolveAt(dir, "sub/../../etc")).toThrow(ErrOutsideWorkspace);
  });

  it("绝对路径在界内合法、界外拒绝", () => {
    expect(resolveAt(dir, join(dir, "hello.txt")).abs).toBeTruthy();
    expect(() => resolveAt(dir, "/etc/passwd")).toThrow(ErrOutsideWorkspace);
  });
});

describe("fs 工具", () => {
  it("fs_read 带行号窗口", async () => {
    const assessment = registry.assess([dir], "fs_read", { path: "hello.txt" });
    expect(assessment.risk).toBe("R0");
    const result = await registry.execute(assessment.prepared!);
    expect(result.output).toContain("1│ line1");
    expect(result.isError).toBe(false);
  });

  it("fs_write 越界路径被拦截为 R3 且不执行", async () => {
    const assessment = registry.assess([dir], "fs_write", { path: "../evil.txt", content: "x" });
    expect(assessment.risk).toBe("R3");
    expect(assessment.prepared?.err).toBeTruthy();
    const result = await registry.execute(assessment.prepared!);
    expect(result.isError).toBe(true);
  });

  it("fs_write 写入并登记备份", async () => {
    writeFileSync(join(dir, "exist.txt"), "old");
    const assessment = registry.assess([dir], "fs_write", { path: "exist.txt", content: "new" });
    const result = await registry.execute(assessment.prepared!);
    expect(result.isError).toBe(false);
    expect(readFileSync(join(dir, "exist.txt"), "utf8")).toBe("new");
    const backups = db.prepare("SELECT COUNT(*) AS n FROM backups").get() as { n: number };
    expect(backups.n).toBe(1);
  });

  it("fs_edit 要求唯一匹配", async () => {
    const ok = registry.assess([dir], "fs_edit", { path: "hello.txt", old_string: "line2", new_string: "LINE2" });
    const okResult = await registry.execute(ok.prepared!);
    expect(okResult.isError).toBe(false);
    const dup = registry.assess([dir], "fs_edit", { path: "hello.txt", old_string: "line", new_string: "X" });
    const dupResult = await registry.execute(dup.prepared!);
    expect(dupResult.isError).toBe(true);
    expect(dupResult.output).toContain("2 处");
  });

  it("fs_glob 后缀匹配", async () => {
    const assessment = registry.assess([dir], "fs_glob", { pattern: "**/*.ts" });
    const result = await registry.execute(assessment.prepared!);
    expect(result.output).toContain("sub/a.ts");
  });

  it("apply_patch 预演与应用", async () => {
    const patch = `@@ -1,3 +1,3 @@\n line1\n-line2\n+LINE2\n line3\n`;
    const dry = registry.assess([dir], "apply_patch", { path: "hello.txt", patch, dry_run: true });
    const dryResult = await registry.execute(dry.prepared!);
    expect(dryResult.output).toContain("未写入文件");
    expect(readFileSync(join(dir, "hello.txt"), "utf8")).toContain("line2");

    const real = registry.assess([dir], "apply_patch", { path: "hello.txt", patch });
    const realResult = await registry.execute(real.prepared!);
    expect(realResult.isError).toBe(false);
    expect(readFileSync(join(dir, "hello.txt"), "utf8")).toContain("LINE2");
  });
});

describe("授权前缀与缓存", () => {
  it("commandPrefix 规则", () => {
    expect(commandPrefix("git status")).toBe("git status");
    expect(commandPrefix("npm test")).toBe("npm test"); // 第二段非选项：前两段
    expect(commandPrefix("ls")).toBe("");
    expect(commandPrefix("sudo rm x")).toBe("");
    expect(commandPrefix("bash -c 'x'")).toBe("");
    expect(commandPrefix("a; b")).toBe("");
    expect(commandPrefix("echo $(whoami)")).toBe("");
  });

  it("会话授权命中", () => {
    const grants = new Grants(db);
    expect(grants.allowed("fs_write", "/tmp/x")).toBe(false);
    grants.grant("fs_write", "/tmp/x", "session");
    expect(grants.allowed("fs_write", "/tmp/x")).toBe(true);
  });

  it("shell 前缀授权命中", () => {
    const grants = new Grants(db);
    grants.grant("shell_exec", "prefix:npm", "session");
    expect(grants.allowed("shell_exec", "prefix:npm")).toBe(true);
    expect(grants.allowed("shell_exec", "prefix:git")).toBe(false);
  });
});
