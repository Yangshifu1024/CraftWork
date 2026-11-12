// 设置中心：左侧导航 + 右侧内容（通用/外观/模型/网络/个性化/技能/子代理/用量/审计/备份/关于）。
// 所有改动立即保存生效。
import { useEffect, useRef, useState } from "react";
import { Palette, Puzzle, UserRound, Bot, Plus, Trash2, ArrowLeft, SlidersHorizontal, Package, Globe, ChartColumn, ScrollText, DatabaseBackup, HardDrive, Eye, EyeOff, MoreHorizontal, Pencil, PlugZap, RefreshCw, X, Download, Undo2, ChevronRight, ChevronDown, Info, FolderOpen, Cable, type LucideIcon } from "lucide-react";
import { Dialog, DropdownMenu, Popover, Switch as RadixSwitch } from "radix-ui";
import { useApp } from "../store/app";
import { WindowControls } from "../components/window-controls";
import { useToasts } from "../store/toast";
import { useOverlays } from "../store/overlays";
import { api } from "../lib/api";
import { applyTheme, saveThemePref } from "../lib/theme";
import { groupProviders, providerStatus, selectableModels, type ProviderStatusTone } from "../lib/provider-list";
import { formatContextK, formatContextTitle } from "../lib/format-context";
import { cleanErrorMessage } from "../lib/errors";
import { isImeComposing } from "../lib/ime";
import { compactHeaderRows, MAX_CUSTOM_HEADERS, validateHeaderPairs } from "../lib/provider-headers";
import { ProviderLogo } from "../components/provider-logo";
import { Select } from "../components/select";
import { formatBytes } from "../lib/format";
// 应用 logo 直接引用打包用的图标（Vite 视为资源并给出 URL），不复制第二份
import appLogo from "../../../../src-tauri/icons/128x128@2x.png";
import { ProviderAddPage, type AddEntry } from "./provider-add";
import { ProviderModelDialog } from "./provider-model-dialog";
import { SubagentProfileDialog } from "./subagent-profile-dialog";
import { McpSection } from "./mcp-section";
import type { AppInfo, AuditEntry, Backup, BuiltinAgentProfile, ConnectionResult, CustomAgentProfile, HeaderPair, MarketView, ModelView, NetworkProbeTargetView, PersonalizationInfo, ProviderView, RecommendedMarketView, Settings, ShellEnvStatus, ShellOption, SkillView, StorageCleanupResult, StorageOverview, StorageProjectGroup, StorageTableRow, StorageThreadRow, StorageThreadsView, StorePluginDetail, StorePluginView, UsageSummary } from "../../../shared/types";

type Section = "general" | "appearance" | "providers" | "network" | "personalization" | "mcp" | "skills" | "subagents" | "usage" | "audit" | "backups" | "storage" | "about";

/** 侧栏分组导航：分组标题 + 图标导航项。 */
const NAV_GROUPS: Array<{ label: string; items: Array<{ id: Section; label: string; icon: LucideIcon }> }> = [
  {
    label: "基础设置",
    items: [
      { id: "general", label: "通用", icon: SlidersHorizontal },
      { id: "appearance", label: "外观", icon: Palette },
      { id: "providers", label: "模型", icon: Package },
      { id: "network", label: "网络", icon: Globe },
    ],
  },
  {
    label: "Agent 能力",
    items: [
      { id: "personalization", label: "个性化", icon: UserRound },
      { id: "mcp", label: "MCP", icon: Cable },
      { id: "skills", label: "技能", icon: Puzzle },
      { id: "subagents", label: "子代理", icon: Bot },
    ],
  },
  {
    label: "数据与统计",
    items: [
      { id: "usage", label: "用量", icon: ChartColumn },
      { id: "audit", label: "审计", icon: ScrollText },
      { id: "backups", label: "备份", icon: DatabaseBackup },
      { id: "storage", label: "存储", icon: HardDrive },
    ],
  },
  {
    label: "其他",
    items: [{ id: "about", label: "关于", icon: Info }],
  },
];

