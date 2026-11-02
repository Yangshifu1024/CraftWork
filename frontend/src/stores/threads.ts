/**
 * 会话 store。
 *
 * 维护左侧会话列表、当前会话的消息与运行状态、待审批队列、搜索结果以及附件草稿，
 * 并把后端推送的事件批次交给 threadReducer 合并成界面状态。
 *
 * 约定：会改动后端数据的 action 都先调用命令、成功后再更新本地状态；
 * 纯读取失败只记录日志，需要用户感知的失败用 toast 提示。
 */
import { create } from "zustand";
import { api } from "../lib/api";
import { onBatch } from "../lib/bus";
import type {
  ApprovalPartData,
  AttachmentInfo,
  Batch,
  ChatMessage,
  ChatPart,
  Mode,
  ThreadSnapshot,
  ThreadView,
} from "../lib/types";
import { uid } from "../lib/utils";
import { reduceThread, updateThread } from "./threadReducer";
import { useAppStore } from "./app";
import { useUiStore } from "./ui";
import { useWorkspaceStore } from "./workspace";

/**
 * 会话 store 的完整状态与可用操作。
 *
 * 字段是界面的只读快照，操作是唯一入口；组件通过选择器订阅需要的字段。
 */
interface ThreadState {
  /** 两种模式各自的会话列表。 */
  threads: Record<Mode, ThreadView[]>;
  /** 两种模式各自选中的会话 id，切换模式时互不影响。 */
  activeID: Record<Mode, string | null>;
  /** 当前会话的消息，按时间正序排列。 */
  messages: ChatMessage[];
  /** 当前会话是否有正在进行的运行。 */
  running: boolean;
  /** 取消确认已就绪：双击 Esc 的第一击已按下，再次按 Esc 才会真正取消。 */
  interruptArmed: boolean;
  /** 正在流式输出的 assistant 消息 id，事件处理靠它定位目标消息。 */
  runningMessageID: string | null;
  /** 是否正在读取会话快照，界面据此显示加载状态。 */
  loading: boolean;
  /** 当前会话里等待用户决定的审批。 */
  pendingApprovals: ApprovalPartData[];
  /** 搜索命中结果；为 null 表示当前没有进行搜索。 */
  searchResults: ThreadView[] | null;
  /** 待发送的附件草稿，最多保留 20 个。 */
  attachments: AttachmentInfo[];
  /** 是否已经订阅事件总线，避免重复注册监听。 */
  connected: boolean;
  /** 当前会话是否还有更早的消息可以继续加载。 */
  hasEarlier: boolean;
  /** 是否正在加载更早的消息。 */
  loadingEarlier: boolean;
  /** 订阅事件总线；重复调用无副作用。 */
  connect(): void;
  /** 启动初始化：读取会话列表并选中合适的会话。 */
  boot(): Promise<void>;
  /** 重新拉取指定模式（默认当前模式）的会话列表。 */
  refresh(mode?: Mode): Promise<void>;
  /** 选中会话，并加载它的最新一页快照。 */
  select(threadID: string): Promise<void>;
  /** 向前翻页，加载更早的消息。 */
  loadEarlier(): Promise<void>;
  /** 在当前模式下新建会话并选中。 */
  create(): Promise<void>;
  /** 新建临时会话：切到内置临时目录并新建绑定它的会话。 */
  createTemp(): Promise<void>;
  /** 复制一个会话，标题追加「（副本）」。 */
  duplicate(threadID: string): Promise<void>;
  /** 修改会话标题。 */
  rename(threadID: string, title: string): Promise<void>;
  /** 归档或取消归档会话。 */
  archive(threadID: string, archived: boolean): Promise<void>;
  /** 永久删除会话。 */
  remove(threadID: string): Promise<void>;
  /** 修改会话使用的模型。 */
  setModel(threadID: string, modelID: string): Promise<void>;
  /** 发送消息；没有当前会话时先自动新建。 */
  send(text: string, attachments?: string[]): Promise<void>;
  /** 运行中追加引导（steer）；仅当前会话在运行中时生效。 */
  steer(text: string, attachments?: string[]): Promise<void>;
  /** 取消当前会话正在进行的运行。 */
  cancel(): Promise<void>;
  /** 标记取消确认就绪（双击 Esc 的第一击）。 */
  armInterrupt(): void;
  /** 解除取消确认标记。 */
  disarmInterrupt(): void;
  /** 让当前会话的最后一轮回复重新生成。 */
  regenerate(): Promise<void>;
  /** 提交审批决定。 */
  decide(approvalID: string, allow: boolean, scope: string): Promise<void>;
  /** 按关键词搜索会话。 */
  search(query: string): Promise<void>;
  /** 清空搜索结果。 */
  clearSearch(): void;
  /** 追加附件草稿，自动去重并限制数量。 */
  addAttachments(paths: string[]): void;
  /** 移除单个附件草稿。 */
  removeAttachment(path: string): void;
  /** 清空全部附件草稿。 */
  clearAttachments(): void;
  /** 把一批事件合并进 store；由事件总线回调调用。 */
  applyBatch(batch: Batch): void;
}

