// 提供商服务：用户提供商的增删改查、密钥、连接测试与模型目录导入。
import type {
  CatalogImportInput,
  CatalogMeta,
  CatalogModelView,
  CatalogProviderView,
  ConnectionResult,
  FetchedModelsResult,
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
import { statusError, transportError, ModelError, Code } from "../model/errors";
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

  /** 启停供应商：读 → 校验 → 写 → reload → 返回刷新后的 ProviderView（spec §8）。 */
  setEnabled(providerId: string, enabled: boolean): ProviderView {
    const all = loadCustomProviders(this.db);
    const idx = all.findIndex((p) => p.id === providerId);
    if (idx < 0) throw new Error("提供商不存在");
    all[idx] = { ...all[idx]!, enabled };
    saveCustomProviders(this.db, all);
    this.catalog.reload();
    return this.toView(all[idx]!);
  }

  /** 供应商改名：名称非空校验，同 mutation 契约返回刷新后的 ProviderView。 */
  rename(providerId: string, name: string): ProviderView {
    const trimmed = String(name ?? "").trim();
    if (!trimmed) throw new Error("提供商名称不能为空");
    const all = loadCustomProviders(this.db);
    const idx = all.findIndex((p) => p.id === providerId);
    if (idx < 0) throw new Error("提供商不存在");
    all[idx] = { ...all[idx]!, name: trimmed };
    saveCustomProviders(this.db, all);
    this.catalog.reload();
    return this.toView(all[idx]!);
  }

  /** 启停模型：读 → 校验 → 写 → reload → 返回刷新后的 ProviderView（spec §8）。 */
  setModelEnabled(providerId: string, modelId: string, enabled: boolean): ProviderView {
    return this.mutateModel(providerId, modelId, (p, mIdx) => {
      const models = [...p.models];
      models[mIdx] = { ...models[mIdx]!, enabled };
      return { ...p, models };
    });
  }

  /** 新增模型：提供商内 ID 唯一，其余校验复用 buildModel 规则；新模型默认启用。 */
  addModel(providerId: string, input: ModelInput): ProviderView {
    const all = loadCustomProviders(this.db);
    const idx = all.findIndex((x) => x.id === providerId);
    if (idx < 0) throw new Error("提供商不存在");
    const provider = all[idx]!;
    const seen = new Set(provider.models.map((m) => m.id));
    const model = this.buildModel(input, seen);
    all[idx] = { ...provider, models: [...provider.models, model] };
    saveCustomProviders(this.db, all);
    this.catalog.reload();
    return this.toView(all[idx]!);
  }

  /** 更新模型：按 modelId 定位后整行更新；ID 不可变，启用状态与来源等非表单字段保留。 */
  updateModel(providerId: string, modelId: string, input: ModelInput): ProviderView {
    if (input.id.trim() !== modelId) throw new Error("模型 ID 不可修改");
    return this.mutateModel(providerId, modelId, (p, mIdx) => {
      const prev = p.models[mIdx]!;
      const seen = new Set(p.models.filter((_, i) => i !== mIdx).map((m) => m.id));
      const model = this.buildModel(input, seen);
      const models = [...p.models];
      models[mIdx] = {
        ...model,
        enabled: prev.enabled,
        source: prev.source,
        cost: input.cost ?? prev.cost,
        status: prev.status,
        canonicalId: prev.canonicalId,
        releaseDate: prev.releaseDate,
      };
      return { ...p, models };
    });
  }

  /** 删除模型：确认由界面侧完成，服务端直接移除。 */
  removeModel(providerId: string, modelId: string): ProviderView {
    return this.mutateModel(providerId, modelId, (p, mIdx) => ({
      ...p,
      models: p.models.filter((_, i) => i !== mIdx),
    }));
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

  /** 拉取模型（票 5 / spec §7）：按协议 GET 端点模型列表，与现有模型按 ID 去重合并。 */
  async fetchModels(providerId: string): Promise<FetchedModelsResult> {
    const persisted = this.requireExists(providerId);
    const key = this.secrets.getFor(providerId, persisted.envVars);
    if (persisted.keyRequired && !key) {
      throw new Error("尚未配置密钥，请先设置 API Key");
    }
    validateEndpoint(persisted.endpoint);
    // fetch 期间数据可能被并发修改，响应回来后重新读取再合并
    const url =
      persisted.kind === "anthropic"
        ? `${joinServiceURL(persisted.endpoint, "/v1/models")}?limit=1000`
        : `${persisted.endpoint.replace(/\/+$/, "")}/models`;
    const headers: Record<string, string> =
      persisted.kind === "anthropic"
        ? { "x-api-key": key, "anthropic-version": "2023-06-01" }
        : { Authorization: `Bearer ${key}` };
    const controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, 10_000);
    let resp: Response;
    try {
      resp = await fetch(url, { headers, signal: controller.signal });
    } catch (err) {
      // 主动超时 abort 归因为 timeout，其余网络错误沿用 transportError 归因；文案复用连接测试映射
      const code = timedOut ? Code.Timeout : transportError(persisted.name, err).code;
      throw new ModelError(code, persisted.name, connectionMessage(code), 0, err);
    } finally {
      clearTimeout(timer);
    }
    if (!resp.ok) {
      const body = (await resp.text().catch(() => "")).slice(0, 8192);
      const err = statusError(persisted.name, resp.status, body);
      throw new ModelError(err.code, persisted.name, connectionMessage(err.code), err.status, err);
    }
    const entries = parseModelEntries(await resp.json().catch(() => null));
    const all = loadCustomProviders(this.db);
    const pIdx = all.findIndex((x) => x.id === providerId);
    if (pIdx < 0) throw new Error("提供商不存在");
    const provider = all[pIdx]!;
    const seen = new Set(provider.models.map((m) => m.id));
    const added: string[] = [];
    const fresh: PersistedModel[] = [];
    for (const entry of entries) {
      if (seen.has(entry.id)) continue; // 已存在：不覆盖、不重排
      seen.add(entry.id);
      added.push(entry.id);
      fresh.push({
        id: entry.id,
        name: entry.name || entry.id,
        contextK: contextKFrom(entry.contextLength) || 32,
        maxTokens: 0,
        caps: { ...DEFAULT_CAPS, stream: true } as Capabilities,
        status: "",
        canonicalId: "",
        releaseDate: "",
        source: "fetched",
        enabled: true,
      });
    }
    all[pIdx] = { ...provider, models: [...provider.models, ...fresh] };
    saveCustomProviders(this.db, all);
    this.catalog.reload();
    return { fetched: entries.length, added, models: this.toView(all[pIdx]!).models };
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
      enabled: true,
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

  /**
   * 模型增量 mutation 公共路径：读全部 → 定位提供商与模型 → mutate → 写 → reload → 返回 toView。
   * mutate 收到目标提供商与目标模型下标，返回更新后的提供商（不可变替换，不原地改）。
   */
  private mutateModel(
    providerId: string,
    modelId: string,
    mutate: (p: PersistedProvider, modelIdx: number) => PersistedProvider,
  ): ProviderView {
    const all = loadCustomProviders(this.db);
    const pIdx = all.findIndex((x) => x.id === providerId);
    if (pIdx < 0) throw new Error("提供商不存在");
    const provider = all[pIdx]!;
    const mIdx = provider.models.findIndex((m) => m.id === modelId);
    if (mIdx < 0) throw new Error(`模型不存在：${modelId}`);
    all[pIdx] = mutate(provider, mIdx);
    saveCustomProviders(this.db, all);
    this.catalog.reload();
    return this.toView(all[pIdx]!);
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
    // 编辑已有提供商时按模型 ID 保留启停与来源等既有状态，避免全量保存悄悄重置（与 provider.enabled 同法）
    if (existing) {
      for (const [i, model] of models.entries()) {
        const prev = existing.models.find((x) => x.id === model.id);
        if (prev) {
          models[i] = { ...model, enabled: prev.enabled, source: preservedModelSource(prev.source, model.source) };
        }
      }
    }

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
      // 更新已有提供商时保留启停状态，避免编辑端点等操作把禁用项悄悄重新启用
      enabled: existing?.enabled ?? true,
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
      enabled: true,
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
      enabled: p.enabled,
      docUrl: p.note,
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
        source: m.source,
        enabled: m.enabled,
        available,
      })),
    };
  }
}

