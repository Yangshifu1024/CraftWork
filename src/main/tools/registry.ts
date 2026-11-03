// 工具框架：集中声明每个工具的静态元数据（名称/描述/Schema/风险/路径参数）；
// 注册、策略判断与审批展示读同一份声明，新增工具只声明一次。
import type { Risk } from "../policy/engine";
import { classifyCommand } from "../policy/command";
import { resolveExisting, relative, type Resolved } from "../policy/resolve";
import { backupName, writeFileAtomic, sha256Hex } from "../core/platform";
import { addBackup } from "../store/records";
import type { DB } from "../store/db";
import { newId } from "../../shared/ids";

/** 统一输出截断（字节级；可能切坏多字节字符——可接受取舍）。 */
export const MAX_OUTPUT_BYTES = 32 * 1024;
export const MAX_READ_BYTES = 256 * 1024;
export const MAX_READ_LINES = 2000;

export function truncateOutput(text: string, limit = MAX_OUTPUT_BYTES): string {
  const buf = Buffer.from(text, "utf8");
  if (buf.length <= limit) return text;
  return `${buf.subarray(0, limit).toString("utf8")}\n…（输出已截断，原始长度 ${buf.length} 字节）`;
}

export interface ToolResult {
  output: string;
  isError: boolean;
  code: string;
  meta?: Record<string, unknown>;
}

export interface ToolSpec {
  name: string;
  description: string;
  schema: Record<string, unknown>;
  risk: Risk;
  /** 路径参数名（执行前统一解析并做越界校验） */
  pathArg?: string;
  /** 命令参数名（执行前重算风险） */
  commandArg?: string;
  /** Work 模式可用 */
  work: boolean;
  title: string;
  summaryArg?: string;
  summaryText?: string;
}

export interface ToolCallCtx {
  name: string;
  args: Record<string, unknown>;
  /** 主路径解析结果（有 pathArg 时） */
  target: string;
  display: string;
  workspace: string;
  roots: string[];
  targetRoot: string;
  /** 参数/路径校验失败原文（置位后 Execute 不再执行） */
  err: Error | null;
  /** 写前备份出口 */
  backup: (data: Uint8Array | string) => Promise<void>;
  /** 额外路径解析（git paths 等）：仅允许主要目录 */
  resolveExtra: (rel: string) => { abs: string; display: string };
}

export interface Tool {
  spec: ToolSpec;
  execute(ctx: ToolCallCtx): Promise<ToolResult> | ToolResult;
}

export interface RegistryOptions {
  backupDir: string;
  db: DB;
}

export interface Assessment {
  risk: Risk;
  reason: string;
  target: string;
  prepared: ToolCallCtx | null;
  exists: boolean;
}

const IGNORED_DIRS = new Set([
  ".git",
  "node_modules",
  "dist",
  "bin",
  ".task",
  "vendor",
  "__pycache__",
  ".venv",
  "build",
  ".next",
  ".cache",
]);

export function isIgnoredDir(name: string): boolean {
  return IGNORED_DIRS.has(name.toLowerCase());
}

export function humanSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = -1;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

export class Registry {
  private tools = new Map<string, Tool>();

  constructor(private readonly opts: RegistryOptions) {}

  register(tool: Tool): void {
    this.tools.set(tool.spec.name, tool); // MCP 动态注册：同名替换
  }

  unregister(name: string): void {
    this.tools.delete(name);
  }

  get(name: string): Tool | undefined {
    return this.tools.get(name);
  }

  specFor(name: string): ToolSpec | undefined {
    return this.tools.get(name)?.spec;
  }

  /** 按名称排序——保证提示词工具顺序稳定。 */
  specs(): ToolSpec[] {
    return [...this.tools.values()].map((t) => t.spec).sort((a, b) => a.name.localeCompare(b.name));
  }

  /** work 模式只保留 spec.work 或 mcp__ 前缀。 */
  specsForMode(mode: string): ToolSpec[] {
    if (mode !== "work") return this.specs();
    return this.specs().filter((s) => s.work || s.name.startsWith("mcp__"));
  }

