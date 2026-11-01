/**
 * 提供商编辑器。
 *
 * 负责：新增或修改一个模型提供商（名称、协议、端点、是否需要密钥），
 * 维护它下面的模型列表，在提交前做本地校验，保存成功后把结果回传给调用方。
 *
 * 密钥不在这里填写：保存提供商后，由设置页的提供商卡片单独设置。
 */
import { useState } from "react";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import type { Capabilities } from "../../../bindings/craftwork/internal/model";
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
};

/** 协议选项：OpenAI 兼容、OpenAI、Anthropic 与 Google Gemini。 */
const kindOptions = [
  { id: "openai-compatible", label: "OpenAI 兼容" },
  { id: "openai", label: "OpenAI" },
  { id: "anthropic", label: "Anthropic" },
  { id: "gemini", label: "Google Gemini" },
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
  const [models, setModels] = useState<ModelDraft[]>(
    provider && provider.models.length > 0 ? provider.models.map(draftFromModel) : [blankModel()],
  );
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);

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
   * 全部通过后组装 ProviderInput 调用后端；保存失败把错误显示在表单底部。
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
      // 新增时 id 为空，由后端生成；编辑时沿用原 id 覆盖配置。
      id: provider?.id ?? "",
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
      })),
    };

    setSaving(true);
    setError("");
    try {
      const saved = await api.providers.saveProvider(input);
      toast({
        title: provider ? "提供商已更新" : "提供商已添加",
        description: `${saved.name} · ${saved.models.length} 个模型`,
        tone: "ok",
      });
      onSaved(saved);
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
          {provider.overridden
            ? " · 保存后覆盖内置配置，可在卡片上恢复默认"
            : provider.custom
              ? " · 自定义提供商"
              : " · 保存后创建内置覆盖配置"}
        </div>
      ) : null}

      <div className="provider-form">
        {/* 基本信息：名称、协议、端点与是否需要密钥 */}
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
        <label className="checkbox-line span-2">
          <input
            type="checkbox"
            checked={keyRequired}
            onChange={(event) => setKeyRequired(event.target.checked)}
          />
          需要 API Key
          <span className="sec-desc">本地服务（Ollama 等）可以关闭；密钥仍按提供商保存在 keys.json。</span>
        </label>
      </div>

      <div className="provider-models">
        <div className="provider-models-head">
          {/* 模型区表头：说明上下文单位，并提供「添加模型」按钮 */}
          <span>模型</span>
          <span className="sec-desc">上下文以千 token 计；流式固定开启。</span>
          <span className="spacer" />
          <button type="button" className="btn sm ghost" onClick={() => setModels((current) => [...current, blankModel()])}>
            <Icon name="plus" className="ico" />
            添加模型
          </button>
        </div>

        {models.map((model, index) => (
          <div className="provider-model-row" key={index}>
            <div className="pmr-main">
              <input
                className="text-input"
                value={model.id}
                placeholder="模型 ID"
                aria-label={`模型 ${index + 1} ID`}
                onChange={(event) => updateModel(index, { id: event.target.value })}
              />
              <input
                className="text-input"
                value={model.name}
                placeholder="显示名称"
                aria-label={`模型 ${index + 1} 名称`}
                onChange={(event) => updateModel(index, { name: event.target.value })}
              />
              <input
                className="text-input pmr-ctx"
                type="number"
                min={1}
                max={10000}
                value={model.contextK}
                aria-label={`模型 ${index + 1} 上下文窗口`}
                onChange={(event) => updateModel(index, { contextK: Number(event.target.value) || 0 })}
              />
              <span className="pmr-unit">k</span>
              <input
                className="text-input pmr-ctx"
                type="number"
                min={0}
                max={200000}
                value={model.maxTokens === 0 ? "" : model.maxTokens}
                placeholder="最大输出"
                aria-label={`模型 ${index + 1} 最大输出 token`}
                onChange={(event) => updateModel(index, { maxTokens: Number(event.target.value) || 0 })}
              />
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
            <button
              type="button"
              className="icon-btn sm"
              aria-label={`移除模型 ${index + 1}`}
              disabled={models.length <= 1}
              onClick={() => setModels((current) => current.filter((_, itemIndex) => itemIndex !== index))}
            >
              <Icon name="trash" />
            </button>
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
