// 设置中心：左侧导航 + 右侧内容（通用/外观/模型与提供商/工具与权限/数据）。
// 所有改动立即保存生效。
import { useEffect, useState } from "react";
import { Palette, ShieldCheck, Plus, Trash2, ArrowLeft, SlidersHorizontal, Package, User, ChartColumn, ScrollText, DatabaseBackup, type LucideIcon } from "lucide-react";
import { useApp } from "../store/app";
import { useToasts } from "../store/toast";
import { useOverlays } from "../store/overlays";
import { api } from "../lib/api";
import { APPROVAL_LEVELS, useApprovalLevelAction } from "../lib/approval";
import { applyTheme, saveThemePref } from "../lib/theme";
import type { AuditEntry, Backup, ProviderView, UsageSummary } from "../../../shared/types";

type Section = "general" | "appearance" | "providers" | "permissions" | "usage" | "audit" | "backups";

/** 侧栏分组导航（对齐 ZCode 设置侧栏）：分组标题 + 图标导航项。 */
const NAV_GROUPS: Array<{ label: string; items: Array<{ id: Section; label: string; icon: LucideIcon }> }> = [
  {
    label: "基础设置",
    items: [
      { id: "general", label: "通用", icon: SlidersHorizontal },
      { id: "appearance", label: "外观", icon: Palette },
      { id: "providers", label: "模型与提供商", icon: Package },
    ],
  },
  {
    label: "Agent 能力",
    items: [{ id: "permissions", label: "工具与权限", icon: ShieldCheck }],
  },
  {
    label: "数据与统计",
    items: [
      { id: "usage", label: "用量", icon: ChartColumn },
      { id: "audit", label: "审计", icon: ScrollText },
      { id: "backups", label: "备份", icon: DatabaseBackup },
    ],
  },
];

export function SettingsOverlay(): React.JSX.Element {
  const close = useOverlays((s) => s.close);
  // 支持外部入口指定初始分区（如模型菜单「管理模型」直达「模型与提供商」）。
  const [section, setSection] = useState<Section>(() => useOverlays.getState().settingsSection ?? "general");

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-background">
      {/* 全宽顶部拖拽留白：交通灯区，对齐主窗口侧栏 h-12 顶条 */}
      <div className="h-12 shrink-0 app-region-drag" />
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-60 shrink-0 flex-col border-r border-border/50 p-3">
          <button
            className="flex h-9 shrink-0 items-center gap-2 rounded-lg bg-surface px-3 text-ui-sm text-foreground hover:bg-surface-hover"
            onClick={close}
          >
            <ArrowLeft className="size-4 shrink-0" />
            返回工作区
          </button>
          <nav className="mt-4 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
            {NAV_GROUPS.map((group) => (
              <div key={group.label} className="flex flex-col gap-0.5">
                <div className="px-3 pb-1 text-ui-xs text-foreground-subtlest">{group.label}</div>
                {group.items.map((s) => (
                  <button
                    key={s.id}
                    className={`flex items-center gap-2 rounded-lg px-3 py-2 text-left text-ui-base ${
                      section === s.id ? "bg-selected text-foreground" : "text-foreground hover:bg-surface-hover"
                    }`}
                    onClick={() => setSection(s.id)}
                  >
                    <s.icon className="size-4 shrink-0" />
                    {s.label}
                  </button>
                ))}
              </div>
            ))}
          </nav>
          <div className="flex h-10 shrink-0 items-center gap-2 px-2 text-ui-sm text-foreground-subtle">
            <User className="size-4 shrink-0" />
            CraftWork
          </div>
        </aside>
        <section className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-3xl px-8 py-8">
            {section === "general" && <GeneralSection />}
            {section === "appearance" && <AppearanceSection />}
            {section === "providers" && <ProvidersSection />}
            {section === "permissions" && <PermissionsSection />}
            {section === "usage" && <UsageSection />}
            {section === "audit" && <AuditSection />}
            {section === "backups" && <BackupsSection />}
          </div>
        </section>
      </div>
    </div>
  );
}

