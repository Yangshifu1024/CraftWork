// 渲染层唯一状态壳（zustand）：会话列表、消息流、草稿槽、设置与界面偏好。
// 数据流 = 快照装载 + 事件流更新：切会话时经 threadSnapshot 装载消息，此后一切变化
// 由 applyBatch 应用后端推送的事件批次（渲染层唯一事件入口）；系统操作全部经 lib/api（通道表）。
import { create } from "zustand";
import { clampWidth, PANE, SIDEBAR } from "../lib/resize";
import type { Batch, Envelope } from "../../../shared/events";
import type {
  CustomAgentProfile,
  CustomAgentProfileInput,
  MarketView,
  MessageSnapshot,
  PartSnapshot,
  RecommendedMarketView,
  Settings,
  SkillView,
  StorePluginDetail,
  StorePluginView,
  StoreRefreshOutcome,
  SubAgentSettings,
  SubagentPart,
  ThreadView,
  WorkspaceView,
} from "../../../shared/types";
import type { ApprovalPart } from "../../../shared/events";
import { api } from "../lib/api";
import { cleanErrorMessage } from "../lib/errors";
import { applyModeTheme, applyTheme, saveModePref, saveThemePref, watchSystemTheme } from "../lib/theme";
import { emptyDraftTable, loadDrafts, saveDrafts, loadThreadComposer, saveThreadComposer } from "../lib/drafts";
import type { DraftTable } from "../lib/drafts";
import { useToasts } from "./toast";

/** 工作模式：Code（开发者工作台）/ Work（文档处理工作台）。 */
export type Mode = "code" | "work";

/** 设置 → Skills 的双 tab（本机技能 / 技能商店）。选中态放商店槽内：
 *  命令面板「浏览技能商店」直达时先置 market 再开设置页。 */
export type StoreTab = "local" | "market";

/** 商店自动刷新节流窗口：距上次刷新尝试 ≥10 分钟才真正拉取（spec 第 11 节；
 *  失败也计入——避免反复进出商店页反复触网，失败红字由市场清单的 lastRefreshFailure 呈现）。 */
const STORE_REFRESH_THROTTLE_MS = 10 * 60 * 1000;

/** 商店插件变更动作统一包装：调用通道 → 成败 toast。调用方负责把成功返回的
 *  插件清单写回槽（失败返回空数组表示「未取得新清单」）。 */
async function storePluginAction(
  action: (id: string) => Promise<StorePluginView[]>,
  id: string,
  okTitle: string,
  okMessage: string,
  failTitle: string,
): Promise<{ ok: boolean; plugins: StorePluginView[] }> {
  try {
    const plugins = await action(id);
    useToasts.getState().push("ok", okTitle, okMessage);
    return { ok: true, plugins };
  } catch (err) {
    useToasts.getState().push("danger", failTitle, cleanErrorMessage(err));
    return { ok: false, plugins: [] };
  }
}

/** 消息 + 部件的运行时可变视图（快照结构 + 流式增量）。 */
export interface ChatState {
  messages: MessageSnapshot[];
  running: boolean;
  lastSeq: number;
  /** 流活跃信号：最近一次收到增量的部件 id 与本地时刻。仅 text.delta / reasoning.delta
   *  更新；快照装载、run 终态等路径缺省 = 流不活跃（ContextChip 实时速率据此回落实测值）。
   *  内存值，不持久化。 */
  lastStreamPartId?: string;
  lastStreamTs?: number;
}

/** 草稿槽：活动会话为空（草稿态）时输入区的未发送内容；modelRef 为草稿态选定的模型引用（空 = 用全局默认）；
 * reasoningLevel 为草稿态选定的思考档位（空 = 未选择）；modelName 为该引用的显示名（chip 展示用）；
 * planMode/useWorktree 为 composer 顶部的发送选项勾选态（随槽保留）。 */
export interface DraftState {
  text: string;
  attachments: string[];
  modelRef: string;
  reasoningLevel: string;
  modelName: string;
  planMode: boolean;
  useWorktree: boolean;
}

/** 右侧面板动态 tab 的形状（D15 的面板补注）。
 *  固定 tab（files / agents）恒定存在不可关闭，不进 paneTabs 列表；动态 tab 打开才出现、
 *  可关闭、跨会话存活，重启清空（它们是界面态，不持久化；事实源在事件表）。
 *  - plan：单例。再点「查看计划」更新内容并激活，不重复开。
 *  - subagent:<agentId>：每个子代理一个，标题为子代理名。 */
export type PaneTab =
  | { id: "plan"; kind: "plan"; text: string }
  | { id: string; kind: "subagent"; agentId: string; title: string };

