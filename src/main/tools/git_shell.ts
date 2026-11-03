// shell_exec / git_* / todo_write 内置工具。
import { run, type RunResult } from "../core/platform";
import type { Tool, ToolResult, ToolCallCtx } from "./registry";
import { readFileSync, statSync } from "node:fs";
import { writeFileAtomic } from "../core/platform";
import { docReadFile, docReplace, docSupported } from "../doc";
import { diff as computeDiff } from "../core/editor";

function str(args: Record<string, unknown>, key: string): string {
  return String(args[key] ?? "");
}

function errResult(message: string, code = "tool_error"): ToolResult {
  return { output: message, isError: true, code };
}

// ---------- shell_exec ----------

export const shellExec: Tool = {
  spec: {
    name: "shell_exec",
    description:
      "在工作区内执行 shell 命令（/bin/sh -c）。输出上限 32KB，超时上限 600 秒（默认 120）。命令在清洗后的环境中运行。",
    schema: {
      type: "object",
      properties: {
        command: { type: "string", description: "要执行的命令" },
        cwd: { type: "string", description: "相对工作区的运行目录，默认工作区根" },
        timeout: { type: "integer", minimum: 1, maximum: 600, description: "超时秒数，默认 120" },
      },
      required: ["command"],
    },
    risk: "R2",
    pathArg: "cwd",
    commandArg: "command",
    work: false,
    title: "执行命令",
    summaryArg: "command",
  },
  async execute(ctx) {
    const command = str(ctx.args, "command").trim();
    if (!command) return errResult("command 不能为空");
    const timeoutSec = Math.min(600, Math.max(1, Math.trunc(Number(ctx.args["timeout"] ?? 120)) || 120));
    const started = Date.now();
    const result = await run({
      file: "/bin/sh",
      args: ["-c", command],
      dir: ctx.target || ctx.workspace,
      timeoutMs: timeoutSec * 1000,
    });
    const duration = Date.now() - started;
    if (result.timedOut) {
      return {
        output: `命令超时（${Math.round(duration / 1000)}s）：${result.output}`,
        isError: true,
        code: "tool_error",
        meta: { exitCode: -1, timedOut: true },
      };
    }
    if (result.exitCode < 0 && result.error) {
      return errResult(`执行命令失败：${result.error}`);
    }
    const output = result.output.replace(/\n+$/, "") || "(无输出)";
    return {
      output: `$ ${command}\n${output}\n[退出码 ${result.exitCode} · ${Math.round(duration / 1000)}s]`,
      isError: result.exitCode !== 0,
      code: result.exitCode !== 0 ? "tool_error" : "",
      meta: { exitCode: result.exitCode },
    };
  },
};

// ---------- git_* ----------

async function git(ctx: ToolCallCtx, args: string[], timeoutSec = 30): Promise<{ result: RunResult; output: string }> {
  const result = await run({ file: "git", args, dir: ctx.workspace, timeoutMs: timeoutSec * 1000 });
  return { result, output: result.output };
}

export const gitStatus: Tool = {
  spec: {
    name: "git_status",
    description: "查看主要工作目录的 Git 状态（分支与未提交改动）。",
    schema: { type: "object", properties: {} },
    risk: "R0",
    work: false,
    title: "",
    summaryText: "git status",
  },
  async execute(ctx) {
    const { result } = await git(ctx, ["status", "--short", "--branch"], 20);
    if (result.exitCode !== 0) {
      return errResult(`git status 失败：${result.output.trim() || "目录不是 Git 仓库"}`);
    }
    const out = result.output.trim();
    return { output: out || "工作区干净（没有未提交的改动）", isError: false, code: "", meta: { path: ctx.workspace } };
  },
};

export const gitDiff: Tool = {
  spec: {
    name: "git_diff",
    description: "查看文件或整个仓库的 Git 差异（默认未暂存；staged=true 查看已暂存）。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "限制差异范围的路径" },
        staged: { type: "boolean", description: "查看已暂存的差异，默认 false" },
      },
    },
    risk: "R0",
    pathArg: "path",
    work: false,
    title: "",
    summaryArg: "path",
    summaryText: "git diff",
  },
  async execute(ctx) {
    const args = ["diff", "--no-color"];
    if (ctx.args["staged"] === true) args.push("--staged");
    if (ctx.display) args.push("--", ctx.display);
    const { result } = await git(ctx, args);
    if (result.exitCode !== 0) return errResult(`git diff 失败：${result.output.trim()}`);
    return {
      output: result.output.trim() || "没有差异",
      isError: false,
      code: "",
      meta: { staged: ctx.args["staged"] === true },
    };
  },
};

