// 添加提供商页（票 6 / spec §7 §3.2 §11）：设置页右栏整体切换的添加视图。
// 两张入口卡：① 从模型目录导入（原目录浏览器移入右栏改造）② 手动创建（内嵌表单，服务端校验）。
// 成功路径统一「创建即编辑」：toast + 经 onDone 自动选中新供应商并返回详情视图。
// 供应商详情卡归供应商详情票维护，本文件不触碰 ProviderDetail。
import { useEffect, useState } from "react";
import { ArrowLeft, BookOpen, PenLine } from "lucide-react";
import { api } from "../lib/api";
import { useToasts } from "../store/toast";
import { ProviderLogo } from "../components/provider-logo";
import { Select } from "../components/select";
import { needsEndpoint, validateManualForm, type ManualFormErrors } from "../lib/provider-add";
import { cleanErrorMessage } from "../lib/errors";
import { normalizeContextK } from "../lib/model-form";
import type { CatalogModelView, CatalogProviderView, ProviderView } from "../../../shared/types";

export type AddEntry = "catalog" | "manual";

/** 添加页：initialEntry 为 null 时先展示两张入口卡，否则直达对应入口（空态引导按钮直达用）。 */
export function ProviderAddPage({
  initialEntry,
  onBack,
  onDone,
}: {
  initialEntry: AddEntry | null;
  onBack(): void;
  onDone(provider: ProviderView): void;
}): React.JSX.Element {
  const [mode, setMode] = useState<AddEntry | null>(initialEntry);
  return (
    <div className="flex min-h-full flex-col gap-4 p-4">
      <div className="flex items-center gap-2">
        <button
          className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
          onClick={onBack}
        >
          <ArrowLeft className="size-3.5" /> 返回
        </button>
        <span className="min-w-0 truncate text-ui-base font-medium text-foreground">添加提供商</span>
        {mode !== null && (
          <button
            className="ml-auto shrink-0 text-ui-sm text-foreground-subtle hover:text-foreground"
            onClick={() => setMode(null)}
          >
            返回选择
          </button>
        )}
      </div>

      {mode === null && <EntryCards onPick={setMode} />}
      {mode === "catalog" && <CatalogImportPanel onDone={onDone} />}
      {mode === "manual" && <ManualCreateForm onDone={onDone} />}
    </div>
  );
}

/** 两张入口卡（spec §7）：目录导入 / 手动创建。 */
function EntryCards({ onPick }: { onPick(entry: AddEntry): void }): React.JSX.Element {
  const cards: Array<{ entry: AddEntry; icon: typeof BookOpen; title: string; desc: string }> = [
    { entry: "catalog", icon: BookOpen, title: "从模型目录导入", desc: "从 models.dev 离线目录挑选提供商与模型，端点自动预填" },
    { entry: "manual", icon: PenLine, title: "手动创建", desc: "自定义名称、协议与端点，直接填写连接信息" },
  ];
  return (
    <div className="grid grid-cols-2 gap-3">
      {cards.map((c) => (
        <button
          key={c.entry}
          className="flex flex-col items-start gap-2 rounded-xl border border-border/50 bg-panel px-4 py-5 text-left hover:border-border hover:bg-surface-hover"
          onClick={() => onPick(c.entry)}
        >
          <c.icon className="size-5 text-foreground-subtle" />
          <span className="text-ui-base font-medium text-foreground">{c.title}</span>
          <span className="text-ui-sm text-foreground-subtlest">{c.desc}</span>
        </button>
      ))}
    </div>
  );
}

// ---------- 入口一：从模型目录导入 ----------

