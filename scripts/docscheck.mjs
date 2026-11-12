#!/usr/bin/env node
// 文档引用校验：只检查机器能判定的引用，不检查语义。
//
// 校验三类引用（在 DOCS 列出的文档里）：
//   1. 反引号包住的仓库路径 —— 完整路径或可后缀匹配到唯一仓库路径
//   2. `pnpm <script>` —— 脚本名必须存在于 package.json
//   3. 决策号 D<n> —— 必须存在于 docs/arch.md 的决策表
//   4. 内置工具名 —— 必须出现在 craft-tools 的工具名里
//
// 用法：node scripts/docscheck.mjs

import { readFileSync, existsSync, readdirSync } from "node:fs";
import { dirname, join, resolve, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const DOCS = ["AGENTS.md", "docs/agents/pitfalls.md", "docs/agents/recipes.md"];

// 预期不存在的路径：生成物（被 gitignore）与数据目录下的运行时文件。
const EXPECTED_ABSENT = [
  "out/", "dist/", "target/", "node_modules/",
  "backups/", "attachments/", "logs/", "workspace/default",
];

// 不是仓库路径的领域标识符。
const NOT_PATHS = new Set([
  "provider/model", "providerID/modelID", "registry/model", "<数据目录>",
  "插件名:技能名", "<市场>", "<插件>", "<版本>", "<会话id>", "<会话短码>",
  "./", "keys.json", "craftwork.db",
]);

const SKIP_DIRS = new Set([".git", "node_modules", "target", "out", "dist", ".DS_Store"]);

const failures = [];
const fail = (file, msg) => failures.push(`${file}: ${msg}`);
/** 仓库内全部路径（相对路径，目录以 / 结尾）。 */
function walk(dir, acc) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return acc;
  }
  for (const e of entries) {
    if (SKIP_DIRS.has(e.name)) continue;
    const abs = join(dir, e.name);
    // 文档里的仓库路径统一用 /；Windows 的 relative() 返回反斜杠，不归一化会导致全部误报
    const rel = relative(root, abs).split(sep).join("/");
    if (e.isDirectory()) {
      acc.add(`${rel}/`);
      walk(abs, acc);
    } else {
      acc.add(rel);
    }
  }
  return acc;
}

const allPaths = walk(root, new Set());
const allList = [...allPaths];

/** 路径是否可在仓库里找到：完整匹配，或以 / 为边界的后缀匹配。 */
function resolvesPath(p) {
  const norm = p.replace(/\/+$/, "");
  if (allPaths.has(p) || allPaths.has(norm) || allPaths.has(`${norm}/`)) return true;
  return allList.some((candidate) => candidate.replace(/\/+$/, "").endsWith(`/${norm}`));
}

const packageScripts = new Set(
  Object.keys(JSON.parse(readFileSync(join(root, "package.json"), "utf8")).scripts ?? {}),
);

const arch = readFileSync(join(root, "docs/arch.md"), "utf8");
const decisionIds = new Set([...arch.matchAll(/^\|\s*(D\d+)\b/gm)].map((m) => m[1]));

/** craft-tools 里声明的工具名：spec 的 `name: "xxx"` 与 NAME 常量两种写法。 */
const toolNames = new Set();
for (const mod of readdirSync(join(root, "crates/craft-tools/src"))) {
  if (!mod.endsWith(".rs")) continue;
  const text = readFileSync(join(root, "crates/craft-tools/src", mod), "utf8");
  for (const m of text.matchAll(/name:\s*"([a-z0-9_]+)"/g)) toolNames.add(m[1]);
  for (const m of text.matchAll(/NAME:\s*&'?[a-z ]*str\s*=\s*"([a-z0-9_]+)"/g)) toolNames.add(m[1]);
}
// MCP 工具名是运行时拼的：mcp__<服务器>__<工具>
const MCP_PATTERN = /^mcp__.+__.+$/;

const TOOL_LIKE = /^(fs|git|doc|xlsx|shell|bash|kill|todo|apply|enter|exit|agent|skill)_[a-z0-9_]+$/;

const inlineCode = (text) => [...text.matchAll(/`([^`\n]+)`/g)].map((m) => m[1].trim());

for (const file of DOCS) {
  const abs = join(root, file);
  if (!existsSync(abs)) {
    fail(file, "文件不存在");
    continue;
  }
  const text = readFileSync(abs, "utf8");

  for (const raw of inlineCode(text)) {
    // 逗号/分号后的解释性从句不是路径的一部分；冒号只在不是 `name:sub` 形式的脚本名时截断。
    const code = raw
      .replace(/^"(.*)"$/, "$1")
      .replace(/[，,;；].*$/, "")
      .replace(/(?<!^pnpm [\w.-]*):.*$/, "")
      .trim();

    // 1. 仓库路径
    const looksLikePath =
      (code.includes("/") || /\.(rs|ts|tsx|md|mjs|json|gz|py|toml|css|conf)$/.test(code)) &&
      !code.startsWith("http") &&
      !code.includes(" ") &&
      !code.includes("*") &&
      !code.includes("<") &&
      !code.startsWith("~") &&
      !code.startsWith("/") &&
      !NOT_PATHS.has(code) &&
      !EXPECTED_ABSENT.includes(code);

    if (looksLikePath && !resolvesPath(code)) {
      fail(file, `路径不存在：\`${code}\``);
    }

    // 2. pnpm 脚本
    const pnpm = code.match(/^pnpm\s+(?:run\s+)?([a-zA-Z][\w:.-]*)/);
    if (pnpm && !packageScripts.has(pnpm[1])) {
      fail(file, `package.json 里没有脚本：pnpm ${pnpm[1]}`);
    }

    // 3. 内置工具名
    if (TOOL_LIKE.test(code) && !toolNames.has(code) && !MCP_PATTERN.test(code)) {
      fail(file, `craft-tools 里没有工具：\`${code}\``);
    }
  }

  // 4. 决策号（反引号内与正文裸写都查）
  for (const m of text.matchAll(/\bD(\d+)\b/g)) {
    const id = `D${m[1]}`;
    if (!decisionIds.has(id)) fail(file, `docs/arch.md 里没有决策 ${id}`);
  }
}

// 5. crate 清单完整性：仓库里每个 crates/craft-* 都要在 AGENTS.md 的归属表里出现。
//    这条针对的腐坏方式是可预测的——新增 crate 后忘记更新文档（craft-git / craft-marketplace 就这么漏过）。
const crateDirs = readdirSync(join(root, "crates"), { withFileTypes: true })
  .filter((e) => e.isDirectory() && e.name.startsWith("craft-"))
  .map((e) => e.name);

const agentsText = existsSync(join(root, "AGENTS.md"))
  ? readFileSync(join(root, "AGENTS.md"), "utf8")
  : "";
for (const crate of crateDirs) {
  if (!agentsText.includes(`\`${crate}\``)) {
    fail("AGENTS.md", `crates/ 下的 ${crate} 没有出现在归属表里`);
  }
}

if (failures.length) {
  console.error(`✗ 文档引用校验失败（${failures.length} 处）：`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(
  `✓ 文档引用校验通过（${DOCS.length} 份文档 / ${decisionIds.size} 条决策 / ${toolNames.size} 个工具名 / ${allPaths.size} 条仓库路径）`,
);