function Row({ title, desc, children }: { title: string; desc?: string; children: React.ReactNode }): React.JSX.Element {
  return (
    <div className="flex items-center justify-between gap-4 rounded-xl border border-border/50 bg-panel px-4 py-3">
      <div className="min-w-0">
        <div className="text-ui-base text-foreground">{title}</div>
        {desc && <div className="text-ui-sm text-foreground-subtlest">{desc}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function GeneralSection(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const save = useApp((s) => s.saveSettings);
  const setMode = useApp((s) => s.setMode);
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">通用</h2>
      <Row title="界面模式" desc="Code 提供完整文件/命令/Git；Work 只提供文件与文档">
        <select
          value={settings.mode}
          onChange={(e) => setMode(e.target.value as "code" | "work")}
          className="h-8 rounded-lg border border-border bg-input px-2 text-ui-sm"
        >
          <option value="code">Code</option>
          <option value="work">Work</option>
        </select>
      </Row>
      <Row title="系统通知" desc="窗口失焦时的审批 / 完成 / 失败提醒">
        <Switch checked={settings.notifications} onChange={(v) => void save({ notifications: v })} />
      </Row>
      <Row title="最大工具轮数" desc="单次运行的模型调用上限，0 表示不限制">
        <input
          type="number"
          min={0}
          max={1000}
          value={settings.maxIterations}
          onChange={(e) => void save({ maxIterations: Math.max(0, Math.min(1000, Number(e.target.value) || 0)) })}
          className="h-8 w-20 rounded-lg border border-border bg-input px-2 text-ui-sm"
        />
      </Row>
    </div>
  );
}

function AppearanceSection(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const save = useApp((s) => s.saveSettings);
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">
        <Palette className="mr-2 inline size-5" />
        外观
      </h2>
      <Row title="主题">
        <div className="flex gap-1">
          {(["system", "dark", "light"] as const).map((t) => (
            <button
              key={t}
              className={`h-8 rounded-lg border px-3 text-ui-sm ${
                settings.theme === t ? "border-brand bg-selected text-foreground" : "border-border text-foreground-subtle hover:bg-surface-hover"
              }`}
              onClick={() => {
                void save({ theme: t });
                saveThemePref(t);
                applyTheme(t);
              }}
            >
              {t === "system" ? "跟随系统" : t === "dark" ? "深色" : "浅色"}
            </button>
          ))}
        </div>
      </Row>
      <Row title="界面字号" desc={`${settings.fontSize}px（11–18）`}>
        <input
          type="range"
          min={11}
          max={18}
          value={settings.fontSize}
          onChange={(e) => {
            const size = Number(e.target.value);
            void save({ fontSize: size });
            document.documentElement.style.setProperty("--ui-font-size", `${size}px`);
          }}
          className="w-40"
        />
      </Row>
      <Row title="终端字号" desc={`${settings.terminalFontSize}px（10–20）`}>
        <input
          type="range"
          min={10}
          max={20}
          value={settings.terminalFontSize}
          onChange={(e) => void save({ terminalFontSize: Number(e.target.value) })}
          className="w-40"
        />
      </Row>
    </div>
  );
}

function ProvidersSection(): React.JSX.Element {
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [catalogOpen, setCatalogOpen] = useState(false);
  const pushToast = useToastsPush();

  const refresh = (): void => {
    void api.providerList().then(setProviders);
  };
  useEffect(refresh, []);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-ui-settings-title font-semibold text-foreground">模型与提供商</h2>
        <button
          className="flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover"
          onClick={() => setCatalogOpen(!catalogOpen)}
        >
          <Plus className="size-3.5" /> 从模型目录添加
        </button>
      </div>

      {catalogOpen && <CatalogBrowser onImported={() => { refresh(); pushToast("ok", "提供商已添加"); }} />}

      {providers.length === 0 && !catalogOpen && (
        <div className="rounded-xl border border-dashed border-border px-6 py-12 text-center text-ui-sm text-foreground-subtlest">
          还没有提供商。点击「从模型目录添加」，或先在上方浏览目录。
        </div>
      )}
      {providers.map((p) => (
        <ProviderCard key={p.id} provider={p} onChanged={refresh} />
      ))}
      <p className="text-ui-xs text-foreground-subtlest">
        应用不内置任何提供商；全部由你手动添加。目录数据来自 models.dev 离线快照。
      </p>
    </div>
  );
}

