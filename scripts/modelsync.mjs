#!/usr/bin/env node
// 模型目录同步：从 models.dev 拉取最新目录，按过滤规则生成离线快照并内嵌进源码；
// 同时下载各提供商 SVG logo，清洗后落盘 assets/catalog/logos/，并内嵌为渲染层资产模块。
// 用法：node scripts/modelsync.mjs [--check]
//   --check  只检查漂移（远端内容与当前快照是否一致），有漂移时以非零码退出。
// 规则（assets/catalog/rules.json）：
//   只保留 openai / anthropic / openai-compatible 三种协议；排除指定提供商；
//   端点覆盖表固定主流服务的可直连地址。
import { readFileSync, writeFileSync, existsSync, mkdirSync, rmSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const checkOnly = process.argv.includes("--check");
const SOURCE = "https://models.dev/api.json";
const LOGO_BASE = "https://models.dev/logos";
const SCHEMA = 1;
const LOGO_MAX_BYTES = 100 * 1024; // 单个 logo 超过 100KB 视为异常体积，丢弃
const LOGO_CONCURRENCY = 8;

const rules = JSON.parse(readFileSync(join(root, "assets/catalog/rules.json"), "utf8"));

/** 端点覆盖：目录里的端点不可直连时使用固定官方地址（与规则文件同一事实源）。 */
const ENDPOINT_OVERRIDES = rules.endpointOverrides ?? {};

const EXCLUDED = new Set(rules.excludedProviders ?? ["google", "google-vertex", "google-vertex-anthropic", "azure", "azure-cognitive-services", "amazon-bedrock"]);
const KIND_BY_NPM = rules.kindByNpm ?? {
  "@ai-sdk/openai": "openai",
  "@ai-sdk/anthropic": "anthropic",
  "@ai-sdk/openai-compatible": "openai-compatible",
};
const FALLBACK_KIND = rules.fallbackKind ?? "openai-compatible";

function kindOf(provider) {
  for (const [npm, kind] of Object.entries(KIND_BY_NPM)) {
    if (provider.npm === npm) return kind;
  }
  return FALLBACK_KIND;
}

/** 端点解析：覆盖表优先，models.dev 新 `api` 字段次之，旧 `baseUrl` 字段兜底。
 *  坏值防御：含 `${` 占位符、非 http(s)、URL 解析失败的一律置空，按「无端点」处理。 */
function resolveEndpoint(id, provider) {
  const raw = ENDPOINT_OVERRIDES[id] ?? provider.api ?? provider.baseUrl ?? "";
  if (typeof raw !== "string") return "";
  const value = raw.trim();
  if (value === "" || value.includes("${")) return "";
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" && url.protocol !== "http:") return "";
  } catch {
    return "";
  }
  return value;
}

function normalizeProvider(id, provider) {
  const kind = kindOf(provider);
  const models = [];
  for (const [modelId, m] of Object.entries(provider.models ?? {})) {
    // models.dev 现语义：attachment=true = 支持附件输入的对话模型，false 多为嵌入/语音/图像类。
    // 保留 attachment 或 tool_call 为 true 的模型，其余（嵌入/语音/图像生成等非对话类）滤除。
    if (m.attachment !== true && m.tool_call !== true) continue;
    models.push({
      id: modelId,
      name: m.name ?? modelId,
      // 窗口缺失保留 0：前端按「窗口未知」显示（不渲染百分比），后端跳过压缩；
      // 不再静默兜底 32K（错误分母会把占用百分比放大几十倍）
      contextK: Math.round((m.limit?.context ?? 0) / 1000),
      maxTokens: m.limit?.output ?? 0,
      caps: {
        stream: true,
        tools: m.tool_call === true,
        vision: m.input_modalities?.includes("image") === true,
        reasoning: m.reasoning === true,
        structured: m.structured_output === true,
      },
      // 思考档位元数据原样保留（effort 枚举 / toggle / budget_tokens），Rust 侧归一化
      reasoningOptions: Array.isArray(m.reasoning_options) ? m.reasoning_options : [],
      cost: m.cost
        ? {
            input: m.cost.input,
            output: m.cost.output,
            cache_read: m.cost.cache_read,
            cache_write: m.cost.cache_write,
          }
        : undefined,
      status: m.status ?? "",
      releaseDate: m.release_date ?? "",
      canonicalId: m.id ?? "",
    });
  }
  if (models.length === 0) return null;
  return {
    id,
    name: provider.name ?? id,
    kind,
    endpoint: resolveEndpoint(id, provider),
    doc: provider.doc ?? "",
    env: provider.env ?? [],
    logo: `${LOGO_BASE}/${id}.svg`,
    models,
  };
}

