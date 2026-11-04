// 提供商服务：用户提供商的增删改查、密钥、连接测试与模型目录导入。
import type {
  CatalogImportInput,
  CatalogMeta,
  CatalogModelView,
  CatalogProviderView,
  ConnectionResult,
  ModelInput,
  ProviderInput,
  ProviderView,
} from "../../shared/types";
import type { DB } from "../store/db";
import type { SecretStore } from "../core/secret";
import type { Catalog } from "../model/catalog";
import {
  catalogModelToPersisted,
  catalogProviderEnv,
  loadCustomProviders,
  saveCustomProviders,
  type PersistedModel,
  type PersistedProvider,
} from "../model/catalog";
import { DEFAULT_CAPS, type Capabilities, type ProviderInfo } from "../model/types";
import { statusError, transportError, ModelError } from "../model/errors";
import { ModelFactory } from "../model/factory";

const MAX_IMPORTED_MODELS = 500;

const CONNECTION_MESSAGES: Record<string, string> = {
  auth: "密钥无效或权限不足，请检查 API Key。",
  rate_limit: "请求过于频繁或额度不足。",
  timeout: "连接超时，请检查网络或端点地址。",
  provider_unavailable: "无法连接服务，请检查端点地址与网络。",
  invalid_request: "端点或参数不被接受，请检查配置。",
};

/** 内置目录提供商 → 提供商元信息（导入用）。 */
function catalogProviderInfo(p: NonNullable<ReturnType<Catalog["builtinProvider"]>>): ProviderInfo {
  return {
    id: p.id,
    name: p.name,
    kind: (p.kind === "anthropic" ? "anthropic" : p.kind === "openai" ? "openai" : "openai-compatible") as ProviderInfo["kind"],
    endpoint: p.endpoint,
    keyRequired: true,
    note: p.doc ?? "",
    source: "catalog",
    catalogId: p.id,
    catalogVersion: "",
    envVars: catalogProviderEnv(p),
    models: [],
  };
}

export class ProviderService {
  private factory: ModelFactory | null = null;

  constructor(
    private readonly db: DB,
    private readonly secrets: SecretStore,
    private readonly catalog: Catalog,
  ) {}

  /** 工厂注入（装配期构造顺序：目录 → 本服务 → 工厂）。 */
  setFactory(factory: ModelFactory): void {
    this.factory = factory;
  }

  list(): ProviderView[] {
    return loadCustomProviders(this.db).map((p) => this.toView(p));
  }

  saveProvider(input: ProviderInput): ProviderView {
    const persisted = this.buildProvider(input);
    const all = loadCustomProviders(this.db);
    const idx = all.findIndex((p) => p.id === persisted.id);
    if (idx >= 0) all[idx] = persisted;
    else all.push(persisted);
    saveCustomProviders(this.db, all);
    this.catalog.reload();
    return this.toView(persisted);
  }

  deleteProvider(providerId: string): void {
    const all = loadCustomProviders(this.db).filter((p) => p.id !== providerId);
    if (all.length === loadCustomProviders(this.db).length) throw new Error("提供商不存在");
    saveCustomProviders(this.db, all);
    this.secrets.delete(providerId);
    this.catalog.reload();
  }

  setKey(providerId: string, key: string): void {
    this.requireExists(providerId);
    this.secrets.set(providerId, key);
  }

  clearKey(providerId: string): void {
    this.requireExists(providerId);
    this.secrets.delete(providerId);
  }

  warnings(): string[] {
    return this.secrets.warnings();
  }

