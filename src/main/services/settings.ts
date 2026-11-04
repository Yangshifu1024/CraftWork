// 设置服务：读改存与合法性钳制（非法值回退不报错，保证一个坏字段不拖垮整体保存）。
import type { Settings } from "../../shared/types";
import { EventType } from "../../shared/events";
import type { Recorder } from "../core/record";
import type { DB } from "../store/db";
import { getSetting, setSetting, addAudit } from "../store/records";
import { Engine, levelWidestRisk, parseLevel } from "../policy/engine";
import type { Catalog } from "../model/catalog";

const SETTINGS_KEY = "app";

export function defaultSettings(): Settings {
  return {
    theme: "system",
    mode: "code",
    panelWidth: 320,
    approvalLevel: "graded",
    defaultModel: "",
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
  };
}

export const APPROVAL_LEVEL_NAMES: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
  free: "自由",
};

export function approvalLevelName(level: string): string {
  return APPROVAL_LEVEL_NAMES[level] ?? "分级";
}

export class SettingsService {
  constructor(
    private readonly db: DB,
    private readonly recorder: Recorder,
    private readonly engine: Engine,
    private readonly catalog: Catalog,
  ) {}

  get(): Settings {
    const saved = getSetting<Partial<Settings>>(this.db, SETTINGS_KEY, {});
    return { ...defaultSettings(), ...saved };
  }

  save(next: Settings): { settings: Settings; error?: Error } {
    const current = this.get();
    const merged: Settings = { ...current };

    if (next.theme === "system" || next.theme === "dark" || next.theme === "light") merged.theme = next.theme;
    if (next.mode === "code" || next.mode === "work") merged.mode = next.mode;
    merged.approvalLevel = parseLevel(next.approvalLevel);
    merged.panelWidth = Math.min(760, Math.max(320, Math.trunc(next.panelWidth) || 320));
    merged.fontSize = next.fontSize >= 11 && next.fontSize <= 18 ? Math.trunc(next.fontSize) : 15;
    merged.language = next.language || current.language;
    merged.codeTheme = next.codeTheme || current.codeTheme;
    merged.autoUpdate = next.autoUpdate;
    merged.notifications = next.notifications;
    merged.updateChannel = next.updateChannel || current.updateChannel;
    merged.telemetry = next.telemetry;
    // 0 合法（不限制）；负数或超上限回退默认
    merged.maxIterations =
      Number.isInteger(next.maxIterations) && next.maxIterations >= 0 && next.maxIterations <= 1000
        ? next.maxIterations
        : 300;

    // 模型设置：必须能解析，能解析则改写为规范引用，否则清空（不静默回退到别的模型）
    merged.defaultModel = this.normalizeModel(next.defaultModel);
    const fallback = this.normalizeModel(next.fallbackModel);
    merged.fallbackModel = fallback && fallback !== merged.defaultModel ? fallback : "";
    merged.activeWorkspace = next.activeWorkspace || "";

    const prevLevel = current.approvalLevel;
    // 先持久化后落审计：档位决定哪些操作免确认，必须可追溯
    setSetting(this.db, SETTINGS_KEY, merged);
    this.engine.setLevel(merged.approvalLevel);
    if (prevLevel !== merged.approvalLevel) {
      try {
        addAudit(this.db, {
          tool: "approval_level",
          risk: levelWidestRisk(merged.approvalLevel),
          decision: "update",
          scope: "user",
          detail: `审批档位：${approvalLevelName(prevLevel)} → ${approvalLevelName(merged.approvalLevel)}`,
        });
      } catch (err) {
        // 审计失败不影响设置生效，如实返回错误
        this.recorder.emit(EventType.SettingsUpdated, undefined, undefined, merged);
        return { settings: merged, error: err instanceof Error ? err : new Error(String(err)) };
      }
    }
    this.recorder.emit(EventType.SettingsUpdated, undefined, undefined, merged);
    return { settings: merged };
  }

  private normalizeModel(ref: string): string {
    if (!ref) return "";
    const normalized = this.catalog.normalize(ref);
    return normalized?.ref ?? "";
  }
}