/** SVG 清洗：剔除 script/foreignObject、事件属性（on*）、javascript: 引用与外部 href/url() 引用。 */
function sanitizeSvg(svg) {
  let s = svg;
  s = s.replace(/<script[\s\S]*?<\/script\s*>/gi, "");
  s = s.replace(/<script[^>]*\/?>/gi, "");
  s = s.replace(/<foreignObject[\s\S]*?<\/foreignObject\s*>/gi, "");
  s = s.replace(/<foreignObject[^>]*\/?>/gi, "");
  s = s.replace(/\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi, "");
  s = s.replace(/\s+(?:xlink:)?href\s*=\s*(["'])\s*javascript:[^"']*\1/gi, "");
  s = s.replace(/javascript:/gi, "");
  // 仅保留同文档锚点引用（# 开头），外部/相对/data: 引用整属性剔除
  s = s.replace(/\s+xlink:href\s*=\s*(["'])(?!#)[^"']*\1/gi, "");
  s = s.replace(/\s+href\s*=\s*(["'])(?!#)[^"']*\1/gi, "");
  // style/fill 等属性中的 url() 引用：仅保留 # 开头的内部引用，其余替换为 none
  s = s.replace(/url\(\s*['"]?\s*(?!#)[^)'"]*['"]?\s*\)/gi, "none");
  return s;
}

/** 下载单个 logo：仅收 image/svg+xml 响应，超限/非 SVG/清洗失败一律返回 null。 */
async function fetchLogo(id) {
  try {
    const resp = await fetch(`${LOGO_BASE}/${id}.svg`, {
      headers: { "User-Agent": "craftwork-modelsync" },
      signal: AbortSignal.timeout(20_000),
    });
    if (!resp.ok) return null;
    const contentType = (resp.headers.get("content-type") ?? "").toLowerCase();
    if (!contentType.includes("image/svg+xml")) return null;
    const buf = Buffer.from(await resp.arrayBuffer());
    if (buf.length === 0 || buf.length > LOGO_MAX_BYTES) return null;
    const text = buf.toString("utf8");
    if (!/<svg[\s>]/i.test(text)) return null;
    return sanitizeSvg(text);
  } catch {
    return null;
  }
}

async function downloadLogos(ids) {
  const logos = new Map();
  let failed = 0;
  for (let i = 0; i < ids.length; i += LOGO_CONCURRENCY) {
    const batch = ids.slice(i, i + LOGO_CONCURRENCY);
    const results = await Promise.all(batch.map(async (id) => [id, await fetchLogo(id)]));
    for (const [id, svg] of results) {
      if (svg === null) failed++;
      else logos.set(id, svg);
    }
  }
  return { logos, failed };
}

/** base64 按 120 字符切片（内嵌 TS 模块统一行宽，切片逻辑只此一处）。 */
function base64Lines(b64, width = 120) {
  const lines = [];
  for (let i = 0; i < b64.length; i += width) lines.push(b64.slice(i, i + width));
  return lines;
}

/** logo 资产模块的多行拼接格式：行间以 `"\n<indent>+ "` 连接。 */
function wrapBase64(b64, indent) {
  return base64Lines(b64).join(`"\n${indent}+ "`);
}

async function main() {
  const resp = await fetch(SOURCE, { headers: { "User-Agent": "craftwork-modelsync" } });
  if (!resp.ok) {
    console.error(`拉取目录失败：HTTP ${resp.status}`);
    process.exit(2);
  }
  const raw = await resp.json();
  const providers = [];
  for (const [id, provider] of Object.entries(raw)) {
    if (EXCLUDED.has(id)) continue;
    const normalized = normalizeProvider(id, provider);
    if (normalized) providers.push(normalized);
  }
  const existingPath = join(root, "assets/catalog/catalog.json.gz");
  let existingVersion = "";
  let existingSha = "";
  if (existsSync(existingPath)) {
    const manifestPath = join(root, "assets/catalog/catalog.manifest.json");
    if (existsSync(manifestPath)) {
      const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
      existingVersion = manifest.version ?? "";
      existingSha = manifest.sha256 ?? "";
    }
  }

  // version 与 sha256 都只基于 providers 内容计算，与 generatedAt 时间戳解耦，
  // 保证 --check 漂移比对幂等（时间戳不参与校验和）。
  const contentJson = JSON.stringify(providers);
  const contentHash = createHash("sha256");
  contentHash.update(contentJson);
  const dataset = {
    schema: SCHEMA,
    generatedAt: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    version: contentHash.copy().digest("hex").slice(0, 16),
    source: SOURCE,
    license: "MIT",
    providers,
  };
  const json = JSON.stringify(dataset);
  const gz = gzipSync(Buffer.from(json, "utf8"));
  const sha256 = contentHash.digest("hex");
  const modelCount = providers.reduce((sum, p) => sum + p.models.length, 0);

  if (checkOnly) {
    const same = sha256 === existingSha;
    console.log(same ? "目录无漂移。" : `目录有漂移（当前快照 version=${existingVersion}）。请运行 node scripts/modelsync.mjs 更新。`);
    process.exit(same ? 0 : 1);
  }

  // 下载并清洗 logo（失败的不收录，渲染层回退字母头像）
  const logoDir = join(root, "assets/catalog/logos");
  console.log(`下载 ${providers.length} 个提供商 logo…`);
  const { logos, failed } = await downloadLogos(providers.map((p) => p.id));
  rmSync(logoDir, { recursive: true, force: true });
  mkdirSync(logoDir, { recursive: true });
  const sortedLogos = [...logos.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  let logoBytes = 0;
  for (const [id, svg] of sortedLogos) {
    const buf = Buffer.from(svg, "utf8");
    logoBytes += buf.length;
    writeFileSync(join(logoDir, `${id}.svg`), buf);
  }

  writeFileSync(existingPath, gz);
  const manifest = {
    schema: SCHEMA,
    generatedAt: dataset.generatedAt,
    version: dataset.version,
    source: SOURCE,
    sourceEtag: resp.headers.get("etag") ?? "",
    license: "MIT",
    providerCount: providers.length,
    modelCount,
    logoCount: sortedLogos.length,
    sha256,
  };
  writeFileSync(join(root, "assets/catalog/catalog.manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);

  // 目录快照由 Rust 侧 include_bytes! 直接嵌入 assets/catalog/catalog.json.gz
  // （Electron 期的 TS 内嵌模块 catalogdata.ts 已随 src/main 删除，勿再生成）。

  // 渲染层 logo 资产模块：providerId → data:image/svg+xml;base64 URL
  const assetsDir = join(root, "src/renderer/src/assets");
  mkdirSync(assetsDir, { recursive: true });
  const logoEntries = sortedLogos
    .map(([id, svg]) => {
      const payload = wrapBase64(Buffer.from(svg, "utf8").toString("base64"), "    ");
      return `  ${JSON.stringify(id)}:\n    "data:image/svg+xml;base64,${payload}",`;
    })
    .join("\n");
  const logosTs =
    "// 由 scripts/modelsync.mjs 生成，请勿手改。更新目录：node scripts/modelsync.mjs。\n" +
    "// 提供商 logo 内嵌资产（来源 https://models.dev/logos/{id}.svg，构建期下载并清洗，\n" +
    "// 仅保留同文档锚点引用；缺失 logo 的提供商由渲染层回退字母头像）。\n" +
    "export const PROVIDER_LOGOS: Record<string, string> = {\n" +
    logoEntries +
    "\n};\n";
  writeFileSync(join(assetsDir, "provider-logos.ts"), logosTs);

  console.log(
    `目录已更新：${providers.length} 个提供商，${modelCount} 个模型（version ${dataset.version}）；` +
      `logo ${sortedLogos.length} 个（失败/缺失 ${failed} 个，清洗后合计 ${(logoBytes / 1024).toFixed(0)}KB）。`,
  );
}

main().catch((err) => {
  console.error("同步失败：", err);
  process.exit(2);
});