  /** 连接测试：anthropic 查 /v1/models，其余查 /models；10 秒超时。 */
  async test(providerId: string): Promise<ConnectionResult> {
    const persisted = this.requireExists(providerId);
    const started = Date.now();
    try {
      const key = this.secrets.getFor(providerId, persisted.envVars);
      if (persisted.keyRequired && !key) {
        return { ok: false, message: "尚未配置密钥，请先粘贴 API Key。", latencyMs: 0, code: "auth" };
      }
      const url =
        persisted.kind === "anthropic"
          ? `${joinServiceURL(persisted.endpoint, "/v1/models")}?limit=1`
          : `${persisted.endpoint.replace(/\/+$/, "")}/models`;
      const headers: Record<string, string> =
        persisted.kind === "anthropic"
          ? { "x-api-key": key, "anthropic-version": "2023-06-01" }
          : { Authorization: `Bearer ${key}` };
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 10_000);
      let resp: Response;
      try {
        resp = await fetch(url, { headers, signal: controller.signal });
      } finally {
        clearTimeout(timer);
      }
      const latencyMs = Date.now() - started;
      if (!resp.ok) {
        const body = (await resp.text().catch(() => "")).slice(0, 8192);
        const err = statusError(persisted.name, resp.status, body);
        return { ok: false, message: connectionMessage(err.code), latencyMs, code: err.code };
      }
      return { ok: true, message: "连接成功。", latencyMs, code: "" };
    } catch (err) {
      const latencyMs = Date.now() - started;
      const wrapped = transportError(persisted.name, err);
      return { ok: false, message: connectionMessage(wrapped.code), latencyMs, code: wrapped.code };
    }
  }

  // ---------- 模型目录（候选列表） ----------

  catalogMeta(): CatalogMeta {
    const meta = this.catalog.builtinMeta();
    return meta;
  }

  catalogProviders(query: string): CatalogProviderView[] {
    const added = new Set(loadCustomProviders(this.db).map((p) => p.catalogId || p.id));
    return this.catalog.searchBuiltinProviders(query).map((p) => ({
      id: p.id,
      name: p.name,
      kind: p.kind,
      endpoint: p.endpoint,
      doc: p.doc ?? "",
      env: p.env ?? [],
      modelCount: p.models.length,
      added: added.has(p.id),
    }));
  }

  catalogModels(providerId: string): CatalogModelView[] {
    const p = this.catalog.builtinProvider(providerId);
    if (!p) throw new Error("目录中不存在该提供商");
    return p.models.map((m) => ({
      id: m.id,
      name: m.name,
      contextK: m.contextK,
      maxTokens: m.maxTokens ?? 0,
      caps: { ...DEFAULT_CAPS, stream: true, ...(m.caps ?? {}) } as Capabilities,
      cost: m.cost
        ? {
            inputPerMTok: m.cost.input ?? 0,
            outputPerMTok: m.cost.output ?? 0,
            cacheReadPerMTok: m.cost.cache_read ?? 0,
            cacheWritePerMTok: m.cost.cache_write ?? 0,
          }
        : undefined,
      status: m.status ?? "",
      releaseDate: m.releaseDate ?? "",
    }));
  }

  /**
   * 跨目录模型查询：先按提供商+模型查；查不到在全部提供商中按模型 ID 找唯一匹配
   * （0 个报不存在；多个要求先给提供商 ID）。
   */
  lookupCatalogModel(providerId: string, modelId: string): CatalogModelView {
    if (providerId) {
      const hit = this.catalogModels(providerId).find((m) => m.id === modelId);
      if (hit) return hit;
    }
    const matches: Array<{ provider: string; model: CatalogModelView }> = [];
    for (const p of this.catalog.builtinProviders()) {
      const hit = p.models.find((m) => m.id === modelId);
      if (hit) {
        matches.push({
          provider: p.id,
          model: {
            id: hit.id,
            name: hit.name,
            contextK: hit.contextK,
            maxTokens: hit.maxTokens ?? 0,
            caps: { ...DEFAULT_CAPS, stream: true, ...(hit.caps ?? {}) } as Capabilities,
            status: hit.status ?? "",
            releaseDate: hit.releaseDate ?? "",
          },
        });
      }
    }
    if (matches.length === 0) throw new Error("目录中找不到该模型");
    if (matches.length > 1) throw new Error("该模型存在于多个提供商，请先填写提供商 ID");
    return matches[0]!.model;
  }

  /** 从内置目录导入提供商：候选必须存在且未添加过；模型上限 500。 */
  importFromCatalog(input: CatalogImportInput): ProviderView {
    const source = this.catalog.builtinProvider(input.providerId);
    if (!source) throw new Error("目录中不存在该提供商");
    const existing = loadCustomProviders(this.db);
    if (existing.some((p) => (p.catalogId || p.id) === source.id)) {
      throw new Error("已经添加，请直接编辑现有配置");
    }
    let models: PersistedModel[];
    if (input.all) {
      models = source.models
        .slice(0, MAX_IMPORTED_MODELS)
        .map((m) => catalogModelToPersisted(m, "catalog"));
    } else {
      const ids = [...new Set(input.modelIds)];
      if (ids.length === 0) throw new Error("请至少选择一个模型");
      if (ids.length > MAX_IMPORTED_MODELS) throw new Error(`一次最多导入 ${MAX_IMPORTED_MODELS} 个模型`);
      models = [];
      for (const id of ids) {
        const m = source.models.find((x) => x.id === id);
        if (!m) throw new Error(`模型不存在：${id}`);
        models.push(catalogModelToPersisted(m, "catalog"));
      }
    }
    const info = catalogProviderInfo(source);
    const persisted: PersistedProvider = {
      id: source.id,
      name: source.name,
      kind: info.kind,
      endpoint: source.endpoint,
      keyRequired: true,
      note: info.note,
      source: "catalog",
      catalogId: source.id,
      catalogVersion: this.catalog.builtinMeta().version,
      envVars: info.envVars,
      models,
    };
    existing.push(persisted);
    saveCustomProviders(this.db, existing);
    this.catalog.reload();
    return this.toView(persisted);
  }

  // ---------- 内部 ----------

  private requireExists(providerId: string): PersistedProvider {
    const p = loadCustomProviders(this.db).find((x) => x.id === providerId);
    if (!p) throw new Error("提供商不存在");
    return p;
  }

  /** 构建并校验提供商输入（含端点 HTTPS 规则与模型清单校验）。 */
  private buildProvider(input: ProviderInput): PersistedProvider {
    const name = input.name.trim();
    if (!name) throw new Error("提供商名称不能为空");
    const kind = (input.kind || "openai-compatible") as PersistedProvider["kind"];
    if (kind !== "openai" && kind !== "anthropic" && kind !== "openai-compatible") {
      throw new Error("不支持的协议类型");
    }
    let id = input.id.trim();
    if (!id) {
      id = `custom_${Math.random().toString(16).slice(2, 10)}`;
    } else if (!/^[a-z0-9_.-]{2,64}$/.test(id) || /^[_.-]/.test(id)) {
      throw new Error("提供商 ID 只能包含小写字母、数字与 . _ -（2–64 位，不能以符号开头）");
    }
    const endpoint = input.endpoint.trim();
    validateEndpoint(endpoint);
    if (input.models.length === 0) throw new Error("至少添加一个模型");

    const seen = new Set<string>();
    const models: PersistedModel[] = input.models.map((m) => this.buildModel(m, seen));
    const existing = loadCustomProviders(this.db).find((p) => p.id === id);

    return {
      id,
      name,
      kind,
      endpoint,
      keyRequired: input.keyRequired,
      note: existing?.note ?? "",
      source: existing?.source ?? "manual",
      catalogId: existing?.catalogId ?? "",
      catalogVersion: existing?.catalogVersion ?? "",
      envVars: existing?.envVars ?? envNameFallback(id),
      models,
    };
  }

  private buildModel(m: ModelInput, seen: Set<string>): PersistedModel {
    const id = m.id.trim();
    if (!id || /\s/.test(id)) throw new Error("模型 ID 不能为空或含空白");
    if (seen.has(id)) throw new Error(`同一提供商内模型 ID 重复：${id}`);
    seen.add(id);
    const contextK = m.contextK > 0 ? Math.trunc(m.contextK) : 32;
    if (contextK > 100_000) throw new Error(`模型 ${id} 的上下文窗口超出上限`);
    const maxTokens = Math.min(4_000_000, Math.max(0, Math.trunc(m.maxTokens) || 0));
    const caps: Capabilities = { ...DEFAULT_CAPS, stream: true, ...(m.caps ?? {}) };
    return {
      id,
      name: m.name.trim() || id,
      contextK,
      maxTokens,
      caps,
      cost: m.cost,
      status: "",
      canonicalId: "",
      releaseDate: "",
      source: "manual",
    };
  }

  private toView(p: PersistedProvider): ProviderView {
    const available = this.factory
      ? (() => {
          try {
            return this.factory!.client(`${p.id}/${p.models[0]?.id ?? ""}`) !== null;
          } catch {
            return false;
          }
        })()
      : false;
    return {
      id: p.id,
      name: p.name,
      kind: p.kind,
      endpoint: p.endpoint,
      keyRequired: p.keyRequired,
      keySource: this.secrets.sourceFor(p.id, p.envVars),
      keyMasked: this.secrets.maskedFor(p.id, p.envVars),
      available,
      catalogId: p.catalogId,
      source: p.source,
      models: p.models.map((m) => ({
        id: m.id,
        ref: `${p.id}/${m.id}`,
        name: m.name,
        provider: p.id,
        providerName: p.name,
        contextK: m.contextK,
        maxTokens: m.maxTokens,
        caps: m.caps,
        cost: m.cost,
        status: m.status,
        canonicalId: m.canonicalId,
        releaseDate: m.releaseDate,
        available,
      })),
    };
  }
}