interface AppState {
  /** 启动装配完成（settings/threads/workspaces 已就绪）；false 时根组件显示启动态。 */
  booted: boolean;
  mode: Mode;
  /** 各模式会话列表（仅元数据；消息内容走 chats）。 */
  threads: Record<Mode, ThreadView[]>;
  /** 各模式活动会话 id；空串 = 该模式处于草稿态。 */
  activeThreadId: Record<Mode, string>;
  /** 草稿槽按归属分桶：key 为工作区 id，空串 "" 为内置临时会话槽；持久化经 lib/drafts。 */
  drafts: Record<Mode, Record<string, DraftState>>;
  /** 各模式当前草稿槽归属（空串 = 临时槽）；活动会话非空时不起作用。 */
  draftWsId: Record<Mode, string>;
  /** 每会话的「字符 → token」换算率（outputChars / outputTokens，最近一轮实测自校准）：
   *  流式期间实时速率 = 字符数 / 换算率 / 秒，修正 chars/4 对中文的低估。内存值，
   *  无实测的会话缺省 2（中文≈1 字/token、英文≈4 字符/token 的折中）。 */
  charsPerToken: Record<string, number>;
  workspaces: WorkspaceView[];
  settings: Settings;
  /** 已装载会话的运行时消息流（按需装载，不常驻全部历史）。 */
  /** 会话级 composer 桶：会话态输入区的文本/附件/发送选项，按会话 id 分桶（草稿态仍走草稿槽）；
   *  切换会话互不干扰、往返保留；持久化经 lib/drafts，删除会话时清理、boot 按现存会话裁剪。 */
  threadComposer: Record<string, ThreadComposer>;
  /** 写会话级 composer 桶（合并 patch + 持久化）；仅会话态使用，草稿态走 setDraft。 */
  setThreadComposer(id: string, patch: Partial<ThreadComposer>): void;
  chats: Record<string, ChatState>;
  // ---------- 右侧面板（D15 的面板补注：固定视图 + 动态 tab） ----------
  /** 面板展开态。tab 状态上移到全局状态壳是跨会话存活的硬要求（组件局部状态做不到）；
   *  三者都是界面态，不持久化，重启清空。 */
  paneOpen: boolean;
  /** 面板最大化抽屉态；收起面板时一并还原。 */
  paneMaximized: boolean;
  /** 动态 tab 列表（按打开顺序；固定 tab files / agents 不在列表内）。 */
  paneTabs: PaneTab[];
  /** 当前激活 tab id：固定 tab 为 "files" / "agents"，动态 tab 为 tab.id；默认 "files"。 */
  activePaneTab: string;
  /** 面板展开态宽度（px）。与 sidebarWidth 一样是**会话内姿态**：不持久化、
   *  重启回默认 380（决策 D17）。 */
  paneWidth: number;
  setPaneWidth(width: number): void;
  /** 跨会话子代理索引：子会话 id → 最新部件载荷（subagent.updated 事件写入）。
   *  服务三个消费点：子代理列表 = 按 parentThreadId 过滤（列表跟随当前会话）；
   *  详情 tab 元信息 = 按子会话 id 直取（不依赖活动会话，切走后状态与进度继续更新）；
   *  运行中徽标 = 索引里当前会话的 running 计数。重启清空，打开面板经 subagentList
   *  兜底恢复当前会话的历史派发记录。 */
  subagentIndex: Record<string, SubagentPart>;
  /** 已做过兜底恢复的会话 id（避免重复拉取；重启即清空）。 */
  subagentRestored: Record<string, true>;
  /** 拉取快照失败的子会话（父会话删除时子会话级联删除）：详情 tab 据此显示空态占位。 */
  subagentChatMissing: Record<string, true>;
  /** 展开/收起面板；收起即还原最大化（下次展开回到默认宽度）。 */
  setPaneOpen(open: boolean): void;
  setPaneMaximized(maximized: boolean): void;
  /** 切换激活 tab（固定 tab 与动态 tab 通用；tab 条与收起态图标共用；激活即展开面板）。 */
  setActivePaneTab(id: string): void;
  /** 打开子代理详情 tab：确保 tab 存在（已存在则聚焦，不重复开）→ 激活 → 展开面板
   *  → 若尚未装载则拉取该子会话快照（快照 + 在途事件流合并），之后靠事件流更新。 */
  openSubagentTab(agentId: string): Promise<void>;
  /** 关闭动态 tab：关闭当前激活的 tab 后激活它左侧相邻的 tab，无左侧则回「子代理」固定视图。 */
  closePaneTab(tabId: string): void;
  /** 拖拽排序：把动态 tab draggedId 移动到 overId 当前所在位置（其余相对顺序不变；
   *  激活 tab 不变）。固定 tab（files / agents）不参与排序，不在列表内。 */
  reorderPaneTabs(draggedId: string, overId: string): void;
  /** 打开/更新计划 tab（单例）：已存在则更新正文并激活，不存在则新建；展开面板。 */
  openPlanTab(text: string): void;
  /** 子代理面板兜底恢复：当前会话的历史派发记录（重启后跨会话索引为空时调用）。 */
  ensureSubagentRestored(): Promise<void>;
  /** 装载子代理详情时间线（快照 + 在途事件流合并，与切换会话同款）；已装载则跳过。 */
  ensureSubagentChat(agentId: string): Promise<void>;
  // ---------- 子代理配置（决策 D15 扩展） ----------
  /** 子代理全局配置（独立键存储；设置页「子代理」分节的数据源） */
  subagentSettings: SubAgentSettings;
  /** 自定义子代理定义列表（含停用的） */
  subagentProfiles: CustomAgentProfile[];
  /** 装载子代理配置与定义（进入设置分节时调用） */
  loadSubagentConfig(): Promise<void>;
  /** 保存子代理全局配置（内置两型的模型与轮数也走这里） */
  saveSubagentSettings(next: SubAgentSettings): Promise<boolean>;
  /** 新建自定义子代理；成功返回新定义，失败 toast 并返回 null */
  createSubagentProfile(input: CustomAgentProfileInput): Promise<CustomAgentProfile | null>;
  /** 更新自定义子代理；成功返回更新后的定义 */
  updateSubagentProfile(id: string, input: CustomAgentProfileInput): Promise<CustomAgentProfile | null>;
  /** 删除自定义子代理 */
  deleteSubagentProfile(id: string): Promise<boolean>;
  /** 启停自定义子代理 */
  setSubagentProfileEnabled(id: string, enabled: boolean): Promise<boolean>;
  /** 技能清单（决策 D11）：composer 斜杠补全专用（用户级 + 当前会话所属工作区）；打开时按需刷新，不常驻轮询。 */
  skills: SkillView[];
  /** 管理视角技能清单（设置页专用）：全量、带工作区归属；与 composer 的 skills 分槽，互不覆盖。 */
  manageSkills: SkillView[];
  // ---------- 技能商店（决策 D12） ----------
  // 独立槽，不覆盖 skills / manageSkills；清单类数据进入商店 tab 时装载，插件详情按需
  // describeStorePlugin 获取、不常驻（spec 第 10 节）。
  /** 双 tab 选中态（本机技能 / 技能商店）。 */
  storeTab: StoreTab;
  /** 已添加市场源（storeListMarkets.markets）。 */
  storeMarkets: MarketView[];
  /** 推荐市场源候选（storeListMarkets.recommended；常驻区块，运行时不内置任何源）。 */
  storeRecommended: RecommendedMarketView[];
  /** 各市场插件清单（含未装条目与更新角标；安装等变更动作整体刷新）。 */
  storePlugins: StorePluginView[];
  /** 商店清单是否已装载过（商店 tab 首次进入装载，之后靠动作局部刷新）。 */
  storeLoaded: boolean;
  /** 商店清单装载中（ensureStoreLoaded 防重入：tab 切换与面板挂载可能双触发）。 */
  storeLoading: boolean;
  /** 最近一次刷新尝试时刻（成功或失败都更新；自动刷新节流与失败防抖共用基准）。 */
  storeLastRefreshAt: number;
  /** 市场刷新进行中（「立即刷新」按钮 loading 与自动刷新防重入）。 */
  storeRefreshing: boolean;
  /** 设置技能 tab（进入商店 tab 时触发装载 + 节流自动刷新）。 */
  setStoreTab(tab: StoreTab): void;
  /** 商店页装载：拉取市场与插件清单（本地数据），有市场且距上次刷新尝试 ≥10 分钟时后台自动刷新。 */
  ensureStoreLoaded(): Promise<void>;
  /** 重拉商店清单（市场 + 插件；本地数据）。失败静默保留旧清单（错误提示由调用方动作负责）。 */
  reloadStoreListings(): Promise<void>;
  /** 添加市场源（输入 https:// URL 或 owner/repo；探测 + 首次 clone 在后端），成功返回 true。 */
  addStoreMarket(input: string): Promise<boolean>;
  /** 移除市场源（市场下仍有已安装插件时后端拒绝，错误 toast 呈现），成功返回 true。 */
  removeStoreMarket(id: string): Promise<boolean>;
  /** 立即刷新市场（id 缺省刷全部；单源失败不阻塞其他源）。返回各成功市场的结果（含加速回退标注），失败返回 null。 */
  refreshStoreMarkets(id?: string): Promise<StoreRefreshOutcome[] | null>;
  /** 安装插件（变更下一轮对话生效），成功返回 true。 */
  installStorePlugin(id: string): Promise<boolean>;
  /** 升级插件到市场侧最新（启停选择保留），成功返回 true。 */
  updateStorePlugin(id: string): Promise<boolean>;
  /** 一键回滚上一版（指针切回、可再切回），成功返回 true。 */
  rollbackStorePlugin(id: string): Promise<boolean>;
  /** 彻底卸载（含全部历史版本），成功返回 true。 */
  uninstallStorePlugin(id: string): Promise<boolean>;
  /** 插件级启停（停用后该插件全部技能根退出扫描），成功返回 true。 */
  setStorePluginEnabled(id: string, enabled: boolean): Promise<boolean>;
  /** 插件详情按需获取（已装读缓存、未装基于市场快照离线描述；失败 toast 并返回 null）。 */
  describeStorePlugin(id: string): Promise<StorePluginDetail | null>;
  sidebarVisible: boolean;
  /** 左侧会话侧栏宽度（px）。界面态：不持久化、重启回默认 264（决策 D17）。 */
  sidebarWidth: number;
  setSidebarWidth(width: number): void;
  /** 已解析主题类（system 偏好下随系统亮暗变化更新）。 */
  themeResolved: "theme-craft-light" | "theme-craft-dark";
  /** 运行平台（`getInfo().platform`：macos / windows / linux）；自绘标题栏窗口控件按它显示。 */
  platform: string;

  /** 启动装配：并行拉取设置/两模式会话/工作区，回放字号与主题，恢复各槽草稿并选中最近会话。 */
  boot(): Promise<void>;
  setMode(mode: Mode): void;
  refreshThreads(mode?: Mode): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  /** 选中会话：登记活动 id 并确保 chat 存在；本地无消息时拉快照（与在途事件流合并，见 mergeStreamedMessages）。 */
  selectThread(id: string): Promise<void>;
  /** 进入当前模式的草稿态：活动会话置空并切到 workspaceId 槽（缺省临时槽），不创建线程、不覆盖槽内容。 */
  newDraft(workspaceId?: string): void;
  /** 写当前草稿槽（按 mode + draftWsId 归属），并同步落本地存储。 */
  setDraft(mode: Mode, patch: Partial<DraftState>): void;
  /** 删除会话（后端物理删除）；删的是活动会话时改选列表第一个，列表空则回草稿态。 */
  deleteThread(id: string): Promise<void>;
  /** 一键清理临时会话（当前模式、未归档）；活动会话被清掉时回临时会话草稿槽。 */
  clearTempThreads(): Promise<void>;
  renameThread(id: string, title: string): Promise<void>;
  applyBatch(batch: Batch): void;
  /** 合并保存设置：以服务端返回的完整值为准覆盖本地。 */
  saveSettings(next: Partial<Settings>): Promise<void>;
  /** 刷新技能清单：传工作区 id 时同时扫该工作区级技能，缺省只扫用户级。 */
  refreshSkills(workspaceId?: string): Promise<void>;
  /** 刷新管理视角全量技能清单（用户级 + 全部工作区级，带归属）。 */
  refreshManageSkills(): Promise<void>;
  toggleSidebar(): void;
}