function CatalogBrowser({ onImported }: { onImported(): void }): React.JSX.Element {
  const pushToast = useToastsPush();
  const [query, setQuery] = useState("");
  const [providers, setProviders] = useState<Array<{ id: string; name: string; kind: string; modelCount: number; added: boolean; endpoint: string; env: string[] }>>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [models, setModels] = useState<Array<{ id: string; name: string; contextK: number }>>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());

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

  return (
    <div className="rounded-xl border border-border/50 bg-panel p-3">
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="搜索提供商…"
        className="mb-2 h-8 w-full rounded-lg border border-border bg-input px-3 text-ui-sm"
      />
      <div className="flex gap-3">
        <div className="max-h-72 w-1/2 overflow-y-auto">
          {providers.map((p) => (
            <button
              key={p.id}
              className={`flex w-full items-center justify-between rounded-lg px-2.5 py-2 text-left text-ui-sm ${
                selected === p.id ? "bg-selected" : "hover:bg-surface-hover"
              }`}
              onClick={() => setSelected(p.id)}
            >
              <span className="truncate">{p.name}</span>
              <span className="shrink-0 text-ui-xs text-foreground-subtlest">
                {p.added ? "已添加" : `${p.modelCount} 模型`}
              </span>
            </button>
          ))}
        </div>
        <div className="max-h-72 w-1/2 overflow-y-auto">
          {selected && (
            <>
              {models.map((m) => (
                <label key={m.id} className="flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-ui-sm hover:bg-surface-hover">
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
                  <span className="truncate">{m.name}</span>
                  <span className="ml-auto shrink-0 text-ui-xs text-foreground-subtlest">{m.contextK}K</span>
                </label>
              ))}
              <button
                className="mt-2 h-8 w-full rounded-lg bg-brand text-ui-sm text-foreground-inverse disabled:opacity-40"
                disabled={picked.size === 0}
                onClick={() => {
                  void api
                    .catalogImport({ providerId: selected, all: picked.size === models.length, modelIds: [...picked] })
                    .then(onImported)
                    .catch((err) => pushToast("danger", "导入失败", String(err?.message ?? err)));
                }}
              >
                导入选中的 {picked.size} 个模型
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function ProviderCard({ provider, onChanged }: { provider: ProviderView; onChanged(): void }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [endpointInput, setEndpointInput] = useState(provider.endpoint);
  const [testing, setTesting] = useState(false);
  const pushToast = useToastsPush();

  return (
    <div className="rounded-xl border border-border/50 bg-panel">
      <button className="flex w-full items-center gap-2 px-4 py-3 text-left" onClick={() => setOpen(!open)}>
        <ChevronRightIcon open={open} />
        <span className="text-ui-base font-medium">{provider.name}</span>
        <span className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">{provider.kind}</span>
        <span className="ml-auto text-ui-xs">
          {provider.keyMasked ? (
            <span className="text-success">{provider.keyMasked}</span>
          ) : provider.keyRequired ? (
            <span className="text-warning">未配置密钥</span>
          ) : (
            <span className="text-foreground-subtlest">无需密钥</span>
          )}
        </span>
      </button>
      {open && (
        <div className="flex flex-col gap-2 border-t border-border/40 px-4 py-3">
          <div className="flex items-center gap-2">
            <span className="shrink-0 text-ui-xs text-foreground-subtlest">端点</span>
            <input
              value={endpointInput}
              onChange={(e) => setEndpointInput(e.target.value)}
              placeholder="https://api.example.com/v1"
              className="h-8 min-w-0 flex-1 rounded-lg border border-border bg-input px-2.5 font-mono text-ui-xs"
            />
            <button
              className="h-8 shrink-0 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:opacity-40"
              disabled={endpointInput.trim() === provider.endpoint}
              onClick={() => {
                void api
                  .providerSave({
                    id: provider.id,
                    name: provider.name,
                    kind: provider.kind,
                    endpoint: endpointInput.trim(),
                    keyRequired: provider.keyRequired,
                    models: provider.models.map((m) => ({
                      id: m.id,
                      name: m.name,
                      contextK: m.contextK,
                      maxTokens: m.maxTokens,
                      caps: m.caps,
                      cost: m.cost,
                    })),
                  })
                  .then(() => {
                    onChanged();
                    pushToast("ok", "端点已保存");
                  })
                  .catch((err) => pushToast("danger", "保存失败", String(err?.message ?? err)));
              }}
            >
              保存端点
            </button>
          </div>
          {provider.models.map((m) => (
            <div key={m.ref} className="flex items-center gap-2 text-ui-sm">
              <span className="truncate">{m.name}</span>
              <span className="ml-auto shrink-0 font-mono text-ui-xs text-foreground-subtlest">
                {m.contextK}K · {m.caps.tools ? "tools" : "chat"}
                {m.caps.vision ? "+vision" : ""}
                {m.caps.reasoning ? "+reasoning" : ""}
              </span>
            </div>
          ))}
          <div className="mt-1 flex items-center gap-2">
            <input
              type="password"
              value={keyInput}
              onChange={(e) => setKeyInput(e.target.value)}
              placeholder="粘贴 API Key…"
              className="h-8 flex-1 rounded-lg border border-border bg-input px-2.5 font-mono text-ui-sm"
            />
            <button
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover"
              onClick={() => {
                void api.providerSetKey(provider.id, keyInput.trim()).then(() => {
                  setKeyInput("");
                  onChanged();
                  pushToast("ok", "密钥已保存");
                });
              }}
            >
              保存密钥
            </button>
            <button
              className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover"
              disabled={testing}
              onClick={() => {
                setTesting(true);
                void api
                  .providerTest(provider.id)
                  .then((r) => pushToast(r.ok ? "ok" : "danger", r.ok ? `连接成功（${r.latencyMs}ms）` : "连接失败", r.message))
                  .finally(() => setTesting(false));
              }}
            >
              {testing ? "测试中…" : "测试连接"}
            </button>
            <button
              className="flex h-8 items-center gap-1 rounded-lg border border-destructive/40 px-3 text-ui-sm text-destructive hover:bg-destructive-surface"
              onClick={() => {
                void api.providerDelete(provider.id).then(onChanged);
              }}
            >
              <Trash2 className="size-3.5" /> 删除
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function ChevronRightIcon({ open }: { open: boolean }): React.JSX.Element {
  return <span className={`inline-block transition-transform ${open ? "rotate-90" : ""}`}>›</span>;
}

function PermissionsSection(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const chooseLevel = useApprovalLevelAction();
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">
        <ShieldCheck className="mr-2 inline size-5" />
        工具与权限
      </h2>
      <div className="grid gap-2">
        {APPROVAL_LEVELS.map((l) => (
          <button
            key={l.id}
            className={`rounded-xl border px-4 py-3 text-left ${
              settings.approvalLevel === l.id ? "border-brand bg-selected" : "border-border/50 bg-panel hover:bg-surface-hover"
            }`}
            onClick={() => chooseLevel(l.id)}
          >
            <div className="text-ui-base font-medium">{l.name}</div>
            <div className="text-ui-sm text-foreground-subtlest">{l.desc}</div>
          </button>
        ))}
      </div>
      <div className="rounded-xl bg-surface px-4 py-3 text-ui-sm text-foreground-subtle">
        风险等级：R0 只读 · R1 写入工作区 · R2 执行命令 · R3 网络或越界（不提供永久授权） · R4 破坏性（阻止）
      </div>
    </div>
  );
}

// ---------- 数据与统计（用量 / 审计 / 备份 三个独立分区） ----------

const fmtCost = (micros: number): string => `$${(micros / 1_000_000).toFixed(4)}`;
const fmtTokens = (n: number): string => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}K` : String(n));
const fmtTime = (ts: number): string => new Date(ts).toLocaleString("zh-CN", { hour12: false });

function RefreshButton({ onClick }: { onClick(): void }): React.JSX.Element {
  return (
    <button className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover" onClick={onClick}>
      刷新
    </button>
  );
}

function UsageSection(): React.JSX.Element {
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    void api.auditUsage().then(setUsage);
  }, [refreshKey]);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-ui-settings-title font-semibold text-foreground">用量</h2>
        <RefreshButton onClick={() => setRefreshKey((k) => k + 1)} />
      </div>
      <Row title="全部数据存于本机" desc="SQLite + keys.json（0600），不收集遥测">
        <span />
      </Row>
      {usage && (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-3 gap-3">
            {(
              [
                ["今天", usage.today],
                ["近 7 天", usage.week],
                ["近 30 天", usage.month],
              ] as const
            ).map(([label, t]) => (
              <div key={label} className="rounded-xl border border-border/50 bg-panel px-4 py-3">
                <div className="text-ui-xs text-foreground-subtlest">{label}</div>
                <div className="pt-1 font-mono text-ui-lg">{fmtTokens(t.inputTokens + t.outputTokens)} tokens</div>
                <div className="text-ui-sm text-foreground-subtlest">{fmtCost(t.costMicros)}</div>
              </div>
            ))}
          </div>
          <div className="rounded-xl border border-border/50 bg-panel">
            <div className="border-b border-border/40 px-4 py-2 text-ui-sm font-medium">按模型（近 30 天）</div>
            {usage.byModel.length === 0 ? (
              <div className="px-4 py-6 text-center text-ui-sm text-foreground-subtlest">暂无用量</div>
            ) : (
              usage.byModel.map((m) => (
                <div key={m.modelId} className="flex items-center gap-2 border-b border-border/20 px-4 py-2 text-ui-sm last:border-0">
                  <span className="truncate font-mono">{m.modelId}</span>
                  <span className="ml-auto shrink-0 text-foreground-subtlest">{fmtTokens(m.inputTokens + m.outputTokens)}</span>
                  <span className="w-20 shrink-0 text-right text-foreground-subtlest">{fmtCost(m.costMicros)}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function AuditSection(): React.JSX.Element {
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    void api.auditList(200).then(setAudit);
  }, [refreshKey]);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-ui-settings-title font-semibold text-foreground">审计</h2>
        <RefreshButton onClick={() => setRefreshKey((k) => k + 1)} />
      </div>
      <div className="overflow-hidden rounded-xl border border-border/50">
        <div className="max-h-[480px] overflow-y-auto">
          {audit.length === 0 ? (
            <div className="py-10 text-center text-ui-sm text-foreground-subtlest">暂无审计记录</div>
          ) : (
            <table className="w-full text-ui-sm">
              <thead className="bg-surface text-left text-ui-xs text-foreground-subtlest">
                <tr>
                  <th className="px-3 py-2">时间</th>
                  <th className="px-3 py-2">工具</th>
                  <th className="px-3 py-2">风险</th>
                  <th className="px-3 py-2">决定</th>
                  <th className="px-3 py-2">范围</th>
                  <th className="px-3 py-2">详情</th>
                </tr>
              </thead>
              <tbody>
                {audit.map((a) => (
                  <tr key={a.id} className="border-t border-border/20">
                    <td className="whitespace-nowrap px-3 py-1.5 font-mono text-ui-xs">{fmtTime(a.ts)}</td>
                    <td className="px-3 py-1.5 font-mono">{a.tool}</td>
                    <td className="px-3 py-1.5">{a.risk}</td>
                    <td className="px-3 py-1.5">{a.decision}</td>
                    <td className="px-3 py-1.5">{a.scope}</td>
                    <td className="max-w-52 truncate px-3 py-1.5 text-foreground-subtle">{a.detail}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}

function BackupsSection(): React.JSX.Element {
  const [backups, setBackups] = useState<Backup[]>([]);
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    void api.auditBackups(50).then(setBackups);
  }, [refreshKey]);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-ui-settings-title font-semibold text-foreground">备份</h2>
        <RefreshButton onClick={() => setRefreshKey((k) => k + 1)} />
      </div>
      <div className="overflow-hidden rounded-xl border border-border/50">
        {backups.length === 0 ? (
          <div className="py-10 text-center text-ui-sm text-foreground-subtlest">暂无备份</div>
        ) : (
          backups.map((b) => (
            <div key={b.id} className="flex items-center gap-3 border-b border-border/20 px-4 py-2 text-ui-sm last:border-0">
              <div className="min-w-0 flex-1">
                <div className="truncate">{b.path}</div>
                <div className="text-ui-xs text-foreground-subtlest">
                  {fmtTime(b.createdAt)} · {b.size} 字节
                </div>
              </div>
              <button
                className="h-7 shrink-0 rounded-lg border border-border px-3 text-ui-xs hover:bg-surface-hover"
                onClick={() => {
                  void api
                    .auditRestore(b.id)
                    .then(() => setRefreshKey((k) => k + 1))
                    .catch((err) => alert(String(err?.message ?? err)));
                }}
              >
                恢复
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

function Switch({ checked, onChange }: { checked: boolean; onChange(v: boolean): void }): React.JSX.Element {
  return (
    <button
      role="switch"
      aria-checked={checked}
      className={`h-5 w-9 rounded-full transition-colors ${checked ? "bg-brand" : "bg-border"}`}
      onClick={() => onChange(!checked)}
    >
      <span className={`block size-4 translate-y-[2px] rounded-full bg-white shadow transition-transform ${checked ? "translate-x-[18px]" : "translate-x-[2px]"}`} />
    </button>
  );
}

function useToastsPush() {
  return useToasts((s) => s.push);
}
