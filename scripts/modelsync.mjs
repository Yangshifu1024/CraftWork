#!/usr/bin/env node
// 模型目录同步：从 models.dev 拉取最新目录，按过滤规则生成离线快照并内嵌进源码。
// 用法：node scripts/modelsync.mjs [--check]
//   --check  只检查漂移（远端内容与当前快照是否一致），有漂移时以非零码退出。
// 规则（assets/catalog/rules.json）：
//   只保留 openai / anthropic / openai-compatible 三种协议；排除指定提供商；
//   端点覆盖表固定主流服务的可直连地址。
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const checkOnly = process.argv.includes("--check");
const SOURCE = "https://models.dev/api.json";
const SCHEMA = 1;

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

function normalizeProvider(id, provider) {
  const kind = kindOf(provider);
  const models = [];
  for (const [modelId, m] of Object.entries(provider.models ?? {})) {
    // 只保留对话模型（带 tool_call 或 reasoning/categorize 为 chat 的都不排除，过滤明显非对话类）
    if (m.attachment === true) continue;
    models.push({
      id: modelId,
      name: m.name ?? modelId,
      contextK: Math.round((m.limit?.context ?? 0) / 1000) || 32,
      maxTokens: m.limit?.output ?? 0,
      caps: {
        stream: true,
        tools: m.tool_call === true,
        vision: m.input_modalities?.includes("image") === true,
        reasoning: m.reasoning === true,
        structured: m.structured_output === true,
      },
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
    endpoint: ENDPOINT_OVERRIDES[id] ?? provider.baseUrl ?? "",
    doc: provider.doc ?? "",
    env: provider.env ?? [],
    logo: `https://models.dev/logos/${id}.svg`,
    models,
  };
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

  const dataset = {
    schema: SCHEMA,
    generatedAt: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    version: createHash("sha256").update(JSON.stringify(providers)).digest("hex").slice(0, 16),
    source: SOURCE,
    license: "MIT",
    providers,
  };
  const json = JSON.stringify(dataset);
  const gz = gzipSync(Buffer.from(json, "utf8"));
  const sha256 = createHash("sha256").update(json).digest("hex");
  const modelCount = providers.reduce((sum, p) => sum + p.models.length, 0);

  if (checkOnly) {
    const same = sha256 === existingSha;
    console.log(same ? "目录无漂移。" : `目录有漂移（当前快照 version=${existingVersion}）。请运行 node scripts/modelsync.mjs 更新。`);
    process.exit(same ? 0 : 1);
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
    sha256,
  };
  writeFileSync(join(root, "assets/catalog/catalog.manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);

  // 重新内嵌为 TS 模块
  const b64 = gz.toString("base64");
  const lines = [];
  for (let i = 0; i < b64.length; i += 120) lines.push(b64.slice(i, i + 120));
  const ts =
    "// 由 scripts/embed-catalog.mjs 从 assets/catalog/catalog.json.gz 生成，请勿手改。\n" +
    "// models.dev 目录快照（构建期内嵌，运行时只读）。\n" +
    "const LINES = [\n" +
    lines.map((l) => `  "${l}",`).join("\n") +
    "\n];\n\n" +
    'export const CATALOG_GZ_BASE64 = LINES.join("");\n';
  writeFileSync(join(root, "src/main/model/catalogdata.ts"), ts);

  console.log(`目录已更新：${providers.length} 个提供商，${modelCount} 个模型（version ${dataset.version}）。`);
}

main().catch((err) => {
  console.error("同步失败：", err);
  process.exit(2);
});