export const gitLog: Tool = {
  spec: {
    name: "git_log",
    description: "查看提交记录（oneline + 修饰，默认最近 20 条，最多 100 条）。",
    schema: {
      type: "object",
      properties: {
        limit: { type: "integer", minimum: 1, maximum: 100, description: "条数，默认 20" },
        path: { type: "string", description: "限制历史范围的路径" },
      },
    },
    risk: "R0",
    pathArg: "path",
    work: false,
    title: "",
    summaryArg: "path",
    summaryText: "git log",
  },
  async execute(ctx) {
    const limit = Math.min(100, Math.max(1, Math.trunc(Number(ctx.args["limit"] ?? 20)) || 20));
    const args = ["log", "--oneline", "--decorate", "-n", String(limit)];
    if (ctx.display) args.push("--", ctx.display);
    const { result } = await git(ctx, args);
    if (result.exitCode !== 0) return errResult(`git log 失败：${result.output.trim()}`);
    return { output: result.output.trim() || "还没有提交记录", isError: false, code: "", meta: { limit } };
  },
};

export const gitCommit: Tool = {
  spec: {
    name: "git_commit",
    description: "提交代码（默认不推送）。有 paths 时先 add 再提交这些文件；否则提交全部已跟踪改动。",
    schema: {
      type: "object",
      properties: {
        message: { type: "string", description: "提交信息" },
        paths: { type: "array", items: { type: "string" }, description: "要提交的文件路径（相对工作区）" },
      },
      required: ["message"],
    },
    risk: "R1",
    work: false,
    title: "提交代码",
    summaryArg: "message",
  },
  async execute(ctx) {
    const message = str(ctx.args, "message").trim();
    if (!message) return errResult("message 不能为空");
    if (message.includes("\0") || Buffer.byteLength(message, "utf8") > 2000) {
      return errResult("提交信息无效（为空、含空字符或超过 2000 字节）");
    }
    const pathsArg = Array.isArray(ctx.args["paths"]) ? (ctx.args["paths"] as unknown[]).map(String) : [];
    const paths: string[] = [];
    for (const p of pathsArg) {
      try {
        const resolved = ctx.resolveExtra(p);
        paths.push(resolved.display);
      } catch (err) {
        return errResult(err instanceof Error ? err.message : String(err));
      }
    }
    if (paths.length > 0) {
      const add = await git(ctx, ["add", "--", ...paths]);
      if (add.result.exitCode !== 0) return errResult(`提交失败: ${add.result.output.trim()}`);
      const commit = await git(ctx, ["commit", "-m", message, "--", ...paths]);
      if (commit.result.exitCode !== 0) return errResult(`提交失败: ${commit.result.output.trim()}`);
    } else {
      const commit = await git(ctx, ["commit", "-am", message]);
      if (commit.result.exitCode !== 0) return errResult(`提交失败: ${commit.result.output.trim()}`);
    }
    const hash = await git(ctx, ["rev-parse", "--short", "HEAD"], 10);
    const hashOut = hash.result.exitCode === 0 ? hash.result.output.trim() : "";
    const firstLine = message.split("\n")[0] ?? message;
    return {
      output: `已提交 ${hashOut}：${firstLine}\n（未推送）`,
      isError: false,
      code: "",
      meta: { hash: hashOut, message, paths },
    };
  },
};

// ---------- doc_read / doc_edit / xlsx_set_cell ----------

export const docRead: Tool = {
  spec: {
    name: "doc_read",
    description: "读取 Word（docx）/ Excel（xlsx）/ PDF 的文本内容（文本级，不支持 OCR）。",
    schema: {
      type: "object",
      properties: { path: { type: "string", description: "文档路径（相对工作区）" } },
      required: ["path"],
    },
    risk: "R0",
    pathArg: "path",
    work: true,
    title: "",
    summaryArg: "path",
  },
  async execute(ctx) {
    if (!docSupported(ctx.target)) {
      return errResult(`不支持的文档格式：${ctx.display}`);
    }
    try {
      const doc = await docReadFile(ctx.target);
      const note = doc.note ? `说明：${doc.note}\n\n` : "";
      const text = doc.text || "文档没有可提取的文本";
      return {
        output: `${note}${text}`,
        isError: false,
        code: "",
        meta: { path: ctx.target, kind: doc.kind, truncated: doc.truncated },
      };
    } catch (err) {
      return errResult(err instanceof Error ? err.message : String(err));
    }
  },
};

