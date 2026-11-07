//! 风险分级与审批档位的核心判定。
//! 风险等级：R0 读 / R1 写工作区 / R2 执行 / R3 网络或越界 / R4 破坏性（参数可提升风险）。
//! 审批档位：graded 分级 / free 自由；档位只是策略引擎的输入，不改变风险分级本身。
//! `Engine::action` 的判定顺序不可调换（AGENTS.md 审批表）。
//! 历史说明：曾存在 strict（逐次询问不提供永久授权）与 readonly（R1+ 直接拒绝）两档，
//! 前者与分级档差异仅是禁用授权缓存、后者被计划模式（提示词约束 + R1+ 强制逐次审批）完全覆盖，
//! 已移除；旧 settings 里的 "strict"/"readonly" 由 [`parse_level`] 回落分级档。

/// 风险等级（与 shared/types.ts 的 Risk 对齐）：
/// R0 读、R1 写工作区、R2 执行、R3 网络或越界、R4 破坏性；工具参数可把基础风险向上提升。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyRisk {
    R0,
    R1,
    R2,
    R3,
    R4,
}

impl PolicyRisk {
    /// 全部风险等级，按 R0→R4 排列（遍历用）。
    pub const ALL: [PolicyRisk; 5] = [PolicyRisk::R0, PolicyRisk::R1, PolicyRisk::R2, PolicyRisk::R3, PolicyRisk::R4];

    /// 存储与审计形式："R0"…"R4"。
    pub fn as_str(&self) -> &'static str {
        match self {
            PolicyRisk::R0 => "R0",
            PolicyRisk::R1 => "R1",
            PolicyRisk::R2 => "R2",
            PolicyRisk::R3 => "R3",
            PolicyRisk::R4 => "R4",
        }
    }

    /// 反序列化 "R0"…"R4"；无法识别返回 None。
    pub fn parse(value: &str) -> Option<PolicyRisk> {
        match value {
            "R0" => Some(PolicyRisk::R0),
            "R1" => Some(PolicyRisk::R1),
            "R2" => Some(PolicyRisk::R2),
            "R3" => Some(PolicyRisk::R3),
            "R4" => Some(PolicyRisk::R4),
            _ => None,
        }
    }

    /// 数值等级（R0=0 … R4=4），供数值阈值比较（如计划软闸门的写类调用计数）。
    pub fn level(self) -> u8 {
        match self {
            PolicyRisk::R0 => 0,
            PolicyRisk::R1 => 1,
            PolicyRisk::R2 => 2,
            PolicyRisk::R3 => 3,
            PolicyRisk::R4 => 4,
        }
    }
}

/// 审批档位：graded 分级（默认）/ free 自由。
/// free 是唯一放行一切（含 R4 与受限工作区）的档位；切换前的确认门由调用方实现，策略层不负责交互。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Graded,
    Free,
}

impl Level {
    /// 存储形式："graded" / "free"。
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Graded => "graded",
            Level::Free => "free",
        }
    }
}

/// 单次策略判定的处置结果：放行 / 逐次审批 / 拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Ask,
    Deny,
}

impl Action {
    /// 存储与事件形式："allow" / "ask" / "deny"。
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Ask => "ask",
            Action::Deny => "deny",
        }
    }
}

/// 风险等级对应的用户可读理由（审批卡片展示用）。
pub fn reason_for(risk: PolicyRisk) -> &'static str {
    match risk {
        PolicyRisk::R0 => "只读操作",
        PolicyRisk::R1 => "写入工作区",
        PolicyRisk::R2 => "执行命令",
        PolicyRisk::R3 => "网络或越界访问",
        PolicyRisk::R4 => "破坏性操作",
    }
}

/// 解析档位字符串；未知值回落 graded（默认档）。
/// 已移除的 "strict"/"readonly"（历史 settings 值）同样回落分级档。
pub fn parse_level(value: &str) -> Level {
    match value {
        "free" => Level::Free,
        _ => Level::Graded,
    }
}

/// 工作区信任标记只有 "trusted" 视为信任；其余取值（含 "restricted" 与空）一律按受限处理。
pub fn is_workspace_trusted(trust: &str) -> bool {
    trust == "trusted"
}

/// 策略引擎：持有当前档位，把（工作区是否受限, 风险等级）判定为 [`Action`]。
#[derive(Debug, Clone)]
pub struct Engine {
    level: Level,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Level::Graded)
    }
}

