/**
 * 终端 store：维护终端会话列表、当前会话、每个会话的输出缓冲区，
 * 并通过后端事件批次把 PTY 输出实时追加到界面上。
 *
 * 输出只保存在内存里，按会话分片；单个会话最多保留最近 20 万个字符，防止长时间运行把内存撑爆。
 */
import { create } from "zustand";
import { api } from "../lib/api";
import { onBatch } from "../lib/bus";
import type { Batch, TerminalView } from "../lib/types";
import { EVENT } from "../lib/types";
import { useUiStore } from "./ui";

/**
 * 终端 store 的状态与操作。
 *
 * - `sessions`：后端返回的会话列表，`running` 表示 PTY 进程是否还活着；
 * - `activeID`：右侧终端面板当前显示的会话；
 * - `outputs`：会话 ID 到已累积输出文本的映射；
 * - `connected`：是否已经订阅过事件，避免重复注册。
 */
interface TerminalState {
  /** 所有终端会话。 */
  sessions: TerminalView[];
  /** 当前显示的会话 ID；没有会话时为 null。 */
  activeID: string | null;
  /** 每个会话累积的输出文本。 */
  outputs: Record<string, string>;
  /** 是否已订阅后端事件，防止重复订阅。 */
  connected: boolean;

  /** 订阅终端相关事件，可重复调用，内部只生效一次。 */
  connect(): void;
  /** 从后端拉取已有会话，默认选中第一个。 */
  boot(): Promise<void>;
  /** 新建一个终端会话并切换到它。 */
  create(): Promise<void>;
  /** 向当前会话写入用户输入。 */
  write(data: string): Promise<void>;
  /** 通知后端当前会话的终端尺寸变化。 */
  resize(cols: number, rows: number): Promise<void>;
  /** 关闭指定会话，并在需要时切换当前会话。 */
  close(sessionID: string): Promise<void>;
  /** 切换当前显示的会话。 */
  setActive(sessionID: string): void;
  /** 向指定会话追加输出，内部按上限截断。 */
  appendOutput(sessionID: string, data: string): void;
}

/**
 * 终端会话与输出的状态容器。
 *
 * 初始状态：没有会话、没有选中项、没有输出、尚未订阅事件。
 */
export const useTerminalStore = create<TerminalState>((set, get) => ({
  sessions: [],
  activeID: null,
  outputs: {},
  connected: false,

  // 事件订阅：一次订阅同时处理两类事件。
  // - terminalOutput：把新增输出追加到对应会话的缓冲区；
  // - terminalExit：把会话标记为已停止，并补一行退出码提示，用户能明确知道进程结束了。
  // 这里在模块级注册一次即可，`connected` 保证 StrictMode 或多次调用不会重复注册。
  connect() {
    if (get().connected) {
      return;
    }
    onBatch((batch: Batch) => {
      for (const event of batch.events) {
        if (event.type === EVENT.terminalOutput) {
          get().appendOutput(event.data.sessionId, event.data.data ?? "");
        } else if (event.type === EVENT.terminalExit) {
          set((state) => ({
            sessions: state.sessions.map((session) =>
              session.id === event.data.sessionId ? { ...session, running: false } : session,
            ),
          }));
          get().appendOutput(event.data.sessionId, `\r\n[进程已退出，退出码 ${event.data.exitCode}]\r\n`);
        }
      }
    });
    set({ connected: true });
  },

  // 启动时补齐会话列表；默认选中第一个，符合「恢复上次工作环境」的预期。
  async boot() {
    const sessions = await api.terminal.list();
    set({ sessions, activeID: sessions[0]?.id ?? null });
  },

  // 新建会话使用空的名称与工作目录，由后端按工作区默认值填充。
  // 成功后追加到列表、切换过去，并初始化空输出缓冲区；失败用 toast 告知。
  async create() {
    try {
      const session = await api.terminal.create("", "");
      set((state) => ({
        sessions: [...state.sessions, session],
        activeID: session.id,
        outputs: { ...state.outputs, [session.id]: "" },
      }));
    } catch (error) {
      useUiStore.getState().toast({ title: "新建终端失败", description: String(error), tone: "danger" });
    }
  },

  // 没有当前会话时直接忽略，避免把输入写到不存在的 PTY。
  async write(data) {
    const id = get().activeID;
    if (!id) {
      return;
    }
    await api.terminal.write(id, data);
  },

  // 尺寸同步属于尽力而为：终端尺寸偶尔同步失败不影响输入输出，因此吞掉错误。
  async resize(cols, rows) {
    const id = get().activeID;
    if (!id) {
      return;
    }
    await api.terminal.resize(id, cols, rows).catch(() => undefined);
  },

  // 关闭会话后从列表移除；如果关闭的正是当前会话，则回退到剩余的会话，没有剩余则置空。
  async close(sessionID) {
    await api.terminal.close(sessionID);
    set((state) => {
      const sessions = state.sessions.filter((session) => session.id !== sessionID);
      return {
        sessions,
        activeID: state.activeID === sessionID ? sessions[0]?.id ?? null : state.activeID,
      };
    });
  },

  setActive(sessionID) {
    set({ activeID: sessionID });
  },

  // 追加输出并保留最近 200000 个字符（slice 负数表示取末尾），
  // 超出部分直接丢弃，保证长时间运行的终端不会无限增长。
  appendOutput(sessionID, data) {
    set((state) => ({
      outputs: {
        ...state.outputs,
        [sessionID]: `${state.outputs[sessionID] ?? ""}${data}`.slice(-200000),
      },
    }));
  },
}));
