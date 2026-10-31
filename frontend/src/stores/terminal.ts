import { create } from "zustand";
import { api } from "../lib/api";
import { onBatch } from "../lib/bus";
import type { Batch, TerminalView } from "../lib/types";
import { EVENT } from "../lib/types";
import { useUiStore } from "./ui";

interface TerminalState {
  sessions: TerminalView[];
  activeID: string | null;
  outputs: Record<string, string>;
  connected: boolean;
  connect(): void;
  boot(): Promise<void>;
  create(): Promise<void>;
  write(data: string): Promise<void>;
  resize(cols: number, rows: number): Promise<void>;
  close(sessionID: string): Promise<void>;
  setActive(sessionID: string): void;
  appendOutput(sessionID: string, data: string): void;
}

export const useTerminalStore = create<TerminalState>((set, get) => ({
  sessions: [],
  activeID: null,
  outputs: {},
  connected: false,

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

  async boot() {
    const sessions = await api.terminal.list();
    set({ sessions, activeID: sessions[0]?.id ?? null });
  },

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

  async write(data) {
    const id = get().activeID;
    if (!id) {
      return;
    }
    await api.terminal.write(id, data);
  },

  async resize(cols, rows) {
    const id = get().activeID;
    if (!id) {
      return;
    }
    await api.terminal.resize(id, cols, rows).catch(() => undefined);
  },

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

  appendOutput(sessionID, data) {
    set((state) => ({
      outputs: {
        ...state.outputs,
        [sessionID]: `${state.outputs[sessionID] ?? ""}${data}`.slice(-200000),
      },
    }));
  },
}));
