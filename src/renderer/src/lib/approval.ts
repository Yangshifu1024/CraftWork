// 审批档位：清单、显示名与图标供设置页与 composer 档位浮层共用；切换动作统一收口
// （切到「自由」放行一切操作含破坏性，必须先经危险确认框——对齐 ZCode yolo 语义）。
import { Eye, Shield, ShieldAlert, ShieldCheck, type LucideIcon } from "lucide-react";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import type { PolicyLevel } from "../../../shared/types";

/** 审批档位清单（设置页与 composer 档位浮层共用）：id 与 PolicyLevel 对齐。 */
export const APPROVAL_LEVELS = [
  { id: "graded", name: "分级", desc: "只读自动放行；写入与执行每次询问；可本会话/本工作区授权" },
  { id: "strict", name: "严格", desc: "全部逐次询问，不提供永久授权" },
  { id: "readonly", name: "只读", desc: "只允许只读操作，其余直接拒绝" },
  { id: "free", name: "自由", desc: "放行一切操作（含破坏性），安全靠审计与备份兜底" },
] as const;

/** 档位 ID → 显示名（chip 等窄场景用）。 */
export const APPROVAL_LEVEL_NAMES: Record<string, string> = Object.fromEntries(
  APPROVAL_LEVELS.map((l) => [l.id, l.name]),
);

/** 档位 ID → 图标；「自由」用 ShieldAlert（高权限警示，对齐 ZCode yolo）。 */
export const APPROVAL_LEVEL_ICONS: Record<PolicyLevel, LucideIcon> = {
  graded: Shield,
  strict: ShieldCheck,
  readonly: Eye,
  free: ShieldAlert,
};

/** ⌘⇧M 的档位循环顺序。 */
const LEVEL_ORDER: readonly PolicyLevel[] = ["graded", "strict", "readonly", "free"];

type SaveSettings = (next: { approvalLevel: PolicyLevel }) => Promise<unknown>;
type OpenFreeLevel = (opts: { onOk(): void }) => void;

/** 选择档位的统一入口：切到「自由」先过自由档确认门（能力清单 + 勾选确认），
 * 确认才写入；其余档位直接写入并立即生效。 */
function confirmThenSave(id: PolicyLevel, current: PolicyLevel, save: SaveSettings, openFreeLevel: OpenFreeLevel): void {
  if (id === "free" && current !== "free") {
    openFreeLevel({ onOk: () => void save({ approvalLevel: id }) });
    return;
  }
  void save({ approvalLevel: id });
}

/** 返回选择档位的动作（组件内使用）。 */
export function useApprovalLevelAction(): (id: PolicyLevel) => void {
  const current = useApp((s) => s.settings.approvalLevel);
  const save = useApp((s) => s.saveSettings);
  const openFreeLevel = useOverlays((s) => s.openFreeLevel);
  return (id) => confirmThenSave(id, current, save, openFreeLevel);
}

/** ⌘⇧M 循环档位（分级 → 严格 → 只读 → 自由 → 分级，对齐 ZCode cycleSessionMode）；切入「自由」同样过确认门。 */
export function cycleApprovalLevel(): void {
  const { settings, saveSettings } = useApp.getState();
  const openFreeLevel = useOverlays.getState().openFreeLevel;
  const idx = LEVEL_ORDER.indexOf(settings.approvalLevel);
  const next = LEVEL_ORDER[(Math.max(idx, 0) + 1) % LEVEL_ORDER.length] ?? "graded";
  confirmThenSave(next, settings.approvalLevel, saveSettings, openFreeLevel);
}