function connectionMessage(code: string): string {
  return CONNECTION_MESSAGES[code] ?? "连接失败，请检查配置。";
}

/**
 * 全量保存时的模型来源保留：已存在模型的 catalog / fetched 来源优先（目录与拉取是事实源，
 * 表单不含此字段）；manual 或旧数据的空串回落到新构建值（manual），避免脏值固化。
 */
function preservedModelSource(prevSource: string, fallback: string): string {
  return prevSource === "catalog" || prevSource === "fetched" ? prevSource : fallback;
}

/** 模型条目解析结果（id 必有，name / context 容错缺失）。 */
interface FetchedModelEntry {
  id: string;
  name: string;
  contextLength: unknown;
}

/** 解析 /models 响应的 data[]：整体结构不符抛可读错误；单条坏数据容错跳过。 */
function parseModelEntries(payload: unknown): FetchedModelEntry[] {
  const message = "模型列表响应格式不符，请确认端点协议是否正确";
  if (!payload || typeof payload !== "object") throw new Error(message);
  const data = (payload as Record<string, unknown>)["data"];
  if (!Array.isArray(data)) throw new Error(message);
  const entries: FetchedModelEntry[] = [];
  for (const item of data) {
    // 少数网关直接返回字符串数组
    if (typeof item === "string") {
      const id = item.trim();
      if (id) entries.push({ id, name: "", contextLength: undefined });
      continue;
    }
    if (!item || typeof item !== "object") continue;
    const rec = item as Record<string, unknown>;
    const id = typeof rec["id"] === "string" ? rec["id"].trim() : "";
    if (!id) continue;
    const name = typeof rec["display_name"] === "string" ? rec["display_name"].trim() : "";
    entries.push({ id, name, contextLength: rec["context_length"] });
  }
  return entries;
}

/** 响应的 context 字段换算为 K：数值按 token 数 /1024，字符串支持 "128k" / "1M"；无效返回 0（调用方落默认）。 */
function contextKFrom(value: unknown): number {
  const clamp = (k: number): number => Math.min(100_000, Math.max(1, Math.trunc(k)));
  if (typeof value === "number" && Number.isFinite(value) && value > 0) {
    return clamp(value / 1024);
  }
  if (typeof value === "string") {
    const m = value.trim().match(/^(\d+(?:\.\d+)?)\s*([kKmM]?)/);
    if (m) {
      const n = Number(m[1]!);
      const unit = (m[2] ?? "").toLowerCase();
      if (unit === "k") return clamp(n);
      if (unit === "m") return clamp(n * 1024);
      return clamp(n / 1024);
    }
  }
  return 0;
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
