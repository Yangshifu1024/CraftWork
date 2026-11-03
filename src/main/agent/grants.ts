// 授权缓存（对应策略层的「本会话 / 本工作区允许」语义）。
// 会话授权只在内存（进程退出失效）；工作区授权写设置（重启有效）。
// 键：精确 `tool|target`；前缀 `tool|prefix:<prefix>`（仅 shell_exec 使用）。
import { setSetting, getSetting } from "../store/records";
import type { DB } from "../store/db";

const WORKSPACE_GRANTS_KEY = "policy.workspace_grants";

export class Grants {
  private memory = new Set<string>();

  constructor(private readonly db: DB) {}

  allowed(tool: string, target: string): boolean {
    if (target && this.memory.has(`${tool}|${target}`)) return true;
    if (tool === "shell_exec" && target) {
      const prefix = commandPrefix(target);
      if (prefix && this.memory.has(`shell_exec|prefix:${prefix}`)) return true;
    }
    const workspace = getSetting<Record<string, boolean>>(this.db, WORKSPACE_GRANTS_KEY, {});
    if (target && workspace[`${tool}|${target}`] === true) return true;
    if (tool === "shell_exec" && target) {
      const prefix = commandPrefix(target);
      if (prefix && workspace[`shell_exec|prefix:${prefix}`] === true) return true;
    }
    return false;
  }

  grant(tool: string, target: string, scope: "session" | "workspace"): void {
    const key = `${tool}|${target}`;
    this.memory.add(key);
    if (scope === "workspace") {
      const workspace = { ...getSetting<Record<string, boolean>>(this.db, WORKSPACE_GRANTS_KEY, {}) };
      workspace[key] = true;
      setSetting(this.db, WORKSPACE_GRANTS_KEY, workspace);
    }
  }

  snapshot(): string[] {
    return [...this.memory];
  }
}

/**
 * 命令授权前缀：空/含分隔符或替换语义 → 无前缀；
 * 首段是 shell 解释器 → 无前缀；只有一段或第二段是选项 → 前缀=第一段；否则前两段。
 */
export function commandPrefix(command: string): string {
  const trimmed = command.trim();
  if (!trimmed) return "";
  const rejectChars = [";", "|", "&", ">", "<", "`", "\n", "\r", "$(", "${"];
  if (rejectChars.some((c) => trimmed.includes(c))) return "";
  const parts = trimmed.split(/\s+/);
  const interpreters = new Set(["env", "sudo", "bash", "sh", "zsh", "fish", "powershell", "cmd", "eval", "exec"]);
  const first = parts[0] ?? "";
  if (parts.length === 1 || interpreters.has(first.toLowerCase())) return "";
  const second = parts[1] ?? "";
  if (second.startsWith("-")) return first;
  return `${first} ${second}`;
}