function connectionMessage(code: string): string {
  return CONNECTION_MESSAGES[code] ?? "连接失败，请检查配置。";
}

/** 端点校验：必须合法 http(s)；非本地端点必须 HTTPS（避免明文传密钥）。 */
function validateEndpoint(endpoint: string): void {
  if (!endpoint) throw new Error("端点不能为空");
  let url: URL;
  try {
    url = new URL(endpoint);
  } catch {
    throw new Error("端点不是合法的 URL");
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    throw new Error("端点必须使用 http(s) 协议");
  }
  if (url.protocol === "http:") {
    const host = url.hostname;
    if (!["localhost", "127.0.0.1", "[::1]", "::1"].includes(host)) {
      throw new Error("非本地端点必须使用 HTTPS");
    }
  }
}

/** 服务路径拼接：base 已含 /v1 时跳过 path 第一段。 */
function joinServiceURL(base: string, path: string): string {
  const cleanBase = base.replace(/\/+$/, "");
  const segments = path.replace(/^\/+/, "");
  const firstSeg = segments.split("/")[0] ?? "";
  if (firstSeg && cleanBase.endsWith(`/${firstSeg}`)) {
    return `${cleanBase}/${segments.slice(firstSeg.length + 1)}`;
  }
  return `${cleanBase}/${segments}`;
}

/** 无目录 env 时的环境变量名推导（如 deepseek → DEEPSEEK_API_KEY）。 */
function envNameFallback(providerId: string): string[] {
  const known: Record<string, string> = {
    openai: "OPENAI_API_KEY",
    deepseek: "DEEPSEEK_API_KEY",
    glm: "ZHIPU_API_KEY",
    zhipu: "ZHIPU_API_KEY",
    qwen: "DASHSCOPE_API_KEY",
    anthropic: "ANTHROPIC_API_KEY",
  };
  const knownName = known[providerId];
  if (knownName) return [knownName];
  return [`${providerId.toUpperCase().replace(/[^A-Z0-9]/g, "_")}_API_KEY`];
}

// ModelError 保留导出（测试与错误路径复用）
export { ModelError };
