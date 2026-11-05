//! 命令风险分类：对 shell 命令做小写子串启发式匹配，映射到 R2 执行 / R3 网络 / R4 破坏性。
//! 判定顺序：破坏性 > 网络 > 默认执行。宁可误判（误判只是多走一次审批）也不放过（漏判没有拦截）。

use crate::engine::PolicyRisk;

/// 破坏性特征子串（小写比较）：覆盖文件系统与系统级破坏、拒绝服务、高危 git 操作。
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

/// 网络访问特征子串（小写比较，多数带尾随空格以减少误命中）：下载、远程连接、
/// 包管理器联网与 git 远程操作——可能引入任意远端代码，按 R3 审批。
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

/// 命令分类：返回（风险等级, 审批理由）。
/// 空白命令按 R2 兜底（仍需审批，不静默放行）；`sudo` 提权或破坏性子串命中即 R4，
/// 网络子串命中 R3，其余 R2。匹配刻意从宽，不做 shell 语法解析。
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