/**
 * 把后端快照里的消息转换成前端消息结构。
 *
 * 后端字段是可选的（分页接口可能省略），这里统一补齐：
 * 缺失的状态按「已完成」处理，缺失的部件列表按空数组处理。
 */
function normalizeMessages(threadID: string, snapshot: ThreadSnapshot): ChatMessage[] {
  return (snapshot.messages ?? []).map((message) => ({
    id: message.id,
    threadId: threadID,
    role: message.role as ChatMessage["role"],
    status: (message.status as ChatMessage["status"]) ?? "complete",
    createdAt: message.createdAt,
    // 后端回填的运行工时（「已工作 N」的事实来源），可选字段直接透传。
    ...(message.workDurationMs ? { workDurationMs: message.workDurationMs } : {}),
    parts: (message.parts ?? []).map((part) => ({
      id: part.id,
      kind: part.kind as ChatPart["kind"],
      seq: part.seq,
      data: part.data,
    })),
  }));
}

/**
 * 从消息里收集仍在等待决定的审批。
 *
 * 用于加载历史快照后恢复待审批队列：后端只把审批记录在消息部件里，
 * 前端据此判断是否有审批需要用户处理，避免重启后审批入口消失。
 */
function collectApprovals(messages: ChatMessage[]): ApprovalPartData[] {
  const result: ApprovalPartData[] = [];
  for (const message of messages) {
    for (const part of message.parts) {
      if (part.kind === "approval" && part.data?.status === "waiting") {
        result.push(part.data as ApprovalPartData);
      }
    }
  }
  return result;
}

