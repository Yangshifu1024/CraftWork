// 设置中心：左侧导航 + 右侧内容（通用/外观/模型与提供商/工具与权限/数据）。
// 所有改动立即保存生效。
import { useEffect, useRef, useState } from "react";
import { Palette, ShieldCheck, Plus, Trash2, ArrowLeft, SlidersHorizontal, Package, User, ChartColumn, ScrollText, DatabaseBackup, Eye, EyeOff, MoreHorizontal, Pencil, PlugZap, type LucideIcon } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { useApp } from "../store/app";
import { useToasts } from "../store/toast";
import { useOverlays } from "../store/overlays";
import { api } from "../lib/api";
import { APPROVAL_LEVELS, useApprovalLevelAction } from "../lib/approval";
import { applyTheme, saveThemePref } from "../lib/theme";
import { groupProviders, providerStatus, type ProviderStatusTone } from "../lib/provider-list";
import { ProviderLogo } from "../components/provider-logo";
import { ProviderAddPage, type AddEntry } from "./provider-add";
import type { AuditEntry, Backup, ConnectionResult, ProviderView, UsageSummary } from "../../../shared/types";

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
          <div className={`mx-auto px-8 py-8 ${section === "providers" ? "max-w-4xl" : "max-w-3xl"}`}>
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  // 右栏视图：详情 ↔ 添加页（spec §7）；selectedId 在添加页期间保持不变，返回即恢复原详情。
  const [view, setView] = useState<"detail" | "add">("detail");
  const [addEntry, setAddEntry] = useState<AddEntry | null>(null);

  const refresh = (): void => setRefreshKey((k) => k + 1);
  const openAdd = (entry: AddEntry | null): void => {
    setAddEntry(entry);
    setView("add");
  };

  useEffect(() => {
    void api.providerList().then((list) => {
      setProviders(list);
      // 选中项保持稳定；被删除或未选中时回落到第一个
      setSelectedId((cur) => (cur !== null && list.some((p) => p.id === cur) ? cur : (list[0]?.id ?? null)));
    });
  }, [refreshKey]);

  const groups = groupProviders(providers);
  const selected = providers.find((p) => p.id === selectedId) ?? null;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h2 className="text-ui-settings-title font-semibold text-foreground">模型与提供商</h2>
          <p className="text-ui-sm text-foreground-subtlest">管理模型供应商与模型，配置后在聊天中选择使用。</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <RefreshButton onClick={refresh} />
          <button
            className="flex h-8 items-center gap-1.5 rounded-lg bg-brand px-3 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
            onClick={() => openAdd(null)}
          >
            <Plus className="size-3.5" /> 添加提供商
          </button>
        </div>
      </div>

      {/* 主从两栏（spec §4）：左列表 224px、右详情自适应，各自独立滚动 */}
      <div className="grid min-h-[32rem] max-h-[calc(100vh-14rem)] grid-cols-[224px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)] overflow-clip rounded-xl border border-border/50 bg-panel">
        <aside className="min-h-0 overflow-y-auto border-r border-border/40 p-2">
          <div role="listbox" aria-label="供应商列表" className="flex flex-col gap-3">
            {groups.map((group) => (
              <div key={group.source} role="group" aria-label={group.label} className="flex flex-col gap-0.5">
                <div className="px-2 pb-1 text-ui-xs text-foreground-subtlest">{group.label}</div>
                {group.providers.map((p) => (
                  <ProviderListItem
                    key={p.id}
                    provider={p}
                    selected={p.id === selectedId && view === "detail"}
                    onSelect={() => {
                      setSelectedId(p.id);
                      setView("detail");
                    }}
                  />
                ))}
              </div>
            ))}
          </div>
        </aside>
        <div className="min-h-0 overflow-y-auto">
          {view === "add" ? (
            /* key 随入口变化：从空态引导直达不同入口时重挂载、内部状态归零 */
            <ProviderAddPage
              key={addEntry ?? "entries"}
              initialEntry={addEntry}
              onBack={() => setView("detail")}
              onDone={(created) => {
                // 创建即编辑：刷新列表、选中新供应商、返回详情视图
                refresh();
                setSelectedId(created.id);
                setView("detail");
              }}
            />
          ) : selected ? (
            <ProviderDetail key={selected.id} provider={selected} onChanged={refresh} />
          ) : (
            <ProvidersEmptyGuide onOpenAdd={openAdd} />
          )}
        </div>
      </div>

      <p className="text-ui-xs text-foreground-subtlest">
        应用不内置任何提供商；全部由你手动添加。目录数据来自 models.dev 离线快照。
      </p>
    </div>
  );
}

