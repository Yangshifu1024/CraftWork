// 设置中心：左侧导航 + 右侧内容（通用/外观/模型与提供商/个性化/Skills/数据）。
// 所有改动立即保存生效。
import { useEffect, useRef, useState } from "react";
import { Palette, Puzzle, UserRound, Plus, Trash2, ArrowLeft, SlidersHorizontal, Package, User, ChartColumn, ScrollText, DatabaseBackup, Eye, EyeOff, MoreHorizontal, Pencil, PlugZap, RefreshCw, X, type LucideIcon } from "lucide-react";
import { Dialog, DropdownMenu, Popover, Switch as RadixSwitch } from "radix-ui";
import { useApp } from "../store/app";
import { useToasts } from "../store/toast";
import { useOverlays } from "../store/overlays";
import { api } from "../lib/api";
import { applyTheme, saveThemePref } from "../lib/theme";
import { groupProviders, providerStatus, type ProviderStatusTone } from "../lib/provider-list";
import { formatContextK, formatContextTitle } from "../lib/format-context";
import { cleanErrorMessage } from "../lib/errors";
import { isImeComposing } from "../lib/ime";
import { compactHeaderRows, MAX_CUSTOM_HEADERS, validateHeaderPairs } from "../lib/provider-headers";
import { ProviderLogo } from "../components/provider-logo";
import { ProviderAddPage, type AddEntry } from "./provider-add";
import { ProviderModelDialog } from "./provider-model-dialog";
import type { AuditEntry, Backup, ConnectionResult, HeaderPair, ModelView, PersonalizationInfo, ProviderView, Settings, SkillView, UsageSummary } from "../../../shared/types";

type Section = "general" | "appearance" | "providers" | "personalization" | "skills" | "usage" | "audit" | "backups";

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
    items: [
      { id: "personalization", label: "个性化", icon: UserRound },
      { id: "skills", label: "Skills", icon: Puzzle },
    ],
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

/** 设置中心全屏浮层：左侧分组导航 + 右侧分区内容；初始分区可由外部入口（overlays store 的 settingsSection，如「管理模型」直达）指定。 */
export function SettingsOverlay(): React.JSX.Element {
  const close = useOverlays((s) => s.close);
  // 支持外部入口指定初始分区（如模型菜单「管理模型」直达「模型与提供商」）。
  const [section, setSection] = useState<Section>(() => useOverlays.getState().settingsSection ?? "general");

  return (
    // 全屏不透明浮层：Esc / 焦点圈定由 radix Dialog 承担；顶部 h-12 拖拽条保留在 Content 首行。
    <Dialog.Root open onOpenChange={(o) => { if (!o && useOverlays.getState().overlay === "settings") close(); }}>
      <Dialog.Content
        aria-describedby={undefined}
        className="fixed inset-0 z-50 flex flex-col bg-background outline-none"
      >
      {/* 全宽顶部拖拽留白：交通灯区，对齐主窗口侧栏 h-12 顶条 */}
      <div className="h-12 shrink-0" data-tauri-drag-region />
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
            {section === "personalization" && <PersonalizationSection />}
            {section === "skills" && <SkillsSection />}
            {section === "usage" && <UsageSection />}
            {section === "audit" && <AuditSection />}
            {section === "backups" && <BackupsSection />}
          </div>
        </section>
      </div>
      </Dialog.Content>
    </Dialog.Root>
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
  // 日志目录随数据目录（getInfo 返回），控件旁展示便于取文件
  const [dataDir, setDataDir] = useState("");
  useEffect(() => {
    void api.getInfo().then((info) => setDataDir(info.dataDir));
  }, []);
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
      <Row title="流式平滑跟随" desc="流式输出时以弹簧动画平滑贴底；关闭后即时跟随（直接跳到底）">
        <Switch checked={settings.smoothFollow} onChange={(v) => void save({ smoothFollow: v })} />
      </Row>
      <Row title="输入预测" desc="任务结束后静置 5 秒，在输入框给出下一句建议，Tab 应用">
        <Switch checked={settings.inputPrediction} onChange={(v) => void save({ inputPrediction: v })} />
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
      <Row
        title="运行日志等级"
        desc={`排查回复截断等问题的观测时间线，保存即生效；目录 ${dataDir ? `${dataDir}/logs/` : "（<数据目录>/logs/）"}`}
      >
        <select
          value={settings.logsLevel}
          onChange={(e) => void save({ logsLevel: e.target.value as Settings["logsLevel"] })}
          className="h-8 rounded-lg border border-border bg-input px-2 text-ui-sm"
        >
          <option value="off">关闭</option>
          <option value="error">仅错误</option>
          <option value="info">标准</option>
          <option value="debug">调试</option>
        </select>
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
    </div>
  );
}

