//! 风险分级与审批档位。
//! R0 只读自动放行；R1 写工作区；R2 执行；R3 网络或越界；R4 破坏性阻止。
//! 判定顺序不可调换（AGENTS.md 审批表）。

/// 风险等级（与 shared/types.ts 的 Risk 对齐）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PolicyRisk {
    R0,
    R1,
    R2,
    R3,
    R4,
}

impl PolicyRisk {
    pub const ALL: [PolicyRisk; 5] = [PolicyRisk::R0, PolicyRisk::R1, PolicyRisk::R2, PolicyRisk::R3, PolicyRisk::R4];

    pub fn as_str(&self) -> &'static str {
        match self {
            PolicyRisk::R0 => "R0",
            PolicyRisk::R1 => "R1",
            PolicyRisk::R2 => "R2",
            PolicyRisk::R3 => "R3",
            PolicyRisk::R4 => "R4",
        }
    }

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Graded,
    Strict,
    Readonly,
    Free,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Graded => "graded",
            Level::Strict => "strict",
            Level::Readonly => "readonly",
            Level::Free => "free",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Ask,
    Deny,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Ask => "ask",
            Action::Deny => "deny",
        }
    }
}

pub fn reason_for(risk: PolicyRisk) -> &'static str {
    match risk {
        PolicyRisk::R0 => "只读操作",
        PolicyRisk::R1 => "写入工作区",
        PolicyRisk::R2 => "执行命令",
        PolicyRisk::R3 => "网络或越界访问",
        PolicyRisk::R4 => "破坏性操作",
    }
}

pub fn parse_level(value: &str) -> Level {
    match value {
        "strict" => Level::Strict,
        "readonly" => Level::Readonly,
        "free" => Level::Free,
        _ => Level::Graded,
    }
}

pub fn is_workspace_trusted(trust: &str) -> bool {
    trust == "trusted"
}

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
    pub fn new(level: Level) -> Self {
        Self { level }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    /// 处置规则：见 AGENTS.md 审批表——判定顺序不可调换。
    pub fn action(&self, restricted: bool, risk: PolicyRisk) -> Action {
        let level = self.level;
        if level == Level::Free {
            return Action::Allow; // 用户显式风险确认，策略层放行，安全靠审计+备份兜底
        }
        if level == Level::Readonly {
            return if risk == PolicyRisk::R0 { Action::Allow } else { Action::Deny };
        }
        if restricted && risk == PolicyRisk::R3 {
            return Action::Deny; // 受限工作区默认断网
        }
        if level == Level::Strict {
            if risk == PolicyRisk::R0 {
                return Action::Allow;
            }
            if risk == PolicyRisk::R4 {
                return Action::Deny;
            }
            return Action::Ask; // 严格档逐次询问，不提供永久允许
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

    /// 永久授权：仅分级档 + 非受限 + R1/R2。
    pub fn allows_permanent(&self, restricted: bool, risk: PolicyRisk) -> bool {
        self.level == Level::Graded && !restricted && matches!(risk, PolicyRisk::R1 | PolicyRisk::R2)
    }

    pub fn allows_write(&self) -> bool {
        self.level != Level::Readonly
    }
}

/// 档位的最宽风险（审计用）。
pub fn level_widest_risk(level: Level) -> PolicyRisk {
    match level {
        Level::Free => PolicyRisk::R4,
        Level::Readonly => PolicyRisk::R0,
        _ => PolicyRisk::R3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 判定顺序与档位规则() {
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

        // strict：逐次询问，不因受限改变
        let strict = Engine::new(Level::Strict);
        assert_eq!(strict.action(false, PolicyRisk::R1), Action::Ask);
        assert_eq!(strict.action(true, PolicyRisk::R1), Action::Ask);
        assert_eq!(strict.action(false, PolicyRisk::R4), Action::Deny);

        // readonly：仅 R0
        let readonly = Engine::new(Level::Readonly);
        assert_eq!(readonly.action(false, PolicyRisk::R0), Action::Allow);
        assert_eq!(readonly.action(false, PolicyRisk::R1), Action::Deny);
        assert_eq!(readonly.action(true, PolicyRisk::R0), Action::Allow);

        // free：全放行（含 R4、受限）
        let free = Engine::new(Level::Free);
        assert_eq!(free.action(true, PolicyRisk::R4), Action::Allow);
    }

    #[test]
    fn 永久授权仅分级档非受限R1R2() {
        let graded = Engine::new(Level::Graded);
        assert!(graded.allows_permanent(false, PolicyRisk::R1));
        assert!(graded.allows_permanent(false, PolicyRisk::R2));
        assert!(!graded.allows_permanent(true, PolicyRisk::R1));
        assert!(!graded.allows_permanent(false, PolicyRisk::R3));
        let strict = Engine::new(Level::Strict);
        assert!(!strict.allows_permanent(false, PolicyRisk::R1));
    }

    #[test]
    fn 档位解析与最宽风险() {
        assert_eq!(parse_level("strict"), Level::Strict);
        assert_eq!(parse_level("其他"), Level::Graded);
        assert_eq!(level_widest_risk(Level::Free), PolicyRisk::R4);
        assert_eq!(level_widest_risk(Level::Readonly), PolicyRisk::R0);
        assert_eq!(level_widest_risk(Level::Graded), PolicyRisk::R3);
        assert!(is_workspace_trusted("trusted"));
        assert!(!is_workspace_trusted("restricted"));
    }
}
