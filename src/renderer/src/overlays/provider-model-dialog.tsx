// 模型增改共用弹窗（票 4 / spec §7）：模型 ID（catalog 来源只读）/ 显示名 / 上下文窗口 K /
// 最大输出 tokens / 能力勾选。保存成功才关窗、失败服务端错误内联红字、保存中锁定正文。
// 受控 fixed 覆盖层（本仓 overlay 体系是全屏浮层，此弹窗为设置页内局部覆盖）；
// Esc 关闭过 IME composing 守卫（spec §10），保存中不可关。
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import type { ModelInput, ModelView, ProviderView } from "../../../shared/types";

/** 上下文窗口服务端上限（buildModel：>100_000 报错）。 */
const CONTEXT_K_MAX = 100_000;
/** 最大输出 tokens 服务端边界（buildModel clamp 到 [0, 4_000_000]）。 */
const MAX_TOKENS_LIMIT = 4_000_000;

/** 客户端预校验（服务端规则的投影，重复 ID 等只能由服务端判定）：字段 → 内联红字。 */
export type ModelFormErrors = Partial<Record<"id" | "contextK", string>>;

export function validateModelForm(input: { id: string; contextK: number }): ModelFormErrors {
  const errs: ModelFormErrors = {};
  const id = input.id.trim();
  if (id === "") errs.id = "模型 ID 不能为空";
  else if (/\s/.test(id)) errs.id = "模型 ID 不能包含空白";
  if (Number.isFinite(input.contextK) && input.contextK > CONTEXT_K_MAX) {
    errs.contextK = `上下文窗口不能超过 ${CONTEXT_K_MAX}K`;
  }
  return errs;
}

/** 上下文窗口提交值归一（对齐服务端 buildModel：>0 取整，否则默认 32）。 */
function normalizeContextK(raw: number): number {
  if (Number.isFinite(raw) && raw > 0) return Math.trunc(raw);
  return 32;
}

/** 最大输出 tokens 提交值归一（对齐服务端 buildModel：clamp 到 [0, 4_000_000]）。 */
function normalizeMaxTokens(raw: number): number {
  if (!Number.isFinite(raw)) return 0;
  return Math.min(MAX_TOKENS_LIMIT, Math.max(0, Math.trunc(raw)));
}

/** 剥掉 Electron IPC 包装，保留服务端可读中文错误。 */
function cleanIpcError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const marker = "Error:";
  const idx = raw.lastIndexOf(marker);
  return (idx >= 0 ? raw.slice(idx + marker.length) : raw).trim();
}

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
  const [caps, setCaps] = useState({
    tools: model?.caps.tools ?? true,
    vision: model?.caps.vision ?? false,
    reasoning: model?.caps.reasoning ?? false,
    structured: model?.caps.structured ?? false,
  });
  const [errors, setErrors] = useState<ModelFormErrors>({});
  const [serverError, setServerError] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      // IME 组合中的 Esc（取消候选）与保存中的 Esc 都不关窗（spec §10）
      if (e.key !== "Escape" || e.isComposing || saving) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose, saving]);

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
        setServerError(cleanIpcError(err));
        setSaving(false);
      });
  };

  const field = (label: string, hint: string, err: string | undefined, control: React.JSX.Element): React.JSX.Element => (
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
    <div
      className="fixed inset-0 z-[55] flex items-center justify-center bg-black/60"
      onClick={() => {
        if (!saving) onClose();
      }}
    >
      <div
        role="dialog"
        aria-label={editing ? "编辑模型" : "添加模型"}
        className="w-[460px] max-w-[calc(100vw-40px)] rounded-2xl border border-popover-border bg-popover p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-ui-lg font-semibold text-foreground">
          {editing ? "编辑模型" : "添加模型"}
          <span className="ml-2 text-ui-sm font-normal text-foreground-subtlest">{providerName}</span>
        </h3>
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
      </div>
    </div>
  );
}
