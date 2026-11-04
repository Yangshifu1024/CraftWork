// 提供商目录：内置 models.dev 快照（候选列表，只读）+ 用户自定义提供商（可调用）。
// 原则：应用不内置任何可用提供商——内置目录只提供「可添加候选」，调用必须经用户添加。
import { gunzipSync } from "node:zlib";
import { Buffer } from "node:buffer";
import { CATALOG_GZ_BASE64 } from "./catalogdata";
import { DEFAULT_CAPS, type Capabilities, type Cost, type ModelInfo } from "./types";
import { getSetting } from "../store/records";
import type { DB } from "../store/db";

// ---------- 内置目录（models.dev 快照） ----------

interface CatalogModel {
  id: string;
  name: string;
  contextK: number;
  maxTokens?: number;
  caps?: Partial<Capabilities>;
  cost?: { input?: number; output?: number; cache_read?: number; cache_write?: number };
  status?: string;
  releaseDate?: string;
  canonicalId?: string;
}

interface CatalogProvider {
  id: string;
  name: string;
  kind: string;
  endpoint: string;
  doc?: string;
  env?: string[];
  logo?: string;
  models: CatalogModel[];
}

interface CatalogDataset {
  schema: number;
  generatedAt: string;
  version: string;
  source: string;
  license: string;
  providers: CatalogProvider[];
}

function loadDataset(): CatalogDataset {
  const json = gunzipSync(Buffer.from(CATALOG_GZ_BASE64, "base64")).toString("utf8");
  const data = JSON.parse(json) as CatalogDataset;
  if (data.schema !== 1) throw new Error(`目录快照 schema 不支持：${data.schema}`);
  if (!Array.isArray(data.providers) || data.providers.length === 0) throw new Error("目录快照为空");
  return data;
}

let builtinCache: CatalogDataset | null = null;

export function builtinCatalog(): CatalogDataset {
  if (builtinCache === null) builtinCache = loadDataset();
  return builtinCache;
}

// ---------- 自定义提供商（持久化形状） ----------

export interface PersistedModel {
  id: string;
  name: string;
  contextK: number;
  maxTokens: number;
  caps: Capabilities;
  cost?: Cost;
  status: string;
  canonicalId: string;
  releaseDate: string;
  source: string;
}

export interface PersistedProvider {
  id: string;
  name: string;
  kind: "openai" | "anthropic" | "openai-compatible";
  endpoint: string;
  keyRequired: boolean;
  note: string;
  source: string; // manual | catalog
  catalogId: string;
  catalogVersion: string;
  envVars: string[];
  /** 启用状态：禁用后不进模型选择器；旧数据无此字段，读取时缺省补 true。 */
  enabled: boolean;
  models: PersistedModel[];
}

export const CUSTOM_PROVIDERS_KEY = "model.custom_providers";

export function loadCustomProviders(db: DB): PersistedProvider[] {
  const value = getSetting<PersistedProvider[]>(db, CUSTOM_PROVIDERS_KEY, []);
  if (!Array.isArray(value)) return [];
  // 旧数据兼容：无 enabled 字段的提供商缺省视为启用（spec §8）
  return value.map((p) => (p && p.enabled === undefined ? { ...p, enabled: true } : p));
}

export function saveCustomProviders(db: DB, providers: PersistedProvider[]): void {
  // 动态 import 避免循环依赖（records → db 无环，但保持与 settings 服务一致的写入路径）
  // 直接写 settings 表
  db.prepare(
    "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
  ).run(CUSTOM_PROVIDERS_KEY, JSON.stringify(providers));
}

function toModelInfo(p: PersistedProvider, m: PersistedModel): ModelInfo {
  return {
    id: m.id,
    name: m.name,
    provider: p.id,
    contextK: m.contextK,
    maxTokens: m.maxTokens,
    caps: m.caps,
    cost: m.cost,
    status: m.status,
    canonicalId: m.canonicalId,
    releaseDate: m.releaseDate,
    source: m.source,
    envVars: p.envVars,
    keyRequired: p.keyRequired,
  };
}

/** 目录查询与解析：单实例持有 db，custom providers 变化后调 reload。 */
export class Catalog {
  private custom: PersistedProvider[] = [];