function emptyChat(): ChatState {
  return { messages: [], running: false, lastSeq: 0 };
}

/** 稳定空草稿槽：槽尚无内容时的回落（引用恒定，避免 zustand selector 每次返回新对象触发重渲染）。 */
export const EMPTY_DRAFT: DraftState = {
  text: "",
  attachments: [],
  modelRef: "",
  reasoningLevel: "",
  modelName: "",
  planMode: false,
  useWorktree: false,
};

/** 会话级 composer 桶（会话态输入区）：文本/附件 + 发送选项勾选态。
 *  空桶常量供 selector 回落（引用恒定，避免每次返回新对象触发重渲染）。 */
export interface ThreadComposer {
  text: string;
  attachments: string[];
  planMode: boolean;
  useWorktree: boolean;
}
export const EMPTY_THREAD_COMPOSER: ThreadComposer = {
  text: "",
  attachments: [],
  planMode: false,
  useWorktree: false,
};

/** 当前模式的草稿槽（draftWsId 指向归属）；槽尚无内容时为 undefined。 */
export function draftSlot(state: AppState, mode: Mode): DraftState | undefined {
  return state.drafts[mode][state.draftWsId[mode]];
}

/** 草稿态生效模型引用：槽选定引用优先，回落全局默认。 */
export function effectiveModelRef(state: AppState, mode: Mode): string {
  return draftSlot(state, mode)?.modelRef || state.settings.defaultModel;
}

/** 草稿态生效思考档位：槽选定档位优先，回落全局默认（空 = 未选择）。 */
export function effectiveReasoningLevel(state: AppState, mode: Mode): string {
  return draftSlot(state, mode)?.reasoningLevel || state.settings.reasoningLevel || "";
}

/** 全局状态壳 store：字段为可渲染状态，方法为动作；组件经 selector 订阅。 */
/** 按现存会话裁剪 composer 桶：保留列表内的条目，回收已删除会话的残留。 */
function pruneThreadComposer(composer: Record<string, ThreadComposer>, alive: string[]): Record<string, ThreadComposer> {
  const aliveSet = new Set(alive);
  const out: Record<string, ThreadComposer> = {};
  for (const [id, bucket] of Object.entries(composer)) {
    if (aliveSet.has(id)) out[id] = bucket;
  }
  return out;
}