/** 设置中心全屏浮层：左侧分组导航 + 右侧分区内容；初始分区可由外部入口（overlays store 的 settingsSection，如「管理模型」直达）指定。 */
export function SettingsOverlay(): React.JSX.Element {
  const platform = useApp((s) => s.platform);
  const close = useOverlays((s) => s.close);
  // 支持外部入口指定初始分区（如模型菜单「管理模型」直达「模型」分节）。
  const [section, setSection] = useState<Section>(() => useOverlays.getState().settingsSection ?? "general");
  // 设置页已打开时再收到外部直达意图（如 macOS 菜单栏「关于」）也要跟着切分区：
  // 挂载时的 useState 初值只在首次生效，后续变化靠这个 effect 跟进。
  const requestedSection = useOverlays((s) => s.settingsSection);
  useEffect(() => {
    if (!requestedSection) return;
    setSection(requestedSection);
    // 读完即消费：否则「切到别的分节 → 再点同一入口」时值没变，effect 不会再跑。
    useOverlays.getState().consumeSettingsSection();
  }, [requestedSection]);

  return (
    // 全屏不透明浮层：Esc / 焦点圈定由 radix Dialog 承担；顶部 h-12 拖拽条保留在 Content 首行。
    <Dialog.Root open onOpenChange={(o) => { if (!o && useOverlays.getState().overlay === "settings") close(); }}>
      <Dialog.Content
        aria-describedby={undefined}
        // 悬浮卡片（D22 卡片语言）：四周 8px 边距露窗口底色，全圆角 + 全边框 + 抬高阴影；
    // overflow-hidden 让内部圆角裁切，顶部 h-12 拖拽条与内联窗口控件仍在卡片内
    className="fixed inset-2 z-50 flex flex-col overflow-hidden rounded-xl border border-border bg-background shadow-2xl outline-none"
      >
      {/* 全宽顶部条：交通灯拖拽区（macOS），对齐主窗口侧栏 h-12 顶条；
          Windows 上右端内联窗口控件（D22 控件内联迁移：全页浮层之上仍可操作窗口）。 */}
      <div className="flex h-12 shrink-0 items-stretch pr-2" data-tauri-drag-region>
        <div className="min-w-0 flex-1" />
        {platform === "windows" && <WindowControls />}
      </div>
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
        </aside>
        <section className="min-h-0 flex-1 overflow-y-auto">
          {/* 内容宽度统一取模型分节的 max-w-4xl，各分区不再各自为政 */}
          <div className="mx-auto max-w-4xl px-8 py-8">
            {section === "general" && <GeneralSection />}
            {section === "appearance" && <AppearanceSection />}
            {section === "providers" && <ProvidersSection />}
            {section === "network" && <NetworkSection />}
            {section === "personalization" && <PersonalizationSection />}
            {section === "mcp" && <McpSection />}
            {section === "skills" && <SkillsSection />}
            {section === "subagents" && <SubagentsSection />}
            {section === "usage" && <UsageSection />}
            {section === "audit" && <AuditSection />}
            {section === "backups" && <BackupsSection />}
            {section === "storage" && <StorageSection />}
            {section === "about" && <AboutSection />}
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

/** 子代理分节（决策 D15 扩展）：总开关 + 并发 + 内置两型的默认模型与迭代上限 + 自定义子代理管理。
 *
 *  为什么独立成节而不是留在「通用」：子代理现在是一整套可配置能力（全局参数 + 按代理的
 *  模型 / 轮数 / 工具面），塞在通用里既挤又难发现。配置存独立的 settings 键，
 *  与全量回传的 Settings 分开，改主题不会重写定义。 */
function SubagentsSection(): React.JSX.Element {
  const subagentSettings = useApp((s) => s.subagentSettings);
  const profiles = useApp((s) => s.subagentProfiles);
  const loadSubagentConfig = useApp((s) => s.loadSubagentConfig);
  const saveSubagentSettings = useApp((s) => s.saveSubagentSettings);
  const deleteSubagentProfile = useApp((s) => s.deleteSubagentProfile);
  const setSubagentProfileEnabled = useApp((s) => s.setSubagentProfileEnabled);
  const pushToast = useToasts((s) => s.push);
  // 可选模型清单：与模型选择器同一过滤口径（启用提供商 × 启用模型）
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [editing, setEditing] = useState<CustomAgentProfile | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    void loadSubagentConfig();
    void api.providerList().then(setProviders);
  }, [loadSubagentConfig]);

  const modelOptions: ModelView[] = selectableModels(providers).flatMap((g) => g.models);
  // 轮数上限的封顶值：服务端返回的唯一事实源（未装载时 0，输入框不设 max、由服务端钳制兜底）
  const cap = subagentSettings.maxIterationsCap;

  /** 内置两型的默认值改动：整体回传（服务端归一后重读） */
  const patchBuiltin = (agentType: "general" | "explore", patch: Partial<BuiltinAgentProfile>): void => {
    const next = subagentSettings.builtinProfiles.map((p) =>
      p.agentType === agentType ? { ...p, ...patch } : p,
    );
    void saveSubagentSettings({ ...subagentSettings, builtinProfiles: next });
  };

  const builtinOf = (agentType: "general" | "explore"): BuiltinAgentProfile =>
    subagentSettings.builtinProfiles.find((p) => p.agentType === agentType) ?? {
      agentType,
      model: "",
      maxIterations: 0,
    };

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">子代理</h2>
      <Row
        title="启用子代理"
        desc="允许主代理把独立的子任务派给子代理；关闭后不再提供派发工具，已派发的照常完成"
      >
        <Switch
          checked={subagentSettings.enabled}
          onChange={(v) => void saveSubagentSettings({ ...subagentSettings, enabled: v })}
        />
      </Row>
      <Row title="并发上限" desc="同一对话内同时运行的子代理数量（1~8）">
        <input
          type="number"
          min={1}
          max={8}
          value={subagentSettings.maxConcurrent}
          disabled={!subagentSettings.enabled}
          onChange={(e) =>
            void saveSubagentSettings({
              ...subagentSettings,
              maxConcurrent: Math.max(1, Math.min(8, Number(e.target.value) || 1)),
            })
          }
          className="h-8 w-20 rounded-lg border border-border bg-input px-2 text-ui-sm disabled:opacity-50"
        />
      </Row>

      <div className="flex items-center gap-2 pt-2">
        <h3 className="text-ui-base font-medium text-foreground">内置子代理</h3>
        <span className="text-ui-sm text-foreground-subtlest">只可调整模型与迭代上限；工具面与角色提示词是内置类型的固定行为</span>
      </div>
      {(["general", "explore"] as const).map((agentType) => {
        const profile = builtinOf(agentType);
        return (
          <div
            key={agentType}
            className="flex flex-wrap items-center gap-3 rounded-xl border border-border/50 bg-panel px-4 py-3"
          >
            <div className="min-w-0 flex-1">
              <div className="text-ui-base text-foreground">
                {agentType === "general" ? "通用（general）" : "探查（explore）"}
              </div>
              <div className="text-ui-sm text-foreground-subtlest">
                {agentType === "general" ? "可写可执行的完整工具集" : "只读：检索、阅读、Git 只读"}
              </div>
            </div>
            <Select
              ariaLabel={`${agentType} 默认模型`}
              value={profile.model}
              onChange={(v) => patchBuiltin(agentType, { model: v })}
              options={[
                { value: "", label: "继承会话模型" },
                ...modelOptions.map((m) => ({ value: m.ref, label: m.name || m.id })),
              ]}
            />
            <label className="flex items-center gap-1.5 text-ui-sm text-foreground-subtle">
              轮数上限
              <input
                type="number"
                min={0}
                max={cap > 0 ? cap : undefined}
                value={profile.maxIterations}
                onChange={(e) =>
                  patchBuiltin(agentType, {
                    maxIterations: cap > 0
                      ? Math.max(0, Math.min(cap, Number(e.target.value) || 0))
                      : Math.max(0, Number(e.target.value) || 0),
                  })
                }
                className="h-8 w-20 rounded-lg border border-border bg-input px-2 text-ui-sm"
              />
              <span className="whitespace-nowrap text-ui-xs text-foreground-subtlest">
                0 = 封顶{cap > 0 ? ` ${cap}` : ""}轮
              </span>
            </label>
          </div>
        );
      })}

      <div className="flex items-center gap-2 pt-2">
        <h3 className="text-ui-base font-medium text-foreground">自定义子代理</h3>
        <span className="text-ui-sm text-foreground-subtlest">{profiles.length} 个</span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="flex items-center gap-1 rounded-lg px-2 py-1 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
        >
          <Plus className="size-3.5" /> 新建
        </button>
      </div>
      {profiles.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border px-4 py-6 text-center text-ui-sm text-foreground-subtlest">
          还没有自定义子代理。新建后，模型可以在派发时按名字选用它（例如只做代码评审的 reviewer）
        </div>
      ) : (
        profiles.map((profile) => (
          <div key={profile.id} className="flex items-start gap-3 rounded-xl border border-border/50 bg-panel px-4 py-3">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="truncate font-mono text-ui-base text-foreground">{profile.name}</span>
                <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtle">
                  {profile.tools.kind === "readOnly" ? "只读" : profile.tools.kind === "all" ? "全部工具" : `指定 ${profile.tools.names.length} 个`}
                </span>
                <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">
                  {profile.maxIterations > 0 ? `${profile.maxIterations} 轮` : "轮数封顶"}
                </span>
                {!profile.enabled && (
                  <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">
                    已停用
                  </span>
                )}
              </div>
              <div className="truncate text-ui-sm text-foreground-subtlest">
                {profile.description || "（无简介）"}
              </div>
              <div className="truncate font-mono text-ui-xs text-foreground-subtlest">
                模型：{profile.model || "继承会话模型"}
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <Switch checked={profile.enabled} onChange={(v) => void setSubagentProfileEnabled(profile.id, v)} />
              <button
                type="button"
                onClick={() => setEditing(profile)}
                className="rounded-md p-1 text-foreground-subtle hover:bg-surface-hover"
                title="编辑"
              >
                <Pencil className="size-3.5" />
              </button>
              <button
                type="button"
                onClick={() => {
                  // 删除是破坏性动作（定义不可恢复），走确认
                  if (window.confirm(`删除子代理 ${profile.name}？该定义将不可恢复。`)) {
                    void deleteSubagentProfile(profile.id);
                  }
                }}
                className="rounded-md p-1 text-foreground-subtlest hover:bg-surface-hover"
                title="删除"
              >
                <Trash2 className="size-3.5" />
              </button>
            </div>
          </div>
        ))
      )}

      {(creating || editing) && (
        <SubagentProfileDialog
          profile={editing}
          models={modelOptions}
          onClose={() => {
            setCreating(false);
            setEditing(null);
          }}
          onSaved={() => {
            void loadSubagentConfig();
            pushToast("ok", editing ? "子代理已更新" : "子代理已创建", "下一轮对话生效");
          }}
        />
      )}
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
  // Shell 环境（D18 / D25）：可选 shell 清单 + 当前生效概况；选中后命令也由该 shell 执行。两者都在进入分区时拉一次，
  // 下拉切换后刷新概况（捕获可能失败，用户需要立刻看到原因）。
  const [shells, setShells] = useState<ShellOption[]>([]);
  // 清单拉取失败的原因：非空时下拉下方常驻红字。失败必须可见——「空清单」与「这台机器
  // 真的没有可选 shell」在下拉外观上完全相同，静默吞掉会把通道故障伪装成系统事实。
  const [shellsError, setShellsError] = useState("");
  const [shellStatus, setShellStatus] = useState<ShellEnvStatus | null>(null);
  // 切换中：保存设置与重新捕获都要跑子进程，状态未落定前禁掉下拉——否则用户会在
  // 两次读取之间再选一次，最终生效哪个值不可预期。
  const [shellBusy, setShellBusy] = useState(false);
  useEffect(() => {
    void api
      .shellsList()
      .then((list) => {
        setShells(list);
        setShellsError("");
      })
      .catch((err) => {
        const message = cleanErrorMessage(err);
        setShells([]);
        setShellsError(message);
        useToasts.getState().push("danger", "读取可选 Shell 失败", message);
      });
    // 初次进入分区也会跑一次捕获：同样禁止切换，避免与用户的首次选择交叉。
    setShellBusy(true);
    void api
      .shellEnvRefresh()
      .then(setShellStatus)
      .catch((err) => {
        setShellStatus(null);
        useToasts.getState().push("danger", "Shell 环境读取失败", cleanErrorMessage(err));
      })
      .finally(() => setShellBusy(false));
  }, []);
  /** 读取当前 shell 的环境概况；失败进 toast。调用方负责 `shellBusy` 的起止。 */
  const fetchShellStatus = async (): Promise<void> => {
    try {
      const status = await api.shellEnvRefresh();
      setShellStatus(status);
      if (status.error) useToasts.getState().push("danger", "Shell 环境读取失败", status.error);
    } catch (err) {
      setShellStatus(null);
      useToasts.getState().push("danger", "Shell 环境读取失败", cleanErrorMessage(err));
    }
  };
  /** 串行化 shell 相关操作：期间下拉禁用，结束后恢复。 */
  const runShellTask = async (task: () => Promise<void>): Promise<void> => {
    setShellBusy(true);
    try {
      await task();
    } finally {
      setShellBusy(false);
    }
  };
  const refreshShellEnv = (): void => {
    void runShellTask(fetchShellStatus);
  };
  const changeShell = (value: string): void => {
    void runShellTask(async () => {
      try {
        await save({ shellPath: value });
        await fetchShellStatus();
      } catch (err) {
        useToasts.getState().push("danger", "Shell 环境切换失败", cleanErrorMessage(err));
      }
    });
  };
  const selectedShell = shells.find((s) => s.path === settings.shellPath);
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">通用</h2>
      <Row title="界面模式" desc="Code 提供完整文件/命令/Git；Work 只提供文件与文档">
        <Select
          ariaLabel="界面模式"
          value={settings.mode}
          onChange={(v) => setMode(v as "code" | "work")}
          options={[
            { value: "code", label: "Code" },
            { value: "work", label: "Work" },
          ]}
        />
      </Row>
      <Row title="系统通知" desc="窗口失焦时的审批 / 完成 / 失败提醒">
        <Switch checked={settings.notifications} onChange={(v) => void save({ notifications: v })} />
      </Row>
      <Row
        title="关闭窗口时退出应用"
        desc="默认关闭窗口只是隐藏到托盘，run、后台命令与 MCP 继续；开启后关闭即退出（与托盘「退出」相同）"
      >
        <Switch checked={settings.quitOnClose} onChange={(v) => void save({ quitOnClose: v })} />
      </Row>
      {/* Shell 环境（D18）：选了就执行它的启动配置并继承其中的环境变量。
          长说明与长路径下拉在单行 Row 里会互相挤压（说明被挤成窄条逐行折行）——
          此节独立成卡片：说明占满整行，下拉独立成行（fullWidth），状态行跟随其后。 */}
      <div className="flex flex-col gap-2.5 rounded-xl border border-border/50 bg-panel px-4 py-3">
        <div className="text-ui-base text-foreground">Shell 环境</div>
        <p className="text-ui-sm leading-relaxed text-foreground-subtlest">
          未选择时命令由系统默认 shell 执行（macOS / Linux 为 <code>/bin/sh</code>，Windows 为 <code>cmd</code>）。
          选择后命令由该 shell 解释，并把它的启动配置文件执行一遍（环境变量剔除密钥类后交给命令）——这样
          nvm / pyenv / pnpm 装的工具才可见。启动配置只在读取环境时跑一次，不会为每条命令重复执行
        </p>
        <div className="flex min-w-0 items-center gap-2">
          <div className="min-w-0 flex-1">
            <Select
              ariaLabel="Shell 环境"
              fullWidth
              value={settings.shellPath}
              disabled={shellBusy}
              onChange={changeShell}
              options={[
                { value: "", label: "不使用（用系统默认 shell）" },
                ...shells.map((sh) => ({
                  value: sh.path,
                  // current = 进程 $SHELL 指向的登录 shell，不是「设置里选中的那个」
                  label: `${sh.path}${sh.current ? "（登录 Shell）" : ""}`,
                })),
              ]}
            />
          </div>
          {settings.shellPath ? (
            <div className="flex shrink-0 items-center gap-1.5 text-ui-xs text-foreground-subtle">
              {shellBusy ? (
                <span className="animated-gradient-text">读取中…</span>
              ) : shellStatus?.error ? (
                "读取失败"
              ) : (
                `已加载 ${shellStatus?.varCount ?? 0} 个变量`
              )}
              <button
                className="inline-flex items-center gap-1 rounded px-1 text-foreground-subtle hover:bg-surface-hover hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                disabled={shellBusy}
                onClick={refreshShellEnv}
              >
                <RefreshCw className="size-3" /> 重新读取
              </button>
            </div>
          ) : null}
        </div>
        {(() => {
          const configs = shellStatus?.configs.length ? shellStatus.configs : selectedShell?.configs;
          return configs?.length ? (
            <div className="text-ui-xs text-foreground-subtlest">
              将读取：{configs.map((c) => c.replace(/^.*\//, "")).join("、")}
            </div>
          ) : null;
        })()}
      </div>
      {shellsError ? (
        <p className="rounded-lg bg-destructive-surface px-2.5 py-1.5 text-ui-xs text-destructive-surface-foreground">
          读取可选 Shell 清单失败，下拉里因此只有「不使用」：{shellsError}
        </p>
      ) : null}
      {settings.shellPath ? (
        <div className="rounded-lg border border-warning/40 bg-warning-surface/40 px-3 py-2 text-ui-xs text-warning-surface-foreground">
          该 shell 的启动配置会在每条命令前执行。若其中导出了密钥类变量（如 <code>*_API_KEY</code>、
          <code>GITHUB_TOKEN</code>），这些变量名会被自动剔除，但配置里的其它内容仍可能被命令读取并写入对话记录与日志。
        </div>
      ) : null}
      <Row
        title="运行日志等级"
        desc={`排查回复截断等问题的观测时间线，保存即生效；目录 ${dataDir ? `${dataDir}/logs/` : "（<数据目录>/logs/）"}`}
      >
        <Select
          ariaLabel="运行日志等级"
          value={settings.logsLevel}
          onChange={(v) => void save({ logsLevel: v as Settings["logsLevel"] })}
          options={[
            { value: "off", label: "关闭" },
            { value: "error", label: "仅错误" },
            { value: "info", label: "标准" },
            { value: "debug", label: "调试" },
          ]}
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
      <h2 className="text-ui-settings-title font-semibold text-foreground">外观</h2>
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
      <Row title="流式平滑跟随" desc="流式输出时以弹簧动画平滑贴底；关闭后即时跟随（直接跳到底）">
        <Switch checked={settings.smoothFollow} onChange={(v) => void save({ smoothFollow: v })} />
      </Row>
    </div>
  );
}

// ---------- 网络（决策 D14） ----------

/** 出站代理协议白名单（spec 第 1 节，与后端 craft-core::network::normalize_proxy_url 同口径）。 */
const PROXY_SCHEMES = ["http", "https", "socks5", "socks5h"];

/** 出站代理地址轻校验：空值允许（清空即不设地址）；无协议头按 http 处理（后端保存时自动补全，
 *  `127.0.0.1:7890` 这类纯 host:port 合法）；带协议头时仅接受白名单协议。返回错误文案，空串表示合法。 */
function validateProxyUrl(raw: string): string {
  const value = raw.trim();
  if (value === "") return "";
  const scheme = value.includes("://") ? (value.split("://")[0] ?? "").toLowerCase() : "http";
  if (!PROXY_SCHEMES.includes(scheme)) {
    return `不支持的代理协议：${scheme || "（空）"}（仅支持 ${PROXY_SCHEMES.join(" / ")}）`;
  }
  return "";
}

/** 网络页探测行的状态（D14 修订）：行固定、只改单格。`idle` = 本次会话尚未探测。
 *  与后端 `NetworkProbeView` 的区别是这里把「探测中」也建模出来——行不需要为了显示
 *  运行态而被替换成别的对象。 */
type ProbeRowState =
  | { kind: "idle" }
  | { kind: "probing" }
  | { kind: "ok"; latencyMs: number }
  | { kind: "fail"; error: string };

/** 网络分节：出站代理三选 + GitHub 加速 + 公开端点探测列表。保存即写库（saveSettings 全量回传），
 *  保存后下一次网络请求生效（模型 client 每次请求新建、gix 每次下载传参，无需重启）。
 *  三张卡片容器结构与 Row 一致，但展开区（地址/前缀输入框、探测结果）需要占满整行，
 *  因此不走通用 Row 组件而按同款样式展开。 */
function NetworkSection(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const save = useApp((s) => s.saveSettings);

  // 出站代理地址（blur / Enter 保存）：草稿态 + 前端轻校验（非法协议红字阻止，后端同样拒绝并保持现值）
  const [proxyUrlDraft, setProxyUrlDraft] = useState(settings.networkProxyUrl);
  const [proxyUrlError, setProxyUrlError] = useState("");

  // GitHub 加速前缀（blur / Enter 保存）：保存时补尾斜杠（后端归一同规则，前端先行回显一致）
  const [prefixDraft, setPrefixDraft] = useState(settings.networkGithubAcceleratorPrefix);

  // 公开端点探测（决策 D14 第 6 节修订）：不再依赖已配置提供商——并发探测目录内九个常用
  // 提供商端点的可达性（GET 不带密钥；任意 HTTP 状态码都算连通，15 秒超时）。
  //
  // 行清单在进入分节时就拉取并**固定渲染**（`targets`），探测只改某一行的状态（`rowStates`）：
  // 不增删行、不整块替换列表。这样做的直接原因是实机事故——首测时整块子树从空态 `<p>`
  // 换成一串行、同时 9 个扫光元素一起动画，WebView2 留下绘制残影（同一行被重复画出十几次，
  // 且按钮状态与行状态互相矛盾）；固定行 + 行内只改一格把它从结构上消除。
  const [targets, setTargets] = useState<NetworkProbeTargetView[]>([]);
  const [targetsError, setTargetsError] = useState("");
  const [rowStates, setRowStates] = useState<Record<string, ProbeRowState>>({});
  const [probingAll, setProbingAll] = useState(false);
  const [probingIds, setProbingIds] = useState<string[]>([]);

  // 行清单只在进入分节时读一次：探测前后行集合不变（目录变化需重进分节，可接受）
  useEffect(() => {
    void api
      .networkProbeTargets()
      .then((rows) => {
        setTargets(rows);
        setTargetsError("");
      })
      .catch((err) => {
        // 读取失败必须可见：空列表与「目录里没有可选端点」外观相同，静默会把通道故障
        // 伪装成系统事实
        setTargets([]);
        setTargetsError(cleanErrorMessage(err));
      });
  }, []);

  /** 单行状态写入（按 id 合并，行顺序由 `targets` 决定，与探测返回顺序无关）。 */
  const patchRow = (id: string, next: ProbeRowState): void => {
    setRowStates((prev) => ({ ...prev, [id]: next }));
  };

  const commitProxyUrl = (): void => {
    const err = validateProxyUrl(proxyUrlDraft);
    if (err) {
      setProxyUrlError(err);
      return;
    }
    setProxyUrlError("");
    const next = proxyUrlDraft.trim();
    if (next === settings.networkProxyUrl) return;
    void save({ networkProxyUrl: next });
  };

  const commitPrefix = (): void => {
    // 归一补尾斜杠后保存；空前缀原样保存（改写函数对空前缀不改写）
    const trimmed = prefixDraft.trim();
    const next = trimmed !== "" && !trimmed.endsWith("/") ? `${trimmed}/` : trimmed;
    setPrefixDraft(next);
    if (next === settings.networkGithubAcceleratorPrefix) return;
    void save({ networkGithubAcceleratorPrefix: next });
  };

  // 全量探测：九行一起置「探测中」，一次并发全测（后端每端点一线程，总耗时 ≈ 最慢者），
  // 返回后按 id 合并回各行；行本身不重建
  const runProbeAll = (): void => {
    if (probingAll || probingIds.length > 0) return;
    setProbingAll(true);
    setRowStates((prev) => {
      const next = { ...prev };
      for (const t of targets) next[t.id] = { kind: "probing" };
      return next;
    });
    api
      .networkProbeEndpoints()
      .then((rows) => {
        setRowStates((prev) => {
          const next = { ...prev };
          for (const row of rows) {
            next[row.id] =
              row.status === "ok" ? { kind: "ok", latencyMs: row.latencyMs } : { kind: "fail", error: row.error ?? "连接失败" };
          }
          return next;
        });
      })
      .catch((err) => {
        // 整批失败：把仍在探测中的行退回未测试，避免永久停在「探测中」
        setRowStates((prev) => {
          const next = { ...prev };
          for (const t of targets) if (next[t.id]?.kind === "probing") next[t.id] = { kind: "idle" };
          return next;
        });
        useToasts.getState().push("danger", "探测失败", cleanErrorMessage(err));
      })
      .finally(() => setProbingAll(false));
  };

  // 单行重测：同一通道传单 id，只写该行状态（其余行保持现值；允许不同行并发重测）
  const runProbeOne = (id: string): void => {
    if (probingAll || probingIds.includes(id)) return;
    setProbingIds((prev) => [...prev, id]);
    patchRow(id, { kind: "probing" });
    api
      .networkProbeEndpoints(id)
      .then((rows) => {
        const row = rows.find((r) => r.id === id);
        if (!row) {
          patchRow(id, { kind: "idle" });
          return;
        }
        patchRow(
          id,
          row.status === "ok" ? { kind: "ok", latencyMs: row.latencyMs } : { kind: "fail", error: row.error ?? "连接失败" },
        );
      })
      .catch((err) => {
        patchRow(id, { kind: "idle" });
        useToasts.getState().push("danger", "探测失败", cleanErrorMessage(err));
      })
      .finally(() => setProbingIds((prev) => prev.filter((x) => x !== id)));
  };

  // 文本输入共用按键：回车等价失焦（保存逻辑统一走 blur）；IME 组合中的回车不拦截
  const commitOnEnter = (e: React.KeyboardEvent<HTMLInputElement>): void => {
    if (isImeComposing(e)) return;
    if (e.key === "Enter") {
      e.preventDefault();
      e.currentTarget.blur();
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">网络</h2>

      {/* 出站代理：三选下拉（系统代理为默认）；自定义模式卡片内展开地址输入框 */}
      <div className="flex flex-col gap-2.5 rounded-xl border border-border/50 bg-panel px-4 py-3">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="text-ui-base text-foreground">出站代理</div>
            <div className="text-ui-sm text-foreground-subtlest">模型请求等出站流量使用的代理；本地与局域网地址始终直连</div>
          </div>
          <Select
            ariaLabel="出站代理"
            value={settings.networkProxyMode}
            onChange={(v) => void save({ networkProxyMode: v as Settings["networkProxyMode"] })}
            options={[
              { value: "system", label: "系统代理" },
              { value: "none", label: "无代理" },
              { value: "custom", label: "自定义代理" },
            ]}
          />
        </div>
        {settings.networkProxyMode === "custom" && (
          <div className="flex flex-col gap-1">
            <input
              value={proxyUrlDraft}
              onChange={(e) => {
                setProxyUrlDraft(e.target.value);
                if (proxyUrlError) setProxyUrlError("");
              }}
              onBlur={commitProxyUrl}
              onKeyDown={commitOnEnter}
              placeholder="http://127.0.0.1:7890"
              aria-label="代理地址"
              aria-invalid={proxyUrlError !== ""}
              className={`h-8 w-full rounded-lg border bg-input px-2.5 font-mono text-ui-xs ${
                proxyUrlError ? "border-destructive" : "border-border"
              }`}
            />
            {proxyUrlError ? (
              <p className="text-ui-xs text-destructive">{proxyUrlError}</p>
            ) : (
              <p className="text-ui-xs text-foreground-subtlest">
                支持 {PROXY_SCHEMES.join(" / ")}；可含用户名密码，凭据明文存储于本地数据库
              </p>
            )}
          </div>
        )}
      </div>

      {/* GitHub 加速：开关 + 前缀输入框（始终可改，保存时补尾斜杠） */}
      <div className="flex flex-col gap-2.5 rounded-xl border border-border/50 bg-panel px-4 py-3">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="text-ui-base text-foreground">GitHub 加速</div>
            <div className="text-ui-sm text-foreground-subtlest">访问 github.com 时前缀改写加速，失败自动回退；第三方服务，国内网络建议开启</div>
          </div>
          <Switch checked={settings.networkGithubAcceleratorEnabled} onChange={(v) => void save({ networkGithubAcceleratorEnabled: v })} />
        </div>
        <div className="flex flex-col gap-1">
          <div className="flex items-center gap-2.5">
            <span className="shrink-0 text-ui-xs text-foreground-subtlest">加速前缀</span>
            <input
              value={prefixDraft}
              onChange={(e) => setPrefixDraft(e.target.value)}
              onBlur={commitPrefix}
              onKeyDown={commitOnEnter}
              placeholder="https://gh-proxy.com/"
              aria-label="加速前缀"
              className="h-8 min-w-0 flex-1 rounded-lg border border-border bg-input px-2.5 font-mono text-ui-xs"
            />
          </div>
          <p className="text-ui-xs text-foreground-subtlest">保存时自动补尾斜杠；可换任意同类前缀式加速服务</p>
        </div>
      </div>

      {/* 公开端点探测（决策 D14 第 6 节修订）：不再依赖已配置提供商，按钮始终可点。
          行清单进入分节即固定渲染（`targets`），探测只改行内状态格——不增删行、不整块替换。
          行内运行态是**纯文字**（不带扫光）：整张卡只保留按钮一处扫光动画，避免多路
          background-clip:text 动画与结构突变叠加触发 WebView2 绘制残影。 */}
      <div className="flex flex-col gap-2.5 rounded-xl border border-border/50 bg-panel px-4 py-3">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="text-ui-base text-foreground">测试连接</div>
            <div className="text-ui-sm text-foreground-subtlest">用当前网络配置探测九个常用提供商端点（含智谱 AI、DeepSeek、MiniMax 国内站）的可达性（15 秒超时，任意响应都算连通）</div>
          </div>
          <button
            className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
            disabled={probingAll || probingIds.length > 0 || targets.length === 0}
            onClick={runProbeAll}
          >
            <PlugZap className="size-3.5 shrink-0" />
            {probingAll ? <span className="animated-gradient-text">测试中…</span> : "测试连接"}
          </button>
        </div>
        {targetsError ? (
          <p className="text-ui-xs text-destructive">读取探测清单失败：{targetsError}</p>
        ) : targets.length === 0 ? (
          <p className="text-ui-xs text-foreground-subtlest">正在读取探测清单…</p>
        ) : (
          <div className="flex flex-col divide-y divide-border/40">
            {targets.map((t) => {
              const state = rowStates[t.id] ?? { kind: "idle" };
              const busy = state.kind === "probing";
              return (
                <div key={t.id} className="flex items-center justify-between gap-3 py-2">
                  <div className="min-w-0">
                    <div className="text-ui-sm text-foreground">{t.name}</div>
                    <div className="truncate font-mono text-ui-xs text-foreground-subtlest" title={t.url}>{t.url}</div>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {state.kind === "probing" ? (
                      <span className="text-ui-xs text-foreground-subtlest">探测中…</span>
                    ) : state.kind === "ok" ? (
                      <span className="text-ui-xs text-success">✓ 连接成功 · {state.latencyMs}ms</span>
                    ) : state.kind === "fail" ? (
                      <span className="max-w-56 truncate text-ui-xs text-destructive" title={state.error}>✗ {state.error}</span>
                    ) : (
                      <span className="text-ui-xs text-foreground-subtlest">未测试</span>
                    )}
                    <button
                      className="rounded-md p-1 text-foreground-subtle hover:bg-surface-hover hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
                      aria-label={`重测 ${t.name}`}
                      title="重测此端点"
                      disabled={busy || probingAll}
                      onClick={() => runProbeOne(t.id)}
                    >
                      <RefreshCw className="size-3.5" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <p className="text-ui-xs text-foreground-subtlest">
        模型调用、目录拉取、技能商店下载共用此网络配置；保存后下一次网络请求生效。
      </p>
    </div>
  );
}

/** 模型分区（主从两栏，spec §4）：左供应商列表（分组 + 状态点）、右详情卡 / 添加页 / 空态引导三态切换。 */
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
          <h2 className="text-ui-settings-title font-semibold text-foreground">模型</h2>
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

/** Skills 设置分节（决策 D11 / D12）：「本机技能 / 技能商店」双 tab。
 *  tab 选中态放 app store 的商店槽（storeTab）——命令面板「浏览技能商店」直达时
 *  先置 market 再打开设置页，这里受控渲染。 */
function SkillsSection(): React.JSX.Element {
  const storeTab = useApp((s) => s.storeTab);
  const setStoreTab = useApp((s) => s.setStoreTab);
  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">技能</h2>
      {/* 双 tab：样式沿用设置页既有 segmented 按钮惯例（与「外观 → 主题」三选一致） */}
      <div className="flex gap-1">
        <button
          className={`h-8 rounded-lg border px-3 text-ui-sm ${
            storeTab === "local" ? "border-brand bg-selected text-foreground" : "border-border text-foreground-subtle hover:bg-surface-hover"
          }`}
          onClick={() => setStoreTab("local")}
        >
          本机技能
        </button>
        <button
          className={`h-8 rounded-lg border px-3 text-ui-sm ${
            storeTab === "market" ? "border-brand bg-selected text-foreground" : "border-border text-foreground-subtle hover:bg-surface-hover"
          }`}
          onClick={() => setStoreTab("market")}
        >
          技能商店
        </button>
      </div>
      {storeTab === "local" ? <LocalSkillsPane /> : <StorePane />}
    </div>
  );
}

/** 本机技能 tab（决策 D11 原样迁移）：用户级 + 全部工作区级技能的浏览 / 启停 / 查看 / 删除与总开关。
 *  技能目录布局与调用语义见 D11。 */
function LocalSkillsPane(): React.JSX.Element {
  const settings = useApp((s) => s.settings);
  const save = useApp((s) => s.saveSettings);
  const manageSkills = useApp((s) => s.manageSkills);
  const refreshManageSkills = useApp((s) => s.refreshManageSkills);
  const workspaces = useApp((s) => s.workspaces);
  const pushToast = useToastsPush();
  const [preview, setPreview] = useState<{ name: string; content: string } | null>(null);

  useEffect(() => {
    // 打开分区时重扫全量（磁盘即事实源，无缓存失效问题）
    void refreshManageSkills();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 分组：用户级在前，其后按工作区首次出现顺序（后端按最近打开时间下发）
  const userSkills = manageSkills.filter((s) => !s.workspaceId);
  const wsGroups: Array<{ id: string; name: string; skills: SkillView[] }> = [];
  for (const skill of manageSkills) {
    if (!skill.workspaceId) continue;
    let group = wsGroups.find((g) => g.id === skill.workspaceId);
    if (!group) {
      group = { id: skill.workspaceId, name: skill.workspaceName ?? "", skills: [] };
      wsGroups.push(group);
    }
    group.skills.push(skill);
  }

  const toggleEnabled = (skill: SkillView, enabled: boolean): void => {
    void api
      .skillsSetEnabled(skill.path, enabled, skill.workspaceId ?? "")
      // 写操作返回的是「用户级 + 该工作区级」的局部列表，不能直接覆盖全量视图：重扫全量
      .then(() => refreshManageSkills())
      .then(() => {
        pushToast("ok", enabled ? "技能已启用" : "技能已禁用", enabled ? "下一个 run 起对 Agent 生效" : "下一个 run 起不再加载");
      })
      .catch((err) => pushToast("danger", "技能启停失败", cleanErrorMessage(String(err))));
  };

  const removeSkill = (skill: SkillView): void => {
    void api
      .skillsRemove(skill.path, skill.workspaceId ?? "")
      .then(() => refreshManageSkills())
      .then(() => {
        pushToast("ok", "技能已删除");
      })
      .catch((err) => pushToast("danger", "技能删除失败", cleanErrorMessage(String(err))));
  };

  const openPreview = (skill: SkillView): void => {
    void api
      .skillsRead(skill.path, skill.workspaceId ?? "")
      .then((res) => setPreview({ name: skill.name, content: res.content }))
      .catch((err) => pushToast("danger", "技能正文读取失败", cleanErrorMessage(String(err))));
  };

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">技能</h2>
      <Row title="启用技能" desc="关闭后 Agent 不加载任何技能，斜杠调用也不可用">
        <Switch checked={settings.skillsEnabled} onChange={(v) => void save({ skillsEnabled: v })} />
      </Row>

      {manageSkills.length === 0 ? (
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
          {wsGroups.map((group) => {
            const trust = workspaces.find((w) => w.id === group.id)?.trust;
            return (
              <SkillGroup
                key={group.id}
                label={`工作区 · ${group.name}`}
                hint={trust === "restricted" ? "受限工作区：Agent 提示词不自动注入，仅显式调用可用" : "随仓库分发给所有协作者"}
                skills={group.skills}
                onToggle={toggleEnabled}
                onRemove={removeSkill}
                onPreview={openPreview}
              />
            );
          })}
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

// ---------- 技能商店 tab（决策 D12 / 10 / 11 节） ----------

/** 技能商店 tab 自上而下：市场源管理条 → 推荐源常驻区块 → 插件区（按市场分组）→ 详情 Dialog。
 *  进入时经 ensureStoreLoaded 装载 + 节流自动刷新（store 动作内部处理节流与失败防抖）。 */
function StorePane(): React.JSX.Element {
  const markets = useApp((s) => s.storeMarkets);
  const recommended = useApp((s) => s.storeRecommended);
  const plugins = useApp((s) => s.storePlugins);
  const refreshing = useApp((s) => s.storeRefreshing);
  const refreshStoreMarkets = useApp((s) => s.refreshStoreMarkets);
  const removeStoreMarket = useApp((s) => s.removeStoreMarket);
  const installStorePlugin = useApp((s) => s.installStorePlugin);
  const describeStorePlugin = useApp((s) => s.describeStorePlugin);
  const openConfirm = useOverlays((s) => s.openConfirm);

  // 详情 Dialog 目标插件 id（按需 describe，不常驻详情数据）
  const [detailId, setDetailId] = useState<string | null>(null);
  // 卡片「安装」正在准备确认信息（describe 取来源与技能数量）的插件 id
  const [preparingInstall, setPreparingInstall] = useState("");

  useEffect(() => {
    void useApp.getState().ensureStoreLoaded();
  }, []);

  // 按市场分组：组序先按市场添加顺序，已装插件的市场已删（上游条目消失等边缘态）跟在后面
  const groups: Array<{ marketplaceId: string; items: StorePluginView[] }> = [];
  {
    const byId = new Map<string, StorePluginView[]>();
    for (const p of plugins) {
      let list = byId.get(p.marketplaceId);
      if (!list) {
        list = [];
        byId.set(p.marketplaceId, list);
      }
      list.push(p);
    }
    for (const m of markets) {
      const items = byId.get(m.id);
      if (items) groups.push({ marketplaceId: m.id, items });
    }
    for (const [id, items] of byId) {
      if (!markets.some((m) => m.id === id)) groups.push({ marketplaceId: id, items });
    }
  }

  // 卡片「安装」：先 describe 取来源与技能数量（spec 第 8 节安装确认框展示来源、插件名与技能数量），
  // 再弹全局确认框（confirm 浮层 z-[55] 高于设置页 z-50）；失败 toast 已由 describe 动作发出
  const installFromCard = (plugin: StorePluginView): void => {
    if (preparingInstall !== "") return;
    setPreparingInstall(plugin.id);
    void describeStorePlugin(plugin.id)
      .then((detail) => {
        if (!detail) return;
        openConfirm({
          title: "安装插件",
          message: `将从市场「${plugin.marketplaceId}」安装「${plugin.name}」（${detail.skills.length} 个技能，来源 ${detail.sourceRef || plugin.marketplaceId}）。变更下一轮对话生效。`,
          onOk: () => void installStorePlugin(plugin.id),
        });
      })
      .finally(() => setPreparingInstall(""));
  };

  return (
    <div className="flex flex-col gap-3">
      {/* 市场源管理条 */}
      <StoreMarketsCard
        markets={markets}
        refreshing={refreshing}
        onRefresh={() => void refreshStoreMarkets()}
        onRemove={(id) => void removeStoreMarket(id)}
      />
      {/* 推荐源常驻区块（spec 第 9 节）：固定展示，不随空状态消失 */}
      <StoreRecommendedCard recommended={recommended} markets={markets} />

      {/* 插件区：空状态引导去推荐源区块 */}
      {markets.length === 0 ? (
        <div className="flex flex-col gap-1.5 rounded-xl border border-dashed border-border px-4 py-8 text-center">
          <span className="text-ui-sm text-foreground-subtle">还没有市场源</span>
          <span className="text-ui-xs text-foreground-subtlest">
            从上方「推荐市场源」一键添加，或在市场源管理条输入仓库地址；添加后即可在此浏览与安装插件。
          </span>
        </div>
      ) : (
        groups.map((group) => (
          <StoreMarketGroup
            key={group.marketplaceId}
            marketplaceId={group.marketplaceId}
            items={group.items}
            preparingInstall={preparingInstall}
            onOpen={(p) => setDetailId(p.id)}
            onInstall={installFromCard}
          />
        ))
      )}

      {/* 插件详情 Dialog（Esc / 焦点圈定由 radix Dialog 承担；层高低于 Popconfirm 的 z-[80]） */}
      {detailId !== null && (
        <StorePluginDetailDialog
          pluginId={detailId}
          fallback={plugins.find((p) => p.id === detailId) ?? null}
          onClose={() => setDetailId(null)}
        />
      )}
    </div>
  );
}

/** 市场源管理条（spec 第 10 节第 1 条）：已添加源（最近刷新时间 / 失败红字不阻塞其他源）
 *  + 单源移除（Popconfirm；有已安装插件时后端拒绝，错误 toast 呈现）+ 立即刷新 + 添加输入框。 */
function StoreMarketsCard({
  markets,
  refreshing,
  onRefresh,
  onRemove,
}: {
  markets: MarketView[];
  refreshing: boolean;
  onRefresh(): void;
  onRemove(id: string): void;
}): React.JSX.Element {
  const addMarket = useApp((s) => s.addStoreMarket);
  const [input, setInput] = useState("");
  const [adding, setAdding] = useState(false);

  const submit = (): void => {
    const value = input.trim();
    if (adding || value === "") return;
    setAdding(true);
    void addMarket(value)
      .then((ok) => {
        if (ok) setInput("");
      })
      .finally(() => setAdding(false));
  };

  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-border/50 bg-panel px-4 py-3">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <div className="text-ui-base text-foreground">市场源</div>
          <div className="text-ui-sm text-foreground-subtlest">应用不内置任何市场源；支持 https:// 仓库地址或 owner/repo 缩写</div>
        </div>
        <button
          className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
          disabled={refreshing}
          onClick={onRefresh}
        >
          <RefreshCw className="size-3.5 shrink-0" />
          {refreshing ? <span className="animated-gradient-text">刷新中…</span> : "立即刷新"}
        </button>
      </div>

      {markets.length > 0 && (
        <div className="flex flex-col gap-1">
          {markets.map((m) => (
            <div key={m.id} className="flex items-center gap-2.5 rounded-lg bg-surface px-2.5 py-1.5">
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-baseline gap-2">
                  <span className="truncate text-ui-sm text-foreground">{m.id}</span>
                  <span className="min-w-0 truncate font-mono text-ui-xs text-foreground-subtlest" title={m.repoUrl}>
                    {m.repoUrl}
                  </span>
                </div>
                {m.lastRefreshFailure ? (
                  // 刷新失败红字呈现（不阻塞其他市场；title 展示完整原因）
                  <div className="truncate text-ui-xs text-destructive" title={m.lastRefreshFailure}>
                    上次刷新失败：{m.lastRefreshFailure}
                  </div>
                ) : m.lastRefreshedAt > 0 ? (
                  <div className="text-ui-xs text-foreground-subtlest">上次刷新 {fmtTime(m.lastRefreshedAt)}</div>
                ) : (
                  <div className="text-ui-xs text-foreground-subtlest">从未刷新成功</div>
                )}
              </div>
              <Popconfirm
                message={`移除市场源「${m.id}」？该市场下仍有已安装插件时会被拒绝（需先卸载插件）。`}
                onConfirm={() => onRemove(m.id)}
              >
                <button
                  type="button"
                  title="移除市场源"
                  className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-destructive"
                >
                  <Trash2 className="size-4" />
                </button>
              </Popconfirm>
            </div>
          ))}
        </div>
      )}

      <div className="flex items-center gap-2">
        <input
          value={input}
          disabled={adding}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            // IME 组合中的 Enter（候选确认）不触发提交
            if (isImeComposing(e)) return;
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            }
          }}
          placeholder="https://github.com/owner/repo 或 owner/repo"
          aria-label="添加市场源"
          className="h-8 min-w-0 flex-1 rounded-lg border border-border bg-input px-2.5 font-mono text-ui-xs"
        />
        <button
          className="flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-brand px-3 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
          disabled={adding || input.trim() === ""}
          onClick={submit}
        >
          <Plus className="size-3.5" />
          {adding ? <span className="animated-gradient-text">添加中…</span> : "添加市场源"}
        </button>
      </div>
    </div>
  );
}

/** 推荐市场源常驻区块（spec 第 9 节）：候选卡固定展示（不随空状态消失），形态徽标
 *  （市场 / 单插件 / 技能集，formLabel 已带中文文案）+ 一句话描述；已添加呈现「已添加」态。 */
function StoreRecommendedCard({
  recommended,
  markets,
}: {
  recommended: RecommendedMarketView[];
  markets: MarketView[];
}): React.JSX.Element {
  const addMarket = useApp((s) => s.addStoreMarket);
  // 正在添加的推荐源（owner/repo）；非空时禁用全部添加按钮（防并发重复 clone）
  const [addingRepo, setAddingRepo] = useState("");

  // 已添加判定：展开 URL 含 owner/repo 段，或用户输入原文即为缩写形态
  const isAdded = (rec: RecommendedMarketView): boolean =>
    markets.some((m) => m.repoUrl.includes(rec.repo) || m.source === rec.repo);

  const add = (rec: RecommendedMarketView): void => {
    if (addingRepo !== "") return;
    setAddingRepo(rec.repo);
    void addMarket(rec.repo).finally(() => setAddingRepo(""));
  };

  if (recommended.length === 0) return <></>;

  return (
    <div className="flex flex-col gap-2 rounded-xl border border-border/50 bg-panel px-4 py-3">
      <div className="min-w-0">
        <div className="text-ui-base text-foreground">推荐市场源</div>
        <div className="text-ui-sm text-foreground-subtlest">内置候选清单仅供参考；点击添加才会拉取仓库，重复添加同名市场会被拒绝</div>
      </div>
      <div className="grid grid-cols-1 gap-2 xl:grid-cols-2">
        {recommended.map((rec) => {
          const added = isAdded(rec);
          const busy = addingRepo === rec.repo;
          return (
            <div key={rec.repo} className="flex items-center gap-2.5 rounded-lg bg-surface px-2.5 py-2">
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-ui-sm text-foreground">{rec.displayName}</span>
                  <span className="shrink-0 rounded border border-border px-1 text-[10px] leading-4 text-foreground-subtlest">
                    {rec.formLabel}
                  </span>
                </div>
                <div className="truncate text-ui-xs text-foreground-subtle" title={rec.description}>
                  {rec.description}
                </div>
                <div className="truncate font-mono text-ui-xs text-foreground-subtlest">{rec.repo}</div>
              </div>
              {added ? (
                <span className="shrink-0 rounded bg-selected px-2 py-1 text-ui-xs text-foreground-subtle">已添加</span>
              ) : (
                <button
                  type="button"
                  className="flex h-7 shrink-0 items-center gap-1 rounded-lg border border-border px-2.5 text-ui-xs hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
                  disabled={addingRepo !== ""}
                  onClick={() => add(rec)}
                >
                  <Plus className="size-3" />
                  {busy ? <span className="animated-gradient-text">添加中…</span> : "添加"}
                </button>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** 插件区单市场分组（spec 第 10 节第 3 条）：市场标题 + 插件卡片列表。 */
function StoreMarketGroup({
  marketplaceId,
  items,
  preparingInstall,
  onOpen,
  onInstall,
}: {
  marketplaceId: string;
  items: StorePluginView[];
  preparingInstall: string;
  onOpen(plugin: StorePluginView): void;
  onInstall(plugin: StorePluginView): void;
}): React.JSX.Element {
  const installedCount = items.filter((p) => p.installed).length;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline gap-2 px-1 pt-1">
        <span className="text-ui-sm font-medium text-foreground">{marketplaceId}</span>
        <span className="text-ui-xs text-foreground-subtlest">
          {items.length} 个插件 · 已安装 {installedCount}
        </span>
      </div>
      {items.map((p) => (
        <StorePluginCard
          key={p.id}
          plugin={p}
          preparing={preparingInstall === p.id}
          prepareDisabled={preparingInstall !== ""}
          onOpen={() => onOpen(p)}
          onInstall={() => onInstall(p)}
        />
      ))}
    </div>
  );
}

/** 更新角标（spec 第 4 节双轴检测三态）：none 不展示、不误报。 */
function updateBadgeOf(plugin: StorePluginView): { label: string; title: string } | null {
  if (!plugin.installed) return null;
  if (plugin.updateStatus.state === "update-available") {
    return {
      label: `可更新${plugin.updateStatus.availableVersion ? ` · v${plugin.updateStatus.availableVersion}` : ""}`,
      title: "市场侧已有更高版本，打开详情可更新",
    };
  }
  if (plugin.updateStatus.state === "version-changed") {
    return { label: "内容已更新", title: "同版本内容已在市场侧变更（来源 pin 变化），打开详情可更新" };
  }
  return null;
}

/** 技能是否 commands 映射条目（spec 第 5 节）：显式 origin 优先，路径含 commands 段兜底。 */
function isCommandSkill(skill: { origin?: "skill" | "command"; path: string }): boolean {
  return skill.origin === "command" || skill.path.includes("/commands/");
}

/** 来源类型中文标注（sourceKind：entry-relative = 市场仓库内条目，repo = 独立仓库二次克隆）。 */
function sourceKindLabel(kind: string): string {
  if (kind === "entry-relative") return "市场仓库内";
  if (kind === "repo") return "独立仓库";
  return kind || "未知";
}

/** 插件卡片：名称 + 版本 + 描述一行截断 + 已安装徽标 + 更新角标 +「安装」按钮 / 详情入口。
 *  主体区域点击开详情，右侧安装按钮独立（避免按钮嵌套）。 */
function StorePluginCard({
  plugin,
  preparing,
  prepareDisabled,
  onOpen,
  onInstall,
}: {
  plugin: StorePluginView;
  /** 本卡「安装」正在准备确认信息（describe 中） */
  preparing: boolean;
  /** 其他卡正在准备时禁用本卡安装按钮（防并发确认） */
  prepareDisabled: boolean;
  onOpen(): void;
  onInstall(): void;
}): React.JSX.Element {
  const badge = updateBadgeOf(plugin);
  return (
    <div
      className={`flex items-center gap-3 rounded-xl border border-border/50 bg-panel px-4 py-3 ${
        plugin.installed && plugin.enabled === false ? "opacity-60" : ""
      }`}
    >
      <button type="button" className="min-w-0 flex-1 cursor-pointer rounded-lg text-left outline-none" onClick={onOpen} title="查看插件详情">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-ui-base text-foreground">{plugin.name}</span>
          {plugin.version && (
            <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">v{plugin.version}</span>
          )}
          {plugin.installed && (
            <span className="shrink-0 rounded bg-success-surface px-1.5 py-0.5 text-ui-xs text-success-surface-foreground">已安装</span>
          )}
          {badge && (
            <span title={badge.title} className="shrink-0 rounded bg-warning-surface px-1.5 py-0.5 text-ui-xs text-warning-surface-foreground">
              {badge.label}
            </span>
          )}
        </div>
        {plugin.description && <div className="truncate text-ui-sm text-foreground-subtle">{plugin.description}</div>}
        <div className="truncate text-ui-xs text-foreground-subtlest">{plugin.marketplaceId}</div>
      </button>
      <div className="flex shrink-0 items-center gap-1.5">
        {!plugin.installed && (
          <button
            type="button"
            className="flex h-7 items-center gap-1 rounded-lg bg-brand px-2.5 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
            disabled={prepareDisabled}
            onClick={onInstall}
          >
            <Download className="size-3.5" />
            {preparing ? <span className="animated-gradient-text">准备中…</span> : "安装"}
          </button>
        )}
        <ChevronRight className="size-4 shrink-0 text-foreground-subtlest" />
      </div>
    </div>
  );
}

/** 插件详情 Dialog（spec 第 10 节第 4 条）：描述、已装版本 / 可用新版本、版本历史、
 *  技能清单（command 来源标「命令」徽标）、组件标注（hooks / agents / MCP 解析后仅供标注、
 *  当前版本不加载）与操作（安装 / 更新 / 回滚上一版 / 插件启停 / 卸载）。
 *  按需 describeStorePlugin 获取（不常驻）；操作成功后重新 describe 局部刷新；
 *  变更一律下一轮对话生效（底部固定注明）。 */
function StorePluginDetailDialog({
  pluginId,
  fallback,
  onClose,
}: {
  pluginId: string;
  /** describe 完成前的兜底展示（商店列表行数据，含更新角标状态） */
  fallback: StorePluginView | null;
  onClose(): void;
}): React.JSX.Element {
  const installStorePlugin = useApp((s) => s.installStorePlugin);
  const updateStorePlugin = useApp((s) => s.updateStorePlugin);
  const rollbackStorePlugin = useApp((s) => s.rollbackStorePlugin);
  const uninstallStorePlugin = useApp((s) => s.uninstallStorePlugin);
  const setStorePluginEnabled = useApp((s) => s.setStorePluginEnabled);
  const describeStorePlugin = useApp((s) => s.describeStorePlugin);

  const [detail, setDetail] = useState<StorePluginDetail | null>(null);
  const [loading, setLoading] = useState(true);
  // 进行中的操作键（install / update / rollback / uninstall / toggle）：按钮 loading 与防重复点击
  const [busy, setBusy] = useState("");

  const reload = (): void => {
    void describeStorePlugin(pluginId).then((d) => {
      setDetail(d);
      setLoading(false);
    });
  };

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId]);

  const runAction = (key: string, action: () => Promise<boolean>): void => {
    if (busy !== "") return;
    setBusy(key);
    void action()
      .then((ok) => {
        // 成功后局部刷新：store 槽已由动作整体刷新，这里重拉详情（版本 / 启停态随指针变化）
        if (ok) reload();
      })
      .finally(() => setBusy(""));
  };

  const installed = detail?.installed ?? fallback?.installed ?? false;
  const updateStatus = fallback?.updateStatus;
  const canUpdate = installed && (updateStatus?.state ?? "none") !== "none";
  const updateLabel =
    updateStatus?.state === "update-available" && updateStatus.availableVersion
      ? `更新到 v${updateStatus.availableVersion}`
      : updateStatus?.state === "version-changed"
        ? "更新到新内容"
        : "更新";
  const canRollback = installed && (detail?.versions.length ?? 0) > 1;

  // 组件标注（解析仅供详情页标注，当前版本不加载；含 hooks 时附注自动激活不可用）
  const annotations: string[] = [];
  if (detail?.hasHooks || fallback?.hasHooks) annotations.push("包含 hooks 配置：自动激活不可用，技能按需或经斜杠调用激活");
  if (detail?.hasAgents || fallback?.hasAgents) annotations.push("包含代理（agents）配置，当前版本不加载");
  if (detail?.hasMcp || fallback?.hasMcp) annotations.push("包含 MCP 配置，当前版本不加载");

  return (
    <Dialog.Root open onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[70] bg-black/40" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[78] flex max-h-[76vh] w-[min(680px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-xl border border-border bg-background p-4 shadow-lg">
          <Dialog.Title className="flex min-w-0 items-center gap-2 text-ui-base font-semibold text-foreground">
            <span className="truncate">{detail?.name ?? fallback?.name ?? pluginId}</span>
            <span className="shrink-0 rounded bg-surface px-1.5 py-0.5 font-mono text-ui-xs text-foreground-subtlest">
              {detail?.marketplaceId ?? fallback?.marketplaceId ?? ""}
            </span>
          </Dialog.Title>

          {loading ? (
            <div className="py-10 text-center text-ui-sm">
              <span className="animated-gradient-text">载入详情中…</span>
            </div>
          ) : !detail ? (
            <div className="py-10 text-center text-ui-sm text-foreground-subtle">详情读取失败，可关闭后重试</div>
          ) : (
            <>
              <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pr-1">
                <p className="text-ui-sm text-foreground-subtle">{detail.description || "暂无描述"}</p>

                {annotations.length > 0 && (
                  <div className="flex flex-col gap-1 rounded-lg bg-warning-surface px-2.5 py-1.5 text-ui-xs text-warning-surface-foreground">
                    {annotations.map((a) => (
                      <p key={a}>{a}</p>
                    ))}
                  </div>
                )}

                {/* 元信息：来源 / 版本 / 来源 pin / 版本历史 */}
                <div className="flex flex-col gap-1.5 rounded-lg bg-surface px-2.5 py-2 text-ui-xs">
                  <div className="flex min-w-0 items-baseline gap-2">
                    <span className="w-16 shrink-0 text-foreground-subtlest">来源</span>
                    <span className="min-w-0 truncate font-mono text-foreground" title={detail.sourceRef}>
                      {sourceKindLabel(detail.sourceKind)} · {detail.sourceRef || "未知"}
                    </span>
                  </div>
                  <div className="flex min-w-0 items-baseline gap-2">
                    <span className="w-16 shrink-0 text-foreground-subtlest">版本</span>
                    <span className="font-mono text-foreground">
                      {installed ? `已装 v${detail.currentVersion}` : `市场版本${fallback?.version ? ` v${fallback.version}` : "未知"}`}
                    </span>
                    {updateStatus?.state === "update-available" && (
                      <span className="text-warning-surface-foreground">
                        可更新到 v{updateStatus.availableVersion ?? "?"}
                      </span>
                    )}
                    {updateStatus?.state === "version-changed" && (
                      <span className="text-warning-surface-foreground">同版本内容已在市场侧变更</span>
                    )}
                  </div>
                  {detail.sourceSha && (
                    <div className="flex min-w-0 items-baseline gap-2">
                      <span className="w-16 shrink-0 text-foreground-subtlest">来源 pin</span>
                      <span className="font-mono text-foreground">{detail.sourceSha}</span>
                    </div>
                  )}
                  {detail.versions.length > 0 && (
                    <div className="flex min-w-0 items-baseline gap-2">
                      <span className="w-16 shrink-0 text-foreground-subtlest">历史版本</span>
                      <div className="flex min-w-0 flex-wrap gap-1">
                        {detail.versions.map((v) => (
                          <span
                            key={v}
                            className={`rounded px-1.5 py-0.5 font-mono ${
                              v === detail.currentVersion ? "bg-selected text-foreground" : "bg-background text-foreground-subtlest"
                            }`}
                          >
                            {v}
                            {v === detail.currentVersion ? "（当前）" : ""}
                          </span>
                        ))}
                      </div>
                    </div>
                  )}
                </div>

                {/* 技能清单（command 映射来源标「命令」徽标） */}
                <div className="flex flex-col gap-1.5">
                  <span className="text-ui-sm font-medium text-foreground">技能清单（{detail.skills.length}）</span>
                  {detail.skills.length === 0 ? (
                    <div className="rounded-lg border border-dashed border-border px-3 py-3 text-center text-ui-xs text-foreground-subtlest">
                      该插件未声明技能
                    </div>
                  ) : (
                    <div className="flex flex-col gap-1">
                      {detail.skills.map((s) => (
                        <div key={s.path} className="flex items-center gap-2 rounded-lg bg-surface px-2.5 py-1.5">
                          <span className="truncate text-ui-sm text-foreground">{s.name}</span>
                          {isCommandSkill(s) && (
                            <span className="shrink-0 rounded border border-border px-1 text-[10px] leading-4 text-foreground-subtlest">命令</span>
                          )}
                          <span className="ml-auto min-w-0 truncate text-ui-xs text-foreground-subtlest" title={s.description}>
                            {s.description}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>

              {/* 底部操作条（不随内容滚动）：操作 + 插件启停 + 生效时机固定注明 */}
              <div className="flex shrink-0 flex-col gap-2 border-t border-border/40 pt-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="flex min-w-0 items-center gap-2">
                    {!installed ? (
                      <Popconfirm
                        message={`将从市场「${detail.marketplaceId}」安装「${detail.name}」（${detail.skills.length} 个技能，来源 ${detail.sourceRef || detail.marketplaceId}）。`}
                        confirmLabel="安装"
                        onConfirm={() => runAction("install", () => installStorePlugin(detail.id))}
                      >
                        <button
                          type="button"
                          className="flex h-8 items-center gap-1.5 rounded-lg bg-brand px-3 text-ui-sm text-foreground-inverse disabled:cursor-not-allowed disabled:opacity-40"
                          disabled={busy !== ""}
                        >
                          <Download className="size-3.5" />
                          {busy === "install" ? <span className="animated-gradient-text">安装中…</span> : "安装"}
                        </button>
                      </Popconfirm>
                    ) : (
                      <>
                        <button
                          type="button"
                          className="flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
                          disabled={busy !== "" || !canUpdate}
                          title={canUpdate ? undefined : "暂无可用更新（距上次刷新的清单对比无变化）"}
                          onClick={() => runAction("update", () => updateStorePlugin(detail.id))}
                        >
                          <RefreshCw className="size-3.5" />
                          {busy === "update" ? <span className="animated-gradient-text">更新中…</span> : updateLabel}
                        </button>
                        {canRollback && (
                          <button
                            type="button"
                            className="flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
                            disabled={busy !== ""}
                            onClick={() => runAction("rollback", () => rollbackStorePlugin(detail.id))}
                          >
                            <Undo2 className="size-3.5" />
                            {busy === "rollback" ? <span className="animated-gradient-text">回滚中…</span> : "回滚上一版"}
                          </button>
                        )}
                        <Popconfirm
                          message={`卸载插件「${detail.name}」？将删除其全部版本（含历史版本与禁用记录），不可恢复。`}
                          onConfirm={() => runAction("uninstall", () => uninstallStorePlugin(detail.id))}
                        >
                          <button
                            type="button"
                            className="flex h-8 items-center gap-1.5 rounded-lg border border-destructive/40 px-3 text-ui-sm text-destructive hover:bg-destructive-surface disabled:cursor-not-allowed disabled:opacity-40"
                            disabled={busy !== ""}
                          >
                            <Trash2 className="size-3.5" />
                            {busy === "uninstall" ? <span className="animated-gradient-text">卸载中…</span> : "卸载"}
                          </button>
                        </Popconfirm>
                      </>
                    )}
                  </div>
                  {installed && (
                    <label className="flex shrink-0 items-center gap-1.5">
                      <span className="text-ui-xs text-foreground-subtle">{detail.enabled === false ? "已停用" : "已启用"}</span>
                      <Switch
                        checked={detail.enabled !== false}
                        onChange={(v) => runAction("toggle", () => setStorePluginEnabled(detail.id, v))}
                      />
                    </label>
                  )}
                </div>
                <p className="text-ui-xs text-foreground-subtlest">
                  技能每轮对话前重新扫描：安装、升级、回滚、启停、卸载均自下一轮对话起生效。
                </p>
              </div>
            </>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
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
  onToggle(skill: SkillView, enabled: boolean): void;
  onRemove(skill: SkillView): void;
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
                {skill.source === "agents" ? "agents" : skill.source === "marketplace" ? "marketplace" : "craftwork"}
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
            <Popconfirm message={`删除技能「${skill.name}」？将同时删除其目录内全部文件，不可恢复。`} onConfirm={() => onRemove(skill)}>
              <button
                type="button"
                title="删除技能"
                className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-destructive"
              >
                <Trash2 className="size-4" />
              </button>
            </Popconfirm>
            <Switch checked={skill.enabled} onChange={(v) => onToggle(skill, v)} />
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

/** 个性化分节：编辑 <数据目录>/AGENTS.md（全局私人指令，每轮 run 注入系统提示词，见 craft-agent::envcontext），
 *  以及两项本机 Agent 行为偏好（输入预测 / 最大工具轮数，原在「通用」分节）。
 *  载入后本地编辑，保存覆盖写（空内容删除文件）；100KB 硬上限——计数超限禁用保存。 */
function PersonalizationSection(): React.JSX.Element {
  const pushToast = useToastsPush();
  const settings = useApp((s) => s.settings);
  const saveSettings = useApp((s) => s.saveSettings);
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
      <h3 className="text-ui-base font-medium text-foreground">自定义指令</h3>
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

      <h3 className="pt-2 text-ui-base font-medium text-foreground">行为偏好</h3>
      <Row title="输入预测" desc="任务结束后静置 5 秒，在输入框给出下一句建议，Tab 应用">
        <Switch checked={settings.inputPrediction} onChange={(v) => void saveSettings({ inputPrediction: v })} />
      </Row>
      <Row title="最大工具轮数" desc="单次运行的模型调用上限，默认 0 表示不限制；模型持续调用工具时不会被迫中断">
        <input
          type="number"
          min={0}
          max={1000}
          value={settings.maxIterations}
          onChange={(e) =>
            void saveSettings({ maxIterations: Math.max(0, Math.min(1000, Number(e.target.value) || 0)) })
          }
          className="h-8 w-20 rounded-lg border border-border bg-input px-2 text-ui-sm"
        />
      </Row>
    </div>
  );
}

// ---------- 数据与统计（用量 / 审计 / 备份 三个独立分区） ----------

const fmtCost = (micros: number): string => `$${(micros / 1_000_000).toFixed(4)}`;
const fmtTokens = (n: number): string => (n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}K` : String(n));

/** 会话类型的展示名（按会话类型的用量聚合用）：后端归一 main，未知值原样展示。 */
const agentTypeLabel = (agentType: string): string =>
  agentType === 'main' ? '主会话' : agentType === 'general' ? '通用子代理' : agentType === 'explore' ? '探查子代理' : agentType;
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
          <div className="rounded-xl border border-border/50 bg-panel">
            <div className="border-b border-border/40 px-4 py-2 text-ui-sm font-medium">按会话类型（近 30 天）</div>
            {usage.byAgentType.length === 0 ? (
              <div className="px-4 py-6 text-center text-ui-sm text-foreground-subtlest">暂无用量</div>
            ) : (
              usage.byAgentType.map((a) => (
                <div key={a.agentType} className="flex items-center gap-2 border-b border-border/20 px-4 py-2 text-ui-sm last:border-0">
                  <span className="truncate">{agentTypeLabel(a.agentType)}</span>
                  <span className="ml-auto shrink-0 text-foreground-subtlest">{fmtTokens(a.inputTokens + a.outputTokens)}</span>
                  <span className="w-20 shrink-0 text-right text-foreground-subtlest">{fmtCost(a.costMicros)}</span>
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

/** 存储分节（决策 D27）：数据库 / 数据目录各类 / 会话占用三块，可清理项带按钮。
 *
 *  三个数据源各自按需拉：概览进分节即拉（首屏）；会话占用点开「会话」卡才拉
 *  （千级会话 + 大库时最贵的一次查询，AGENTS.md §7.1 "Fetch large objects on demand"）；
 *  表级明细点开才拉（dbstat 要整库走页表）。全部不轮询、不缓存——体积数据一 stale
 *  就会误导清理决定，用户看到的必须是这一次点击的真实快照。
 *
 *  门禁口径（`overview.busyReason`）由服务端计算：非空即禁用全部清理按钮并显示原因。
 *  前端禁用只是体验层，服务端在 `storageCleanup` 里还会再判一次。 */
function StorageSection(): React.JSX.Element {
  const [overview, setOverview] = useState<StorageOverview | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [busy, setBusy] = useState(false);
  const [tables, setTables] = useState<StorageTableRow[] | null>(null);
  const [tablesOpen, setTablesOpen] = useState(false);
  const [threads, setThreads] = useState<StorageThreadsView | null>(null);
  const [threadsOpen, setThreadsOpen] = useState(false);
  const pushToast = useToastsPush();

  useEffect(() => {
    void api
      .storageOverview()
      .then(setOverview)
      .catch((err) => pushToast("danger", "读取存储占用失败", cleanErrorMessage(err)));
  }, [refreshKey, pushToast]);

  // 表级明细与会话占用都只在展开时拉一次；刷新后清空，重新展开再拉新值
  useEffect(() => {
    if (!tablesOpen || tables) return;
    void api
      .storageDbTables()
      .then(setTables)
      .catch(() => setTables([]));
  }, [tablesOpen, tables, refreshKey]);

  useEffect(() => {
    if (!threadsOpen || threads) return;
    void api
      .storageThreads()
      .then(setThreads)
      .catch((err) => pushToast("danger", "读取会话占用失败", cleanErrorMessage(err)));
  }, [threadsOpen, threads, refreshKey]);

  const refresh = () => {
    setTables(null);
    setThreads(null);
    setRefreshKey((k) => k + 1);
  };

  /** 执行清理：成功报释放量并重取全部体积；服务端拒绝（门禁）按错误提示。 */
  const runCleanup = (kind: string) => {
    setBusy(true);
    void api
      .storageCleanup(kind)
      .then((result: StorageCleanupResult) => {
        const parts = [`已释放 ${formatBytes(result.freedBytes)}`];
        if (result.removedItems > 0) parts.push(`删除 ${result.removedItems} 项`);
        if (result.skippedItems > 0) parts.push(`跳过 ${result.skippedItems} 项`);
        if (result.failures.length > 0) parts.push(`${result.failures.length} 项失败`);
        pushToast(result.failures.length > 0 ? "warn" : "ok", "清理完成", parts.join(" · "));
        refresh();
      })
      .catch((err) => pushToast("danger", "清理失败", cleanErrorMessage(err)))
      .finally(() => setBusy(false));
  };

  const blocked = Boolean(overview?.busyReason) || busy;

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-ui-settings-title font-semibold text-foreground">存储</h2>
        <RefreshButton onClick={refresh} />
      </div>
      <p className="px-1 text-ui-sm text-foreground-subtle">
        分「数据库内」与「数据目录文件」两部分统计，不合并成单数。清理只动可丢弃数据（备份、日志、后台输出、孤儿工作树与计划文件），不会删除任何会话内容。
      </p>

      {overview?.busyReason && (
        <div className="rounded-xl border border-brand/40 bg-brand/10 px-4 py-2.5 text-ui-sm text-foreground">
          {overview.busyReason}。清理与压缩在任务结束后才能进行。
        </div>
      )}

      {/* ---------- 数据库 ---------- */}
      <div className="rounded-xl border border-border/50 bg-panel">
        <div className="flex items-start justify-between gap-4 border-b border-border/40 px-4 py-3">
          <div className="min-w-0">
            <div className="text-ui-base text-foreground">数据库</div>
            <div className="text-ui-sm text-foreground-subtlest">
              {overview
                ? `${formatBytes(overview.database.mainBytes)} 主库 · ${formatBytes(overview.database.walBytes)} WAL · 事件载荷 ${formatBytes(overview.database.eventBytes)}`
                : "正在统计…"}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <span className="font-mono text-ui-lg text-foreground">{overview ? formatBytes(overview.database.bytes) : "…"}</span>
            <Popconfirm
              message="压缩数据库会把已删除会话腾出的页真正还回磁盘。期间可能耗时数十秒、临时占用约一倍磁盘空间，且写入会短暂阻塞。"
              confirmLabel="压缩"
              onConfirm={() => runCleanup("vacuum")}
            >
              <button
                className="h-8 rounded-lg border border-border px-3 text-ui-sm hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
                disabled={blocked}
              >
                压缩数据库
              </button>
            </Popconfirm>
          </div>
        </div>
        <button
          className="flex w-full items-center gap-1.5 px-4 py-2 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
          onClick={() => setTablesOpen((v) => !v)}
        >
          {tablesOpen ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
          按表明细
        </button>
        {tablesOpen && (
          <div className="border-t border-border/30">
            {tables === null ? (
              <div className="px-4 py-3 text-ui-sm text-foreground-subtlest">正在统计…</div>
            ) : tables.length === 0 ? (
              <div className="px-4 py-3 text-ui-sm text-foreground-subtlest">当前构建不支持表级明细</div>
            ) : (
              tables.map((t) => (
                <div key={t.name} className="flex items-center gap-2 border-b border-border/20 px-4 py-1.5 text-ui-sm last:border-0">
                  <span className="truncate font-mono">{t.name}</span>
                  <span className="ml-auto shrink-0 text-foreground-subtlest">{formatBytes(t.bytes)}</span>
                </div>
              ))
            )}
          </div>
        )}
      </div>

      {/* ---------- 数据目录 ---------- */}
      <div className="rounded-xl border border-border/50 bg-panel">
        <div className="flex items-center justify-between gap-4 border-b border-border/40 px-4 py-3">
          <div className="min-w-0">
            <div className="text-ui-base text-foreground">数据目录（除数据库）</div>
            <div className="truncate font-mono text-ui-xs text-foreground-subtlest">{overview?.dataDir ?? "…"}</div>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <span className="font-mono text-ui-lg text-foreground">
              {overview ? formatBytes(overview.filesTotalBytes) : "…"}
            </span>
            <button
              type="button"
              title="在文件管理器中打开数据目录"
              aria-label="在文件管理器中打开数据目录"
              disabled={!overview}
              onClick={() => {
                void api.openDataDir().catch((err) => pushToast("danger", "打开数据目录失败", cleanErrorMessage(err)));
              }}
              className="flex size-8 items-center justify-center rounded-lg border border-border text-foreground-subtle hover:bg-surface-hover hover:text-foreground disabled:opacity-50"
            >
              <FolderOpen className="size-4" />
            </button>
          </div>
        </div>
        {overview?.categories.map((c) => (
          <div key={c.kind} className="flex items-start gap-3 border-b border-border/20 px-4 py-2.5 text-ui-sm last:border-0">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-foreground">{c.label}</span>
                <span className="text-ui-xs text-foreground-subtlest">
                  {c.truncated ? "≥" : ""}
                  {formatBytes(c.bytes)} · {c.files} 个文件
                </span>
              </div>
              <div className="text-ui-xs text-foreground-subtlest">{c.description}</div>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {c.cleanups.length === 0 ? (
                <span className="text-ui-xs text-foreground-subtlest">不可清理</span>
              ) : (
                c.cleanups.map((action) =>
                  action.destructive ? (
                    <CleanupDialogButton
                      key={action.kind}
                      label={action.label}
                      message={action.confirm}
                      disabled={blocked}
                      onConfirm={() => runCleanup(action.kind)}
                    />
                  ) : (
                    <Popconfirm key={action.kind} message={action.confirm} confirmLabel={action.label} onConfirm={() => runCleanup(action.kind)}>
                      <button
                        className="h-7 rounded-lg border border-border px-2.5 text-ui-xs hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
                        disabled={blocked}
                      >
                        {action.label}
                      </button>
                    </Popconfirm>
                  ),
                )
              )}
            </div>
          </div>
        ))}
      </div>

      {/* ---------- 会话占用 ---------- */}
      <div className="rounded-xl border border-border/50 bg-panel">
        <button
          className="flex w-full items-center gap-1.5 px-4 py-3 text-left"
          onClick={() => setThreadsOpen((v) => !v)}
        >
          {threadsOpen ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
          <span className="text-ui-base text-foreground">各项目 / 各会话</span>
          <span className="ml-auto text-ui-xs text-foreground-subtlest">
            {threads ? `${threads.threadTotal} 个会话` : "点开统计"}
          </span>
        </button>
        {threadsOpen && (
          <div className="border-t border-border/40">
            {threads === null ? (
              <div className="px-4 py-6 text-center text-ui-sm text-foreground-subtlest">正在统计会话占用…</div>
            ) : threads.projects.length === 0 && threads.orphanThreads.length === 0 ? (
              <div className="px-4 py-6 text-center text-ui-sm text-foreground-subtlest">暂无会话</div>
            ) : (
              <>
                {threads.projects.map((p) => (
                  <ProjectStorageGroup key={p.workspaceId} group={p} />
                ))}
                {threads.orphanThreads.length > 0 && (
                  <div>
                    <div className="flex items-center gap-2 bg-surface px-4 py-1.5 text-ui-xs text-foreground-subtlest">
                      <span>未归属项目（工作区已删除）</span>
                      <span className="ml-auto font-mono">{threads.orphanThreads.length} 个会话</span>
                    </div>
                    {threads.orphanThreads.map((t) => (
                      <StorageThreadLine key={t.threadId} thread={t} />
                    ))}
                  </div>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

/** 项目分组：组头给项目名与合计，组内按体积降序列出会话（默认前 10 条）。 */
function ProjectStorageGroup({ group }: { group: StorageProjectGroup }): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const limit = 10;
  const visible = expanded ? group.threads : group.threads.slice(0, limit);
  const hiddenByLimit = group.threads.length - visible.length;
  return (
    <div className="border-b border-border/20 last:border-0">
      <div className="flex items-center gap-2 bg-surface px-4 py-1.5 text-ui-xs text-foreground-subtlest">
        <span className="truncate text-foreground">{group.workspaceName}</span>
        {group.isConversation && <span className="shrink-0 rounded bg-border/40 px-1.5 py-0.5">临时</span>}
        <span className="ml-auto shrink-0 font-mono">
          数据库 {formatBytes(group.dbBytes)} · 文件 {formatBytes(group.fileBytes)}
        </span>
      </div>
      {visible.map((t) => (
        <StorageThreadLine key={t.threadId} thread={t} />
      ))}
      {(hiddenByLimit > 0 || group.omittedThreads > 0) && (
        <button className="w-full px-4 py-1.5 text-left text-ui-xs text-foreground-subtlest hover:bg-surface-hover" onClick={() => setExpanded(true)}>
          展开其余{" "}
          {[hiddenByLimit > 0 ? `${hiddenByLimit} 个` : "", group.omittedThreads > 0 ? `（另有 ${group.omittedThreads} 个更小的未列出）` : ""]
            .filter(Boolean)
            .join("")}
        </button>
      )}
    </div>
  );
}

/** 单条会话：标题 + 数据库/文件两列；下方分解日志、后台输出、工作树、计划四项。 */
function StorageThreadLine({ thread }: { thread: StorageThreadRow }): React.JSX.Element {
  const parts: string[] = [];
  if (thread.logBytes > 0) parts.push(`日志 ${formatBytes(thread.logBytes)}`);
  if (thread.shellTaskBytes > 0) parts.push(`后台输出 ${formatBytes(thread.shellTaskBytes)}`);
  if (thread.worktreeBytes > 0) parts.push(`工作树 ${formatBytes(thread.worktreeBytes)}`);
  if (thread.planBytes > 0) parts.push(`计划 ${formatBytes(thread.planBytes)}`);
  return (
    <div className="flex items-start gap-2 border-t border-border/20 px-4 py-1.5 text-ui-sm first:border-0">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-foreground">{thread.title || "（无标题）"}</span>
          {thread.isSubagent && <span className="shrink-0 rounded bg-border/40 px-1 text-ui-xs text-foreground-subtle">子代理</span>}
          {thread.archived && <span className="shrink-0 rounded bg-border/40 px-1 text-ui-xs text-foreground-subtle">已归档</span>}
        </div>
        <div className="truncate text-ui-xs text-foreground-subtlest">
          {thread.events} 条事件 · {thread.runs} 次运行
          {parts.length > 0 ? ` · ${parts.join(" · ")}` : ""}
        </div>
      </div>
      <div className="shrink-0 text-right font-mono text-ui-xs text-foreground-subtle">
        <div>{formatBytes(thread.dbBytes)}</div>
        <div className="text-foreground-subtlest">{formatBytes(thread.fileBytes)}</div>
      </div>
    </div>
  );
}

/** 影响面大的清理动作：危险确认对话框（区别于就近气泡），聚焦取消而不是确认。 */
function CleanupDialogButton({
  label,
  message,
  disabled,
  onConfirm,
}: {
  label: string;
  message: string;
  disabled: boolean;
  onConfirm(): void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Trigger asChild>
        <button
          className="h-7 rounded-lg border border-border px-2.5 text-ui-xs hover:bg-surface-hover disabled:cursor-not-allowed disabled:opacity-40"
          disabled={disabled}
        >
          {label}
        </button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[80] bg-black/40" />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-1/2 z-[81] w-96 -translate-x-1/2 -translate-y-1/2 rounded-xl border border-border bg-background p-5 shadow-2xl outline-none"
        >
          <Dialog.Title className="text-ui-base font-semibold text-foreground">确认{label}？</Dialog.Title>
          <p className="pt-2 text-ui-sm text-foreground-subtle">{message}</p>
          <div className="flex justify-end gap-2 pt-4">
            <Dialog.Close asChild>
              <button className="h-8 rounded-lg border border-border px-4 text-ui-sm text-foreground-subtle hover:bg-surface-hover">
                取消
              </button>
            </Dialog.Close>
            <button
              className="h-8 rounded-lg bg-destructive px-4 text-ui-sm text-white hover:opacity-90"
              onClick={() => {
                setOpen(false);
                onConfirm();
              }}
            >
              确认清空
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** 关于分节：应用身份（logo / 名称）+ 构建元信息（版本、Git 提交、编译时间）。
 *
 *  数据一次取自 getInfo（切换分节即重新挂载，天然刷新）：版本与 Git 哈希、编译时间
 *  都是构建期固化值。**库体积与数据目录不在这里**——它们已移到「数据与统计 → 存储」
 *  分节（D27）：关于页只回答「这是什么应用」，占多少、能清什么归存储页。
 *  logo 直接引用打包用的应用图标（src-tauri/icons），不复制第二份，图标更新后此处自动同步。 */
function AboutSection(): React.JSX.Element {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const pushToast = useToastsPush();

  useEffect(() => {
    void api.getInfo().then(setInfo).catch((err) => pushToast("danger", "读取应用信息失败", cleanErrorMessage(err)));
  }, [pushToast]);

  const version = info ? info.version : "…";
  const gitHash = info?.gitHash && info.gitHash !== "unknown" ? info.gitHash : "";

  return (
    <div className="flex flex-col gap-3">
      <h2 className="text-ui-settings-title font-semibold text-foreground">关于</h2>

      {/* 身份卡：logo + 名称 + 一句话定位 */}
      <div className="flex items-center gap-4 rounded-xl border border-border/50 bg-panel px-5 py-4">
        <img src={appLogo} alt="" draggable={false} className="size-14 shrink-0 rounded-xl object-contain" />
        <div className="min-w-0">
          <div className="text-ui-lg font-semibold text-foreground">{info?.name ?? "CraftWork"}</div>
          <div className="text-ui-sm text-foreground-subtle">可审计的桌面 Agent</div>
        </div>
      </div>

      <Row title="版本" desc="发版时以 tauri.conf.json 为产物版本事实源">
        <span className="font-mono text-ui-base text-foreground">
          {version}
          {gitHash && <span className="text-foreground-subtle"> · {gitHash}</span>}
        </span>
      </Row>
      <Row title="编译时间" desc="本机构建该版本二进制的时刻（本地时区）">
        <span className="font-mono text-ui-base text-foreground">{info?.buildTime ?? "…"}</span>
      </Row>
      <p className="px-1 text-ui-xs text-foreground-subtlest">
        {info ? `${info.platform} / ${info.arch}` : ""}
        {info ? " · 全部数据保存在本机，不上传任何遥测" : ""}
      </p>
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

/** 就近确认气泡（对齐 ant.design Popconfirm）：锚定触发元素的小型确认浮层，用于低层级删除等操作。
 *  打开即聚焦确认按钮——聚焦态下回车 / 空格直接确认（浏览器原生按钮行为）；
 *  confirmLabel 缺省「删除」（主要场景），商店安装确认等可传入自定义文案。 */
function Popconfirm({
  message,
  onConfirm,
  confirmLabel,
  children,
}: {
  message: string;
  onConfirm(): void;
  /** 确认按钮文案；缺省「删除」（危险配色），自定义文案时按钮改用品牌配色 */
  confirmLabel?: string;
  children: React.ReactNode;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const danger = confirmLabel === undefined || confirmLabel === "删除";
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
              className={`h-7 rounded-md px-2.5 text-ui-xs text-white hover:opacity-90 ${danger ? "bg-destructive" : "bg-brand"}`}
              onClick={() => {
                setOpen(false);
                onConfirm();
              }}
            >
              {confirmLabel ?? "删除"}
            </button>
          </div>
          <Popover.Arrow className="fill-popover" />
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