impl Engine {
    /// 以指定档位构建引擎。
    pub fn new(level: Level) -> Self {
        Self { level }
    }

    /// 当前档位。
    pub fn level(&self) -> Level {
        self.level
    }

    /// 切换档位；free 档的切换确认门由调用方实现，这里不做任何拦截。
    pub fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    /// 处置规则（见 AGENTS.md 审批表，判定顺序不可调换）：
    /// 1. free 全放行（安全兜底退化为审计 + 写前备份）
    /// 2. 受限工作区 R3 直接拒绝（先于档位判定，对分级档生效）
    /// 3. graded：R0 放行、R4 拒绝、其余逐次询问；永久授权见 [`Engine::allows_permanent`]
    pub fn action(&self, restricted: bool, risk: PolicyRisk) -> Action {
        let level = self.level;
        if level == Level::Free {
            return Action::Allow; // 用户显式风险确认，策略层放行，安全靠审计+备份兜底
        }
        if restricted && risk == PolicyRisk::R3 {
            return Action::Deny; // 受限工作区默认断网
        }
        // graded
        if risk == PolicyRisk::R0 {
            return Action::Allow;
        }
        if risk == PolicyRisk::R4 {
            return Action::Deny;
        }
        Action::Ask
    }

    /// 是否允许永久授权（session/workspace 范围）：非受限工作区 + R1/R2。
    /// R3/R4 与受限工作区一律逐次审批；allowPermanent=false 时调用方也应把 session/workspace 降为 once。
    pub fn allows_permanent(&self, restricted: bool, risk: PolicyRisk) -> bool {
        !restricted && matches!(risk, PolicyRisk::R1 | PolicyRisk::R2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_order_and_level_rules() {
        // graded
        let graded = Engine::new(Level::Graded);
        assert_eq!(graded.action(false, PolicyRisk::R0), Action::Allow);
        assert_eq!(graded.action(false, PolicyRisk::R1), Action::Ask);
        assert_eq!(graded.action(false, PolicyRisk::R3), Action::Ask);
        assert_eq!(graded.action(false, PolicyRisk::R4), Action::Deny);
        // 受限工作区 R3 直接拒绝（graded）
        assert_eq!(graded.action(true, PolicyRisk::R3), Action::Deny);
        // 受限工作区 R1/R2 仍逐次询问
        assert_eq!(graded.action(true, PolicyRisk::R1), Action::Ask);

        // free：全放行（含 R4、受限）
        let free = Engine::new(Level::Free);
        assert_eq!(free.action(true, PolicyRisk::R4), Action::Allow);
    }

    #[test]
    fn permanent_grant_only_unrestricted_r1_r2() {
        let graded = Engine::new(Level::Graded);
        assert!(graded.allows_permanent(false, PolicyRisk::R1));
        assert!(graded.allows_permanent(false, PolicyRisk::R2));
        assert!(!graded.allows_permanent(true, PolicyRisk::R1));
        assert!(!graded.allows_permanent(false, PolicyRisk::R3));
    }

    #[test]
    fn level_parsing_falls_back_to_graded() {
        assert_eq!(parse_level("free"), Level::Free);
        assert_eq!(parse_level("graded"), Level::Graded);
        // 已移除的历史档位与未知值一律回落分级档（旧 settings 兼容）
        assert_eq!(parse_level("strict"), Level::Graded);
        assert_eq!(parse_level("readonly"), Level::Graded);
        assert_eq!(parse_level("其他"), Level::Graded);
        assert!(is_workspace_trusted("trusted"));
        assert!(!is_workspace_trusted("restricted"));
    }

    #[test]
    fn risk_level_maps_r0_to_r4() {
        // 数值等级供阈值比较（如计划软闸门按 R2+ 计数写类调用）
        assert_eq!(PolicyRisk::R0.level(), 0);
        assert_eq!(PolicyRisk::R1.level(), 1);
        assert_eq!(PolicyRisk::R2.level(), 2);
        assert_eq!(PolicyRisk::R3.level(), 3);
        assert_eq!(PolicyRisk::R4.level(), 4);
        assert!(PolicyRisk::R2.level() >= PolicyRisk::R2.level());
        assert!(PolicyRisk::R1.level() < PolicyRisk::R2.level());
    }
}