export const useApp = create<AppState>((set, get) => ({
  booted: false,
  mode: "code",
  threads: { code: [], work: [] },
  activeThreadId: { code: "", work: "" },
  drafts: emptyDraftTable(),
  draftWsId: { code: "", work: "" },
  threadComposer: loadThreadComposer(),
  charsPerToken: {},
  workspaces: [],
  settings: {
    theme: "system",
    mode: "code",
    approvalLevel: "graded",
    defaultModel: "",
    reasoningLevel: "",
    fallbackModel: "",
    activeWorkspace: "",
    language: "zh-CN",
    fontSize: 15,
    codeTheme: "dark",
    autoUpdate: true,
    notifications: true,
    updateChannel: "stable",
    telemetry: false,
    maxIterations: 0,
    logsLevel: "info",
    // 默认不使用（D18）：命令跑在最小环境里，由用户在设置 → 通用里显式选择 shell
    shellPath: "",
    smoothFollow: false,
    inputPrediction: true,
    quitOnClose: false,
    skillsEnabled: true,
    networkProxyMode: "system",
    networkProxyUrl: "",
    networkGithubAcceleratorEnabled: false,
    networkGithubAcceleratorPrefix: "https://gh-proxy.com/",
    subagentsEnabled: true,
    subagentMaxConcurrent: 4,
  },
  chats: {},
  paneOpen: false,
  paneMaximized: false,
  paneTabs: [],
  activePaneTab: "files",
  subagentIndex: {},
  subagentRestored: {},
  subagentChatMissing: {},
  subagentSettings: {
    enabled: true,
    maxConcurrent: 4,
    builtinProfiles: [
      { agentType: "general", model: "", maxIterations: 0 },
      { agentType: "explore", model: "", maxIterations: 0 },
    ],
    // 封顶值占位（0 = 未装载）：设置页消费时以服务端返回为准（loadSubagentConfig 覆盖）
    maxIterationsCap: 0,
  },
  subagentProfiles: [],
  skills: [],
  manageSkills: [],
  storeTab: "local",
  storeMarkets: [],
  storeRecommended: [],
  storePlugins: [],
  storeLoaded: false,
  storeLoading: false,
  storeLastRefreshAt: 0,
  storeRefreshing: false,
  sidebarVisible: true,
  // 宽度初值 = D17 的默认值（SIDEBAR.initial / PANE.initial）。
  // 不持久化：窗口尺寸都不记，栏宽属同一类「会话内姿态」（决策 D17）。
  sidebarWidth: 264,
  paneWidth: 380,
  themeResolved: "theme-craft-dark",
  // 未知平台按「无内联控件」处理：宁可少画，也不在 macOS 上多画
  platform: "unknown",

  async boot() {
    const [settings, threadsCode, threadsWork, workspaces, info] = await Promise.all([
      api.settingsGet(),
      api.threadList("code", false),
      api.threadList("work", false),
      api.workspaceList(),
      // 平台只用于界面分支（自绘窗口控件），取不到就按未知平台继续，不阻断启动
      api.getInfo().catch(() => null),
    ]);
    // 界面字号缩放：设置页只在拖动时写 --ui-font-size，boot 必须回放持久化值（否则重启回退 14px）
    if (settings.fontSize >= 11 && settings.fontSize <= 18) {
      document.documentElement.style.setProperty("--ui-font-size", `${settings.fontSize}px`);
    }
    // 主题以 settings 为事实源：校准启动缓存；system 偏好下跟随系统亮暗变化。
    saveThemePref(settings.theme);
    const themeResolved = applyTheme(settings.theme);
    watchSystemTheme(
      () => useApp.getState().settings.theme,
      (resolved) => useApp.setState({ themeResolved: resolved }),
    );
    // 模式主色 class 以 settings 为事实源校准（localStorage 缓存已在 main.tsx 预应用，防闪烁）
    const mode: "code" | "work" = settings.mode === "work" ? "work" : "code";
    applyModeTheme(mode);
    saveModePref(mode);
    set({
      booted: true,
      settings,
      themeResolved,
      platform: info?.platform ?? "unknown",
      mode,
      threads: { code: threadsCode, work: threadsWork },
      workspaces,
      // 启动恢复各槽草稿文本与附件；槽归属不恢复（恒为临时槽），
      // activeThreadId 选择逻辑不动：有会话选中最近会话，即「恢复草稿但不自动进入草稿态」。
      drafts: loadDrafts(),
      // 会话级 composer 桶按现存会话裁剪：批量删除（如一键清理临时会话）留下的死条目在此回收
      threadComposer: pruneThreadComposer(loadThreadComposer(), [...threadsCode, ...threadsWork].map((t) => t.id)),
      activeThreadId: {
        code: threadsCode[0]?.id ?? "",
        work: threadsWork[0]?.id ?? "",
      },
    });
    const active = get().activeThreadId[get().mode];
    if (active) await get().selectThread(active);
  },

  setMode(mode) {
    set({ mode });
    // 模式主色 class 与启动缓存同步收口：所有切换入口（侧栏 / ⌘K / 设置页）都经此生效
    applyModeTheme(mode);
    saveModePref(mode);
    const active = get().activeThreadId[mode];
    if (active) void get().selectThread(active);
    void api.settingsSave({ ...get().settings, mode }).then((s) => set({ settings: s }));
  },

  async refreshThreads(mode) {
    const m = mode ?? get().mode;
    const list = await api.threadList(m, false);
    set((state) => ({ threads: { ...state.threads, [m]: list } }));
  },

  async refreshWorkspaces() {
    const workspaces = await api.workspaceList();
    set({ workspaces });
  },

  async selectThread(id) {
    const mode = get().mode;
    set((state) => ({
      activeThreadId: { ...state.activeThreadId, [mode]: id },
      chats: state.chats[id] ? state.chats : { ...state.chats, [id]: state.chats[id] ?? emptyChat() },
    }));
    // 打开即视为已读：先本地清位（点立刻消失，不等往返），再落库并广播。
    // 只为「确实有未读」的会话写库：切换会话很频繁，没必要每次空写一行时间戳。
    if (id && get().threads[mode].some((t) => t.id === id && t.hasUnread)) {
      set((state) => ({
        threads: {
          ...state.threads,
          [mode]: state.threads[mode].map((t) => (t.id === id ? { ...t, hasUnread: false } : t)),
        },
      }));
      void api.threadMarkRead(id).catch(() => {
        // 落库失败不阻断打开流程：本地已清位，下一次 thread.updated 重拉列表会按库内值纠正
      });
    }
    if (!get().chats[id] || get().chats[id]!.messages.length === 0) {
      const snap = await api.threadSnapshot(id);
      set((state) => ({
        chats: {
          ...state.chats,
          [id]: {
            // 快照与在途事件流合并：快照查询期间流式增量可能已写入 chats（run.started/text.delta 先于快照返回），
            // 整体覆盖会丢掉已累积的流式内容。快照为序的权威，事件流独有的 streaming 消息追加到尾部。
            messages: mergeStreamedMessages(snap.messages, state.chats[id]?.messages ?? []),
            running: snap.thread.status === "running" || snap.thread.status === "awaiting",
            lastSeq: snap.lastSeq,
          },
        },
      }));
    }
  },

  newDraft(workspaceId = "") {
    const mode = get().mode;
    // 草稿态：活动会话置空并切到归属槽（缺省临时槽），不调线程创建（草稿是纯渲染层状态）；
    // 槽已有内容原样保留（重复 ⌘N 回到同一条草稿）。
    set((state) => ({
      activeThreadId: { ...state.activeThreadId, [mode]: "" },
      draftWsId: { ...state.draftWsId, [mode]: workspaceId },
    }));
  },

  setDraft(mode, patch) {
    const wsId = get().draftWsId[mode];
    const prev = get().drafts;
    const slot = prev[mode][wsId] ?? EMPTY_DRAFT;
    const drafts = { ...prev, [mode]: { ...prev[mode], [wsId]: { ...slot, ...patch } } };
    saveDrafts(drafts);
    set({ drafts });
  },

  setThreadComposer(id, patch) {
    if (!id) return;
    const prev = get().threadComposer;
    const bucket = prev[id] ?? EMPTY_THREAD_COMPOSER;
    const next = { ...prev, [id]: { ...bucket, ...patch } };
    saveThreadComposer(next);
    set({ threadComposer: next });
  },

  async deleteThread(id) {
    await api.threadDelete(id);
    // 清理会话级 composer 桶（输入/附件/发送选项随会话一起删）
    const composer = { ...get().threadComposer };
    if (composer[id]) {
      delete composer[id];
      saveThreadComposer(composer);
      set({ threadComposer: composer });
    }
    // 清理字符→token 换算率（会话已删，校准值无意义）
    if (get().charsPerToken[id] !== undefined) {
      const rates = { ...get().charsPerToken };
      delete rates[id];
      set({ charsPerToken: rates });
    }
    const mode = get().mode;
    await get().refreshThreads(mode);
    if (get().activeThreadId[mode] === id) {
      const next = get().threads[mode][0]?.id ?? "";
      set((state) => ({ activeThreadId: { ...state.activeThreadId, [mode]: next } }));
      if (next) await get().selectThread(next);
    }
  },

  /** 一键清理临时会话：后端删除（运行中跳过），活动会话被清掉时回临时会话草稿槽；有跳过时提示。 */
  async clearTempThreads() {
    const mode = get().mode;
    const activeId = get().activeThreadId[mode];
    const result = await api.threadClearTemp(mode);
    await get().refreshThreads(mode);
    if (activeId && !get().threads[mode].some((t) => t.id === activeId)) {
      get().newDraft("");
    }
    if (result.skipped > 0) {
      useToasts.getState().push("warn", "已清理临时会话", `${result.removed} 个已删除，${result.skipped} 个运行中已跳过`);
    }
  },

  async renameThread(id, title) {
    await api.threadRename(id, title);
    await get().refreshThreads();
  },

  /** 事件批量应用：唯一的事件入口（按 type 分发到消息/会话状态）。 */
  applyBatch(batch) {
    for (const event of batch.events) {
      applyEvent(set, get, event);
    }
  },

  async saveSettings(next) {
    const saved = await api.settingsSave({ ...get().settings, ...next });
    set({ settings: saved });
  },

  // ---------- 右侧面板（D15 的面板补注：固定视图 + 动态 tab） ----------

  setPaneOpen(open: boolean): void {
    // 收起即还原：下次展开回到默认宽度，不残留最大化态
    set(open ? { paneOpen: true } : { paneOpen: false, paneMaximized: false });
  },
  setPaneMaximized(maximized: boolean): void {
    set({ paneMaximized: maximized });
  },
  setActivePaneTab(id: string): void {
    set({ activePaneTab: id, paneOpen: true });
  },

  /** 打开子代理详情 tab：确保 tab 存在（已存在则聚焦，不重复开）→ 激活 → 展开面板
   *  → 装载时间线。消息流卡片「在右侧栏查看」与面板列表行点击共用这一个动作。 */
  async openSubagentTab(agentId: string): Promise<void> {
    const meta = get().subagentIndex[agentId];
    const tabId = `subagent:${agentId}`;
    set((state) => {
      const exists = state.paneTabs.some((t) => t.kind === "subagent" && t.agentId === agentId);
      return {
        paneOpen: true,
        activePaneTab: tabId,
        paneTabs: exists
          ? state.paneTabs
          : [...state.paneTabs, { id: tabId, kind: "subagent" as const, agentId, title: meta?.name ?? agentId }],
      };
    });
    await get().ensureSubagentChat(agentId);
  },
  closePaneTab(tabId: string): void {
    const tabs = get().paneTabs;
    const idx = tabs.findIndex((t) => t.id === tabId);
    if (idx < 0) return;
    const active = get().activePaneTab;
    set({
      paneTabs: tabs.filter((t) => t.id !== tabId),
      // 关闭当前激活的 tab 后激活它左侧相邻的 tab；无左侧（首个动态 tab）回「子代理」固定视图
      activePaneTab: active === tabId ? (idx > 0 ? tabs[idx - 1]!.id : "agents") : active,
    });
  },
  reorderPaneTabs(draggedId: string, overId: string): void {
    if (draggedId === overId) return;
    const tabs = [...get().paneTabs];
    const from = tabs.findIndex((t) => t.id === draggedId);
    const to = tabs.findIndex((t) => t.id === overId);
    if (from < 0 || to < 0) return;
    const [moved] = tabs.splice(from, 1);
    tabs.splice(to, 0, moved!);
    set({ paneTabs: tabs });
  },
  /** 打开/更新计划 tab（单例）：先后查看不同计划复用同一个 tab（更新内容并激活）。 */
  openPlanTab(text: string): void {
    set((state) => {
      const exists = state.paneTabs.some((t) => t.kind === "plan");
      return {
        paneOpen: true,
        activePaneTab: "plan",
        paneTabs: exists
          ? state.paneTabs.map((t) => (t.kind === "plan" ? { ...t, text } : t))
          : [...state.paneTabs, { id: "plan" as const, kind: "plan" as const, text }],
      };
    });
  },
  /** 兜底恢复当前会话的历史派发记录（重启后跨会话索引为空）：只补缺失的条目，
   *  已有的事件流值（运行中实时进度）不覆盖。每个会话只拉一次。 */
  async ensureSubagentRestored(): Promise<void> {
    const mode = get().mode;
    const threadId = get().activeThreadId[mode];
    if (!threadId || get().subagentRestored[threadId]) return;
    set((state) => ({ subagentRestored: { ...state.subagentRestored, [threadId]: true } }));
    try {
      const list = await api.subagentList(threadId);
      if (list.length === 0) return;
      set((state) => {
        const index = { ...state.subagentIndex };
        for (const view of list) {
          if (!index[view.agentId]) index[view.agentId] = view;
        }
        return { subagentIndex: index };
      });
    } catch {
      // 恢复失败静默：面板显示空态可接受，toast 反而打扰（下次打开面板会重试）
      set((state) => {
        const restored = { ...state.subagentRestored };
        if (threadId) delete restored[threadId];
        return { subagentRestored: restored };
      });
    }
  },
  /** 装载子代理详情时间线：快照 + 在途事件流合并（与切换会话同款 mergeStreamedMessages，
   *  子会话的实时增量可能已随事件流写入 chats，整体覆盖会丢掉已累积的流式内容）。 */
  async ensureSubagentChat(agentId: string): Promise<void> {
    const chat = get().chats[agentId];
    if (chat && chat.messages.length > 0) return;
    try {
      const snap = await api.threadSnapshot(agentId);
      set((state) => ({
        chats: {
          ...state.chats,
          [agentId]: {
            messages: mergeStreamedMessages(snap.messages, state.chats[agentId]?.messages ?? []),
            running: snap.thread.status === "running" || snap.thread.status === "awaiting",
            lastSeq: snap.lastSeq,
          },
        },
      }));
    } catch {
      // 拉不到快照（父会话删除时子会话级联删除等）：详情 tab 显示「该子代理会话已删除」
      // 的空态占位，tab 手动关闭；不打 toast——这是预期内的边界而不是错误。
      set((state) => ({ subagentChatMissing: { ...state.subagentChatMissing, [agentId]: true } }));
    }
  },

  // ---------- 子代理配置（决策 D15 扩展） ----------

  /** 装载子代理配置与定义：两个独立键并行读取，任一失败只 toast 不阻塞界面。 */
  async loadSubagentConfig(): Promise<void> {
    try {
      const [settings, profiles] = await Promise.all([api.subagentSettingsGet(), api.subagentProfilesList()]);
      set({ subagentSettings: settings, subagentProfiles: profiles });
    } catch (err) {
      useToasts.getState().push("danger", "子代理配置读取失败", cleanErrorMessage(err));
    }
  },
  /** 保存全局配置：以服务端返回为准重读一次（归一规则在服务端，本地不重复实现）。 */
  async saveSubagentSettings(next: SubAgentSettings): Promise<boolean> {
    try {
      await api.subagentSettingsSave(next);
      await get().loadSubagentConfig();
      return true;
    } catch (err) {
      useToasts.getState().push("danger", "子代理设置保存失败", cleanErrorMessage(err));
      return false;
    }
  },
  async createSubagentProfile(input: CustomAgentProfileInput): Promise<CustomAgentProfile | null> {
    try {
      const profile = await api.subagentProfileCreate(input);
      await get().loadSubagentConfig();
      useToasts.getState().push("ok", "子代理已创建", "下一轮对话即可派发");
      return profile;
    } catch (err) {
      useToasts.getState().push("danger", "子代理创建失败", cleanErrorMessage(err));
      return null;
    }
  },
  async updateSubagentProfile(id: string, input: CustomAgentProfileInput): Promise<CustomAgentProfile | null> {
    try {
      const profile = await api.subagentProfileUpdate(id, input);
      await get().loadSubagentConfig();
      useToasts.getState().push("ok", "子代理已更新", "下一轮对话生效");
      return profile;
    } catch (err) {
      useToasts.getState().push("danger", "子代理更新失败", cleanErrorMessage(err));
      return null;
    }
  },
  async deleteSubagentProfile(id: string): Promise<boolean> {
    try {
      const list = await api.subagentProfileDelete(id);
      set({ subagentProfiles: list });
      useToasts.getState().push("ok", "子代理已删除");
      return true;
    } catch (err) {
      useToasts.getState().push("danger", "子代理删除失败", cleanErrorMessage(err));
      return false;
    }
  },
  async setSubagentProfileEnabled(id: string, enabled: boolean): Promise<boolean> {
    try {
      const list = await api.subagentProfileSetEnabled(id, enabled);
      set({ subagentProfiles: list });
      return true;
    } catch (err) {
      useToasts.getState().push("danger", enabled ? "子代理启用失败" : "子代理停用失败", cleanErrorMessage(err));
      return false;
    }
  },
  /** 刷新技能清单（决策 D11）：composer 斜杠补全用；失败静默保持旧列表（toast 由调用方负责）。 */
  async refreshSkills(workspaceId?: string): Promise<void> {
    try {
      const skills = await api.skillsList(workspaceId ?? "");
      set({ skills });
    } catch {
      // 扫描失败不阻塞界面：保留旧清单
    }
  },
  /** 刷新管理视角清单（设置页用）：全量扫描，写入独立状态槽 manageSkills。 */
  async refreshManageSkills(): Promise<void> {
    try {
      const list = await api.skillsListAll();
      set({ manageSkills: list });
    } catch {
      // 扫描失败不阻塞界面：保留旧清单
    }
  },

  // ---------- 技能商店（决策 D12） ----------

  setStoreTab(tab) {
    set({ storeTab: tab });
    // 切入商店 tab 即装载 + 节流自动刷新；重复切入由节流窗口挡住，不重复触网
    if (tab === "market") void get().ensureStoreLoaded();
  },

  async ensureStoreLoaded() {
    // 防重入：setStoreTab 与商店面板挂载可能双触发，同一时刻只装载一次
    if (get().storeLoading) return;
    set({ storeLoading: true });
    try {
      const [marketsRes, plugins] = await Promise.all([api.storeListMarkets(), api.storeListPlugins()]);
      set({
        storeMarkets: marketsRes.markets,
        storeRecommended: marketsRes.recommended,
        storePlugins: plugins,
        storeLoaded: true,
      });
    } catch (err) {
      // 首次装载失败要给可读解释（后续局部重拉失败才静默保留旧值）
      useToasts.getState().push("danger", "技能商店装载失败", cleanErrorMessage(err));
      return;
    } finally {
      set({ storeLoading: false });
    }
    // 节流自动刷新（spec 第 11 节）：有已添加市场且距上次刷新尝试 ≥10 分钟才真正拉取。
    // 内存基准（storeLastRefreshAt，含失败）与后端持久化值（lastRefreshedAt，跨重启生效）取较大者。
    if (get().storeRefreshing) return;
    const markets = get().storeMarkets;
    const backendLatest = markets.reduce((acc, m) => Math.max(acc, m.lastRefreshedAt), 0);
    if (markets.length > 0 && Date.now() - Math.max(get().storeLastRefreshAt, backendLatest) >= STORE_REFRESH_THROTTLE_MS) {
      void get().refreshStoreMarkets();
    }
  },

  async reloadStoreListings() {
    try {
      const [marketsRes, plugins] = await Promise.all([api.storeListMarkets(), api.storeListPlugins()]);
      set({
        storeMarkets: marketsRes.markets,
        storeRecommended: marketsRes.recommended,
        storePlugins: plugins,
        storeLoaded: true,
      });
    } catch {
      // 本地重拉失败不阻塞：保留旧清单
    }
  },

  async addStoreMarket(input) {
    try {
      const res = await api.storeAddMarket(input);
      set({ storeMarkets: res.markets, storeRecommended: res.recommended });
      // 新市场首次刷新已在其添加编排内完成（clone + 探测），直接拉一次插件清单
      await get().reloadStoreListings();
      useToasts.getState().push("ok", "市场源已添加", "清单探测与首次拉取已完成");
      return true;
    } catch (err) {
      useToasts.getState().push("danger", "市场源添加失败", cleanErrorMessage(err));
      return false;
    }
  },

  async removeStoreMarket(id) {
    try {
      const res = await api.storeRemoveMarket(id);
      set({ storeMarkets: res.markets, storeRecommended: res.recommended });
      await get().reloadStoreListings();
      useToasts.getState().push("ok", "市场源已移除");
      return true;
    } catch (err) {
      // 市场下仍有已安装插件时后端拒绝（提示先卸载），错误原文直接呈现
      useToasts.getState().push("danger", "市场源移除失败", cleanErrorMessage(err));
      return false;
    }
  },

  async refreshStoreMarkets(id) {
    if (get().storeRefreshing) return null;
    set({ storeRefreshing: true });
    try {
      const outcomes = await api.storeRefreshMarkets(id ?? "");
      set({ storeLastRefreshAt: Date.now() });
      await get().reloadStoreListings();
      // 加速回退标注（spec 第 7 节）：本次有市场经「加速失败 → 原始 URL 回退」成功时提示
      if (outcomes.some((o) => o.fellBack)) {
        useToasts.getState().push("warn", "部分市场经加速失败已回退", "GitHub 加速前缀请求失败，已改用原始地址完成刷新");
      }
      return outcomes;
    } catch (err) {
      // 全量刷新彻底失败（如完全断网）：失败也计入防抖基准，失败详情由市场清单红字呈现
      set({ storeLastRefreshAt: Date.now() });
      await get().reloadStoreListings();
      useToasts.getState().push("danger", "市场刷新失败", cleanErrorMessage(err));
      return null;
    } finally {
      set({ storeRefreshing: false });
    }
  },

  async installStorePlugin(id) {
    const r = await storePluginAction(api.storeInstallPlugin, id, "插件已安装", "变更下一轮对话生效", "插件安装失败");
    if (r.ok) set({ storePlugins: r.plugins });
    return r.ok;
  },

  async updateStorePlugin(id) {
    const r = await storePluginAction(api.storeUpdatePlugin, id, "插件已升级", "变更下一轮对话生效", "插件升级失败");
    if (r.ok) set({ storePlugins: r.plugins });
    return r.ok;
  },

  async rollbackStorePlugin(id) {
    const r = await storePluginAction(api.storeRollbackPlugin, id, "已回滚上一版", "变更下一轮对话生效，可再次回滚切回", "插件回滚失败");
    if (r.ok) set({ storePlugins: r.plugins });
    return r.ok;
  },

  async uninstallStorePlugin(id) {
    const r = await storePluginAction(api.storeUninstallPlugin, id, "插件已卸载", "全部版本已删除，变更下一轮对话生效", "插件卸载失败");
    if (r.ok) set({ storePlugins: r.plugins });
    return r.ok;
  },

  async setStorePluginEnabled(id, enabled) {
    const r = await storePluginAction(
      (pid) => api.storeSetPluginEnabled(pid, enabled),
      id,
      enabled ? "插件已启用" : "插件已停用",
      "变更下一轮对话生效",
      enabled ? "插件启用失败" : "插件停用失败",
    );
    if (r.ok) set({ storePlugins: r.plugins });
    return r.ok;
  },

  async describeStorePlugin(id) {
    try {
      return await api.storeDescribePlugin(id);
    } catch (err) {
      useToasts.getState().push("danger", "插件详情读取失败", cleanErrorMessage(err));
      return null;
    }
  },

  toggleSidebar() {
    set((state) => ({ sidebarVisible: !state.sidebarVisible }));
  },
  setSidebarWidth(width: number) {
    set({ sidebarWidth: clampWidth(width, SIDEBAR.min, SIDEBAR.max) });
  },
  setPaneWidth(width: number) {
    set({ paneWidth: clampWidth(width, PANE.min, PANE.max) });
  },
}));