/** 左栏列表项：logo（或字母头像）+ 名称 + 三色状态点，选中高亮（spec §5）。 */
function ProviderListItem({
  provider,
  selected,
  onSelect,
}: {
  provider: ProviderView;
  selected: boolean;
  onSelect(): void;
}): React.JSX.Element {
  const status = providerStatus({
    enabled: provider.enabled,
    keyMasked: provider.keyMasked,
    keyRequired: provider.keyRequired,
    modelCount: provider.models.length,
  });
  return (
    <button
      role="option"
      aria-selected={selected}
      className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-ui-sm ${
        selected ? "bg-selected text-foreground" : "text-foreground hover:bg-surface-hover"
      }`}
      onClick={onSelect}
    >
      <ProviderLogo providerId={provider.catalogId || provider.id} name={provider.name} className="size-4" />
      <span className="min-w-0 flex-1 truncate">{provider.name}</span>
      <span title={status.label} className={`size-1.5 shrink-0 rounded-full ${STATUS_DOT[status.tone]}`} />
    </button>
  );
}

/** 状态点三色的语义色映射（判定逻辑在 lib/provider-list，纯展示映射不测）。 */
const STATUS_DOT: Record<ProviderStatusTone, string> = {
  ok: "bg-success",
  warn: "bg-warning",
  off: "bg-foreground-subtlest",
};

/** 空态引导（spec §11）：三步引导 + 两个入口按钮（直达添加页对应入口）。 */
function ProvidersEmptyGuide({ onOpenAdd }: { onOpenAdd(entry: AddEntry): void }): React.JSX.Element {
  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-4 p-8 text-center">
      <div className="text-ui-base font-medium text-foreground">添加你的第一个提供商</div>
      <ol className="flex flex-col gap-1.5 text-ui-sm text-foreground-subtle">
        <li>① 从模型目录导入或手动创建提供商</li>
        <li>② 配置端点与 API 密钥</li>
        <li>③ 回到聊天选择模型开始使用</li>
      </ol>
      <div className="flex gap-2">
        <button
          className="h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground hover:bg-surface-hover"
          onClick={() => onOpenAdd("catalog")}
        >
          从模型目录导入
        </button>
        <button
          className="h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground hover:bg-surface-hover"
          onClick={() => onOpenAdd("manual")}
        >
          手动创建
        </button>
      </div>
    </div>
  );
}

/** 右栏供应商详情卡（spec §3.2 / §6 / §10）：
    头部 = logo + 内联改名 + 协议徽章 + 模型数 + 启用开关 + ⋯菜单（测试连接 / 删除）；
    连接卡 = 端点（blur/Enter 保存、失败内联红字、空值必填态）+ 协议只读 + 密钥（masked / 明文切换 / 设置 / 清除确认）
    + 测试连接（内联结果反馈）；禁用后详情区降透明度但控件保持可操作。
    模型列表管理与添加入口页由票 4 / 票 5 接入。 */
function ProviderDetail({ provider, onChanged }: { provider: ProviderView; onChanged(): void }): React.JSX.Element {
  const openConfirm = useOverlays((s) => s.openConfirm);
  const pushToast = useToastsPush();

  // 改名（内联编辑态）
  const [nameEditing, setNameEditing] = useState(false);
  const [nameDraft, setNameDraft] = useState(provider.name);
  const [renaming, setRenaming] = useState(false);
  const nameEscRef = useRef(false);

  // 端点（blur / Enter 保存）
  const [endpointInput, setEndpointInput] = useState(provider.endpoint);
  const [endpointError, setEndpointError] = useState("");
  const [savingEndpoint, setSavingEndpoint] = useState(false);
  const endpointRef = useRef<HTMLInputElement>(null);

  // 密钥（输入态 + 明文切换）
  const [keyEditing, setKeyEditing] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [keyVisible, setKeyVisible] = useState(false);
  const [savingKey, setSavingKey] = useState(false);

  // 测试连接（内联结果）
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ConnectionResult | null>(null);

  // 空端点必填态：导入无端点提供商后聚焦引导（spec §3.2）
  const endpointMissing = endpointInput.trim() === "";
  useEffect(() => {
    if (provider.endpoint === "") endpointRef.current?.focus();
    // 仅挂载时引导一次（组件按 provider.id 重建）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const startRename = (): void => {
    setNameDraft(provider.name);
    setNameEditing(true);
  };

  const commitRename = (): void => {
    if (nameEscRef.current) {
      nameEscRef.current = false;
      return;
    }
    const next = nameDraft.trim();
    if (renaming || !next || next === provider.name) {
      setNameEditing(false);
      return;
    }
    setRenaming(true);
    api.providerRename(provider.id, next)
      .then(() => {
        setNameEditing(false);
        onChanged();
      })
      .catch((err) => pushToast("danger", "改名失败", cleanErrorMessage(err)))
      .finally(() => {
        setRenaming(false);
        setNameEditing(false);
      });
  };

  const saveEndpoint = (): void => {
    const next = endpointInput.trim();
    if (savingEndpoint) return;
    if (next === provider.endpoint) {
      setEndpointError("");
      return;
    }
    setSavingEndpoint(true);
    api.providerSave({
      id: provider.id,
      name: provider.name,
      kind: provider.kind,
      endpoint: next,
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
        setEndpointError("");
        onChanged();
        pushToast("ok", "端点已保存");
      })
      .catch((err) => setEndpointError(cleanErrorMessage(err)))
      .finally(() => setSavingEndpoint(false));
  };

  const saveKey = (): void => {
    const key = keyInput.trim();
    if (savingKey || !key) return;
    setSavingKey(true);
    api.providerSetKey(provider.id, key)
      .then(() => {
        setKeyEditing(false);
        setKeyInput("");
        setKeyVisible(false);
        onChanged();
        pushToast("ok", "密钥已保存");
      })
      .catch((err) => pushToast("danger", "密钥保存失败", cleanErrorMessage(err)))
      .finally(() => setSavingKey(false));
  };

  const confirmClearKey = (): void => {
    openConfirm({
      title: "清除密钥",
      message: `将清除「${provider.name}」已保存的 API 密钥，清除后需要重新配置才能调用。`,
      danger: true,
      onOk: () => {
        void api.providerClearKey(provider.id)
          .then(() => {
            onChanged();
            pushToast("ok", "密钥已清除");
          })
          .catch((err) => pushToast("danger", "密钥清除失败", cleanErrorMessage(err)));
      },
    });
  };

  const runTest = (): void => {
    if (testing) return;
    setTesting(true);
    api.providerTest(provider.id)
      .then(setTestResult)
      .catch((err) => setTestResult({ ok: false, message: cleanErrorMessage(err), latencyMs: 0, code: "" }))
      .finally(() => setTesting(false));
  };

  const confirmDelete = (): void => {
    openConfirm({
      title: "删除供应商",
      message: `将删除「${provider.name}」，其已保存的 API 密钥与 ${provider.models.length} 个模型配置会一并删除，且不可恢复。`,
      danger: true,
      onOk: () => {
        void api.providerDelete(provider.id)
          .then(onChanged)
          .catch((err) => pushToast("danger", "删除失败", cleanErrorMessage(err)));
      },
    });
  };

  const setEnabled = (enabled: boolean): void => {
    api.providerSetEnabled(provider.id, enabled)
      .then(() => onChanged())
      .catch((err) => pushToast("danger", enabled ? "启用失败" : "停用失败", cleanErrorMessage(err)));
  };

  return (
    <div className={`flex flex-col gap-4 p-4 ${provider.enabled ? "" : "opacity-60"}`}>
      {/* 头部：logo + 内联改名 + 协议徽章 + 模型数 + 启用开关 + ⋯菜单 */}
      <div className="flex items-center gap-2.5">
        <ProviderLogo providerId={provider.catalogId || provider.id} name={provider.name} className="size-6" />
        {nameEditing ? (
          <input
            autoFocus
            value={nameDraft}
            disabled={renaming}
            onChange={(e) => setNameDraft(e.target.value)}
            onBlur={commitRename}
            onKeyDown={(e) => {
              // IME 组合中的 Enter（候选确认）不触发提交（spec §10）
              if (e.nativeEvent.isComposing || e.keyCode === 229) return;
              if (e.key === "Enter") {
                e.preventDefault();
                e.currentTarget.blur();
              } else if (e.key === "Escape") {
                nameEscRef.current = true;
                e.preventDefault();
                setNameEditing(false);
              }
            }}
            aria-label="供应商名称"
            className="h-7 min-w-0 max-w-56 rounded-md border border-brand bg-input px-1.5 text-ui-lg font-medium text-foreground"
          />
        ) : (
          <button
            className="group flex min-w-0 items-center gap-1 rounded-md text-left"
            onClick={startRename}
            title="点击重命名"
          >
            <span className="min-w-0 truncate text-ui-lg font-medium text-foreground group-hover:text-foreground-subtle">
              {provider.name}
            </span>
            <Pencil className="size-3 shrink-0 text-foreground-subtlest opacity-0 transition-opacity group-hover:opacity-100" />
          </button>
        )}
        <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">{provider.kind}</span>
        <span className="shrink-0 text-ui-xs text-foreground-subtlest">{provider.models.length} 个模型</span>
        <div className="ml-auto flex shrink-0 items-center gap-2.5">
          <label className="flex items-center gap-1.5">
            <span className="text-ui-xs text-foreground-subtle">{provider.enabled ? "已启用" : "已停用"}</span>
            <Switch checked={provider.enabled} onChange={setEnabled} />
          </label>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button aria-label="更多操作" className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-foreground">
                <MoreHorizontal className="size-4" />
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content align="end" sideOffset={6} className="z-50 min-w-32 rounded-xl border border-popover-border bg-popover p-1 shadow-md">
                <DropdownMenu.Item
                  onSelect={runTest}
                  className="flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-ui-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
                >
                  <PlugZap className="size-3.5 shrink-0 text-foreground-subtle" /> 测试连接
                </DropdownMenu.Item>
                <DropdownMenu.Item
                  onSelect={confirmDelete}
                  className="flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-ui-sm text-destructive outline-none data-[highlighted]:bg-destructive-surface"
                >
                  <Trash2 className="size-3.5 shrink-0" /> 删除…
                </DropdownMenu.Item>
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>
      </div>

      {/* 连接配置卡（spec §6）：端点 / 协议 / 密钥 / 测试连接 */}
      <div className="flex flex-col gap-3.5 rounded-xl border border-border/50 bg-panel p-4">
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-2.5">
            <span className="w-9 shrink-0 text-ui-xs text-foreground-subtlest">端点</span>
            <input
              ref={endpointRef}
              value={endpointInput}
              disabled={savingEndpoint}
              onChange={(e) => {
                setEndpointInput(e.target.value);
                if (endpointError) setEndpointError("");
              }}
              onBlur={saveEndpoint}
              onKeyDown={(e) => {
                if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                if (e.key === "Enter") {
                  e.preventDefault();
                  e.currentTarget.blur(); // 保存逻辑统一走 blur
                }
              }}
              placeholder="https://api.example.com/v1"
              aria-invalid={endpointMissing || endpointError !== ""}
              className={`h-8 min-w-0 flex-1 rounded-lg border bg-input px-2.5 font-mono text-ui-xs ${
                endpointError ? "border-destructive" : endpointMissing ? "border-warning" : "border-border"
              }`}
            />
          </div>
          {endpointMissing ? (
            <p className="flex items-center gap-1.5 rounded-lg bg-warning-surface px-2.5 py-1.5 text-ui-xs text-warning-surface-foreground">
              请填写服务端点后才能调用
            </p>
          ) : endpointError ? (
            <p className="px-0.5 text-ui-xs text-destructive">{endpointError}</p>
          ) : null}
        </div>

        <div className="flex items-center gap-2.5">
          <span className="w-9 shrink-0 text-ui-xs text-foreground-subtlest">协议</span>
          <span title="协议创建后不可修改（如需更换请删除后重建）" className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">
            {provider.kind}
          </span>
          {provider.docUrl && (
            <span className="min-w-0 truncate text-ui-xs text-foreground-subtlest" title={provider.docUrl}>
              {provider.docUrl}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2.5">
          <span className="w-9 shrink-0 text-ui-xs text-foreground-subtlest">密钥</span>
          {provider.keyRequired === false ? (
            <span className="rounded bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">无需密钥</span>
          ) : keyEditing ? (
            <div className="flex min-w-0 flex-1 items-center gap-1.5">
              <div className="relative min-w-0 flex-1">
                <input
                  autoFocus
                  type={keyVisible ? "text" : "password"}
                  value={keyInput}
                  disabled={savingKey}
                  onChange={(e) => setKeyInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                    if (e.key === "Enter") {
                      e.preventDefault();
                      saveKey();
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      setKeyEditing(false);
                      setKeyInput("");
                    }
                  }}
                  placeholder="粘贴 API Key…"
                  className="h-8 w-full rounded-lg border border-border bg-input pl-2.5 pr-8 font-mono text-ui-sm"
                />
                <button
                  type="button"
                  aria-label={keyVisible ? "隐藏密钥" : "显示密钥"}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-foreground-subtlest hover:text-foreground"
                  onClick={() => setKeyVisible((v) => !v)}
                >
                  {keyVisible ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                </button>
              </div>
              <button
                className="h-8 shrink-0 rounded-lg bg-brand px-3 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
                disabled={!keyInput.trim() || savingKey}
                onClick={saveKey}
              >
                {savingKey ? "保存中…" : "保存"}
              </button>
              <button
                className="h-8 shrink-0 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
                onClick={() => {
                  setKeyEditing(false);
                  setKeyInput("");
                  setKeyVisible(false);
                }}
              >
                取消
              </button>
            </div>
          ) : (
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <span className={`min-w-0 truncate font-mono text-ui-sm ${provider.keyMasked ? "text-foreground" : "text-foreground-subtlest"}`}>
                {provider.keyMasked || "未配置"}
              </span>
              {provider.keySource === "env" && (
                <span title="密钥来自环境变量" className="shrink-0 rounded bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">
                  环境变量
                </span>
              )}
              <div className="ml-auto flex shrink-0 items-center gap-1.5">
                <button
                  className="h-7 rounded-lg border border-border px-2.5 text-ui-sm hover:bg-surface-hover"
                  onClick={() => {
                    setKeyInput("");
                    setKeyVisible(false);
                    setKeyEditing(true);
                  }}
                >
                  {provider.keyMasked ? "更换密钥" : "设置密钥"}
                </button>
                {provider.keyMasked && (
                  <button
                    className="h-7 rounded-lg border border-destructive/40 px-2.5 text-ui-sm text-destructive hover:bg-destructive-surface"
                    onClick={confirmClearKey}
                  >
                    清除
                  </button>
                )}
              </div>
            </div>
          )}
        </div>

        <div className="flex items-center gap-2.5">
          <span className="w-9 shrink-0 text-ui-xs text-foreground-subtlest" />
          <button
            className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
            disabled={testing}
            onClick={runTest}
          >
            <PlugZap className="size-3.5 shrink-0" />
            {testing ? <span className="animated-gradient-text">测试中…</span> : "测试连接"}
          </button>
          {testing ? (
            <span className="text-ui-xs text-foreground-subtlest">正在请求服务端，最长 10 秒…</span>
          ) : testResult ? (
            testResult.ok ? (
              <span className="text-ui-sm text-success">✓ 连接成功 · {testResult.latencyMs}ms</span>
            ) : (
              <span className="min-w-0 truncate text-ui-sm text-destructive" title={testResult.message}>
                ✗ {testResult.message}
              </span>
            )
          ) : (
            <span className="text-ui-xs text-foreground-subtlest">用当前端点与密钥请求一次模型列表</span>
          )}
        </div>
      </div>

      {/* 模型概览（完整管理由票 4 接入） */}
      <div className="flex flex-col gap-1">
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
      </div>
    </div>
  );
}

/** 清洗 IPC 错误：剥掉「Error invoking remote method ...: Error: 」前缀，只留可读文案。 */
function cleanErrorMessage(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const marker = "Error:";
  const idx = raw.lastIndexOf(marker);
  return (idx >= 0 ? raw.slice(idx + marker.length) : raw).trim();
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
