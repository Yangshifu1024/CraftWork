// 平台基础能力：原子写、哈希、二进制探测、环境清洗、子进程执行。
// 安全语义必须逐条保留：
// 环境清洗防止模型密钥泄漏给工具子进程；进程组杀防止孤儿进程；有界缓冲防止输出撑爆内存。
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { openSync, closeSync, writeSync, renameSync, unlinkSync, chmodSync } from "node:fs";
import { dirname, basename } from "node:path";

/** 原子写文件：同目录临时文件 → 写入 → 原地 rename。 */
export function writeFileAtomic(path: string, data: Uint8Array | string, mode = 0o644): void {
  const tmp = `${dirname(path)}/.craftwork-${process.pid}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}.tmp`;
  try {
    const fd = openSync(tmp, "wx", mode);
    try {
      writeSync(fd, typeof data === "string" ? Buffer.from(data, "utf8") : Buffer.from(data));
    } finally {
      closeSync(fd);
    }
    try {
      chmodSync(tmp, mode);
    } catch {
      // chmod 失败不影响写入结果
    }
    renameSync(tmp, path);
  } catch (err) {
    try {
      unlinkSync(tmp);
    } catch {
      // 清理失败忽略
    }
    throw err;
  }
}

export function sha256Hex(data: Uint8Array | string): string {
  return createHash("sha256").update(data).digest("hex");
}

/** 启发式二进制判定：前 8000 字节含 NUL 即视为二进制。 */
export function isBinary(data: Uint8Array): boolean {
  const n = Math.min(data.length, 8000);
  for (let i = 0; i < n; i++) {
    if (data[i] === 0) return true;
  }
  return false;
}

/** 备份文件名：<base>.<yyyy-MM-dd-HH-mm-ss.SSS>（毫秒级时间戳）。 */
export function backupName(absPath: string): string {
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  const d = new Date();
  const ts =
    `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}` +
    `-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
  return `${basename(absPath)}.${ts}`;
}

// ---------- 环境清洗 ----------

/** 子进程环境白名单：只保留无害变量。 */
const ENV_ALLOW = new Set([
  "PATH",
  "HOME",
  "USER",
  "LOGNAME",
  "SHELL",
  "LANG",
  "TERM",
  "TMPDIR",
  "TZ",
  "PWD",
  "COLORTERM",
  "SSH_AUTH_SOCK",
]);

const ENV_DENY_SUBSTR = ["API_KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];
const ENV_DENY_PREFIX = ["AWS_", "GITHUB_", "GH_", "OPENAI_", "ANTHROPIC_", "DEEPSEEK_", "ZHIPU_", "DASHSCOPE_"];

/**
 * 清洗当前进程环境：白名单 + 敏感词剔除 + 云厂商标识前缀剔除。
 * 目的：防止模型密钥通过环境变量泄漏给工具子进程。
 */
export function sanitizedEnv(): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (value === undefined) continue;
    const upper = key.toUpperCase();
    if (ENV_ALLOW.has(key) || upper.startsWith("LC_")) {
      env[key] = value;
      continue;
    }
    if (ENV_DENY_SUBSTR.some((s) => upper.includes(s))) continue;
    if (ENV_DENY_PREFIX.some((p) => upper.startsWith(p))) continue;
  }
  env["CRAFTWORK"] = "1";
  return env;
}

// ---------- 子进程执行 ----------

export interface RunSpec {
  /** 可执行文件路径（如 /bin/sh）；执行命令时由调用方拼 [-c, command] */
  file: string;
  args: string[];
  dir?: string;
  timeoutMs?: number;
  outputLimit?: number;
}

export interface RunResult {
  output: string;
  exitCode: number; // 超时/信号 = -1
  timedOut: boolean;
  error?: string; // 非零退出码不算错
}

/** 有界缓冲：满后丢弃后续写入并记录溢出，读取时追加截断说明。 */
class LimitedBuffer {
  private chunks: string[] = [];
  private length = 0;
  private overflow = false;
  constructor(private readonly limit: number) {}

  write(text: string): void {
    if (this.overflow) return;
    if (this.length + text.length > this.limit) {
      this.chunks.push(text.slice(0, Math.max(0, this.limit - this.length)));
      this.length = this.limit;
      this.overflow = true;
      return;
    }
    this.chunks.push(text);
    this.length += text.length;
  }

  toString(): string {
    const base = this.chunks.join("");
    return this.overflow ? `${base}\n…（输出已截断）` : base;
  }
}

/** 杀死整个进程组（子进程以独立进程组启动）。 */
function killProcessGroup(pid: number): void {
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // 进程已退出
    }
  }
}

/**
 * 执行子进程：独立进程组、清洗环境、共享 stdout/stderr 有界缓冲、超时优先上报。
 * 超时优先于其他错误上报。
 */
export function run(spec: RunSpec): Promise<RunResult> {
  const limit = spec.outputLimit ?? 32 * 1024;
  const timeoutMs = spec.timeoutMs ?? 120_000;
  return new Promise((resolve) => {
    let child: ReturnType<typeof spawn>;
    try {
      child = spawn(spec.file, spec.args, {
        cwd: spec.dir || undefined,
        env: sanitizedEnv(),
        detached: process.platform !== "win32", // 独立进程组，便于整组杀
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      });
    } catch (err) {
      resolve({ output: "", exitCode: -1, timedOut: false, error: String(err) });
      return;
    }

    const buf = new LimitedBuffer(limit);
    let settled = false;
    let timedOut = false;
    let spawnErr: string | undefined;

    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve({
        output: buf.toString(),
        exitCode: child.exitCode ?? (timedOut ? -1 : child.signalCode ? -1 : 0),
        timedOut,
        error: spawnErr,
      });
    };

    const timer = setTimeout(() => {
      timedOut = true;
      if (child.pid !== undefined) killProcessGroup(child.pid);
      // 给 Wait 留出收尾时间
      setTimeout(finish, 200);
    }, timeoutMs);

    child.stdout?.on("data", (d: Buffer) => buf.write(d.toString("utf8")));
    child.stderr?.on("data", (d: Buffer) => buf.write(d.toString("utf8")));
    child.on("error", (err) => {
      spawnErr = String(err);
      finish();
    });
    child.on("close", () => finish());
  });
}
