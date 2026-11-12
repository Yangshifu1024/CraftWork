// 模型增改共用弹窗（票 4 / spec §7）：模型 ID（catalog 来源只读）/ 显示名 / 上下文窗口 K /
// 最大输出 tokens / 能力勾选。保存成功才关窗、失败服务端错误内联红字、保存中锁定正文。
// 设置页内局部覆盖层：遮罩 / Esc / 焦点圈定由 radix Dialog 承担；
// Esc 关闭过 IME composing 守卫（spec §10），保存中不可关。
import { useState } from "react";
import { Dialog } from "radix-ui";
import { Select } from "../components/select";
import { api } from "../lib/api";
import { cleanErrorMessage } from "../lib/errors";
import {
  MAX_TOKENS_LIMIT,
  normalizeContextK,
  normalizeMaxTokens,
  validateModelForm,
  type ModelFormErrors,
} from "../lib/model-form";
import type { ModelInput, ModelView, ProviderView } from "../../../shared/types";

/** 思考档位声明预设（effort 集，注入 reasoning_effort；budget 型仅 Anthropic 原生协议，不对手动声明开放）。
 *  levels undefined = 保留现值（保存时不携带档位字段）。 */
const REASONING_PRESETS: Array<{ id: string; label: string; kind?: string; levels?: string[] }> = [
  { id: "keep", label: "跟随目录 / 保留现状" },
  { id: "none", label: "不支持（清除声明）", kind: "", levels: [] },
  { id: "deepseek-off", label: "关闭 / 低 / 高 / 最高", kind: "effort", levels: ["none", "low", "high", "max"] },
  { id: "deepseek", label: "低 / 高 / 最高", kind: "effort", levels: ["low", "high", "max"] },
  { id: "openai", label: "极低 / 低 / 中 / 高", kind: "effort", levels: ["minimal", "low", "medium", "high"] },
];

