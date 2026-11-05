//! 命令风险分类：对 shell 命令做小写子串启发式匹配，映射到 R0 只读 / R2 执行 / R3 网络 / R4 破坏性。
//! 判定顺序：破坏性 > 网络 > 只读 > 默认执行。宁可误判（误判只是多走一次审批）也不放过（漏判没有拦截）。

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

/// 只读程序白名单：这些程序（在任何参数下）都不产生本地写入或执行副作用。
/// 刻意从窄：find（-delete/-exec）、rg（--pre 处理器）、sed/awk、xargs、git branch（-m/-D 写）
/// 等有副作用参数面的程序一律不入选；有疑虑就留在白名单外走 R2 审批。
const READONLY_PROGRAMS: &[&str] = &[
    "ls", "pwd", "echo", "printf", "cat", "head", "tail", "wc", "file", "du", "df", "stat", "tree",
    "grep", "which", "whoami", "uname", "hostname", "date",
];

/// git 的只读子命令（第二词）。branch（-m/-D 写）、reflog（expire 写）不入选；
/// 含 `--output` 参数的段在段级判定中整段拒绝（git diff --output=file 写文件）。
const GIT_READONLY_SUBCOMMANDS: &[&str] = &[
    "log", "status", "diff", "show", "blame", "shortlog", "describe", "rev-parse", "ls-files", "grep",
];

/// 包管理器的只读查询子命令（install/add/update 等写类不入选）。
const PNPM_READONLY_SUBCOMMANDS: &[&str] = &["ls", "list", "why", "outdated"];
const NPM_READONLY_SUBCOMMANDS: &[&str] = &["ls", "list", "outdated"];

/// 出现任一即整条命令不可判只读的结构性元素：重定向（> >> < 与 &>）、
/// 命令替换（$( 与反引号）、换行（多命令）。误伤（引号内的同名符号）方向安全——只是多走审批。
const UNSAFE_SHELL_CHARS: &[char] = &['>', '<', '$', '`', '\n', '\r'];

/// 单段（管道/逻辑连接切出的最小命令段）是否只读：首词在白名单，
/// git/pnpm/npm 再看子命令白名单。空段、含 `--output` 写参数的段一律不只读。
fn is_readonly_segment(segment: &str) -> bool {
    let seg = segment.trim();
    if seg.is_empty() || seg.contains("--output") {
        return false;
    }
    let mut words = seg.split_whitespace();
    let Some(program) = words.next() else { return false };
    // 绝对/相对路径形式（/bin/ls、./cat）取最后一段程序名再匹配
    let program = program.rsplit('/').next().unwrap_or(program);
    match program {
        p if READONLY_PROGRAMS.contains(&p) => true,
        "git" => {
            let sub = words.next().unwrap_or_default();
            GIT_READONLY_SUBCOMMANDS.contains(&sub)
        }
        "pnpm" => {
            let sub = words.next().unwrap_or_default();
            PNPM_READONLY_SUBCOMMANDS.contains(&sub)
        }
        "npm" => {
            let sub = words.next().unwrap_or_default();
            NPM_READONLY_SUBCOMMANDS.contains(&sub)
        }
        _ => false,
    }
}

/// 整条命令是否只读：无结构性 shell 元素（重定向/命令替换/换行/后台 `&`），
/// 且按 `&&`/`||` 与管道 `|` 切出的每一段都只读。引号内的管道符会被当作管道切分，
/// 可能误判为非只读——方向安全（回落 R2 多走审批）。
fn is_readonly_command(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() || cmd.chars().any(|c| UNSAFE_SHELL_CHARS.contains(&c)) {
        return false;
    }
    // 逻辑连接占位成哨兵后，残留的裸 `&` 即后台执行——拒绝；
    // 管道 `|` 保留并按段判定（两端都只读才只读）。
    let normalized = cmd.replace("&&", "\u{1}").replace("||", "\u{1}");
    if normalized.contains('&') {
        return false;
    }
    normalized
        .split('\u{1}')
        .all(|part| part.split('|').all(is_readonly_segment))
}

