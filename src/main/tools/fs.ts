// fs_* / apply_patch 内置工具：读行号窗口、glob/grep 限额、写前备份、补丁预演与逐块应用。
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, extname, join, relative as relPath } from "node:path";
import { isBinary, writeFileAtomic, sha256Hex } from "../core/platform";
import { humanSize, isIgnoredDir, type Tool, type ToolResult, MAX_READ_LINES, MAX_READ_BYTES } from "./registry";

function str(args: Record<string, unknown>, key: string): string {
  return String(args[key] ?? "");
}

function int(args: Record<string, unknown>, key: string, fallback: number): number {
  const raw = Number(args[key]);
  return Number.isFinite(raw) ? Math.trunc(raw) : fallback;
}

function errResult(message: string, code = "tool_error"): ToolResult {
  return { output: message, isError: true, code };
}

// ---------- fs_list ----------

export const fsList: Tool = {
  spec: {
    name: "fs_list",
    description: "列出目录内容或查看文件信息（默认一层，最多三层）。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "相对工作区的路径，默认工作区根" },
        depth: { type: "integer", minimum: 1, maximum: 3, description: "目录深度，默认 1" },
      },
    },
    risk: "R0",
    pathArg: "path",
    work: true,
    title: "",
    summaryArg: "path",
  },
  async execute(ctx) {
    const depth = Math.min(3, Math.max(1, int(ctx.args, "depth", 1)));
    let st;
    try {
      st = statSync(ctx.target);
    } catch {
      return errResult(`路径不存在：${ctx.display}`);
    }
    if (!st.isDirectory()) {
      return { output: `${ctx.display}\t${humanSize(st.size)}`, isError: false, code: "", meta: { path: ctx.target } };
    }
    const lines: string[] = [];
    const walk = (dir: string, displayDir: string, level: number): void => {
      let entries;
      try {
        entries = readdirSync(dir, { withFileTypes: true });
      } catch {
        return;
      }
      const sorted = entries
        .filter((e) => e.name !== "." && (e.isDirectory() ? !e.name.startsWith(".") && !isIgnoredDir(e.name) : !e.name.startsWith(".") || e.name === ".env.example"))
        .sort((a, b) => (a.isDirectory() === b.isDirectory() ? a.name.localeCompare(b.name) : a.isDirectory() ? -1 : 1));
      for (const entry of sorted) {
        const abs = join(dir, entry.name);
        const display = displayDir ? `${displayDir}/${entry.name}` : entry.name;
        if (entry.isDirectory()) {
          lines.push(`${"  ".repeat(level)}${display}/`);
          if (level + 1 < depth) walk(abs, display, level + 1);
        } else {
          let size = 0;
          try {
            size = statSync(abs).size;
          } catch {
            size = 0;
          }
          lines.push(`${"  ".repeat(level)}${display}\t${humanSize(size)}`);
        }
        if (lines.length > 2000) return;
      }
    };
    walk(ctx.target, "", 0);
    return { output: lines.join("\n") || "（空目录）", isError: false, code: "", meta: { path: ctx.target } };
  },
};

// ---------- fs_read ----------