export function ProviderModelDialog({
  providerId,
  providerName,
  model,
  onClose,
  onSaved,
}: {
  providerId: string;
  providerName: string;
  /** null = 新增；非 null = 编辑该模型 */
  model: ModelView | null;
  onClose(): void;
  /** 保存成功后回调（父层刷新列表）；关窗由本组件负责 */
  onSaved(view: ProviderView): void;
}): React.JSX.Element {
  const editing = model !== null;
  // catalog 来源的模型 ID 只读（目录事实源，改 ID 等于换了模型）
  const idReadonly = editing && model.source === "catalog";

  const [id, setId] = useState(model?.id ?? "");
  const [name, setName] = useState(editing && model.name !== model.id ? model.name : "");
  const [contextK, setContextK] = useState(String(model?.contextK ?? 32));
  const [maxTokens, setMaxTokens] = useState(String(model?.maxTokens ?? 0));
  // stream 无勾选项（协议层恒启用），落库时补全完整 Capabilities
  const [caps, setCaps] = useState({
    stream: true,
    tools: model?.caps.tools ?? true,
    vision: model?.caps.vision ?? false,
    reasoning: model?.caps.reasoning ?? false,
    structured: model?.caps.structured ?? false,
  });
  const [errors, setErrors] = useState<ModelFormErrors>({});
  const [serverError, setServerError] = useState("");
  const [saving, setSaving] = useState(false);
  // 思考档位声明预设：目录外的网关模型无元数据，手动声明后模型浮层才出思考强度区。
  // keep = 保存时不携带档位字段（服务端保留现值：目录归一值或上次声明）。
  const currentLevels = model?.reasoningLevels ?? [];
  const matchedPreset = REASONING_PRESETS.find(
    (p) => p.levels !== undefined && p.levels.join(",") === currentLevels.join(","),
  );
  const [reasoningPreset, setReasoningPreset] = useState<string>(matchedPreset?.id ?? (currentLevels.length > 0 ? "keep" : "none"));

  const submit = (): void => {
    if (saving) return;
    const idValue = id.trim();
    const errs = validateModelForm({ id: idValue, contextK: Number(contextK) });
    setErrors(errs);
    if (Object.keys(errs).length > 0) return;
    const payload: ModelInput = {
      id: idValue,
      // 显示名留空默认取 ID（服务端 buildModel 同法兜底）
      name: name.trim() || idValue,
      contextK: normalizeContextK(Number(contextK)),
      maxTokens: normalizeMaxTokens(Number(maxTokens)),
      caps,
    };
    // 档位声明：keep 不携带字段（服务端保留现值）；其余按预设写入（none = 显式清除）
    const preset = REASONING_PRESETS.find((p) => p.id === reasoningPreset);
    if (preset && preset.levels !== undefined) {
      payload.reasoningKind = preset.kind;
      payload.reasoningLevels = preset.levels;
    }
    setSaving(true);
    setServerError("");
    const call = editing
      ? api.providerUpdateModel(providerId, model.id, payload)
      : api.providerAddModel(providerId, payload);
    call
      .then((view) => {
        onSaved(view);
        onClose();
      })
      .catch((err) => {
        setServerError(cleanErrorMessage(err));
        setSaving(false);
      });
  };

  const field = (label: string, hint: string | undefined, err: string | undefined, control: React.JSX.Element): React.JSX.Element => (
    <div className="flex flex-col gap-1">
      <label className="text-ui-xs text-foreground-subtle">
        {label}
        {hint && <span className="ml-1.5 text-foreground-subtlest">{hint}</span>}
      </label>
      {control}
      {err && <span className="text-ui-xs text-destructive">{err}</span>}
    </div>
  );

  const inputCls = (invalid: boolean): string =>
    `h-8 w-full rounded-lg border bg-input px-2.5 text-ui-sm disabled:cursor-not-allowed disabled:opacity-50 ${
      invalid ? "border-destructive" : "border-border"
    }`;

  const CAP_OPTIONS: Array<{ key: keyof typeof caps; label: string }> = [
    { key: "tools", label: "工具调用" },
    { key: "vision", label: "视觉" },
    { key: "reasoning", label: "推理" },
    { key: "structured", label: "结构化输出" },
  ];

  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o && !saving) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
        <Dialog.Content
          aria-label={editing ? "编辑模型" : "添加模型"}
          aria-describedby={undefined}
          onEscapeKeyDown={(e) => {
            // IME 组合中的 Esc（取消候选）与保存中的 Esc 都不关窗（spec §10）；事件为原生 KeyboardEvent
            if (saving || e.isComposing) e.preventDefault();
          }}
          onInteractOutside={(e) => {
            if (saving) e.preventDefault();
          }}
          className="fixed left-1/2 top-1/2 z-[55] w-[460px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl outline-none"
        >
        <Dialog.Title className="text-ui-lg font-semibold text-foreground">
          {editing ? "编辑模型" : "添加模型"}
          <span className="ml-2 text-ui-sm font-normal text-foreground-subtlest">{providerName}</span>
        </Dialog.Title>
        <div className="flex flex-col gap-3 pt-4">
          <div className="grid grid-cols-2 gap-3">
            {field(
              "模型 ID",
              idReadonly ? "目录来源，不可修改" : "必填",
              errors.id,
              <input
                autoFocus
                value={id}
                readOnly={idReadonly}
                disabled={saving}
                onChange={(e) => {
                  setId(e.target.value);
                  if (errors.id) setErrors({});
                }}
                placeholder="例如：deepseek-chat"
                aria-invalid={errors.id !== undefined}
                className={`${inputCls(errors.id !== undefined)} font-mono text-ui-xs`}
              />,
            )}
            {field(
              "显示名",
              "留空取 ID",
              undefined,
              <input
                value={name}
                disabled={saving}
                onChange={(e) => setName(e.target.value)}
                placeholder={id.trim() || "与模型 ID 相同"}
                className={inputCls(false)}
              />,
            )}
          </div>
          <div className="grid grid-cols-2 gap-3">
            {field(
              "上下文窗口（K）",
              "正整数",
              errors.contextK,
              <input
                type="number"
                min={1}
                value={contextK}
                disabled={saving}
                onChange={(e) => {
                  setContextK(e.target.value);
                  if (errors.contextK) setErrors({});
                }}
                aria-invalid={errors.contextK !== undefined}
                className={inputCls(errors.contextK !== undefined)}
              />,
            )}
            {field(
              "最大输出（tokens）",
              "0 – 4,000,000",
              undefined,
              <input
                type="number"
                min={0}
                max={MAX_TOKENS_LIMIT}
                value={maxTokens}
                disabled={saving}
                onChange={(e) => setMaxTokens(e.target.value)}
                className={inputCls(false)}
              />,
            )}
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-ui-xs text-foreground-subtle">能力</span>
            <div className="flex flex-wrap gap-x-4 gap-y-1.5">
              {CAP_OPTIONS.map((o) => (
                <label key={o.key} className="flex cursor-pointer items-center gap-1.5 text-ui-sm text-foreground">
                  <input
                    type="checkbox"
                    checked={caps[o.key]}
                    disabled={saving}
                    onChange={(e) => setCaps((c) => ({ ...c, [o.key]: e.target.checked }))}
                  />
                  {o.label}
                </label>
              ))}
            </div>
          </div>
          {field(
            "思考档位",
            model?.source === "catalog" ? undefined : "目录外网关模型可手动声明",
            undefined,
            <Select
              ariaLabel="思考档位"
              fullWidth
              disabled={saving}
              value={reasoningPreset}
              onChange={setReasoningPreset}
              options={REASONING_PRESETS.map((p) => ({
                value: p.id,
                label:
                  p.id === "keep" && currentLevels.length > 0
                    ? `${p.label}（当前：${currentLevels.join(" / ")}）`
                    : p.label,
              }))}
            />,
          )}
          {serverError && <p className="text-ui-xs text-destructive">{serverError}</p>}
          <div className="flex justify-end gap-2 pt-1">
            <button
              className="h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
              disabled={saving}
              onClick={() => onClose()}
            >
              取消
            </button>
            <button
              className="h-8 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
              disabled={saving}
              onClick={submit}
            >
              {saving ? (
                <span className="animated-gradient-text">保存中…</span>
              ) : editing ? (
                "保存修改"
              ) : (
                "添加模型"
              )}
            </button>
          </div>
        </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
