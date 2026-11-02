/**
 * 审批档位切换的共享逻辑。
 *
 * 「自由」档位会放行包括 R4 在内的所有操作，属于高危选择，
 * 因此切换入口（设置页与输入框浮层）统一走这里的守卫：
 * 目标是自由档位且当前不在自由档位时，先弹出风险确认弹层，
 * 用户勾选「我已了解风险」并确认后才允许写入设置；取消则什么都不改，
 * 停留在原档位。弹层的内容与交互见 FreeLevelDialog 组件。
 */
import { useUiStore } from "../stores/ui";
import type { ApprovalLevel } from "./types";

/**
 * 审批档位切换守卫。
 *
 * props：target 是要切换到的档位；current 是当前档位，来自后端设置的
 * 原始字符串（Wails 绑定中为 string），只做相等比较。
 * 返回：true 表示允许写入设置（无需确认或用户已确认），false 表示用户取消，
 * 调用方应保持原档位不变。
 */
export async function guardFreeLevel(target: ApprovalLevel, current: string): Promise<boolean> {
  // 只有「切到自由」这一步需要确认；切出自由与其他档位间切换直接放行。
  if (target !== "free" || current === "free") {
    return true;
  }
  return useUiStore.getState().askFreeConfirm();
}