export const fsRead: Tool = {
  spec: {
    name: "fs_read",
    description: "读取文本文件内容，带行号；支持 offset/limit 分页（默认最多 2000 行）。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "相对工作区的文件路径" },
        offset: { type: "integer", minimum: 0, description: "起始行（从 1 计），默认 1" },
        limit: { type: "integer", minimum: 1, maximum: 2000, description: "读取行数，默认 2000" },
      },
      required: ["path"],
    },
    risk: "R0",
    pathArg: "path",
    work: true,
    title: "",
    summaryArg: "path",
  },
  async execute(ctx) {
    let st;
    try {
      st = statSync(ctx.target);
    } catch {
      return errResult(`文件不存在：${ctx.display}`);
    }
    if (st.isDirectory()) return errResult(`${ctx.display} 是目录，请使用 fs_list`);
    if (st.size > 8 * 1024 * 1024) {
      return errResult(`文件过大（${humanSize(st.size)}），超过 8MB 读取上限`);
    }
    const raw = readFileSync(ctx.target);
    if (isBinary(raw)) {
      return {
        output: `二进制文件 ${ctx.display}\n大小: ${humanSize(st.size)}\nSHA-256: ${sha256Hex(raw)}`,
        isError: false,
        code: "",
        meta: { path: ctx.target, binary: true },
      };
    }
    const capped = raw.length > MAX_READ_BYTES ? raw.subarray(0, MAX_READ_BYTES).toString("utf8") : raw.toString("utf8");
    const lines = capped.split("\n");
    if (capped.endsWith("\n")) lines.pop();
    const offset = Math.max(0, int(ctx.args, "offset", 1) - 1);
    const limit = Math.min(MAX_READ_LINES, Math.max(1, int(ctx.args, "limit", MAX_READ_LINES)));
    if (offset >= lines.length) {
      return errResult(`offset=${offset + 1} 超出文件行数（共 ${lines.length} 行）`);
    }
    const end = Math.min(lines.length, offset + limit);
    const out: string[] = [];
    for (let i = offset; i < end; i++) {
      out.push(`${String(i + 1).padStart(5, " ")}│ ${lines[i] ?? ""}`);
    }
    let output = out.join("\n");
    if (end < lines.length) {
      output += `\n…（还有 ${lines.length - end} 行，使用 offset=${end + 1} 继续读取）`;
    }
    return { output, isError: false, code: "", meta: { path: ctx.target } };
  },
};

// ---------- fs_glob / fs_grep ----------

