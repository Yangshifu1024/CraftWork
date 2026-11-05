//! 命令风险分类：小写子串匹配，宁可误判也不放过。

use crate::engine::PolicyRisk;

const DESTRUCTIVE_SUBSTR: &[&str] = &[
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

const NETWORK_SUBSTR: &[&str] = &[
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

pub fn classify_command(command: &str) -> (PolicyRisk, &'static str) {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return (PolicyRisk::R2, "执行命令");
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("sudo ") || DESTRUCTIVE_SUBSTR.iter().any(|s| lower.contains(s)) {
        return (PolicyRisk::R4, "包含破坏性命令");
    }
    if NETWORK_SUBSTR.iter().any(|s| lower.contains(s)) {
        return (PolicyRisk::R3, "命令可能访问网络");
    }
    (PolicyRisk::R2, "在工作区内执行命令")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_prefer_false_positive_over_miss() {
        assert_eq!(classify_command("ls -la"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("  "), (PolicyRisk::R2, "执行命令"));
        assert_eq!(classify_command("sudo rm x"), (PolicyRisk::R4, "包含破坏性命令"));
        assert_eq!(classify_command("rm -rf /"), (PolicyRisk::R4, "包含破坏性命令"));
        assert_eq!(classify_command("git push --force origin main"), (PolicyRisk::R4, "包含破坏性命令"));
        assert_eq!(classify_command("GIT PUSH --force"), (PolicyRisk::R4, "包含破坏性命令"), "大小写不敏感");
        assert_eq!(classify_command("curl https://x.com"), (PolicyRisk::R3, "命令可能访问网络"));
        assert_eq!(classify_command("npm install left-pad"), (PolicyRisk::R3, "命令可能访问网络"));
        assert_eq!(classify_command("git pull"), (PolicyRisk::R3, "命令可能访问网络"));
        // 破坏性优先于网络
        assert_eq!(classify_command("curl x && git reset --hard"), (PolicyRisk::R4, "包含破坏性命令"));
    }
}
