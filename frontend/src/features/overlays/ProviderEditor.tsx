/**
 * 提供商编辑器。
 *
 * 负责：新增或修改一个模型提供商（名称、协议、端点、是否需要密钥、密钥），
 * 维护它下面的模型列表，在提交前做本地校验，保存成功后把结果回传给调用方。
 *
 * 密钥不会回显：编辑已有提供商时输入框始终为空，留空表示保留原密钥；
 * 填写后与提供商配置一起保存，明文只写入 Go 侧的 keys.json。
 */
import { useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import type { Capabilities, Cost } from "../../../bindings/craftwork/internal/model";
import type { ModelView, ProviderInput, ProviderView } from "../../lib/types";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";

/** 表单里的模型草稿，字段与后端 ProviderInput 中的模型结构基本一致。 */
type ModelDraft = {
  id: string;
  name: string;
  contextK: number;
  maxTokens: number;
  caps: Capabilities;
  cost: Cost | null;
  status: string;
  canonicalId: string;
  releaseDate: string;
};

/** 协议选项：只保留 OpenAI、Anthropic 与 OpenAI 兼容三种。 */
const kindOptions = [
  { id: "openai-compatible", label: "OpenAI 兼容" },
  { id: "openai", label: "OpenAI" },
  { id: "anthropic", label: "Anthropic" },
];

/** 模型能力开关；流式能力固定开启，因此不在此列表中。 */
const capabilityOptions: Array<{ key: "tools" | "vision" | "reasoning" | "structured"; label: string }> = [
  { key: "tools", label: "工具" },
  { key: "vision", label: "图片" },
  { key: "reasoning", label: "推理" },
  { key: "structured", label: "结构化" },
];

/** 把后端返回的模型转换成表单草稿，并强制开启流式能力。 */
function draftFromModel(model: ModelView): ModelDraft {
  return {
    id: model.id,
    name: model.name,
    contextK: model.contextK,
    maxTokens: model.maxTokens ?? 0,
    caps: { ...model.caps, stream: true },
    cost: model.cost ?? null,
    status: model.status ?? "",
    canonicalId: model.canonicalId ?? "",
    releaseDate: model.releaseDate ?? "",
  };
}

/** 生成一个空的模型草稿，作为新增模型时的初始值。 */
function blankModel(): ModelDraft {
  return {
    id: "",
    name: "",
    contextK: 32,
    maxTokens: 0,
    caps: { stream: true, tools: true, vision: false, reasoning: false, structured: false },
    cost: null,
    status: "",
    canonicalId: "",
    releaseDate: "",
  };
}

/**
 * 提供商编辑器组件。
 *
 * props：
 * - provider：要编辑的提供商；传 null 表示新增；
 * - onCancel：取消编辑；
 * - onSaved：保存成功后回调，参数是后端返回的最新提供商数据。
 *
 * 表单字段用独立 state 保存；error 与 saving 用于展示校验结果和提交状态。
 */
export function ProviderEditor({
  provider,
  onCancel,
  onSaved,
}: {
  provider: ProviderView | null;
  onCancel: () => void;
  onSaved: (provider: ProviderView) => void;
}) {
  const toast = useUiStore((state) => state.toast);
  const [name, setName] = useState(provider?.name ?? "");
  const [kind, setKind] = useState(provider?.kind ?? "openai-compatible");
  const [endpoint, setEndpoint] = useState(provider?.endpoint ?? "");
  const [keyRequired, setKeyRequired] = useState(provider?.keyRequired ?? true);
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  // 保存成功后记住后端生成的 ID：密钥写入失败时用户可以重试，
  // 重试会覆盖同一个提供商，而不是再新增一份配置。
  const [providerId, setProviderId] = useState(provider?.id ?? "");
  const [models, setModels] = useState<ModelDraft[]>(
    provider && provider.models.length > 0 ? provider.models.map(draftFromModel) : [blankModel()],
  );
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  // lookup 记录“从目录补全”的进度与结果，按模型行号展示。
  const [lookup, setLookup] = useState<{ index: number; text: string; error: boolean } | null>(null);

  // 已有密钥时只给出掩码提示，输入框保持为空，避免把密钥回填到页面。
  const keyConfigured = Boolean(provider?.keySource);

  /**
   * 按模型 ID 从 models.dev 目录补全上下文、输出上限、能力与价格。
   *
   * 提供商 ID 为空时后端会在整个目录中查找；只有唯一匹配才返回，
   * 多个提供商都有同名模型时会提示用户先填写提供商 ID。
   */
  async function fillFromCatalog(index: number) {
    const modelID = models[index]?.id.trim() ?? "";
    if (!modelID) {
      return;
    }
    setLookup({ index, text: "正在查找目录…", error: false });
    try {
      const found = await api.providers.lookupCatalogModel(providerId, modelID);
      if (!found) {
        setLookup({ index, text: "目录中没有这个模型", error: true });
        return;
      }
      setModels((current) =>
        current.map((item, itemIndex) =>
          itemIndex === index
            ? {
                ...item,
                // 名称只在用户没填时补全，避免覆盖自定义展示名称。
                name: item.name.trim() || found.name,
                contextK: found.contextK,
                maxTokens: found.maxTokens ?? 0,
                caps: { ...found.caps, stream: true },
                cost: found.cost ?? null,
                status: found.status ?? "",
                canonicalId: found.canonicalId ?? "",
                releaseDate: found.releaseDate ?? "",
              }
            : item,
        ),
      );
      setLookup({ index, text: `已从 ${found.providerName} 目录补全`, error: false });
    } catch (caught) {
      setLookup({ index, text: String(caught), error: true });
    }
  }

  /** 更新第 index 个模型的部分字段。 */
  function updateModel(index: number, patch: Partial<ModelDraft>) {
    setModels((current) => current.map((model, itemIndex) => (itemIndex === index ? { ...model, ...patch } : model)));
  }

  /** 切换第 index 个模型的某项能力开关。 */
  function updateCapability(
    index: number,
    key: "tools" | "vision" | "reasoning" | "structured",
    value: boolean,
  ) {
    setModels((current) =>
      current.map((model, itemIndex) =>
        itemIndex === index ? { ...model, caps: { ...model.caps, [key]: value } } : model,
      ),
    );
  }

  /**
   * 校验并提交表单。
   *
   * 校验顺序：名称 → 端点 → 至少一个模型 → 每个模型的 ID 与上下文窗口。
   * 全部通过后组装 ProviderInput 调用后端；如果填了密钥，紧接着写入 keys.json。
   * 保存失败把错误显示在表单底部。
   */
  async function save() {
    const trimmedName = name.trim();
    const trimmedEndpoint = endpoint.trim();
    if (!trimmedName) {
      setError("请填写提供商名称");
      return;
    }
    if (!trimmedEndpoint) {
      setError("请填写端点地址");
      return;
    }
    if (models.length === 0) {
      setError("至少添加一个模型");
      return;
    }
    for (const model of models) {
      if (!model.id.trim()) {
        setError("模型 ID 不能为空");
        return;
      }
      if (model.contextK <= 0) {
        setError("上下文窗口必须大于 0");
        return;
      }
    }

    const input: ProviderInput = {
      // 新增时 id 为空，由后端生成；编辑或重试时沿用已保存的 id 覆盖配置。
      id: providerId,
      name: trimmedName,
      kind,
      endpoint: trimmedEndpoint,
      keyRequired,
      models: models.map((model) => ({
        id: model.id.trim(),
        // 显示名称留空时回退为模型 ID，保证列表里总有可读的名称。
        name: model.name.trim() || model.id.trim(),
        contextK: model.contextK,
        maxTokens: model.maxTokens,
        caps: { ...model.caps, stream: true },
        cost: model.cost,
        status: model.status,
        canonicalId: model.canonicalId,
        releaseDate: model.releaseDate,
      })),
    };

    const trimmedKey = apiKey.trim();
    const writeKey = keyRequired && trimmedKey !== "";

    setSaving(true);
    setError("");
    try {
      const saved = await api.providers.saveProvider(input);
      setProviderId(saved.id);
      let result = saved;
      if (writeKey) {
        await api.providers.setKey(saved.id, trimmedKey);
        // 重新拉取列表，拿到最新的密钥来源与掩码后再回传给调用方，
        // 这样设置页不会误报「还需配置 API Key」。
        const providers = await api.providers.list();
        result = providers.find((item) => item.id === saved.id) ?? saved;
      }
      toast({
        title: provider ? "提供商已更新" : "提供商已添加",
        description: `${result.name} · ${result.models.length} 个模型${writeKey ? " · 密钥已保存" : ""}`,
        tone: "ok",
      });
      onSaved(result);
    } catch (caught) {
      setError(String(caught));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="provider-editor">
      <div className="provider-editor-head">
        {/* 标题根据是新增还是编辑切换 */}
        <Icon name="cpu" />
        <h3>{provider ? "编辑提供商" : "添加提供商"}</h3>
        <span className="spacer" />
        <button type="button" className="icon-btn sm" aria-label="关闭提供商编辑器" onClick={onCancel}>
          <Icon name="x" />
        </button>
      </div>

      {provider ? (
        <div className="sec-desc">
          提供商 ID：<span className="mono">{provider.id}</span>
        </div>
      ) : null}

      <div className="provider-form">
        {/* 基本信息：名称、协议、端点与密钥设置 */}
        <label className="field">
          <span>名称</span>
          <input
            className="text-input wide"
            value={name}
            placeholder="例如 公司网关"
            onChange={(event) => setName(event.target.value)}
          />
        </label>
        <label className="field">
          <span>协议</span>
          <select className="select" value={kind} onChange={(event) => setKind(event.target.value)}>
            {kindOptions.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <label className="field span-2">
          <span>端点</span>
          <input
            className="text-input wide mono"
            value={endpoint}
            placeholder="https://api.example.com/v1"
            onChange={(event) => setEndpoint(event.target.value)}
          />
        </label>

        <div className="key-settings span-2">
          <div className="checkbox-box">
            <input
              id="provider-key-required"
              type="checkbox"
              checked={keyRequired}
              onChange={(event) => setKeyRequired(event.target.checked)}
            />
            <div className="checkbox-text">
              <label className="checkbox-title" htmlFor="provider-key-required">
                需要 API Key
              </label>
              <div className="checkbox-desc">本地服务（Ollama 等）可以关闭；密钥只写入本机 keys.json。</div>
            </div>
          </div>

          {keyRequired ? (
            <div className="field">
              <span>API Key</span>
              <div className="key-input">
                <input
                  className="text-input wide mono"
                  type={showKey ? "text" : "password"}
                  value={apiKey}
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={keyConfigured ? `已配置 ${provider?.keyMasked || "密钥"}；留空则保持不变` : "粘贴 API Key"}
                  aria-label="API Key"
                  onChange={(event) => setApiKey(event.target.value)}
                />
                <button
                  type="button"
                  className="icon-btn sm"
                  aria-label={showKey ? "隐藏 API Key" : "显示 API Key"}
                  title={showKey ? "隐藏密钥" : "显示密钥"}
                  onClick={() => setShowKey((current) => !current)}
                >
                  <Icon name={showKey ? "eye-off" : "eye"} />
                </button>
              </div>
            </div>
          ) : null}
        </div>
      </div>

      <div className="provider-models">
        <div className="provider-models-head">
          {/* 模型区表头：流式固定开启，并提供「添加模型」按钮 */}
          <span>模型</span>
          <span className="sec-desc">流式固定开启。</span>
          <span className="spacer" />
          <button type="button" className="btn sm ghost" onClick={() => setModels((current) => [...current, blankModel()])}>
            <Icon name="plus" className="ico" />
            添加模型
          </button>
        </div>

        {models.map((model, index) => (
          <div className="provider-model-row" key={index}>
            <div className="pmr-head">
              <span className="pmr-title">模型 {index + 1}</span>
              <span className="spacer" />
              <button
                type="button"
                className="icon-btn sm pmr-remove"
                aria-label={`移除模型 ${index + 1}`}
                title={models.length <= 1 ? "至少保留一个模型" : "移除模型"}
                disabled={models.length <= 1}
                onClick={() => setModels((current) => current.filter((_, itemIndex) => itemIndex !== index))}
              >
                <Icon name="trash" />
              </button>
            </div>

            {/* 模型字段：每个输入都有可见标签，单位写在标签里 */}
            <div className="pmr-main">
              <label className="field">
                <span>模型 ID</span>
                <div className="key-input">
                  <input
                    className="text-input"
                    value={model.id}
                    placeholder="例如 gpt-4.1"
                    aria-label={`模型 ${index + 1} ID`}
                    onChange={(event) => updateModel(index, { id: event.target.value })}
                    onBlur={() => void fillFromCatalog(index)}
                  />
                  <button
                    type="button"
                    className="btn sm ghost"
                    title="从 models.dev 目录补全上下文、能力与价格"
                    onClick={() => void fillFromCatalog(index)}
                  >
                    <Icon name="search" className="ico" />
                    目录补全
                  </button>
                </div>
              </label>
              <label className="field">
                <span>显示名称</span>
                <input
                  className="text-input"
                  value={model.name}
                  placeholder="例如 GPT-4.1"
                  aria-label={`模型 ${index + 1} 名称`}
                  onChange={(event) => updateModel(index, { name: event.target.value })}
                />
              </label>
              <label className="field">
                <span>上下文（千 token）</span>
                <input
                  className="text-input pmr-ctx"
                  type="number"
                  min={1}
                  max={100000}
                  value={model.contextK}
                  placeholder="32"
                  aria-label={`模型 ${index + 1} 上下文窗口`}
                  onChange={(event) => updateModel(index, { contextK: Number(event.target.value) || 0 })}
                />
              </label>
              <label className="field">
                <span>最大输出（token）</span>
                <input
                  className="text-input pmr-ctx"
                  type="number"
                  min={0}
                  max={4000000}
                  value={model.maxTokens === 0 ? "" : model.maxTokens}
                  placeholder="默认"
                  aria-label={`模型 ${index + 1} 最大输出 token`}
                  onChange={(event) => updateModel(index, { maxTokens: Number(event.target.value) || 0 })}
                />
              </label>
            </div>

            <div className="pmr-caps">
              {/* 流式固定开启，用只读勾选框展示；其余能力可自由勾选 */}
              <label className={cn("cap-check", "is-fixed")}>
                <input type="checkbox" checked readOnly disabled />
                流式
              </label>
              {capabilityOptions.map((option) => (
                <label key={option.key} className="cap-check">
                  <input
                    type="checkbox"
                    checked={Boolean(model.caps[option.key])}
                    onChange={(event) => updateCapability(index, option.key, event.target.checked)}
                  />
                  {option.label}
                </label>
              ))}
            </div>
            {lookup?.index === index ? (
              <div className={cn("pmr-hint", lookup.error && "is-error")}>{lookup.text}</div>
            ) : null}
          </div>
        ))}
      </div>

      {error ? <div className="provider-editor-error">{error}</div> : null}

      <div className="provider-editor-actions">
        {/* 底部操作：取消与保存；保存期间禁用按钮并显示加载图标 */}
        <button type="button" className="btn ghost" onClick={onCancel}>
          取消
        </button>
        <button type="button" className="btn primary" disabled={saving} onClick={() => void save()}>
          {saving ? <Icon name="loader" className="ico spin" /> : null}
          保存提供商
        </button>
      </div>
    </div>
  );
}