/** 目录导入面板（原 CatalogBrowser 改造移入右栏）：搜索 → 选提供商 → 勾模型 → 导入。 */
function CatalogImportPanel({ onDone }: { onDone(provider: ProviderView): void }): React.JSX.Element {
  const pushToast = useToasts((s) => s.push);
  const [query, setQuery] = useState("");
  const [providers, setProviders] = useState<CatalogProviderView[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [models, setModels] = useState<CatalogModelView[]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    void api.catalogProviders(query).then(setProviders);
  }, [query]);

  useEffect(() => {
    if (!selected) {
      setModels([]);
      return;
    }
    void api.catalogModels(selected).then(setModels);
    setPicked(new Set());
  }, [selected]);

  const selectedProvider = providers.find((p) => p.id === selected) ?? null;
  const addedSelected = selectedProvider?.added ?? false;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="搜索提供商…"
        className="h-8 w-full shrink-0 rounded-lg border border-border bg-input px-3 text-ui-sm"
      />
      <div className="grid min-h-0 flex-1 grid-cols-2 gap-3">
        {/* 左列：提供商列表（logo + 名称 + 端点徽标 + 模型数 / 已添加） */}
        <div className="min-h-0 overflow-y-auto rounded-xl border border-border/40 p-1.5">
          {providers.length === 0 ? (
            <div className="px-3 py-8 text-center text-ui-sm text-foreground-subtlest">未找到匹配的提供商</div>
          ) : (
            providers.map((p) => (
              <button
                key={p.id}
                className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-ui-sm ${
                  selected === p.id ? "bg-selected text-foreground" : "text-foreground hover:bg-surface-hover"
                }`}
                onClick={() => setSelected(p.id)}
              >
                <ProviderLogo providerId={p.id} name={p.name} className="size-4" />
                <span className="min-w-0 flex-1 truncate">{p.name}</span>
                {needsEndpoint(p) && <EndpointBadge />}
                <span className="shrink-0 text-ui-xs text-foreground-subtlest">
                  {p.added ? "已添加" : `${p.modelCount} 模型`}
                </span>
              </button>
            ))
          )}
        </div>
        {/* 右列：模型勾选 + 导入按钮 */}
        <div className="flex min-h-0 flex-col overflow-y-auto rounded-xl border border-border/40 p-1.5">
          {!selectedProvider ? (
            <div className="px-3 py-8 text-center text-ui-sm text-foreground-subtlest">选择一个提供商查看模型</div>
          ) : (
            <>
              {models.map((m) => (
                <label
                  key={m.id}
                  className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-ui-sm hover:bg-surface-hover"
                >
                  <input
                    type="checkbox"
                    checked={picked.has(m.id)}
                    onChange={(e) => {
                      const next = new Set(picked);
                      if (e.target.checked) next.add(m.id);
                      else next.delete(m.id);
                      setPicked(next);
                    }}
                  />
                  <span className="min-w-0 flex-1 truncate">{m.name}</span>
                  <span className="shrink-0 font-mono text-ui-xs text-foreground-subtlest">{m.contextK}K</span>
                </label>
              ))}
              <div className="sticky bottom-0 mt-auto bg-panel pt-2">
                {selectedProvider.added ? (
                  <button
                    disabled
                    title="已经添加，请直接编辑现有配置"
                    className="h-8 w-full rounded-lg border border-border text-ui-sm text-foreground-subtlest disabled:cursor-not-allowed"
                  >
                    已添加
                  </button>
                ) : (
                  <button
                    className="h-8 w-full rounded-lg bg-brand text-ui-sm text-foreground-inverse disabled:opacity-40"
                    disabled={picked.size === 0 || importing}
                    onClick={() => {
                      if (!selected) return;
                      setImporting(true);
                      void api
                        .catalogImport({ providerId: selected, all: picked.size === models.length, modelIds: [...picked] })
                        .then((view) => {
                          pushToast("ok", "提供商已导入", view.name);
                          onDone(view);
                        })
                        .catch((err) => pushToast("danger", "导入失败", cleanErrorMessage(err)))
                        .finally(() => setImporting(false));
                    }}
                  >
                    {importing ? "导入中…" : `导入选中的 ${picked.size} 个模型`}
                  </button>
                )}
              </div>
            </>
          )}
        </div>
      </div>
      {selectedProvider && needsEndpoint(selectedProvider) && !addedSelected && (
        <p className="shrink-0 text-ui-xs text-warning">该提供商目录中没有端点，导入后请在详情卡中填写服务端点。</p>
      )}
    </div>
  );
}

/** 「需填写端点」warning 徽标（spec §3.2）：目录提供商端点为空时显示。 */
function EndpointBadge(): React.JSX.Element {
  return (
    <span className="shrink-0 rounded bg-warning-surface px-1.5 py-0.5 text-ui-xs text-warning-surface-foreground" title="目录中没有该提供商的端点">
      需填写端点
    </span>
  );
}

// ---------- 入口二：手动创建 ----------

const KIND_OPTIONS: Array<{ value: "openai" | "anthropic" | "openai-compatible"; label: string }> = [
  { value: "openai-compatible", label: "OpenAI 兼容" },
  { value: "openai", label: "OpenAI" },
  { value: "anthropic", label: "Anthropic" },
];

/** 手动创建表单（spec §7）：名称 / 协议 / 端点 / 密钥可选 / 首个模型。保存走 providerSave 服务端校验。 */
function ManualCreateForm({ onDone }: { onDone(provider: ProviderView): void }): React.JSX.Element {
  const pushToast = useToasts((s) => s.push);
  const [name, setName] = useState("");
  const [kind, setKind] = useState<"openai" | "anthropic" | "openai-compatible">("openai-compatible");
  const [endpoint, setEndpoint] = useState("");
  const [key, setKey] = useState("");
  const [modelId, setModelId] = useState("");
  const [contextK, setContextK] = useState("32");
  const [errors, setErrors] = useState<ManualFormErrors>({});
  const [serverError, setServerError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const submit = (): void => {
    const errs = validateManualForm({ name, endpoint, modelId, contextK: Number(contextK) });
    setErrors(errs);
    if (Object.keys(errs).length > 0) return;
    setSubmitting(true);
    setServerError(null);
    void api
      .providerSave({
        id: "", // 空串由服务端生成 ID（custom_*），返回的 ProviderView 携带新 id
        name: name.trim(),
        kind,
        endpoint: endpoint.trim(),
        keyRequired: true,
        models: [{ id: modelId.trim(), name: modelId.trim(), contextK: normalizeContextK(Number(contextK)), maxTokens: 0 }],
      })
      .then((view) => {
        // 密钥可选：填写了就在创建后立即保存；失败不阻塞（详情卡可再设）
        if (key.trim() !== "") {
          void api
            .providerSetKey(view.id, key.trim())
            .catch((err) => pushToast("danger", "密钥保存失败", cleanErrorMessage(err)));
        }
        pushToast("ok", "提供商已创建", view.name);
        onDone(view);
      })
      .catch((err) => {
        setServerError(cleanErrorMessage(err));
        setSubmitting(false);
      });
  };

  const field = (label: string, text: string, err: string | undefined, control: React.JSX.Element): React.JSX.Element => (
    <div className="flex flex-col gap-1">
      <label className="text-ui-xs text-foreground-subtle">
        {label}
        {text && <span className="ml-1.5 text-foreground-subtlest">{text}</span>}
      </label>
      {control}
      {err && <span className="text-ui-xs text-destructive">{err}</span>}
    </div>
  );

  const inputCls = "h-8 w-full rounded-lg border border-border bg-input px-2.5 text-ui-sm";

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      <div className="grid grid-cols-2 gap-3">
        {field(
          "名称",
          "必填",
          errors.name,
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="例如：本地网关" className={inputCls} />,
        )}
        {field(
          "协议",
          "创建后只读",
          undefined,
          <Select
            ariaLabel="提供商协议"
            fullWidth
            value={kind}
            onChange={(v) => setKind(v as typeof kind)}
            options={KIND_OPTIONS}
          />,
        )}
      </div>
      {field(
        "端点",
        "必填",
        errors.endpoint,
        <input
          value={endpoint}
          onChange={(e) => setEndpoint(e.target.value)}
          placeholder="https://api.example.com/v1"
          className={`${inputCls} font-mono text-ui-xs`}
        />,
      )}
      {field(
        "密钥",
        "可选，也可创建后在详情卡设置",
        undefined,
        <input
          type="password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="粘贴 API Key…"
          className={`${inputCls} font-mono`}
        />,
      )}
      <div className="grid grid-cols-[minmax(0,1fr)_9rem] gap-3">
        {field(
          "首个模型 ID",
          "必填",
          errors.modelId,
          <input
            value={modelId}
            onChange={(e) => setModelId(e.target.value)}
            placeholder="例如：deepseek-chat"
            className={`${inputCls} font-mono text-ui-xs`}
          />,
        )}
        {field(
          "上下文窗口（K）",
          "默认 32",
          errors.contextK,
          <input
            type="number"
            min={1}
            value={contextK}
            onChange={(e) => setContextK(e.target.value)}
            className={inputCls}
          />,
        )}
      </div>
      {serverError && <p className="text-ui-xs text-destructive">{serverError}</p>}
      <div className="flex justify-end">
        <button
          className="h-8 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:opacity-40"
          disabled={submitting}
          onClick={submit}
        >
          {submitting ? "创建中…" : "创建提供商"}
        </button>
      </div>
    </div>
  );
}