// ---------- 事件应用 ----------

type SetFn = (partial: Partial<AppState> | ((state: AppState) => Partial<AppState>)) => void;
type GetFn = () => AppState;

/** 在目标消息上创建或合并 part（payload 浅合并，保留既有字段）；消息不存在则新建占位。 */
function upsertPart(
  chat: ChatState,
  messageId: string,
  partId: string,
  kind: PartSnapshot["kind"],
  payload: Record<string, unknown>,
  ts = 0,
): MessageSnapshot[] {
  const messages = [...chat.messages];
  let target = messages.find((m) => m.id === messageId);
  if (!target) {
    target = { id: messageId, role: "assistant", parts: [], status: "streaming", seq: 0, createdAt: ts };
    messages.push(target);
  }
  const parts = [...target.parts];
  const idx = parts.findIndex((p) => p.id === partId);
  const part: PartSnapshot = { id: partId, kind, seq: 0, payload };
  if (idx >= 0) parts[idx] = { ...parts[idx]!, payload: { ...parts[idx]!.payload, ...payload } };
  else parts.push(part);
  target = { ...target, parts };
  const msgIdx = messages.findIndex((m) => m.id === messageId);
  messages[msgIdx] = target;
  return messages;
}

function findMessageIdForPart(chat: ChatState, partId: string): string | null {
  for (const m of chat.messages) {
    if (m.parts.some((p) => p.id === partId)) return m.id;
  }
  return null;
}

