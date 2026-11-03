// 环境上下文快照：日期 / 平台 / Shell + Git 摘要 + 指令文件（AGENTS.md 两级）。
// 任何一步失败都降级省略，不阻断运行——环境公告是增强信息。
import { readFileSync, statSync } from "node:fs";
import { basename, join } from "node:path";
import { run } from "../core/platform";

export interface EnvInstruction {
  Path: string;
  Scope: "global" | "project";
  Body: string;
  Truncated: boolean;
}

export interface EnvContextSnapshot {
  Env: { Today: string; Platform: string; Shell: string };
  Git?: { Branch: string; Status: string; Commits: string };
  Instructions: EnvInstruction[];
}

const GIT_TIMEOUT_MS = 3000;
const GIT_OUTPUT_LIMIT = 64 * 1024;
const MAX_INSTRUCTION_LINES = 200;
const MAX_INSTRUCTION_CHARS = 25000;

/** 采集环境快照（Git 失败或超时返回空串并跳过；任何失败都降级省略）。 */
export async function collectEnvContext(dataDir: string, workspace: string): Promise<EnvContextSnapshot | null> {
  const today = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  const snapshot: EnvContextSnapshot = {
    Env: {
      Today: `${today.getFullYear()}-${pad(today.getMonth() + 1)}-${pad(today.getDate())}`,
      Platform: `${process.platform}/${process.arch}`,
      Shell: process.env["SHELL"] ? basename(process.env["SHELL"]) : "",
    },
    Instructions: [],
  };

  if (workspace) {
    const git: { Branch: string; Status: string; Commits: string } = { Branch: "", Status: "", Commits: "" };
    const inside = await run({ file: "git", args: ["rev-parse", "--is-inside-work-tree"], dir: workspace, timeoutMs: GIT_TIMEOUT_MS, outputLimit: GIT_OUTPUT_LIMIT });
    if (inside.exitCode === 0 && inside.output.trim() === "true") {
      const branch = await run({ file: "git", args: ["symbolic-ref", "--short", "-q", "HEAD"], dir: workspace, timeoutMs: GIT_TIMEOUT_MS, outputLimit: GIT_OUTPUT_LIMIT });
      git.Branch = branch.exitCode === 0 ? branch.output.trim() : "";
      const status = await run({ file: "git", args: ["status", "--short"], dir: workspace, timeoutMs: GIT_TIMEOUT_MS, outputLimit: GIT_OUTPUT_LIMIT });
      git.Status = status.exitCode === 0 ? status.output.trim() : "";
      const log = await run({ file: "git", args: ["log", "--oneline", "-n", "5"], dir: workspace, timeoutMs: GIT_TIMEOUT_MS, outputLimit: GIT_OUTPUT_LIMIT });
      git.Commits = log.exitCode === 0 ? log.output.trim() : "";
      snapshot.Git = git;
    }

    // 指令文件两级：全局 → 项目
    const globalPath = join(dataDir, "AGENTS.md");
    const projectPath = join(workspace, "AGENTS.md");
    const globalInst = readInstruction(globalPath, "global");
    if (globalInst) snapshot.Instructions.push(globalInst);
    const projectInst = readInstruction(projectPath, "project");
    if (projectInst) snapshot.Instructions.push(projectInst);
  }

  return snapshot;
}

function readInstruction(path: string, scope: "global" | "project"): EnvInstruction | null {
  try {
    const st = statSync(path);
    if (!st.isFile() || st.size === 0) return null;
    const body = readFileSync(path, "utf8");
    if (!body.trim()) return null;
    const lines = body.split("\n");
    if (lines.length > MAX_INSTRUCTION_LINES || body.length > MAX_INSTRUCTION_CHARS) {
      return {
        Path: path,
        Scope: scope,
        Body: lines.slice(0, MAX_INSTRUCTION_LINES).join("\n").slice(0, MAX_INSTRUCTION_CHARS),
        Truncated: true,
      };
    }
    return { Path: path, Scope: scope, Body: body, Truncated: false };
  } catch {
    return null;
  }
}
