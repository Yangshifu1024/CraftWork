// 提供商列表纯逻辑：状态点三色判定、左栏按来源分组、模型选择器过滤。
// 无 React / DOM 依赖，供设置页左栏与模型选择器消费并由 vitest 直测（tests/provider-list.test.ts）。
import type { ModelView, ProviderView } from "../../../shared/types";

export type ProviderStatusTone = "ok" | "warn" | "off";

/** 状态点判定入参：ProviderView 的最小投影（enabled 缺省视为启用，本票尚无启停字段）。 */
export interface ProviderStatusInput {
  enabled?: boolean;
  keyMasked: string;
  keyRequired: boolean;
  modelCount: number;
}

export interface ProviderStatus {
  tone: ProviderStatusTone;
  /** 状态点悬停提示（title） */
  label: string;
}

/**
 * 三色状态点（spec §5）：
 * 灰 = enabled === false；绿 = 启用且密钥就绪（或无需密钥）且有模型；
 * 橙 = 启用但缺密钥或无模型。
 */
export function providerStatus(p: ProviderStatusInput): ProviderStatus {
  if (p.enabled === false) return { tone: "off", label: "已停用" };
  if (p.keyRequired && p.keyMasked === "") return { tone: "warn", label: "未配置密钥" };
  if (p.modelCount === 0) return { tone: "warn", label: "暂无模型" };
  return { tone: "ok", label: "可用" };
}

/** 左栏分组（spec §5）：目录导入在前、手动添加在后；组标题常显、空组隐藏。 */
export interface ProviderGroup<T> {
  source: "catalog" | "manual";
  label: string;
  providers: T[];
}

/**
 * 按来源分组：仅 source === "catalog" 进目录导入组，
 * 其余（manual / 缺失 / 未知）归手动添加，与服务端缺省（manual）一致。
 */
export function groupProviders<T extends { source?: string }>(providers: T[]): Array<ProviderGroup<T>> {
  const catalog: T[] = [];
  const manual: T[] = [];
  for (const p of providers) {
    if (p.source === "catalog") catalog.push(p);
    else manual.push(p);
  }
  const groups: Array<ProviderGroup<T>> = [
    { source: "catalog", label: "目录导入", providers: catalog },
    { source: "manual", label: "手动添加", providers: manual },
  ];
  return groups.filter((g) => g.providers.length > 0);
}

/** 选择器分组：提供商与其可选模型（过滤后非空才有分组）。 */
export interface SelectableModelGroup {
  provider: ProviderView;
  models: ModelView[];
}

/**
 * 模型选择器过滤（spec §9 / 票 #7）：只列「启用提供商 × 启用模型」，
 * 跳过无可用模型的提供商（不产生空分组）。历史会话引用不在选择器层面清理，
 * 仅由 factory.client 在调用侧拦截。
 */
export function selectableModels(providers: ProviderView[]): Array<SelectableModelGroup> {
  const groups: Array<SelectableModelGroup> = [];
  for (const p of providers) {
    if (p.enabled === false) continue;
    const models = p.models.filter((m) => m.enabled !== false);
    if (models.length > 0) groups.push({ provider: p, models });
  }
  return groups;
}
