// 代码着色：给工具卡的 diff 逐行上色（D21）。
//
// 为什么复用 shiki 而不是另引高亮器：正文代码块已经由 @streamdown/code 用 shiki 着色，
// 同一份主题、同一套 token 颜色，diff 与正文看起来才是同一个应用的两种视图。shiki 走纯 JS
// 正则引擎，不加载 oniguruma wasm、不联网。
//
// 这里的着色是**展示增强**：任何失败都静默降级为纯文本，不能让工具卡渲染出错。
// 高亮只产出内联 style，CSP 由既有的 style-src 'unsafe-inline' 放行，不新增例外。
//
// 语言由扩展名推断（LANGUAGE_BY_EXTENSION），未收录一律纯文本——不按内容猜语言，
// 猜错比不着色更难看懂。整段文本 tokenize 后按行切分：逐行 highlight 会丢跨行语法状态。
import { createHighlighter, type BundledLanguage, type Highlighter } from "shiki";

/** 主题与 markdown 代码块保持一致（@streamdown/code 的默认双主题）。 */
const THEME_LIGHT = "github-light";
const THEME_DARK = "github-dark";

/** 单个 token 的颜色；null 表示不着色（纯文本或高亮未就绪）。 */
export interface HighlightToken {
  content: string;
  color?: string;
}

/** 扩展名 → shiki 语言 id。未收录一律按纯文本处理，不猜语言。 */
const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  mts: "typescript",
  cts: "typescript",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  py: "python",
  go: "go",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  swift: "swift",
  c: "c",
  h: "c",
  cc: "cpp",
  cpp: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  cs: "csharp",
  rb: "ruby",
  php: "php",
  json: "json",
  jsonc: "jsonc",
  css: "css",
  scss: "scss",
  less: "less",
  html: "html",
  htm: "html",
  vue: "vue",
  svelte: "svelte",
  md: "markdown",
  markdown: "markdown",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  ini: "ini",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "fish",
  sql: "sql",
  xml: "xml",
  svg: "xml",
  pat: "diff",
  diff: "diff",
};

/** 由文件路径推语言 id；无扩展名或未收录返回空串（= 纯文本）。 */
export function languageForPath(path: string): string {
  const leaf = path.split(/[\\/]/).pop() ?? "";
  const dot = leaf.lastIndexOf(".");
  if (dot <= 0 || dot === leaf.length - 1) return "";
  return LANGUAGE_BY_EXTENSION[leaf.slice(dot + 1).toLowerCase()] ?? "";
}

/** 高亮器按主题 + 语言懒建并缓存：一次会话里同一语言的 diff 反复出现，只加载一份语法。 */
const highlighters = new Map<string, Promise<Highlighter>>();

function highlighterFor(theme: string, lang: string): Promise<Highlighter> {
  const key = `${theme}:${lang}`;
  const cached = highlighters.get(key);
  if (cached) return cached;
  const pending = createHighlighter({ themes: [theme], langs: [lang] });
  highlighters.set(key, pending);
  // 失败不留在缓存里：语法加载失败（未知语言、包损坏）若被缓存成永久 rejected promise，
  // 之后同一语言的每次 diff 都只能拿到同一个失败。
  pending.catch(() => highlighters.delete(key));
  return pending;
}

/** 逐行 token 缓存：同一段文本在展开/折叠、滚动重渲染时不必重新 tokenize。 */
const tokenCache = new Map<string, HighlightToken[][]>();
const TOKEN_CACHE_LIMIT = 50;

function cacheKey(code: string, lang: string, dark: boolean): string {
  // 用首尾片段 + 长度做键：整段文本做键会让大 diff 的键本身就占几十 KB。
  return `${dark ? "d" : "l"}:${lang}:${code.length}:${code.slice(0, 64)}:${code.slice(-64)}`;
}

/** 整段代码 tokenize 后按行切分（着色必须整段做：单独 highlight 每一行会丢掉跨行语法状态）。 */
export async function highlightLines(code: string, lang: string, dark: boolean): Promise<HighlightToken[][] | null> {
  if (!lang || code.length === 0) return null;
  const key = cacheKey(code, lang, dark);
  const cached = tokenCache.get(key);
  if (cached) return cached;

  const theme = dark ? THEME_DARK : THEME_LIGHT;
  const highlighter = await highlighterFor(theme, lang);
  if (!highlighter.getLoadedLanguages().includes(lang)) return null;
  // 语言 id 由 languageForPath 产出，天然是 BundledLanguage 的子集；这里断言只是为了
  // 收窄 shiki 的联合字面量类型（它不接受任意 string）。
  const { tokens } = highlighter.codeToTokens(code, { lang: lang as BundledLanguage, theme });
  const rows: HighlightToken[][] = tokens.map((line) => line.map((token) => ({ content: token.content, color: token.color })));

  tokenCache.set(key, rows);
  // 插入序淘汰（Map 保留插入顺序）：上限只为兜住内存，不做 LRU 记账。
  while (tokenCache.size > TOKEN_CACHE_LIMIT) {
    const oldest = tokenCache.keys().next().value;
    if (oldest === undefined) break;
    tokenCache.delete(oldest);
  }
  return rows;
}