function walkFiles(root: string, cb: (abs: string, rel: string) => void): void {
  const stack: Array<{ dir: string; prefix: string }> = [{ dir: root, prefix: "" }];
  while (stack.length > 0) {
    const item = stack.pop()!;
    let entries;
    try {
      entries = readdirSync(item.dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (entry.name.startsWith(".")) continue;
      const abs = join(item.dir, entry.name);
      const rel = item.prefix ? `${item.prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) {
        if (!isIgnoredDir(entry.name)) stack.push({ dir: abs, prefix: rel });
      } else {
        cb(abs, rel);
      }
    }
  }
}

/** glob 匹配：模式含「星号星号斜杠」时取后缀匹配 base 名与相对路径后缀；否则整路径或 base 名匹配。 */
function matchGlob(pattern: string, rel: string): boolean {
  const marker = pattern.indexOf("**/");
  if (marker >= 0) {
    const suffix = pattern.slice(marker + 3);
    const base = basename(rel);
    if (globToRegExp(suffix).test(base)) return true;
    const trimmed = suffix.startsWith("*") ? suffix.slice(1) : suffix;
    return rel.endsWith(trimmed);
  }
  return globToRegExp(pattern).test(rel) || globToRegExp(pattern).test(basename(rel));
}

function globToRegExp(glob: string): RegExp {
  let source = "";
  for (let i = 0; i < glob.length; i++) {
    const ch = glob[i]!;
    if (ch === "*") {
      if (glob[i + 1] === "*") {
        source += ".*";
        i++;
      } else {
        source += "[^/]*";
      }
    } else if (ch === "?") {
      source += "[^/]";
    } else {
      source += ch.replace(/[.+^${}()|[\]\\]/g, "\\$&");
    }
  }
  return new RegExp(`^${source}$`);
}

export const fsGlob: Tool = {
  spec: {
    name: "fs_glob",
    description: "按 glob 模式搜索文件（如 *.ts、src/**/*.go），最多返回 200 条。",
    schema: {
      type: "object",
      properties: {
        pattern: { type: "string", description: "glob 模式" },
        path: { type: "string", description: "搜索根目录，默认工作区根" },
      },
      required: ["pattern"],
    },
    risk: "R0",
    pathArg: "path",
    work: true,
    title: "",
    summaryArg: "pattern",
  },
  async execute(ctx) {
    const pattern = str(ctx.args, "pattern");
    if (!pattern) return errResult("pattern 不能为空");
    const matches: string[] = [];
    walkFiles(ctx.target, (_abs, rel) => {
      if (matches.length < 200 && matchGlob(pattern, rel)) matches.push(rel);
    });
    matches.sort();
    return {
      output: matches.length > 0 ? matches.join("\n") : "没有匹配的文件",
      isError: false,
      code: "",
      meta: { path: ctx.target, count: matches.length },
    };
  },
};

export const fsGrep: Tool = {
  spec: {
    name: "fs_grep",
    description: "在工作区内搜索文本（默认子串匹配，可选正则），最多返回 200 条。",
    schema: {
      type: "object",
      properties: {
        pattern: { type: "string", description: "搜索内容" },
        path: { type: "string", description: "搜索根目录，默认工作区根" },
        regex: { type: "boolean", description: "按正则解释 pattern，默认 false" },
      },
      required: ["pattern"],
    },
    risk: "R0",
    pathArg: "path",
    work: true,
    title: "",
    summaryArg: "pattern",
  },
  async execute(ctx) {
    const pattern = str(ctx.args, "pattern");
    if (!pattern) return errResult("pattern 不能为空");
    const useRegex = Boolean(ctx.args["regex"]);
    let regex: RegExp | null = null;
    if (useRegex) {
      try {
        regex = new RegExp(pattern, "g");
      } catch (err) {
        return errResult(`正则无效：${err instanceof Error ? err.message : String(err)}`);
      }
    }
    const needle = pattern;
    const results: string[] = [];
    walkFiles(ctx.target, (abs, rel) => {
      if (results.length >= 200) return;
      let st;
      try {
        st = statSync(abs);
      } catch {
        return;
      }
      if (st.size > 1024 * 1024) return;
      const raw = readFileSync(abs);
      if (isBinary(raw)) return;
      const lines = raw.toString("utf8").split("\n");
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i] ?? "";
        const hit = regex ? (regex.lastIndex = 0, regex.test(line)) : line.includes(needle);
        if (hit) {
          results.push(`${rel}:${i + 1}: ${line.trim()}`);
          if (results.length >= 200) return;
        }
      }
    });
    return {
      output: results.length > 0 ? results.join("\n") : "没有匹配的内容",
      isError: false,
      code: "",
      meta: { path: ctx.target, count: results.length },
    };
  },
};

// ---------- fs_write / fs_edit ----------

export const fsWrite: Tool = {
  spec: {
    name: "fs_write",
    description: "写入（或创建）文件；已存在文件会先自动备份。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "相对工作区的文件路径" },
        content: { type: "string", description: "完整文件内容" },
      },
      required: ["path", "content"],
    },
    risk: "R1",
    pathArg: "path",
    work: true,
    title: "写入文件",
    summaryArg: "path",
  },
  async execute(ctx) {
    const content = str(ctx.args, "content");
    let existing: Buffer | null = null;
    if (existsSync(ctx.target)) {
      existing = readFileSync(ctx.target);
      await ctx.backup(existing);
    }
    try {
      mkdirSync(dirname(ctx.target), { recursive: true, mode: 0o755 });
      writeFileAtomic(ctx.target, Buffer.from(content, "utf8"), 0o644);
    } catch (err) {
      return errResult(`写入失败：${err instanceof Error ? err.message : String(err)}`);
    }
    const size = Buffer.byteLength(content, "utf8");
    return {
      output: `已写入 ${ctx.display}（${humanSize(size)}）`,
      isError: false,
      code: "",
      meta: { path: ctx.target, bytes: size, backup: existing !== null },
    };
  },
};

export const fsEdit: Tool = {
  spec: {
    name: "fs_edit",
    description: "精确替换文件中的唯一文本片段；old_string 必须在文件中恰好出现一次。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "相对工作区的文件路径" },
        old_string: { type: "string", description: "要替换的原文（必须唯一）" },
        new_string: { type: "string", description: "替换后的内容" },
      },
      required: ["path", "old_string", "new_string"],
    },
    risk: "R1",
    pathArg: "path",
    work: true,
    title: "修改文件",
    summaryArg: "path",
  },
  async execute(ctx) {
    const oldStr = str(ctx.args, "old_string");
    const newStr = str(ctx.args, "new_string");
    if (!oldStr) return errResult("old_string 不能为空");
    const raw = readFileSync(ctx.target);
    const text = raw.toString("utf8");
    const count = text.split(oldStr).length - 1;
    if (count === 0) {
      return errResult("没有找到要替换的内容；请先读取文件确认原文");
    }
    if (count > 1) {
      return errResult(`匹配到 ${count} 处相同内容，请提供更长的唯一片段`);
    }
    await ctx.backup(raw);
    writeFileAtomic(ctx.target, Buffer.from(text.replace(oldStr, newStr), "utf8"), 0o644);
    return {
      output: `已替换 ${ctx.display} 中的 1 处内容（${oldStr.slice(0, 40)} → ${newStr.slice(0, 40)}）`,
      isError: false,
      code: "",
      meta: { path: ctx.target },
    };
  },
};

// ---------- apply_patch ----------

interface PatchHunk {
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: Array<{ tag: " " | "-" | "+"; text: string }>;
}

function parsePatch(patch: string): PatchHunk[] {
  const normalized = patch.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop(); // 补丁常以换行结尾
  const hunks: PatchHunk[] = [];
  let current: PatchHunk | null = null;
  const headerRe = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

  for (const line of lines) {
    const match = headerRe.exec(line);
    if (match) {
      current = {
        oldStart: Number(match[1]),
        oldCount: match[2] === undefined ? 1 : Number(match[2]),
        newStart: Number(match[3]),
        newCount: match[4] === undefined ? 1 : Number(match[4]),
        lines: [],
      };
      hunks.push(current);
      continue;
    }
    if (!current) continue; // @@ 之前的内容忽略
    if (line.startsWith("\\")) continue; // "\ No newline" 提示行
    if (line === "") {
      current.lines.push({ tag: " ", text: "" });
      continue;
    }
    const tag = line[0] as " " | "-" | "+";
    if (tag === " " || tag === "-" || tag === "+") {
      current.lines.push({ tag, text: line.slice(1) });
    } else {
      current.lines.push({ tag: " ", text: line });
    }
  }
  // 校验每块行数与声明一致
  for (const [i, hunk] of hunks.entries()) {
    const del = hunk.lines.filter((l) => l.tag !== "+").length;
    const add = hunk.lines.filter((l) => l.tag !== "-").length;
    if (del !== hunk.oldCount || add !== hunk.newCount) {
      throw new Error(`第 ${i + 1} 个补丁块的旧/新内容行数不符`);
    }
  }
  return hunks;
}

/** 应用单个 hunk：优先按修正位置匹配，找不到从文件头重搜（容忍行号偏移）；纯新增块直接插入。 */
function applyHunk(fileLines: string[], hunk: PatchHunk, delta: number): number {
  const oldLines = hunk.lines.filter((l) => l.tag !== "+").map((l) => l.text);
  const newLines = hunk.lines.filter((l) => l.tag !== "-").map((l) => l.text);

  if (oldLines.length === 0) {
    // 纯新增块：在 oldStart-1+delta 处插入
    const at = Math.max(0, Math.min(fileLines.length, hunk.oldStart - 1 + delta));
    fileLines.splice(at, 0, ...newLines);
    return newLines.length;
  }

  const matchesAt = (at: number): boolean => {
    if (at < 0 || at + oldLines.length > fileLines.length) return false;
    return oldLines.every((line, i) => fileLines[at + i] === line);
  };

  let pos = -1;
  const preferred = hunk.oldStart - 1 + delta;
  if (matchesAt(preferred)) pos = preferred;
  else {
    for (let at = 0; at <= fileLines.length - oldLines.length; at++) {
      if (matchesAt(at)) {
        pos = at;
        break;
      }
    }
  }
  if (pos < 0) throw new Error("补丁上下文与文件内容不匹配，无法应用");
  fileLines.splice(pos, oldLines.length, ...newLines);
  return newLines.length - oldLines.length;
}

export const applyPatch: Tool = {
  spec: {
    name: "apply_patch",
    description:
      "对已存在的文件应用统一 diff（unified diff）补丁；支持 dry_run 预演与按块（hunks，从 1 起）选择性应用。",
    schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "目标文件（相对工作区）" },
        patch: { type: "string", description: "统一 diff 补丁文本" },
        dry_run: { type: "boolean", description: "只校验不写入，默认 false" },
        hunks: { type: "array", items: { type: "integer" }, description: "要应用的补丁块序号（从 1 起），默认全部" },
      },
      required: ["path", "patch"],
    },
    risk: "R1",
    pathArg: "path",
    work: true,
    title: "应用补丁",
    summaryArg: "path",
  },
  async execute(ctx) {
    const patchText = str(ctx.args, "patch");
    if (!patchText.trim()) return errResult("patch 不能为空");
    let st;
    try {
      st = statSync(ctx.target);
    } catch {
      return errResult(`apply_patch 只修改已有文件，目标不存在：${ctx.display}`);
    }
    if (st.isDirectory()) return errResult(`${ctx.display} 是目录`);

    let hunks: PatchHunk[];
    try {
      hunks = parsePatch(patchText);
    } catch (err) {
      return errResult(err instanceof Error ? err.message : String(err));
    }
    if (hunks.length === 0) return errResult("补丁中没有可应用的内容块");

    const selected = (Array.isArray(ctx.args["hunks"]) ? (ctx.args["hunks"] as unknown[]) : [])
      .map((v) => Math.trunc(Number(v)))
      .filter((v) => Number.isFinite(v));
    const uniqueSelected = [...new Set(selected)].sort((a, b) => a - b);
    if (uniqueSelected.some((n) => n < 1 || n > hunks.length)) {
      return errResult(`hunks 序号超出范围（共 ${hunks.length} 块）`);
    }
    const chosen = uniqueSelected.length > 0 ? uniqueSelected.map((n) => hunks[n - 1]!) : hunks;

    const original = readFileSync(ctx.target);
    const text = original.toString("utf8");
    const fileLines = text.split("\n");
    let delta = 0;
    let added = 0;
    let removed = 0;
    try {
      for (const hunk of chosen) {
        const oldLen = hunk.lines.filter((l) => l.tag !== "+").length;
        const newLen = hunk.lines.filter((l) => l.tag !== "-").length;
        delta += applyHunk(fileLines, hunk, delta);
        added += newLen - Math.min(newLen, oldLen) + Math.max(0, newLen - oldLen);
        removed += Math.max(0, oldLen - newLen);
      }
    } catch (err) {
      return errResult(err instanceof Error ? err.message : String(err));
    }

    if (ctx.args["dry_run"] === true) {
      return { output: `补丁预演通过：将应用 ${chosen.length} 个补丁块到 ${ctx.display}。未写入文件。`, isError: false, code: "" };
    }

    await ctx.backup(original);
    writeFileAtomic(ctx.target, Buffer.from(fileLines.join("\n"), "utf8"), 0o644);
    return {
      output: `已应用 ${chosen.length} 个补丁块到 ${ctx.display}（+${added} −${removed} 行）。`,
      isError: false,
      code: "",
      meta: { path: ctx.target },
    };
  },
};

// 供相对路径展示复用
void relPath;
void extname;