  allows(mode: string, name: string): boolean {
    return this.specsForMode(mode).some((s) => s.name === name);
  }

  /** 只读评估：解析路径与命令分类，不产生副作用。 */
  assess(roots: string[], name: string, args: Record<string, unknown>): Assessment {
    const tool = this.tools.get(name);
    if (!tool) {
      return { risk: "R2", reason: "执行命令", target: "", prepared: null, exists: false };
    }
    const spec = tool.spec;
    let risk = spec.risk;
    let reason = "";
    let target = "";
    let display = "";
    let targetRoot = "";
    let err: Error | null = null;

    if (spec.commandArg) {
      const command = String(args[spec.commandArg] ?? "");
      const cls = classifyCommand(command);
      risk = cls.risk;
      reason = cls.reason;
      target = command;
    }

    if (spec.pathArg) {
      const relRaw = String(args[spec.pathArg] ?? "").trim() || ".";
      try {
        const resolved: Resolved = resolveExisting(roots, relRaw);
        target = resolved.abs;
        display = relative(roots[0] ?? "", resolved.abs);
        targetRoot = resolved.root;
        // git 系工具只支持主要目录（保持「主要目录仓库」语义）
        if (name.startsWith("git_") && resolved.rootIndex !== 0) {
          risk = "R3";
          reason = `${display} 属于附加目录，Git 操作仅支持主要目录`;
        }
      } catch (pathErr) {
        // 路径越界：风险提到 R3，Execute 不再执行
        risk = "R3";
        reason = pathErr instanceof Error ? pathErr.message : "路径超出工作区及附加目录";
        err = pathErr instanceof Error ? pathErr : new Error(String(pathErr));
      }
    }

    const ctx: ToolCallCtx = {
      name,
      args,
      target,
      display,
      workspace: roots[0] ?? "",
      roots,
      targetRoot,
      err,
      backup: async (data) => {
        if (!this.opts.backupDir || !target) return;
        const { statSync, mkdirSync } = await import("node:fs");
        const pathMod = await import("node:path");
        let size = 0;
        try {
          size = statSync(target).size;
        } catch {
          size = 0;
        }
        const backupPath = pathMod.join(this.opts.backupDir, backupName(target));
        mkdirSync(this.opts.backupDir, { recursive: true, mode: 0o700 });
        writeFileAtomic(backupPath, data, 0o600);
        addBackup(this.opts.db, {
          path: target,
          backupPath,
          hash: sha256Hex(typeof data === "string" ? Buffer.from(data, "utf8") : Buffer.from(data)),
          size,
        });
      },
      resolveExtra: (rel: string) => {
        // 额外路径（git paths）：仅允许主要目录
        const main = roots[0] ?? "";
        const { resolveAt } = require("../policy/resolve") as typeof import("../policy/resolve");
        const resolved = resolveAt(main, rel);
        return { abs: resolved.abs, display: relative(main, resolved.abs) };
      },
    };

    const summary = spec.summaryArg ? String(args[spec.summaryArg] ?? "") : (spec.summaryText ?? "");
    ctx.args = args;
    void summary;

    return {
      risk,
      reason: reason || spec.risk,
      target,
      prepared: ctx,
      exists: true,
    };
  }

  /** 统一执行出口：err 置位直接返回错误；输出统一截断。 */
  async execute(ctx: ToolCallCtx): Promise<ToolResult> {
    const tool = this.tools.get(ctx.name);
    if (!tool) return { output: `未知工具：${ctx.name}`, isError: true, code: "unknown_tool" };
    if (ctx.err) {
      return { output: ctx.err.message, isError: true, code: "tool_error" };
    }
    try {
      const result = await tool.execute(ctx);
      return { ...result, output: truncateOutput(result.output) };
    } catch (err) {
      return { output: err instanceof Error ? err.message : String(err), isError: true, code: "tool_error" };
    }
  }
}

/** 生成分页提示。 */
export function moreHint(remaining: number): string {
  return `…（还有 ${remaining} 行，使用 offset 继续读取）`;
}

export { newId };
