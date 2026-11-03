// 终端会话管理：真实 PTY（独立进程组），输出经推送通道进渲染层。
// 面板终端是用户终端：不经过 Agent 审批；Agent 的 shell_exec 走策略引擎。
import { createRequire } from "node:module";
import type { BrowserWindow } from "electron";
import { PushChannels, type TerminalDataPayload, type TerminalExitPayload } from "../../shared/channels";

const require = createRequire(import.meta.url);
const { spawn: ptySpawn } = require("node-pty") as {
  spawn: (file: string, args: string[], options: Record<string, unknown>) => PtyProcess;
};

interface PtyProcess {
  write(data: string): void;
  resize(cols: number, rows: number): void;
  kill(): void;
  on(data: "data", listener: (chunk: string) => void): void;
  on(data: "exit", listener: (code: number, signal: number) => void): void;
  pid: number;
}

interface Session {
  process: PtyProcess;
  cwd: string;
}

export class TerminalService {
  private sessions = new Map<string, Session>();

  constructor(private readonly getWindow: () => BrowserWindow | null) {}

  create(sessionId: string, cwd: string, rows: number, cols: number): void {
    this.dispose(sessionId);
    const shell = process.env["SHELL"] || "/bin/zsh";
    const pty = ptySpawn(shell, ["-l"], {
      name: "xterm-256color",
      cwd: existsDir(cwd) ? cwd : undefined,
      cols: Math.max(2, cols),
      rows: Math.max(2, rows),
      env: { ...process.env, TERM: "xterm-256color" } as Record<string, string>,
    });
    const session: Session = { process: pty, cwd };
    this.sessions.set(sessionId, session);
    pty.on("data", (chunk) => {
      const win = this.getWindow();
      win?.webContents.send(PushChannels.TerminalData, { sessionId, data: chunk } satisfies TerminalDataPayload);
    });
    pty.on("exit", (code) => {
      this.sessions.delete(sessionId);
      const win = this.getWindow();
      win?.webContents.send(PushChannels.TerminalExit, { sessionId, exitCode: code } satisfies TerminalExitPayload);
    });
  }

  write(sessionId: string, data: string): void {
    this.sessions.get(sessionId)?.process.write(data);
  }

  resize(sessionId: string, rows: number, cols: number): void {
    this.sessions.get(sessionId)?.process.resize(Math.max(2, cols), Math.max(2, rows));
  }

  /** 关闭会话并杀进程组（重开终端 tab 由 renderer 重新 create）。 */
  dispose(sessionId: string): void {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    this.sessions.delete(sessionId);
    try {
      // 杀整个进程组（spawn 时 detached 进程组语义由 node-pty 内部保证）
      if (session.process.pid > 0) {
        try {
          process.kill(-session.process.pid, "SIGKILL");
        } catch {
          session.process.kill();
        }
      }
    } catch {
      // 进程已退出
    }
  }

  disposeAll(): void {
    for (const sessionId of [...this.sessions.keys()]) {
      this.dispose(sessionId);
    }
  }
}

function existsDir(path: string): boolean {
  try {
    return statDir(path);
  } catch {
    return false;
  }
}

function statDir(path: string): boolean {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { statSync } = require("node:fs") as typeof import("node:fs");
  return statSync(path).isDirectory();
}