/** 模型与提供商分区（主从两栏，spec §4）：左供应商列表（分组 + 状态点）、右详情卡 / 添加页 / 空态引导三态切换。 */
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
    enabledModelCount: provider.models.filter((m) => m.enabled !== false).length,
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
    + 测试连接（内联结果反馈）；模型列表区 = 完整管理（增改弹窗 / 删除确认 / 启停 / 徽章，票 4）；
    禁用后详情区降透明度但控件保持可操作。添加入口页由票 5 提供。 */
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

  // 自定义请求头（编辑态行表单 + 报错）：随每次模型请求附加（如网关路由头 x-opencode-session）
  const [headersEditing, setHeadersEditing] = useState(false);
  const [headersDraft, setHeadersDraft] = useState<HeaderPair[]>([]);
  const [headersError, setHeadersError] = useState("");
  const [savingHeaders, setSavingHeaders] = useState(false);

  const saveHeaders = (): void => {
    if (savingHeaders) return;
    const pairs: HeaderPair[] = compactHeaderRows(headersDraft);
    const localErr = validateHeaderPairs(pairs);
    if (localErr) {
      setHeadersError(localErr);
      return;
    }
    setSavingHeaders(true);
    api.providerSave({
      id: provider.id,
      name: provider.name,
      kind: provider.kind,
      endpoint: provider.endpoint,
      keyRequired: provider.keyRequired,
      headers: pairs,
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
        setHeadersEditing(false);
        setHeadersError("");
        onChanged();
        pushToast("ok", "请求头已保存");
      })
      .catch((err) => setHeadersError(cleanErrorMessage(err)))
      .finally(() => setSavingHeaders(false));
  };

  // 请求头行输入框共用按键：⌘↩ 保存、Esc 退出编辑（IME 组合中不拦截）
  const headerRowKeyDown = (e: React.KeyboardEvent<HTMLInputElement>): void => {
    if (isImeComposing(e)) return;
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      saveHeaders();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setHeadersEditing(false);
      setHeadersError("");
    }
  };

  // 模型增改弹窗（受控）：null 关闭；{ mode: "add" } 新增；{ mode: "edit", model } 编辑
  const [modelDialog, setModelDialog] = useState<{ mode: "add" } | { mode: "edit"; model: ModelView } | null>(null);

  // 拉取模型（票 5 / spec §7）：进行中 loading + 内联反馈（成功计数 / 失败归因文案）
  const [fetching, setFetching] = useState(false);
  const [fetchFeedback, setFetchFeedback] = useState<{ ok: boolean; text: string } | null>(null);

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
      headers: provider.headers, // 部分更新仍显式带回现值（服务端对缺省字段本就保留，双保险）
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

  // ---------- 模型列表管理（票 4 / spec §6） ----------

  const setModelEnabled = (model: ModelView, enabled: boolean): void => {
    api.providerSetModelEnabled(provider.id, model.id, enabled)
      .then(() => onChanged())
      .catch((err) => pushToast("danger", enabled ? "启用失败" : "停用失败", cleanErrorMessage(err)));
  };

  const confirmRemoveModel = (model: ModelView): void => {
    // 就近 Popconfirm 已在按钮处确认过，这里直接执行删除
    void api
      .providerRemoveModel(provider.id, model.id)
      .then(() => onChanged())
      .catch((err) => pushToast("danger", "删除失败", cleanErrorMessage(err)));
  };

  /** 拉取模型（票 5 / spec §7）：进行中防重复；结果内联反馈在模型列表区内部，不走全局 toast。 */
  const runFetch = (): void => {
    if (fetching || endpointMissing) return;
    setFetching(true);
    setFetchFeedback(null);
    api.providerFetchModels(provider.id)
      .then((result) => {
        setFetchFeedback({
          ok: true,
          text:
            result.added.length > 0
              ? `拉取到 ${result.fetched} 个模型，新增 ${result.added.length} 个`
              : `拉取到 ${result.fetched} 个模型，无新增模型`,
        });
        onChanged();
      })
      .catch((err) => setFetchFeedback({ ok: false, text: cleanErrorMessage(err) }))
      .finally(() => setFetching(false));
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
              if (isImeComposing(e)) return;
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

      {/* 连接配置卡（spec §6）：缺密钥 warning 条 / 端点 / 协议 / 密钥 / 测试连接 */}
      <div className="flex flex-col gap-3.5 rounded-xl border border-border/50 bg-panel p-4">
        {provider.keyRequired !== false && !provider.keyMasked && (
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg bg-warning-surface px-2.5 py-1.5 text-ui-xs text-warning-surface-foreground">
            <span>尚未配置密钥，模型无法调用。</span>
            <button
              className="shrink-0 rounded border border-warning-surface-foreground/40 px-1.5 text-ui-xs font-medium hover:bg-warning-surface-foreground/10"
              onClick={() => {
                setKeyInput("");
                setKeyVisible(false);
                setKeyEditing(true); // 进入输入态后输入框 autoFocus，即引导聚焦密钥输入（spec §11）
              }}
            >
              设置密钥
            </button>
          </p>
        )}
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
                if (isImeComposing(e)) return;
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
                    if (isImeComposing(e)) return;
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

        <div className="flex items-start gap-2.5">
          <span className="w-9 shrink-0 pt-1.5 text-ui-xs text-foreground-subtlest">请求头</span>
          {headersEditing ? (
            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
              <div className="flex min-w-0 flex-col gap-1">
                {headersDraft.map((row, i) => (
                  <div key={i} className="flex min-w-0 items-center gap-1.5">
                    <input
                      autoFocus={i === 0}
                      value={row.name}
                      disabled={savingHeaders}
                      placeholder="x-opencode-session"
                      title="请求头名称"
                      onChange={(e) => {
                        setHeadersDraft(headersDraft.map((r, j) => (j === i ? { ...r, name: e.target.value } : r)));
                        if (headersError) setHeadersError("");
                      }}
                      onKeyDown={headerRowKeyDown}
                      className="h-7 w-40 shrink-0 rounded-lg border border-border bg-input px-2 font-mono text-ui-xs"
                    />
                    <input
                      value={row.value}
                      disabled={savingHeaders}
                      placeholder={"{sessionId}"}
                      title="请求头值"
                      onChange={(e) => {
                        setHeadersDraft(headersDraft.map((r, j) => (j === i ? { ...r, value: e.target.value } : r)));
                        if (headersError) setHeadersError("");
                      }}
                      onKeyDown={headerRowKeyDown}
                      className="h-7 min-w-0 flex-1 rounded-lg border border-border bg-input px-2 font-mono text-ui-xs"
                    />
                    <button
                      className="flex size-7 shrink-0 items-center justify-center rounded-lg text-foreground-subtlest hover:bg-surface-hover hover:text-destructive disabled:cursor-not-allowed disabled:opacity-40"
                      title="删除此行"
                      disabled={savingHeaders}
                      onClick={() => {
                        setHeadersDraft(headersDraft.filter((_, j) => j !== i));
                        if (headersError) setHeadersError("");
                      }}
                    >
                      <X className="size-3.5" />
                    </button>
                  </div>
                ))}
              </div>
              <button
                className="flex h-6 items-center gap-1 self-start rounded-lg border border-border px-2 text-ui-xs text-foreground-subtle hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
                disabled={savingHeaders || headersDraft.length >= MAX_CUSTOM_HEADERS}
                title={headersDraft.length >= MAX_CUSTOM_HEADERS ? `最多 ${MAX_CUSTOM_HEADERS} 条` : undefined}
                onClick={() => {
                  setHeadersDraft([...headersDraft, { name: "", value: "" }]);
                  setHeadersError("");
                }}
              >
                <Plus className="size-3" />
                添加一条
              </button>
              {headersError ? (
                <p className="text-ui-xs text-destructive">{headersError}</p>
              ) : (
                <p className="text-ui-xs text-foreground-subtlest">
                  随每次模型请求附加；<code className="font-mono">{"{sessionId}"}</code> 自动替换为当前会话 ID（每会话独立），不能与密钥头同名。
                </p>
              )}
              <div className="flex items-center gap-1.5">
                <button
                  className="h-7 rounded-lg bg-brand px-3 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
                  disabled={savingHeaders}
                  onClick={saveHeaders}
                >
                  {savingHeaders ? "保存中…" : "保存"}
                </button>
                <button
                  className="h-7 rounded-lg border border-border px-3 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
                  onClick={() => {
                    setHeadersEditing(false);
                    setHeadersError("");
                  }}
                >
                  取消
                </button>
                <span className="ml-auto text-ui-xs text-foreground-subtlest">⌘↩ 保存 · Esc 取消</span>
              </div>
            </div>
          ) : provider.headers.length > 0 ? (
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <div className="flex min-w-0 flex-col gap-0.5 rounded-lg bg-surface px-2.5 py-1.5">
                {provider.headers.map((h) => (
                  <div key={h.name} className="flex min-w-0 items-baseline gap-2 font-mono text-ui-xs">
                    <span className="shrink-0 text-foreground">{h.name}</span>
                    <span className="min-w-0 truncate text-foreground-subtlest" title={h.value}>{h.value}</span>
                  </div>
                ))}
              </div>
              <button
                className="h-6 self-start rounded-lg border border-border px-2 text-ui-xs text-foreground-subtle hover:bg-surface-hover"
                onClick={() => {
                  setHeadersDraft(provider.headers.map((h) => ({ name: h.name, value: h.value })));
                  setHeadersError("");
                  setHeadersEditing(true);
                }}
              >
                编辑
              </button>
            </div>
          ) : (
            <button
              className="h-7 rounded-lg border border-border px-2.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
              onClick={() => {
                setHeadersDraft([{ name: "", value: "" }]);
                setHeadersError("");
                setHeadersEditing(true);
              }}
            >
              添加请求头
            </button>
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

      {/* 模型列表区（票 4 / spec §6）：标题 + [拉取模型][添加模型] + 行管理（徽章 / 启停 / 编辑 / 删除）/ 空态引导 */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <span className="text-ui-sm font-medium text-foreground">模型列表</span>
          <div className="flex items-center gap-1.5">
            <button
              className="flex h-7 items-center gap-1 rounded-lg border border-border px-2.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
              disabled={fetching || endpointMissing}
              title={endpointMissing ? "请先填写服务端点" : undefined}
              onClick={runFetch}
            >
              <RefreshCw className="size-3.5" />
              {fetching ? <span className="animated-gradient-text">拉取中…</span> : "拉取模型"}
            </button>
            <button
              className="flex h-7 items-center gap-1 rounded-lg border border-border px-2.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover hover:text-foreground"
              onClick={() => setModelDialog({ mode: "add" })}
            >
              <Plus className="size-3.5" /> 添加模型
            </button>
          </div>
        </div>
        {fetchFeedback && (
          <p
            className={`text-ui-xs ${fetchFeedback.ok ? "text-success" : "text-destructive"}`}
            role="status"
          >
            {fetchFeedback.text}
          </p>
        )}
        {provider.models.length === 0 ? (
          <div className="flex flex-col items-center gap-2.5 rounded-xl border border-dashed border-border px-4 py-8 text-center">
            <span className="text-ui-sm text-foreground-subtlest">当前没有配置模型，添加模型后可在聊天中使用。</span>
            <button
              className="h-7 rounded-lg border border-border px-3 text-ui-sm text-foreground hover:bg-surface-hover"
              onClick={() => setModelDialog({ mode: "add" })}
            >
              添加模型
            </button>
          </div>
        ) : (
          <div className="flex flex-col gap-1">
            {provider.models.map((m) => (
              <ModelRow
                key={m.ref}
                model={m}
                onEdit={() => setModelDialog({ mode: "edit", model: m })}
                onToggle={(enabled) => setModelEnabled(m, enabled)}
                onDelete={() => confirmRemoveModel(m)}
              />
            ))}
          </div>
        )}
      </div>

      {/* 模型增改共用弹窗（spec §7）：保存成功才关窗，失败内联红字 */}
      {modelDialog && (
        <ProviderModelDialog
          providerId={provider.id}
          providerName={provider.name}
          model={modelDialog.mode === "edit" ? modelDialog.model : null}
          onClose={() => setModelDialog(null)}
          onSaved={() => onChanged()}
        />
      )}
    </div>
  );
}

/** 模型列表行（spec §6）：名称 + ID（同名不重复显示）+ 上下文徽章 + 能力徽章（视觉 / 推理）
    + 编辑 / 删除 / 启用开关；禁用行弱化不隐藏；行内错误仅在异常数据时出现（防御渲染）。 */
function ModelRow({
  model,
  onEdit,
  onToggle,
  onDelete,
}: {
  model: ModelView;
  onEdit(): void;
  onToggle(enabled: boolean): void;
  onDelete(): void;
}): React.JSX.Element {
  const invalid = !(model.contextK > 0);
  return (
    <div
      className={`flex items-center gap-2 rounded-lg border border-border/40 bg-panel px-2.5 py-1.5 ${
        model.enabled ? "" : "opacity-50"
      }`}
    >
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="truncate text-ui-sm text-foreground">{model.name}</span>
          {model.id !== model.name && (
            <span className="truncate font-mono text-ui-xs text-foreground-subtlest">{model.id}</span>
          )}
        </div>
        {invalid && <span className="text-ui-xs text-destructive">上下文窗口配置不完整，请编辑模型修正</span>}
      </div>
      <span
        title={formatContextTitle(model.contextK * 1000)}
        className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtle"
      >
        {formatContextK(model.contextK * 1000)}
      </span>
      {model.caps.vision && (
        <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtle">视觉</span>
      )}
      {model.caps.reasoning && (
        <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtle">推理</span>
      )}
      <div className="ml-1 flex shrink-0 items-center gap-1.5">
        <button
          aria-label={`编辑模型 ${model.name}`}
          title="编辑模型"
          className="flex size-6 items-center justify-center rounded-md text-foreground-subtle hover:bg-surface-hover hover:text-foreground"
          onClick={onEdit}
        >
          <Pencil className="size-3.5" />
        </button>
        <Popconfirm message={`删除模型「${model.name}」？引用它的会话需重新选择模型，且不可恢复。`} onConfirm={onDelete}>
          <button
            aria-label={`删除模型 ${model.name}`}
            title="删除模型"
            className="flex size-6 items-center justify-center rounded-md text-foreground-subtle hover:bg-destructive-surface hover:text-destructive"
          >
            <Trash2 className="size-3.5" />
          </button>
        </Popconfirm>
        <Switch checked={model.enabled} onChange={onToggle} />
      </div>
    </div>
  );
}

/** Skills 管理（决策 D11）：用户级 + 工作区级技能的浏览 / 启停 / 查看 / 删除与总开关。
 *  技能目录布局与调用语义见 docs/skills/spec.md。 */
function SkillsSection(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const save = useApp((s) => s.saveSettings);
  const skills = useApp((s) => s.skills);
  const refreshSkills = useApp((s) => s.refreshSkills);
  const workspaces = useApp((s) => s.workspaces);
  const pushToast = useToastsPush();
  // 管理视角的工作区（扫工作区级技能）；空串 = 只扫用户级
  const [wsId, setWsId] = useState("");
  const [preview, setPreview] = useState<{ name: string; content: string } | null>(null);

  useEffect(() => {
    void refreshSkills(wsId);
    // 切换工作区或打开分区时重扫（磁盘即事实源，无缓存失效问题）
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wsId]);

  const activeWorkspace = workspaces.find((w) => w.id === wsId);
  const userSkills = skills.filter((s) => s.scope === "user");
  const wsSkills = skills.filter((s) => s.scope === "workspace");

  const toggleEnabled = (path: string, enabled: boolean): void => {
    void api
      .skillsSetEnabled(path, enabled, wsId)
      .then((next) => {
        useApp.setState({ skills: next });
        pushToast("ok", enabled ? "技能已启用" : "技能已禁用", enabled ? "下一个 run 起对 Agent 生效" : "下一个 run 起不再加载");
      })
      .catch((err) => pushToast("danger", "技能启停失败", cleanErrorMessage(String(err))));
  };

  const removeSkill = (path: string): void => {
    void api
      .skillsRemove(path, wsId)
      .then((next) => {
        useApp.setState({ skills: next });
        pushToast("ok", "技能已删除");
      })
      .catch((err) => pushToast("danger", "技能删除失败", cleanErrorMessage(String(err))));
  };

  const openPreview = (skill: SkillView): void => {
    void api
      .skillsRead(skill.path, wsId)
      .then((res) => setPreview({ name: skill.name, content: res.content }))
      .catch((err) => pushToast("danger", "技能正文读取失败", cleanErrorMessage(String(err))));
  };

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">
        <Puzzle className="mr-2 inline size-5" />
        Skills
      </h2>
      <Row title="启用技能" desc="关闭后 Agent 不加载任何技能，斜杠调用也不可用">
        <Switch checked={settings.skillsEnabled} onChange={(v) => void save({ skillsEnabled: v })} />
      </Row>
      {workspaces.length > 0 && (
        <Row title="查看工作区" desc="切换后同时扫描该工作区下的 .craftwork/skills 与 .agents/skills">
          <select value={wsId} onChange={(e) => setWsId(e.target.value)} className="h-8 max-w-56 rounded-lg border border-border bg-input px-2 text-ui-sm">
            <option value="">（仅用户级）</option>
            {workspaces.map((w) => (
              <option key={w.id} value={w.id}>
                {w.name}
              </option>
            ))}
          </select>
        </Row>
      )}

      {skills.length === 0 ? (
        <div className="flex flex-col gap-2 rounded-xl bg-surface px-4 py-3 text-ui-sm text-foreground-subtle">
          <p>还没有技能。在技能目录下按「一个目录一个技能」放置 SKILL.md 即可：</p>
          <pre className="overflow-x-auto rounded-lg border border-border/60 bg-background px-3 py-2 text-ui-xs leading-5 text-foreground-subtle">{`~/.craftwork/skills/demo/SKILL.md        （用户级，跨会话可用）
<工作区>/.craftwork/skills/demo/SKILL.md （工作区级，随仓库分发）
<工作区>/.agents/skills/demo/SKILL.md    （兼容 agents 生态）`}</pre>
          <pre className="overflow-x-auto rounded-lg border border-border/60 bg-background px-3 py-2 text-ui-xs leading-5 text-foreground-subtle">{`---
name: demo
description: 一句话说明这个技能做什么、何时使用
---

# 技能正文（Markdown）

具体步骤与约定；辅助脚本放在本目录内，
正文中用 \${CRAFTWORK_SKILL_DIR} 引用技能目录。`}</pre>
        </div>
      ) : (
        <>
          <SkillGroup label="用户级" hint="所有会话可用（数据目录与 ~/.agents）" skills={userSkills} onToggle={toggleEnabled} onRemove={removeSkill} onPreview={openPreview} />
          {wsId !== "" && (
            <SkillGroup
              label={activeWorkspace ? `工作区 · ${activeWorkspace.name}` : "工作区"}
              hint={activeWorkspace?.trust === "restricted" ? "受限工作区：Agent 提示词不自动注入，仅显式调用可用" : "随仓库分发给所有协作者"}
              skills={wsSkills}
              onToggle={toggleEnabled}
              onRemove={removeSkill}
              onPreview={openPreview}
            />
          )}
        </>
      )}

      {/* 正文预览（剥 frontmatter、目录变量已替换后的展示文本） */}
      <Dialog.Root open={preview !== null} onOpenChange={(o) => !o && setPreview(null)}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[70] bg-black/40" />
          <Dialog.Content className="fixed left-1/2 top-1/2 z-[80] flex max-h-[72vh] w-[min(720px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-xl border border-border bg-background p-4 shadow-lg">
            <Dialog.Title className="text-ui-base font-semibold text-foreground">技能预览 · {preview?.name}</Dialog.Title>
            <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border/60 bg-surface px-3 py-2 text-ui-xs leading-5 text-foreground-subtle">
              {preview?.content}
            </pre>
            <div className="flex justify-end">
              <button className="h-8 rounded-lg border border-border px-3 text-ui-sm text-foreground hover:bg-surface-hover" onClick={() => setPreview(null)}>
                关闭
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </div>
  );
}

/** 技能分组列表：标题 + 说明 + 条目（名称/来源徽标/描述/诊断/启停/查看/删除）。 */
function SkillGroup({
  label,
  hint,
  skills,
  onToggle,
  onRemove,
  onPreview,
}: {
  label: string;
  hint: string;
  skills: SkillView[];
  onToggle(path: string, enabled: boolean): void;
  onRemove(path: string): void;
  onPreview(skill: SkillView): void;
}): React.JSX.Element {
  if (skills.length === 0) return <></>;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline gap-2 px-1 pt-1">
        <span className="text-ui-sm font-medium text-foreground">{label}</span>
        <span className="text-ui-xs text-foreground-subtlest">{hint}</span>
      </div>
      {skills.map((skill) => (
        <div
          key={skill.path}
          className={`flex items-center gap-3 rounded-xl border border-border/50 bg-panel px-4 py-3 ${skill.enabled ? "" : "opacity-60"}`}
        >
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-ui-base text-foreground">{skill.name}</span>
              <span className="shrink-0 rounded border border-border px-1 text-[10px] leading-4 text-foreground-subtlest">
                {skill.source === "agents" ? "agents" : "craftwork"}
              </span>
            </div>
            {skill.description && <div className="truncate text-ui-sm text-foreground-subtle">{skill.description}</div>}
            {skill.error ? (
              <div className="truncate text-ui-xs text-destructive">{skill.error}</div>
            ) : (
              <div className="truncate text-ui-xs text-foreground-subtlest" title={skill.path}>
                {skill.path}
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <button
              type="button"
              title="查看正文"
              className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover"
              onClick={() => onPreview(skill)}
            >
              <Eye className="size-4" />
            </button>
            <Popconfirm message={`删除技能「${skill.name}」？将同时删除其目录内全部文件，不可恢复。`} onConfirm={() => onRemove(skill.path)}>
              <button
                type="button"
                title="删除技能"
                className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-destructive"
              >
                <Trash2 className="size-4" />
              </button>
            </Popconfirm>
            <Switch checked={skill.enabled} onChange={(v) => onToggle(skill.path, v)} />
          </div>
        </div>
      ))}
    </div>
  );
}

/** 个性化指令字节上限（与后端 PERSONALIZATION_MAX_BYTES 对齐）。 */
const PERSONALIZATION_MAX_BYTES = 100 * 1024;

/** 字节数的 KB 展示（上限对齐场景，1 位小数内自适应）。 */
function formatKb(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
}

/** 个性化分节：编辑 <数据目录>/AGENTS.md（全局私人指令，每轮 run 注入系统提示词，见 craft-agent::envcontext）。
 *  载入后本地编辑，保存覆盖写（空内容删除文件）；100KB 硬上限——计数超限禁用保存。 */
function PersonalizationSection(): React.JSX.Element {
  const pushToast = useToastsPush();
  const [info, setInfo] = useState<PersonalizationInfo | null>(null);
  const [text, setText] = useState("");
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    void api
      .personalizationGet()
      .then((res) => {
        setInfo(res);
        setText(res.content);
      })
      .catch((err) => pushToast("danger", "个性化指令读取失败", cleanErrorMessage(String(err))));
    // 仅装载时读取一次；「取消」回滚到 info.content
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const bytes = new TextEncoder().encode(text).length;
  const overLimit = bytes > PERSONALIZATION_MAX_BYTES;
  const changed = dirty && text !== (info?.content ?? "");
  const canSave = !saving && !overLimit && changed;

  const save = () => {
    setSaving(true);
    void api
      .personalizationSave(text)
      .then((res) => {
        setInfo(res);
        setText(res.content);
        setDirty(false);
        pushToast("ok", "个性化指令已保存", res.exists ? "对后续会话生效" : "全局指令已清除");
      })
      .catch((err) => pushToast("danger", "个性化指令保存失败", cleanErrorMessage(String(err))))
      .finally(() => setSaving(false));
  };

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">个性化</h2>
      <p className="text-ui-sm text-foreground-subtle">
        添加自定义指令来个性化 Agent 的回复，指令会应用到本机 Agent 的后续会话（保存到 &lt;数据目录&gt;/AGENTS.md，随每轮会话注入）。
      </p>
      <textarea
        ref={areaRef}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setDirty(true);
        }}
        spellCheck={false}
        placeholder={"## 语言约束\n1. 使用中文进行思考和回复\n\n## 开发约束\n1. 所有相关 SDK 都安装到了 ~/Developer 中，必须使用这些工具"}
        className="h-80 w-full resize-y rounded-xl border border-border bg-input px-4 py-3 font-mono text-ui-sm leading-relaxed text-foreground outline-none placeholder:text-foreground-subtlest focus:border-brand"
      />
      <div className="flex items-center justify-between">
        <div className={`text-ui-xs ${overLimit ? "text-destructive" : "text-foreground-subtlest"}`}>
          {formatKb(bytes)} / 100 KB{overLimit && " · 已超出上限，请精简后保存"}
        </div>
        <div className="flex items-center gap-2">
          <button
            className="h-8 rounded-lg border border-border px-4 text-ui-sm text-foreground-subtle hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
            disabled={!canSave && !dirty}
            onClick={() => {
              setText(info?.content ?? "");
              setDirty(false);
            }}
          >
            取消
          </button>
          <button
            className="h-8 rounded-lg bg-brand px-4 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
            disabled={!canSave}
            onClick={save}
          >
            保存
          </button>
        </div>
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

// 基于 radix-ui 的开关：Root 提供语义与键盘交互，滑块经 flex 垂直居中、
// 由 data-state 驱动位移与配色，不再依赖 button 匿名布局与 JS 切类。
function Switch({ checked, onChange }: { checked: boolean; onChange(v: boolean): void }): React.JSX.Element {
  return (
    <RadixSwitch.Root
      checked={checked}
      onCheckedChange={onChange}
      className="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand data-[state=checked]:bg-brand data-[state=unchecked]:bg-border"
    >
      <RadixSwitch.Thumb className="pointer-events-none block size-4 translate-x-[2px] rounded-full bg-white shadow transition-transform data-[state=checked]:translate-x-[18px]" />
    </RadixSwitch.Root>
  );
}

function useToastsPush() {
  return useToasts((s) => s.push);
}

/** 就近确认气泡（对齐 ant.design Popconfirm）：锚定触发元素的小型确认浮层，用于低层级删除操作。
 *  打开即聚焦删除按钮——聚焦态下回车 / 空格直接确认（浏览器原生按钮行为）。 */
function Popconfirm({ message, onConfirm, children }: { message: string; onConfirm(): void; children: React.ReactNode }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const confirmRef = useRef<HTMLButtonElement>(null);
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>{children}</Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="end"
          sideOffset={6}
          className="z-[80] w-72 rounded-lg border border-popover-border bg-popover p-3 shadow-md"
          onOpenAutoFocus={(e) => {
            e.preventDefault();
            confirmRef.current?.focus();
          }}
        >
          <p className="text-ui-sm text-foreground">{message}</p>
          <div className="flex justify-end gap-2 pt-2.5">
            <button
              className="h-7 rounded-md border border-border px-2.5 text-ui-xs text-foreground hover:bg-surface-hover"
              onClick={() => setOpen(false)}
            >
              取消
            </button>
            <button
              ref={confirmRef}
              className="h-7 rounded-md bg-destructive px-2.5 text-ui-xs text-white hover:opacity-90"
              onClick={() => {
                setOpen(false);
                onConfirm();
              }}
            >
              删除
            </button>
          </div>
          <Popover.Arrow className="fill-popover" />
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