export const docEdit: Tool = {
  spec: {
    name: "doc_edit",
    description: "替换 Word / Excel 文档中的一处文本（保留原有结构；写入前自动备份）。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "文档路径（相对工作区）" },
        old_text: { type: "string", description: "要替换的原文" },
        new_text: { type: "string", description: "替换后的文本" },
      },
      required: ["path", "old_text", "new_text"],
    },
    risk: "R1",
    pathArg: "path",
    work: true,
    title: "修改文档",
    summaryArg: "path",
  },
  async execute(ctx) {
    const oldText = str(ctx.args, "old_text");
    if (!oldText) return errResult("old_text 不能为空");
    if (!docSupported(ctx.target)) return errResult(`不支持的文档格式：${ctx.display}`);
    const original = readFileSync(ctx.target);
    await ctx.backup(original);
    try {
      await docReplace(ctx.target, { kind: "text", old: oldText, new: str(ctx.args, "new_text") });
    } catch (err) {
      return errResult(err instanceof Error ? err.message : String(err));
    }
    return { output: `已替换 ${ctx.display} 中的 1 处文本。`, isError: false, code: "", meta: { path: ctx.target } };
  },
};

export const xlsxSetCell: Tool = {
  spec: {
    name: "xlsx_set_cell",
    description:
      "更新 Excel 单元格（如 B8）。value 以 = 开头写入公式（不计算）；数字写为数值；其余写为文本。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "xlsx 路径（相对工作区）" },
        sheet: { type: "string", description: "工作表名称或从 1 起的序号，默认第一个" },
        cell: { type: "string", description: "单元格引用，如 B8" },
        value: { type: "string", description: "要写入的值" },
      },
      required: ["path", "cell", "value"],
    },
    risk: "R1",
    pathArg: "path",
    work: true,
    title: "修改表格",
    summaryArg: "path",
  },
  async execute(ctx) {
    const cell = str(ctx.args, "cell").trim();
    if (!cell) return errResult("cell 不能为空");
    if (!docSupported(ctx.target)) return errResult(`不支持的文档格式：${ctx.display}`);
    const original = readFileSync(ctx.target);
    await ctx.backup(original);
    try {
      await docReplace(ctx.target, {
        kind: "cell",
        sheet: str(ctx.args, "sheet"),
        cell,
        value: str(ctx.args, "value"),
      });
    } catch (err) {
      return errResult(err instanceof Error ? err.message : String(err));
    }
    return {
      output: `已更新 ${ctx.display} 的单元格 ${cell.toUpperCase()}。`,
      isError: false,
      code: "",
      meta: { path: ctx.target },
    };
  },
};

// ---------- todo_write ----------

export const todoWrite: Tool = {
  spec: {
    name: "todo_write",
    description: "更新任务清单（任务级进度展示给用户）。",
    schema: {
      type: "object",
      properties: {
        items: {
          type: "array",
          description: "任务列表",
          items: {
            type: "object",
            properties: {
              text: { type: "string" },
              status: { type: "string", enum: ["pending", "active", "done"] },
            },
            required: ["text", "status"],
          },
        },
      },
      required: ["items"],
    },
    risk: "R0",
    work: true,
    title: "",
    summaryText: "更新任务清单",
  },
  async execute(ctx) {
    const items = Array.isArray(ctx.args["items"]) ? (ctx.args["items"] as unknown[]) : [];
    if (items.length === 0) return errResult("items 不能为空");
    const mark = { pending: "○", active: "◐", done: "●" } as const;
    const lines = items.slice(0, 50).map((raw) => {
      const item = (raw ?? {}) as { text?: unknown; status?: unknown };
      const text = String(item.text ?? "").slice(0, 200);
      const status = item.status === "active" || item.status === "done" ? item.status : "pending";
      return `${mark[status]} ${text}`;
    });
    return { output: lines.join("\n"), isError: false, code: "", meta: { items: items.length } };
  },
};

// 编辑器保存路径也要 diff（供服务层复用）
export { computeDiff, statSync, writeFileAtomic };
