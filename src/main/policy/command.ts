// 命令风险分类：小写子串匹配，宁可误判也不放过。
import type { Risk } from "./engine";

const DESTRUCTIVE_SUBSTR = [
  "rm -rf /",
  "rm -rf ~",
  "rm -rf .",
  "mkfs",
  "dd if=",
  "chmod 777",
  "chmod -r 777",
  "chown -r",
  "> /dev/",
  ":(){:|:&};:",
  "shutdown",
  "reboot",
  "halt",
  "git push --force",
  "git reset --hard",
  "git clean -fdx",
];

const NETWORK_SUBSTR = [
  "curl ",
  "wget ",
  "ssh ",
  "scp ",
  "sftp ",
  "nc ",
  "netcat ",
  "npm install",
  "npm i ",
  "pnpm add",
  "yarn add",
  "go get",
  "pip install",
  "docker pull",
  "git clone",
  "git fetch",
  "git pull",
  "git push",
];

export function classifyCommand(command: string): { risk: Risk; reason: string } {
  const trimmed = command.trim();
  if (!trimmed) return { risk: "R2", reason: "执行命令" };
  const lower = trimmed.toLowerCase();
  if (lower.includes("sudo ") || DESTRUCTIVE_SUBSTR.some((s) => lower.includes(s))) {
    return { risk: "R4", reason: "包含破坏性命令" };
  }
  if (NETWORK_SUBSTR.some((s) => lower.includes(s))) {
    return { risk: "R3", reason: "命令可能访问网络" };
  }
  return { risk: "R2", reason: "在工作区内执行命令" };
}
