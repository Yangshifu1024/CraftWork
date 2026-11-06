// 渲染层唯一状态壳（zustand）：会话列表、消息流、草稿槽、设置与界面偏好。
// 数据流 = 快照装载 + 事件流更新：切会话时经 threadSnapshot 装载消息，此后一切变化
// 由 applyBatch 应用后端推送的事件批次（渲染层唯一事件入口）；系统操作全部经 lib/api（通道表）。
import { create } from "zustand";
import type { Batch, Envelope } from "../../../shared/events";
import type {
  MessageSnapshot,
  PlanInfo,
  PartSnapshot,
  Settings,
  ThreadView,
  WorkspaceView,
} from "../../../shared/types";
import type { ApprovalPart } from "../../../shared/events";
import { api } from "../lib/api";
import { applyModeTheme, applyTheme, saveModePref, saveThemePref, watchSystemTheme } from "../lib/theme";
import { emptyDraftTable, loadDrafts, saveDrafts, loadThreadComposer, saveThreadComposer } from "../lib/drafts";
import type { DraftTable } from "../lib/drafts";
import { useToasts } from "./toast";

/** 工作模式：Code（开发者工作台）/ Work（文档处理工作台）。 */
export type Mode = "code" | "work";

/** 消息 + 部件的运行时可变视图（快照结构 + 流式增量）。 */
export interface ChatState {
  messages: MessageSnapshot[];
  running: boolean;
  lastSeq: number;
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
  workspaces: WorkspaceView[];
  settings: Settings;
  /** 已装载会话的运行时消息流（按需装载，不常驻全部历史）。 */
  /** 会话级 composer 桶：会话态输入区的文本/附件/发送选项，按会话 id 分桶（草稿态仍走草稿槽）；
   *  切换会话互不干扰、往返保留；持久化经 lib/drafts，删除会话时清理、boot 按现存会话裁剪。 */
  threadComposer: Record<string, ThreadComposer>;
  /** 写会话级 composer 桶（合并 patch + 持久化）；仅会话态使用，草稿态走 setDraft。 */
  setThreadComposer(id: string, patch: Partial<ThreadComposer>): void;
  chats: Record<string, ChatState>;
  /** 各会话的最新计划（计划模式产物；无计划/未装载为 undefined）。
   *  「批准并执行」提示条依据：filePath 非空且未批准——中断/失败的计划无文件不出现。 */
  plans: Record<string, PlanInfo | undefined>;
  sidebarVisible: boolean;
  /** 已解析主题类（system 偏好下随系统亮暗变化更新）。 */
  themeResolved: "theme-craft-light" | "theme-craft-dark";

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
  /** 批准计划（approved 置 1，提示条消失）；确认消息由调用方随后发送。 */
  approvePlan(threadId: string): Promise<void>;
  ignorePlan(threadId: string): Promise<void>;
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
  workspaces: [],
  settings: {
    theme: "system",
    mode: "code",
    panelWidth: 320,
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
    maxIterations: 300,
    logsLevel: "info",
    smoothFollow: false,
    inputPrediction: true,
  },
  chats: {},
  plans: {},
  sidebarVisible: true,
  themeResolved: "theme-craft-dark",

  async boot() {
    const [settings, threadsCode, threadsWork, workspaces] = await Promise.all([
      api.settingsGet(),
      api.threadList("code", false),
      api.threadList("work", false),
      api.workspaceList(),
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
    // 会话的最新计划（批准并执行提示条的数据源）；失败降级为无计划
    await api
      .planGet(id)
      .then((plan) => set((state) => ({ plans: { ...state.plans, [id]: plan ?? undefined } })))
      .catch(() => {});
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

  /** 批准计划：后端置 approved，本地同步（提示条消失），随后调用方发送确认消息。 */
  async approvePlan(threadId: string): Promise<void> {
    const plan = get().plans[threadId];
    if (!plan) return;
    await api.planApprove(plan.id);
    set((state) => ({
      plans: { ...state.plans, [threadId]: { ...plan, approved: true } },
    }));
  },
  /** 忽略计划：ignored 置 1（提示条消失），不做任何后续动作。 */
  async ignorePlan(threadId: string): Promise<void> {
    const plan = get().plans[threadId];
    if (!plan) return;
    await api.planIgnore(plan.id);
    set((state) => ({
      plans: { ...state.plans, [threadId]: { ...plan, ignored: true } },
    }));
  },

  toggleSidebar() {
    set((state) => ({ sidebarVisible: !state.sidebarVisible }));
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
      setChat(set, threadId, { ...chat, messages });
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
      break;
    }
    case "plan.created": {
      const planThreadId = event.threadId ?? threadId;
      set((state) => ({
        plans: {
          ...state.plans,
          [planThreadId]: {
            id: String(data["planId"] ?? ""),
            threadId: planThreadId,
            filePath: String(data["filePath"] ?? ""),
            approved: false,
            ignored: false,
            updatedAt: typeof data["updatedAt"] === "number" ? data["updatedAt"] : 0,
          },
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
    case "error": {
      const partId = String(data["partId"] ?? "");
      const messageId = lastStreamingId(chat) || [...chat.messages].reverse()[0]?.id || "";
      const messages = upsertPart(chat, messageId, partId, "error", data, event.ts);
      setChat(set, threadId, { ...chat, messages });
      break;
    }
    case "notice":
    case "context.compressed": {
      const partId = String(data["partId"] ?? event.seq);
      const messageId = lastStreamingId(chat) || [...chat.messages].reverse()[0]?.id || "";
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
      const patchThreads = (list: ThreadView[]): ThreadView[] =>
        list.map((t) => {
          if (t.id !== threadId) return t;
          const next = { ...t, tokens: t.tokens + input + output };
          if (contextTokens > 0) {
            next.contextTokens = contextTokens;
            next.contextCacheTokens = cacheTokens;
            next.contextSpeed = speed;
          }
          return next;
        });
      set((state) => ({ threads: { code: patchThreads(state.threads.code), work: patchThreads(state.threads.work) } }));
      break;
    }
    case "thread.updated": {
      if (data["deleted"] === true) {
        void get().refreshThreads();
        break;
      }
      void get().refreshThreads();
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
