// 模型客户端工厂：按引用解析提供商 → 构造协议客户端（统一套重试）→ 密钥校验与回退链。
import { Code, ModelError } from "./errors";
import type { Catalog } from "./catalog";
import type { Client, ModelInfo } from "./types";
import { AnthropicClient } from "./anthropic";
import { OpenAICompatClient } from "./openai";
import { RetryClient } from "./retry";

export interface KeyProvider {
  getFor(providerId: string, envNames: string[]): string;
}

export class ModelFactory {
  private fallbackResolver: (() => string) | null = null;

  constructor(
    public readonly catalog: Catalog,
    private readonly keys: KeyProvider,
  ) {}

  /** 解析引用或原始 ID 为模型信息（找不到返回 null）。 */
  resolve(refOrId: string): ModelInfo | null {
    return this.catalog.resolve(refOrId);
  }

  /** 提供商协议类型（费用估算的缓存口径分支用）。 */
  providerKind(providerId: string): string {
    return this.catalog.findProvider(providerId)?.kind ?? "";
  }

  /** 实时读取备用模型设置（改设置免重启）。 */
  setFallbackResolver(fn: () => string): void {
    this.fallbackResolver = fn;
  }

  client(refOrId: string): Client {
    const info = this.catalog.resolve(refOrId);
    if (!info) {
      throw new ModelError(Code.ProviderUnavailable, refOrId, "模型不存在或已被删除，请重新选择模型");
    }
    const provider = this.catalog.findProvider(info.provider);
    if (!provider) {
      throw new ModelError(Code.ProviderUnavailable, info.provider, "提供商不存在，请重新选择模型");
    }
    const displayName = provider.name || provider.id;
    // 启停语义在调用侧收口（spec §9）：停用提供商 / 模型不可调用，但 resolve/findModel
    // 保持不变——历史会话引用仍可解析显示名称，只在真正调用时给出可读停用提示。
    if (provider.enabled === false) {
      throw new ModelError(
        Code.ProviderUnavailable,
        provider.id,
        `${displayName}：该模型已停用，请重新选择模型`,
      );
    }
    const persisted = provider.models.find((m) => m.id === info.id);
    if (persisted?.enabled === false) {
      throw new ModelError(
        Code.ProviderUnavailable,
        provider.id,
        `${displayName}：该模型已停用，请重新选择模型`,
      );
    }
    const key = this.keys.getFor(provider.id, provider.envVars);
    if (provider.keyRequired && !key) {
      throw new ModelError(Code.Auth, provider.name, `${provider.name} 尚未配置密钥，请在设置 → 模型 中粘贴 API Key`);
    }
    let inner: Client;
    if (provider.kind === "anthropic") {
      inner = new AnthropicClient(provider.endpoint, key, displayName);
    } else {
      // openai 与 openai-compatible 共用 OpenAI 协议适配器
      inner = new OpenAICompatClient(provider.endpoint, key, displayName);
    }
    return new RetryClient(inner, displayName, 3, 700, 5000);
  }

  /** 备用模型引用：存在、与当前不同、可解析、未停用、需要密钥时密钥可得。 */
  fallbackModel(refOrId: string): string {
    if (!this.fallbackResolver) return "";
    const candidate = this.fallbackResolver();
    if (!candidate) return "";
    const current = this.catalog.resolve(refOrId);
    const fallback = this.catalog.resolve(candidate);
    if (!fallback || !current) return "";
    const currentRef = `${current.provider}/${current.id}`;
    const fallbackRef = `${fallback.provider}/${fallback.id}`;
    if (currentRef === fallbackRef) return "";
    const provider = this.catalog.findProvider(fallback.provider);
    if (!provider) return "";
    // 与「密钥不可得返回空」同模式：停用候选不参与失败转移（spec §9）
    if (provider.enabled === false) return "";
    const persisted = provider.models.find((m) => m.id === fallback.id);
    if (persisted?.enabled === false) return "";
    if (provider.keyRequired && !this.keys.getFor(provider.id, provider.envVars)) return "";
    return fallbackRef;
  }
}
