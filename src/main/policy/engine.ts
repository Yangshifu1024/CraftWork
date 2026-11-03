// 风险分级与审批档位。
// R0 只读自动放行；R1 写工作区；R2 执行；R3 网络或越界；R4 破坏性阻止。

export type Risk = "R0" | "R1" | "R2" | "R3" | "R4";
export type Level = "graded" | "strict" | "readonly" | "free";
export type Action = "allow" | "ask" | "deny";

export const RISKS: Risk[] = ["R0", "R1", "R2", "R3", "R4"];

export function reasonFor(risk: Risk): string {
  switch (risk) {
    case "R0":
      return "只读操作";
    case "R1":
      return "写入工作区";
    case "R2":
      return "执行命令";
    case "R3":
      return "网络或越界访问";
    case "R4":
      return "破坏性操作";
  }
}

export function parseLevel(value: string): Level {
  if (value === "strict" || value === "readonly" || value === "free") return value;
  return "graded";
}

export function isWorkspaceTrusted(trust: string): boolean {
  return trust === "trusted";
}

export class Engine {
  private levelValue: Level = "graded";

  constructor(level: Level = "graded") {
    this.levelValue = level;
  }

  get level(): Level {
    return this.levelValue;
  }

  setLevel(level: Level): void {
    this.levelValue = level;
  }

  /** 处置规则：见 AGENTS.md 审批表——判定顺序不可调换。 */
  action(restricted: boolean, risk: Risk): Action {
    const level = this.levelValue;
    if (level === "free") return "allow"; // 用户显式风险确认，策略层放行，安全靠审计+备份兜底
    if (level === "readonly") return risk === "R0" ? "allow" : "deny";
    if (restricted && risk === "R3") return "deny"; // 受限工作区默认断网
    if (level === "strict") {
      if (risk === "R0") return "allow";
      if (risk === "R4") return "deny";
      return "ask"; // 严格档逐次询问，不提供永久允许
    }
    // graded
    if (risk === "R0") return "allow";
    if (risk === "R4") return "deny";
    return "ask";
  }

  /** 永久授权：仅分级档 + 非受限 + R1/R2。 */
  allowsPermanent(restricted: boolean, risk: Risk): boolean {
    return this.levelValue === "graded" && !restricted && (risk === "R1" || risk === "R2");
  }

  allowsWrite(): boolean {
    return this.levelValue !== "readonly";
  }
}

/** 档位的最宽风险（审计用）。 */
export function levelWidestRisk(level: Level): Risk {
  switch (level) {
    case "free":
      return "R4";
    case "readonly":
      return "R0";
    default:
      return "R3";
  }
}