  constructor(private readonly db: DB) {
    this.reload();
  }

  reload(): void {
    try {
      this.custom = loadCustomProviders(this.db);
    } catch {
      this.custom = []; // 读取失败按空目录：宁可提示重新配置也不崩溃
    }
  }

  /** 全部可调用提供商（用户添加的）。 */
  providers(): PersistedProvider[] {
    return this.custom;
  }

  findProvider(id: string): PersistedProvider | null {
    return this.custom.find((p) => p.id === id) ?? null;
  }

  findModel(providerId: string, modelId: string): ModelInfo | null {
    const p = this.findProvider(providerId);
    if (!p) return null;
    const m = p.models.find((x) => x.id === modelId);
    return m ? toModelInfo(p, m) : null;
  }

  findModelByRef(ref: string): ModelInfo | null {
    const idx = ref.indexOf("/");
    if (idx <= 0) return null;
    return this.findModel(ref.slice(0, idx), ref.slice(idx + 1));
  }

  /**
   * 解析引用或原始 ID：先按完整引用（第一个斜杠切分，提供商 ID 无斜杠而模型 ID 可含），
   * 再按原始模型 ID 全局匹配（兼容旧数据）。找不到返回 null，调用方提示重选，不静默回退。
   */
  resolve(refOrId: string): ModelInfo | null {
    if (!refOrId) return null;
    const byRef = this.findModelByRef(refOrId);
    if (byRef) return byRef;
    for (const p of this.custom) {
      const m = p.models.find((x) => x.id === refOrId);
      if (m) return toModelInfo(p, m);
    }
    return null;
  }

  /** 规范化引用：可解析则返回规范 ref 与显示名，否则 null。 */
  normalize(refOrId: string): { ref: string; modelName: string } | null {
    const info = this.resolve(refOrId);
    if (!info) return null;
    return { ref: `${info.provider}/${info.id}`, modelName: info.name };
  }

  // ---------- 内置候选目录查询 ----------

  builtinMeta(): { generatedAt: string; version: string; providerCount: number; modelCount: number } {
    const data = builtinCatalog();
    let modelCount = 0;
    for (const p of data.providers) modelCount += p.models.length;
    return { generatedAt: data.generatedAt, version: data.version, providerCount: data.providers.length, modelCount };
  }

  builtinProviders(): CatalogProvider[] {
    return builtinCatalog().providers;
  }

  builtinProvider(id: string): CatalogProvider | null {
    return builtinCatalog().providers.find((p) => p.id === id) ?? null;
  }

  searchBuiltinProviders(query: string): CatalogProvider[] {
    const q = query.trim().toLowerCase();
    const all = builtinCatalog().providers;
    if (!q) return all;
    return all.filter((p) => p.id.toLowerCase().includes(q) || p.name.toLowerCase().includes(q));
  }
}

/** store/view 引用的规范化入口。 */
export function normalizeModelRef(db: DB, refOrId: string): { ref: string; modelName: string } | null {
  return new Catalog(db).normalize(refOrId);
}

/** 目录模型 → 持久化模型（从内置目录导入时）。 */
export function catalogModelToPersisted(m: CatalogModel, source: string): PersistedModel {
  return {
    id: m.id,
    name: m.name,
    contextK: m.contextK,
    maxTokens: m.maxTokens ?? 0,
    caps: { ...DEFAULT_CAPS, stream: true, ...(m.caps ?? {}) },
    cost: m.cost
      ? {
          inputPerMTok: m.cost.input ?? 0,
          outputPerMTok: m.cost.output ?? 0,
          cacheReadPerMTok: m.cost.cache_read ?? 0,
          cacheWritePerMTok: m.cost.cache_write ?? 0,
        }
      : undefined,
    status: m.status ?? "",
    canonicalId: m.canonicalId ?? "",
    releaseDate: m.releaseDate ?? "",
    source,
  };
}

/** 内置目录提供商的 env 变量名（导入时带入，密钥可从环境变量回退）。 */
export function catalogProviderEnv(p: CatalogProvider): string[] {
  return p.env ?? [];
}