/// 命令分类：返回（风险等级, 审批理由）。
/// 空白命令按 R2 兜底（仍需审批，不静默放行）；`sudo` 提权或破坏性子串命中即 R4，
/// 网络子串命中 R3，整条只读（含管道各段）判 R0，其余 R2。匹配刻意从宽，不做 shell 语法解析。
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
    if is_readonly_command(trimmed) {
        return (PolicyRisk::R0, "只读命令");
    }
    (PolicyRisk::R2, "在工作区内执行命令")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_prefer_false_positive_over_miss() {
        assert_eq!(classify_command("sudo rm x"), (PolicyRisk::R4, "包含破坏性命令"));
        assert_eq!(classify_command("rm -rf /"), (PolicyRisk::R4, "包含破坏性命令"));
        assert_eq!(classify_command("git push --force origin main"), (PolicyRisk::R4, "包含破坏性命令"));
        assert_eq!(classify_command("GIT PUSH --force"), (PolicyRisk::R4, "包含破坏性命令"), "大小写不敏感");
        assert_eq!(classify_command("curl https://x.com"), (PolicyRisk::R3, "命令可能访问网络"));
        assert_eq!(classify_command("npm install left-pad"), (PolicyRisk::R3, "命令可能访问网络"));
        assert_eq!(classify_command("git pull"), (PolicyRisk::R3, "命令可能访问网络"));
        assert_eq!(classify_command("  "), (PolicyRisk::R2, "执行命令"));
        // 破坏性优先于网络
        assert_eq!(classify_command("curl x && git reset --hard"), (PolicyRisk::R4, "包含破坏性命令"));
    }

    #[test]
    fn readonly_whitelist_maps_to_r0() {
        assert_eq!(classify_command("ls -la"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("cat package.json"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("git log --oneline -60 -- package.json pnpm-lock.yaml | head -60"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("git status --short"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("git diff HEAD~1"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("git log && git status"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("pnpm outdated"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("pnpm ls --depth 2"), (PolicyRisk::R0, "只读命令"));
        // 首词带绝对路径也按程序名匹配
        assert_eq!(classify_command("/bin/ls /tmp"), (PolicyRisk::R0, "只读命令"));
    }

    #[test]
    fn readonly_judgment_stays_conservative() {
        // 白名单外程序、写类子命令、危险结构全部回落 R2
        assert_eq!(classify_command("find . -name a"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("rg pattern"), (PolicyRisk::R2, "在工作区内执行命令"), "rg --pre 可执行处理器，不入白名单");
        assert_eq!(classify_command("git branch -D x"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("git reflog expire --expire=now"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("pnpm install"), (PolicyRisk::R3, "命令可能访问网络"), "既有行为：pnpm install 含子串 npm install，联网安装按 R3");
        assert_eq!(classify_command("git diff --output=x HEAD"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("echo hi > out.txt"), (PolicyRisk::R2, "在工作区内执行命令"), "重定向拒绝只读");
        assert_eq!(classify_command("cat $(ls)"), (PolicyRisk::R2, "在工作区内执行命令"), "命令替换拒绝只读");
        assert_eq!(classify_command("git log &"), (PolicyRisk::R2, "在工作区内执行命令"), "后台执行拒绝只读");
        assert_eq!(classify_command("ls\nrm x"), (PolicyRisk::R2, "在工作区内执行命令"), "多行多命令拒绝只读");
        assert_eq!(classify_command("git log | xargs rm"), (PolicyRisk::R2, "在工作区内执行命令"), "管道尾段不在白名单");
        assert_eq!(classify_command("sudo ls"), (PolicyRisk::R4, "包含破坏性命令"), "sudo 优先于只读");
    }
}