/** 单事件应用：消息类事件先定位目标消息（partId 反查 → 最近 streaming 消息 → 尾部兜底），
 * 再创建/合并 part 并整体替换 chats；threadId 缺省回落当前活动会话，仍为空则丢弃（草稿态不积压事件）。 */
function applyEvent(set: SetFn, get: GetFn, event: Envelope): void {
  // workspace.updated 与会话无关（threadId 为空）：草稿态（无活动会话）下也会到达，
  // 必须在 threadId 短路之前处理，否则删项目清槽事件会被丢弃。
  if (event.type === "workspace.updated") {
    const payload = event.data as Record<string, unknown>;
    const removedWsId = payload["removed"] === true && typeof payload["workspaceId"] === "string" ? payload["workspaceId"] : "";
    if (removedWsId) {
      // 删除项目：清掉该工作区在两个模式下的草稿槽（无残留），并同步本地存储。
      set((state) => {
        const drafts: DraftTable = { code: { ...state.drafts.code }, work: { ...state.drafts.work } };
        delete drafts.code[removedWsId];
        delete drafts.work[removedWsId];
        saveDrafts(drafts);
        return { drafts };
      });
    }
    void get().refreshWorkspaces();
    return;
  }
  const state = get();
  const threadId = event.threadId ?? state.activeThreadId[state.mode];
  if (!threadId) return;
  const chat = state.chats[threadId] ?? emptyChat();
  const data = event.data as Record<string, unknown>;

  switch (event.type) {
    case "run.started": {
      const messageId = String(data["messageId"] ?? "");
      // 「待送达」（via=steer）标记不在此处清除：接力新 run 时队列里的引导尚未消费，
      // 提前全清会误伤；标记生命周期完全由 steer.delivered 回执驱动（消费/丢弃逐条摘除）
      setChat(set, threadId, {
        ...chat,
        running: true,
        messages: chat.messages.some((m) => m.id === messageId)
          ? chat.messages
          : [...chat.messages, { id: messageId, role: "assistant", parts: [], status: "streaming", seq: 0, createdAt: event.ts }],
      });
      break;
    }
    case "steer.delivered": {
      // 引导投递回执：按 messageId 摘除「待送达」标记；带 nextMessageId 时把旧
      // streaming 消息封口（空壳占位直接删除），新 streaming 消息插到引导气泡之后——
      // 后续输出在气泡下方续流，时间线时序与视觉位置一致
      const steerMessageId = String(data["messageId"] ?? "");
      const nextMessageId = data["nextMessageId"] == null ? "" : String(data["nextMessageId"]);
      // 空壳 streaming 消息（run.started 刚建的占位、尚无部件）先移除，避免与封口判断互斥
      const withoutShell = chat.messages.filter(
        (m) => !(m.role === "assistant" && m.status === "streaming" && m.parts.length === 0 && m.id !== nextMessageId),
      );
      const messages = withoutShell.map((m) => {
        if (m.id === steerMessageId) {
          return {
            ...m,
            parts: m.parts.map((p) => (p.payload["via"] ? { ...p, payload: { ...p.payload, via: undefined } } : p)),
          };
        }
        if (nextMessageId && m.id !== nextMessageId && m.role === "assistant" && m.status === "streaming") {
          return { ...m, status: "complete" as const };
        }
        return m;
      });
      if (!nextMessageId) {
        setChat(set, threadId, { ...chat, messages });
        break;
      }
      const steerIdx = messages.findIndex((m) => m.id === steerMessageId);
      // 同迭代多条引导共用同一 nextMessageId：已存在（前一条回执刚建）则不重复插入
      if (messages.some((m) => m.id === nextMessageId)) {
        setChat(set, threadId, { ...chat, messages });
        break;
      }
      const fresh: MessageSnapshot = {
        id: nextMessageId,
        role: "assistant",
        parts: [],
        status: "streaming",
        seq: 0,
        createdAt: event.ts,
      };
      if (steerIdx < 0) {
        setChat(set, threadId, { ...chat, messages: [...messages, fresh] });
      } else {
        const next = [...messages];
        next.splice(steerIdx + 1, 0, fresh);
        setChat(set, threadId, { ...chat, messages: next });
      }
      break;
    }
    case "run.completed":
    case "run.canceled": {
      const messages = chat.messages.map((m) =>
        m.status === "streaming" ? { ...m, status: event.type === "run.completed" ? ("complete" as const) : ("incomplete" as const) } : m,
      );
      mergeWorkDuration(messages, data["durationMs"], event.ts);
      setChat(set, threadId, { messages, running: false, lastSeq: chat.lastSeq });
      // 用户正开着这个会话看它跑完：直接算已读，别留下「已完成但未读」的假标记。
      // 判定要同时满足「是本模式的活动会话」与「窗口有焦点」——最小化或切到别的应用时，
      // 完成了就该亮未读点（这正是未读要提醒的场景）。
      const { mode, activeThreadId } = get();
      if (event.type === "run.completed" && threadId && activeThreadId[mode] === threadId && document.hasFocus()) {
        set((state) => ({
          threads: {
            ...state.threads,
            [mode]: state.threads[mode].map((t) => (t.id === threadId ? { ...t, hasUnread: false } : t)),
          },
        }));
        void api.threadMarkRead(threadId).catch(() => {});
      }
      break;
    }
    case "run.failed": {
      const messages = chat.messages.map((m) => (m.status === "streaming" ? { ...m, status: "error" as const } : m));
      mergeWorkDuration(messages, data["durationMs"], event.ts);
      setChat(set, threadId, { messages, running: false, lastSeq: chat.lastSeq });
      break;
    }
    case "text.delta":
    case "reasoning.delta": {
      const partId = String(data["partId"] ?? "");
      let messageId = findMessageIdForPart(chat, partId);
      if (!messageId) {
        // 流式开始：挂到最后一条 streaming 消息（或新建）
        messageId = [...chat.messages].reverse().find((m) => m.status === "streaming")?.id ?? "";
      }
      if (!messageId) break;
      const kind = event.type === "text.delta" ? "text" : "reasoning";
      const messages = upsertAppend(chat, messageId, partId, kind, String(data["text"] ?? ""));
      // 流活跃信号（本地时钟，与 ContextChip 的 Date.now() 同基准）：速率估算只认
      // 「最近有增量产出的部件」，思考/正文交错与工具执行停顿都不会再稀释估算。
      setChat(set, threadId, { ...chat, messages, lastStreamPartId: partId, lastStreamTs: Date.now() });
      break;
    }
    case "text.completed":
    case "reasoning.completed": {
      const partId = String(data["partId"] ?? "");
      const messageId = findMessageIdForPart(chat, partId);
      if (!messageId) break;
      const kind = event.type === "text.completed" ? "text" : "reasoning";
      const messages = upsertPart(chat, messageId, partId, kind, data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "tool.started": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "tool", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "tool.completed": {
      const partId = String(data["partId"] ?? "");
      const messageId = findMessageIdForPart(chat, partId) ?? lastStreamingId(chat);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "tool", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "approval.requested": {
      const partId = String(data["partId"] ?? "");
      // 审批请求复用工具调用的 partId：必须以独立 id（approval_ 前缀）建 part，
      // 否则会浅合并进已存在的 tool part——kind 保持 "tool"、status 被 "waiting"
      // 覆盖，审批卡永不渲染、工具卡假显「完成」。
      const key = `approval_${partId}`;
      // lastStreamingId 找不到时返回空串（非 nullish），必须用 || 回退到最后一条消息，
      // 否则审批事件会被静默丢弃（run 无声卡死）
      const messageId = lastStreamingId(chat) || [...chat.messages].reverse()[0]?.id;
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, key, "approval", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "approval.resolved": {
      // 与 approval.requested 同一前缀键定位（快照重建侧 Rust 同规则，见 craft-store::events）
      const key = `approval_${String(data["partId"] ?? "")}`;
      const messageId = findMessageIdForPart(chat, key);
      if (!messageId) break;
      // 决策事件只更新决策字段（status/decision/scope），保留原请求 payload（标题/风险/参数）供卡片继续展示
      const messages = upsertPart(chat, messageId, key, "approval", {
        ...approvalPayloadOf(chat, key),
        status: data["status"],
        decision: data["decision"],
        scope: data["scope"],
      }, event.ts);
      setChat(set, threadId, { ...chat, messages });
      // 计划审阅批准：定向关闭 composer 勾选（批准后关闭计划标记；
      // 拒绝/反馈保持计划态，勾选不动）。同一决定只处理一次（resolved 不重复广播）。
      if (data["status"] === "approved" && approvalPayloadOf(chat, key)["tool"] === "exit_plan") {
        set((state) => ({
          threadComposer: {
            ...state.threadComposer,
            [threadId]: { ...(state.threadComposer[threadId] ?? EMPTY_THREAD_COMPOSER), planMode: false },
          },
        }));
      }
      break;
    }
    case "plan.entered": {
      // enter_plan 工具执行（模型主动进入计划态）：同步 composer 勾选态为勾选
      // （工具主动进入时定向开启）。纯广播事件，只在实时流出现，快照不回放。
      const planThreadId = event.threadId ?? threadId;
      set((state) => ({
        threadComposer: {
          ...state.threadComposer,
          [planThreadId]: { ...(state.threadComposer[planThreadId] ?? EMPTY_THREAD_COMPOSER), planMode: true },
        },
      }));
      break;
    }
    case "todo.updated": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat);
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "todo", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "subagent.updated": {
      // 跨会话子代理索引（D15 的面板补注）：先写索引再更新卡片。
      // 索引不依赖会话定位——即使父会话的消息定位失败（chats 尚未装载等），详情 tab 的
      // 元信息、运行中徽标与跨会话实时更新也照常可用。切换会话后详情 tab 继续实时更新靠它。
      const agentId = typeof data["agentId"] === "string" ? data["agentId"] : "";
      if (agentId) {
        set((state) => ({
          subagentIndex: { ...state.subagentIndex, [agentId]: data as unknown as SubagentPart },
        }));
      }
      // 子代理卡片（决策 D15）：同一 partId 的 running → 中间态 → 终态多次事件按浅合并
      // 收口成一张卡（upsertPart 的合并语义，进度数字在动、卡不增）。定位顺序：已在某消息里
      // → 该消息；否则挂到最后一条 streaming 消息；后台完成时主会话可能已经没有 streaming
      // 消息，回落到最后一条消息。
      const partId = String(data["partId"] ?? "");
      const messageId = findMessageIdForPart(chat, partId) || lastStreamingId(chat) || [...chat.messages].reverse()[0]?.id || "";
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "subagent", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "error": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat) || [...chat.messages].reverse()[0]?.id || "";
      const messages = upsertPart(chat, messageId, partId, "error", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "error.resolved": {
      // 新一轮运行开始即覆盖上一轮失败：摘除全部错误卡片。快照重建（重启/切会话）
      // 由 error_resolved 事件做同样的摘除，两侧保持一致。
      const messages = chat.messages.map((m) =>
        m.parts.some((p) => p.kind === "error") ? { ...m, parts: m.parts.filter((p) => p.kind !== "error") } : m,
      );
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "notice":
    case "context.compressed": {
      // context.compressed 是纯广播（只 emit 不落库、不带 messageId）：它是统计口径的
      // 事件，不产生可见部件——把它的 summary 渲染出来会在对话流里复制一份摘要正文。
      // 可见提示由 notice（落库事件）承担。
      if (event.type === "context.compressed") break;
      const partId = String(data["partId"] ?? event.seq);
      // messageId 由 Recorder 自动注入（事件行的 message_id），与快照重建的分组键同源：
      // 优先用它，实时流与刷新后的落点才能一致。缺省才回落到「最近 streaming」。
      const explicitId = typeof data["messageId"] === "string" ? data["messageId"] : "";
      const messageId = explicitId || lastStreamingId(chat) || [...chat.messages].reverse()[0]?.id || "";
      if (!messageId) break;
      const messages = upsertPart(chat, messageId, partId, "notice", { text: data["text"] ?? data["summary"] ?? "" }, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "user.message": {
      const messageId = String(data["messageId"] ?? "");
      if (chat.messages.some((m) => m.id === messageId)) break;
      const message: MessageSnapshot = {
        id: messageId,
        role: "user",
        parts: [
          {
            id: `part_${messageId}`,
            kind: "text",
            seq: 0,
            payload: {
              text: data["text"] ?? "",
              attachments: data["attachments"] ?? [],
              via: data["via"],
              // 计划模式标记：计划 run 结束后前端据此展示「批准并执行」
              planMode: data["planMode"] === true,
              // 斜杠技能调用（决策 D11）：消息流渲染胶囊头 + 参数正文
              skillInvoke: data["skillInvoke"],
            },
          },
        ],
        status: "complete",
        seq: 0,
        createdAt: event.ts,
      };
      setChat(set, threadId, { ...chat, messages: [...chat.messages, message] });
      break;
    }
    case "usage.updated": {
      // 用量事件：会话累计 token 累加；上下文占用/缓存命中/输出速度是覆盖语义
      // （仅主对话请求携带这些字段，自动命名等 sidecar 小请求缺失时保持原值）。
      // 本地累加只服务实时展示，重拉快照时以后端为权威。
      const input = Number(data["inputTokens"] ?? 0);
      const output = Number(data["outputTokens"] ?? 0);
      const contextTokens = Number(data["contextTokens"] ?? 0);
      const cacheTokens = Number(data["cacheTokens"] ?? 0);
      const speed = Number(data["tokensPerSecond"] ?? 0);
      // 字符 → token 换算率自校准：最近一轮实测（输出字符数 / 输出 token 数），
      // 流式实时速率用它折算，修正 chars/4 对中文的低估（无效值保持原换算率）
      const outputChars = Number(data["outputChars"] ?? 0);
      const patchThreads = (list: ThreadView[]): ThreadView[] =>
        list.map((t) => {
          if (t.id !== threadId) return t;
          const next = { ...t, tokens: t.tokens + input + output };
          if (contextTokens > 0) {
            next.contextTokens = contextTokens;
            next.contextCacheTokens = cacheTokens;
            next.contextSpeed = speed;
            next.contextTtftMs = Number(data["ttftMs"] ?? 0);
          }
          return next;
        });
      set((state) => ({
        threads: { code: patchThreads(state.threads.code), work: patchThreads(state.threads.work) },
        charsPerToken:
          outputChars > 0 && output > 0
            ? { ...state.charsPerToken, [threadId]: Math.max(outputChars / output, 0.1) }
            : state.charsPerToken,
      }));
      break;
    }
    case "thread.updated": {
      // 两个模式都刷：事件不带模式，而后台完成的往往是「非当前模式」的会话——只刷当前模式会让
      // 另一个模式的 hasUnread 永远停在旧值（蓝点不亮、角标不涨）。thread.updated 是低频事件
      // （状态切换 / 改名 / 标记已读），多刷一次的开销可忽略。
      if (data["deleted"] === true) {
        void Promise.all([get().refreshThreads("code"), get().refreshThreads("work")]);
        break;
      }
      void Promise.all([get().refreshThreads("code"), get().refreshThreads("work")]);
      break;
    }
    case "settings.updated": {
      const next = data as unknown as Settings;
      // 主题偏好可能被外部改动：重新应用并同步启动缓存
      if (next.theme !== get().settings.theme) {
        saveThemePref(next.theme);
        set({ themeResolved: applyTheme(next.theme) });
      }
      // 界面模式同理：重放模式主色 class 并同步启动缓存
      if (next.mode !== get().settings.mode) {
        const mode: "code" | "work" = next.mode === "work" ? "work" : "code";
        applyModeTheme(mode);
        saveModePref(mode);
        set({ mode });
      }
      set({ settings: next });
      break;
    }
    default:
      break;
  }
}

function approvalPayloadOf(chat: ChatState, partId: string): Record<string, unknown> {
  for (const m of chat.messages) {
    const part = m.parts.find((p) => p.id === partId);
    if (part) return part.payload;
  }
  return {} as ApprovalPart as unknown as Record<string, unknown>;
}

function lastStreamingId(chat: ChatState): string {
  return [...chat.messages].reverse().find((m) => m.status === "streaming")?.id ?? "";
}

/** 流式增量追加：找到 part 追加文本，没有则创建。 */
function upsertAppend(
  chat: ChatState,
  messageId: string,
  partId: string,
  kind: PartSnapshot["kind"],
  text: string,
): MessageSnapshot[] {
  const messages = [...chat.messages];
  const msgIdx = messages.findIndex((m) => m.id === messageId);
  if (msgIdx < 0) return messages;
  const target = messages[msgIdx]!;
  const parts = [...target.parts];
  const idx = parts.findIndex((p) => p.id === partId);
  if (idx >= 0) {
    const prev = parts[idx]!;
    parts[idx] = { ...prev, payload: { ...prev.payload, text: String(prev.payload["text"] ?? "") + text } };
  } else {
    parts.push({ id: partId, kind, seq: 0, payload: { text } });
  }
  messages[msgIdx] = { ...target, parts };
  return messages;
}

function setChat(set: SetFn, threadId: string, chat: ChatState): void {
  set((state) => ({ chats: { ...state.chats, [threadId]: chat } }));
}

/** run 终态的 durationMs 与结束时刻并入最近一条缺省的消息：折叠块「持续 X 秒」实时显示、
 * 消息时间戳显示结束时刻（快照重建路径按 runs 表计算，事件流此前把这两个字段丢弃）。 */
function mergeWorkDuration(messages: MessageSnapshot[], durationMs: unknown, finishedAt: number): void {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]!;
    if (m.role === "assistant" && (m.workDurationMs === undefined || m.finishedAt === undefined)) {
      messages[i] = {
        ...m,
        workDurationMs: m.workDurationMs ?? (typeof durationMs === "number" && durationMs > 0 ? durationMs : undefined),
        finishedAt: m.finishedAt ?? (finishedAt > 0 ? finishedAt : undefined),
      };
      return;
    }
  }
}

/** 快照与在途事件流的消息合并：快照为序的权威；同 id 消息快照 parts 为空（run 占位）而
 * 事件流已累积增量时保留事件流 parts；事件流独有的 streaming 消息（事件尚未落库）追加到尾部。 */
function mergeStreamedMessages(snapshot: MessageSnapshot[], streamed: MessageSnapshot[]): MessageSnapshot[] {
  if (streamed.length === 0) return snapshot;
  const streamedById = new Map(streamed.map((m) => [m.id, m]));
  const known = new Set(snapshot.map((m) => m.id));
  const merged = snapshot.map((m) => {
    const s = streamedById.get(m.id);
    if (!s) return m;
    if (m.parts.length === 0 && s.parts.length > 0) return { ...m, parts: s.parts };
    if (m.createdAt === 0 && s.createdAt > 0) return { ...m, createdAt: s.createdAt };
    return m;
  });
  const extras = streamed.filter((m) => !known.has(m.id) && (m.status === "streaming" || m.parts.length > 0));
  return [...merged, ...extras];
}

// ---------- 应用角标（macOS 停靠栏：未读或待审批） ----------
// 放在模块级订阅而不是各个写未读的分支里：未读来源分散（切会话清零、后台完成置位、
// 事件回流刷新列表），逐处补调用必漏；订阅以「总数」为选择器，只在数字变化时跨进程写一次。
let lastBadgeCount = -1;

/** 两个模式的待关注总数：未读或等待审批（同一会话只算一次；侧栏跨模式全局计）。 */
function attentionTotal(state: Pick<AppState, "threads">): number {
  return (
    state.threads.code.filter((t) => t.hasUnread || t.status === "awaiting").length +
    state.threads.work.filter((t) => t.hasUnread || t.status === "awaiting").length
  );
}

useApp.subscribe((state) => {
  const total = attentionTotal(state);
  if (total === lastBadgeCount) return;
  lastBadgeCount = total;
  // 角标是纯装饰：失败（非 macOS、窗口未就绪）静默忽略，不影响任何业务路径
  void api.setBadgeCount(total > 0 ? total : null).catch(() => {});
});