/** 会话 store：界面读取会话数据、触发会话操作的唯一入口。 */
export const useThreadStore = create<ThreadState>((set, get) => ({
  threads: { code: [], work: [] },
  activeID: { code: null, work: null },
  messages: [],
  running: false,
  interruptArmed: false,
  runningMessageID: null,
  loading: false,
  pendingApprovals: [],
  searchResults: null,
  attachments: [],
  connected: false,
  hasEarlier: false,
  loadingEarlier: false,

  /**
   * 订阅事件总线。
   *
   * 用 `connected` 做幂等保护：热更新或组件重复挂载时不会注册出多个监听。
   */
  connect() {
    if (get().connected) {
      return;
    }
    onBatch((batch) => get().applyBatch(batch));
    set({ connected: true });
  },

  /**
   * 启动初始化。
   *
   * 先拉当前模式的会话列表，再优先恢复上次选中的会话；
   * 选中项已不存在时退回列表第一条，列表为空则清空消息区并退出运行状态。
   */
  async boot() {
    const mode = useAppStore.getState().mode;
    await get().refresh(mode);
    const list = get().threads[mode];
    const current = get().activeID[mode];
    const target = list.find((thread) => thread.id === current) ?? list[0];
    if (target) {
      await get().select(target.id);
    } else {
      set((state) => ({
        messages: [],
        activeID: { ...state.activeID, [mode]: null },
        running: false,
        runningMessageID: null,
        pendingApprovals: [],
      }));
    }
  },

  /**
   * 重新拉取会话列表。
   *
   * 不传模式时以应用当前模式为准；读取失败只记录日志，保留旧列表继续可用。
   */
  async refresh(mode) {
    const target = mode ?? useAppStore.getState().mode;
    try {
      const list = await api.threads.list(target, false);
      set((state) => ({ threads: { ...state.threads, [target]: list } }));
    } catch (error) {
      console.error("读取会话列表失败", error);
    }
  },

  /**
   * 选中会话并加载最新一页快照。
   *
   * 先把选中 id 写进当前模式的槽位，同时清空旧消息与旧审批，避免切换瞬间
   * 显示上一个会话的内容；随后读取快照并据此恢复消息、运行状态与待审批队列。
   */
  async select(threadID) {
    const mode = useAppStore.getState().mode;
    // 选中状态按模式隔离，切换 code / work 时各自记住自己的会话。
    set((state) => ({
      activeID: { ...state.activeID, [mode]: threadID },
      loading: true,
      messages: [],
      pendingApprovals: [],
    }));
    try {
      // 首屏只取最近 80 条；更早的消息由 loadEarlier 按需向前翻页。
      const snapshot = await api.threads.snapshotPage(threadID, 80, 0);
      const messages = normalizeMessages(threadID, snapshot);
      // 运行中可能有最后一条仍未写完的消息，事件处理需要它的 id 作为写入目标。
      const lastStreaming = [...messages].reverse().find((message) => message.status === "streaming");
      set({
        messages,
        // awaiting 表示正在等审批，对界面来说同样属于「运行中」。
        running: snapshot.thread.status === "running" || snapshot.thread.status === "awaiting",
        runningMessageID: lastStreaming?.id ?? null,
        pendingApprovals: collectApprovals(messages),
        loading: false,
        hasEarlier: snapshot.hasMore ?? false,
        loadingEarlier: false,
        // 快照里的会话状态可能比列表新，顺手同步回会话列表。
        threads: updateThread(get().threads, threadID, {
          status: snapshot.thread.status as ThreadView["status"],
        }),
      });
    } catch (error) {
      console.error("读取会话快照失败", error);
      set({ loading: false });
    }
  },

  /**
   * 加载更早的消息。
   *
   * 以前端当前第一条消息的最小部件序号作为游标向后端翻页；
   * 新旧消息按 id 去重后拼接到前面，保证不会因为事件补写而出现重复气泡。
   */
  async loadEarlier() {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    // 没有当前会话、已经加载到头，或正在加载时直接返回，避免重复请求。
    if (!threadID || !get().hasEarlier || get().loadingEarlier) {
      return;
    }
    const firstMessage = get().messages[0];
    // 部件序号从 1 开始，0 表示占位；取其中最小值作为「更早」的分界。
    const firstSeq = firstMessage?.parts.reduce(
      (smallest, part) => (part.seq > 0 && part.seq < smallest ? part.seq : smallest),
      Number.MAX_SAFE_INTEGER,
    );
    // 第一条消息没有任何有效序号时无法继续翻页，直接标记为没有更早消息。
    if (!firstSeq || firstSeq === Number.MAX_SAFE_INTEGER) {
      set({ hasEarlier: false });
      return;
    }
    set({ loadingEarlier: true });
    try {
      const snapshot = await api.threads.snapshotPage(threadID, 80, firstSeq);
      const older = normalizeMessages(threadID, snapshot);
      const current = get().messages;
      const existing = new Set(current.map((message) => message.id));
      // 只保留当前列表里还没有的消息，再整体拼到前面。
      const prepended = older.filter((message) => !existing.has(message.id));
      const merged = [...prepended, ...current];
      set({
        messages: merged,
        hasEarlier: snapshot.hasMore ?? false,
        loadingEarlier: false,
        // 更早的历史里可能还有未处理的审批，重新汇总一次。
        pendingApprovals: collectApprovals(merged),
      });
    } catch (error) {
      set({ loadingEarlier: false });
      useUiStore.getState().toast({
        title: "加载更早消息失败",
        description: String(error),
        tone: "danger",
      });
    }
  },

  /**
   * 新建会话。
   *
   * 归属当前模式与当前工作区；成功后把新会话插到列表最前面并立即选中，
   * 同时清空消息区，让界面呈现一个空对话。
   */
  async create() {
    const app = useAppStore.getState();
    try {
      const thread = await api.threads.create(app.mode, app.activeWorkspace?.id ?? "", "");
      set((state) => ({
        threads: { ...state.threads, [app.mode]: [thread, ...state.threads[app.mode]] },
        activeID: { ...state.activeID, [app.mode]: thread.id },
        messages: [],
        pendingApprovals: [],
        running: false,
        runningMessageID: null,
      }));
    } catch (error) {
      console.error("新建会话失败", error);
    }
  },

  /**
   * 新建临时会话。
   *
   * 先切到内置的临时会话目录（purpose = conversation 的工作区），文件树
   * 与终端随活动工作区一起落到临时目录，再按普通流程新建会话。已经在
   * 临时目录时直接新建，不重复切换。内置目录缺失（初始化失败的极端
   * 情况）时提示并保持现状。
   */
  async createTemp() {
    const app = useAppStore.getState();
    const conversation = app.workspaces.find((workspace) => workspace.purpose === "conversation");
    if (!conversation) {
      useUiStore.getState().toast({
        title: "临时会话不可用",
        description: "内置临时目录尚未初始化，请重启应用后重试",
        tone: "warn",
      });
      return;
    }
    if (app.activeWorkspace?.id !== conversation.id) {
      await app.setActiveWorkspace(conversation.id);
      useWorkspaceStore.getState().reset();
      await useWorkspaceStore.getState().loadRoot();
    }
    await get().create();
  },

  /**
   * 复制会话。
   *
   * 优先沿用当前工作区，没有时退回源会话所属的工作区；
   * 复制完成后重新拉列表（后端会给副本一个新的位置），再选中副本。
   */
  async duplicate(threadID) {
    const mode = useAppStore.getState().mode;
    const source = get().threads[mode].find((thread) => thread.id === threadID);
    if (!source) {
      return;
    }
    const app = useAppStore.getState();
    const thread = await api.threads.create(
      mode,
      app.activeWorkspace?.id ?? source.workspaceId,
      `${source.title}（副本）`,
    );
    await get().refresh(mode);
    await get().select(thread.id);
  },

  /** 重命名会话：以服务端结果为准，成功后同步列表里的标题。 */
  async rename(threadID, title) {
    await api.threads.rename(threadID, title);
    set((state) => ({ threads: updateThread(state.threads, threadID, { title }) }));
  },

  /**
   * 归档或取消归档会话。
   *
   * 列表默认不包含归档会话，因此操作后必须重新拉取；
   * 若归档的正是当前会话，则切到列表第一条，没有可选会话时清空消息区。
   */
  async archive(threadID, archived) {
    await api.threads.archive(threadID, archived);
    const mode = useAppStore.getState().mode;
    await get().refresh(mode);
    if (archived && get().activeID[mode] === threadID) {
      const next = get().threads[mode][0];
      if (next) {
        await get().select(next.id);
      } else {
        set({ messages: [], activeID: { ...get().activeID, [mode]: null } });
      }
    }
  },

  /**
   * 删除会话。
   *
   * 与归档类似：删除后刷新列表；如果删掉的是当前会话，
   * 自动选中剩下的第一条，避免界面停留在已经不存在的数据上。
   */
  async remove(threadID) {
    await api.threads.remove(threadID);
    const mode = useAppStore.getState().mode;
    await get().refresh(mode);
    if (get().activeID[mode] === threadID) {
      const next = get().threads[mode][0];
      if (next) {
        await get().select(next.id);
      } else {
        set({ messages: [], activeID: { ...get().activeID, [mode]: null } });
      }
    }
  },

  /** 切换会话使用的模型：先写入后端，再更新列表里的模型字段。 */
  async setModel(threadID, modelID) {
    await api.threads.setModel(threadID, modelID);
    set((state) => ({ threads: updateThread(state.threads, threadID, { modelId: modelID }) }));
  },

  /**
   * 发送消息。
   *
   * 没有当前会话时先自动新建一个；内容为空且没有附件时不做任何事。
   * 发送前先把用户消息乐观地插入列表，让界面立刻看到自己的输入，
   * 后端确认后再清空附件草稿；调用失败则退出运行状态并提示。
   */
  async send(text, attachments = []) {
    const app = useAppStore.getState();
    const mode = app.mode;
    let threadID = get().activeID[mode];
    if (!threadID) {
      await get().create();
      threadID = get().activeID[mode];
    }
    // 新建也拿不到会话 id 时只能放弃，避免把消息发到空会话。
    if (!threadID) {
      return;
    }
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) {
      return;
    }

    // 乐观消息：前端先构造一条已完成的用户消息，助手回复随后由事件流补上。
    const userMessage: ChatMessage = {
      id: uid("msg"),
      threadId: threadID,
      role: "user",
      status: "complete",
      createdAt: Date.now(),
      parts: [
        {
          id: uid("part"),
          kind: "text",
          seq: 0,
          data: { text: trimmed, attachments },
        },
      ],
    };
    set((state) => ({ messages: [...state.messages, userMessage], running: true }));
    try {
      await api.chat.send(threadID, trimmed, attachments);
      // 只有后端接受后才清空附件，失败时保留草稿方便重发。
      get().clearAttachments();
    } catch (error) {
      set({ running: false });
      useUiStore.getState().toast({
        title: "发送失败",
        description: String(error),
        tone: "danger",
      });
    }
  },

  /**
   * 运行中追加引导（steer）。
   *
   * 与 send 的区别：不做乐观插入——后端落库后会广播 user.message 事件，
   * 由 reducer 插入气泡，避免同一条消息出现两份。后端会在下一个工具
   * 批次结束后把它并入模型请求；会话恰好已结束时后端自动回退为普通
   * 发送，前端无感。调用失败只提示，不改动运行状态。
   */
  async steer(text, attachments = []) {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    // 不在运行中时直接忽略：composer 的分流逻辑保证正常交互不会走到这里。
    if (!threadID || !get().running) {
      return;
    }
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) {
      return;
    }
    try {
      await api.chat.steer(threadID, trimmed, attachments);
      // 只有后端接受后才清空附件草稿，失败时保留方便重发。
      get().clearAttachments();
    } catch (error) {
      useUiStore.getState().toast({
        title: "引导发送失败",
        description: String(error),
        tone: "danger",
      });
    }
  },

  /** 取消当前会话的运行；本地立即退出运行状态，不等后端事件回来。 */
  async cancel() {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    if (!threadID) {
      return;
    }
    await api.chat.cancel(threadID);
    set({ running: false, interruptArmed: false });
  },

  armInterrupt() {
    set({ interruptArmed: true });
  },

  disarmInterrupt() {
    set({ interruptArmed: false });
  },

  /**
   * 重新生成最后一轮回复。
   *
   * 立即标记为运行中让按钮进入停止状态；命令失败时回退标记并提示。
   */
  async regenerate() {
    const mode = useAppStore.getState().mode;
    const threadID = get().activeID[mode];
    if (!threadID) {
      return;
    }
    set({ running: true });
    try {
      await api.chat.regenerate(threadID);
    } catch (error) {
      set({ running: false });
      useUiStore.getState().toast({ title: "重新生成失败", description: String(error), tone: "danger" });
    }
  },

  /**
   * 提交审批决定。
   *
   * 采用乐观更新：先按用户的点击把审批部件改成已批准 / 已拒绝并移出队列，
   * 让按钮立刻响应；命令失败时把待审批队列恢复成提交前的快照并提示。
   */
  async decide(approvalID, allow, scope) {
    const previous = get().pendingApprovals;
    const nextStatus = allow ? "approved" : "denied";
    set((state) => ({
      pendingApprovals: state.pendingApprovals.filter((item) => item.approvalId !== approvalID),
      messages: state.messages.map((message) => ({
        ...message,
        parts: message.parts.map((part) =>
          part.kind === "approval" && part.data?.approvalId === approvalID
            ? { ...part, data: { ...part.data, status: nextStatus, decision: allow ? "allow" : "deny", scope } }
            : part,
        ),
      })),
    }));
    try {
      await api.chat.decideApproval(approvalID, allow, scope);
    } catch (error) {
      set({ pendingApprovals: previous });
      useUiStore.getState().toast({ title: "审批提交失败", description: String(error), tone: "danger" });
    }
  },

  /** 按关键词搜索会话；关键词为空时清空结果，恢复普通列表。 */
  async search(query) {
    const mode = useAppStore.getState().mode;
    if (!query.trim()) {
      set({ searchResults: null });
      return;
    }
    const results = await api.threads.search(mode, query);
    set({ searchResults: results });
  },

  /** 退出搜索状态，清空命中结果。 */
  clearSearch() {
    set({ searchResults: null });
  },

  /**
   * 追加附件草稿。
   *
   * 依据路径去重，重复选择同一个文件不会产生第二条记录；
   * 名称取路径最后一段；整体最多保留 20 个，避免一次发送过多文件。
   */
  addAttachments(paths) {
    const existing = new Set(get().attachments.map((item) => item.path));
    const next = paths
      .filter((path) => path && !existing.has(path))
      .map((path) => ({ path, name: path.split("/").pop() ?? path }));
    set((state) => ({ attachments: [...state.attachments, ...next].slice(0, 20) }));
  },

  /** 按路径移除单个附件草稿。 */
  removeAttachment(path) {
    set((state) => ({ attachments: state.attachments.filter((item) => item.path !== path) }));
  },

  /** 清空全部附件草稿，通常在一次发送成功后调用。 */
  clearAttachments() {
    set({ attachments: [] });
  },

  /**
   * 把事件批次合并进 store。
   *
   * 只把处理需要的字段交给 reduceThread，当前会话 id 用来判断哪些事件属于本会话；
   * 处理结果显示需要刷新时（其他会话有更新），异步重新拉取会话列表，不阻塞事件处理。
   */
  applyBatch(batch) {
    const state = get();
    const mode = useAppStore.getState().mode;
    const result = reduceThread(
      {
        messages: state.messages,
        running: state.running,
        runningMessageID: state.runningMessageID,
        pendingApprovals: state.pendingApprovals,
        threads: state.threads,
      },
      batch,
      state.activeID[mode] ?? "",
    );
    set({
      messages: result.messages,
      running: result.running,
      runningMessageID: result.runningMessageID,
      pendingApprovals: result.pendingApprovals,
      threads: result.threads,
    });
    if (result.refreshNeeded) {
      void get().refresh(mode);
    }
  },
}));
