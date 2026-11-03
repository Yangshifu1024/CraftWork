#!/usr/bin/env node
// 将 assets/catalog/catalog.json.gz 内嵌为 src/main/model/catalogdata.ts（base64）。
// 用法：node scripts/embed-catalog.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const gz = readFileSync(join(root, "assets/catalog/catalog.json.gz"));
const b64 = gz.toString("base64");
const lines = [];
for (let i = 0; i < b64.length; i += 120) lines.push(b64.slice(i, i + 120));
const ts =
  "// 由 scripts/embed-catalog.mjs 从 assets/catalog/catalog.json.gz 生成，请勿手改。\n" +
  "// models.dev 目录快照（构建期内嵌，运行时只读）。更新目录：node scripts/modelsync.mjs。\n" +
  "const LINES = [\n" +
  lines.map((l) => `  "${l}",`).join("\n") +
  "\n];\n\n" +
  'export const CATALOG_GZ_BASE64 = LINES.join("");\n';
writeFileSync(join(root, "src/main/model/catalogdata.ts"), ts);
console.log(`catalogdata.ts 生成完成（base64 ${b64.length} 字符）`);
