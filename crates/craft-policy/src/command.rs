//! 命令风险分类：对 shell 命令做小写子串启发式匹配，映射到 R0 只读 / R2 执行 / R3 网络 / R4 破坏性。
//! 判定顺序：破坏性 > 网络 > 只读 > 默认执行。宁可误判（误判只是多走一次审批）也不放过（漏判没有拦截）。
//! 复合命令按未加引号的 `&&`/`||`/`;`/管道 `|` 引号感知切段，逐段独立判定。

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
/// 刻意从窄：find（-delete/-exec）、rg（--pre 处理器）、sed/awk（-i 写文件、`e` 执行命令、
/// `w` 写出脚本输出）、xargs、git branch（-m/-D 写）等有副作用参数面的程序一律不入选；
/// 有疑虑就留在白名单外走 R2 审批。计划模式下靠提示词引导改用 head/cat/grep/fs_read
/// 取代 `sed -n` 取片段（2026-11 复核维持排除）。
const READONLY_PROGRAMS: &[&str] = &[
    "ls", "pwd", "echo", "printf", "cat", "head", "tail", "wc", "file", "du", "df", "stat", "tree",
    "grep", "which", "whoami", "uname", "hostname", "date",
];

/// git 的只读子命令（第二词）。branch（-m/-D 写）、reflog（expire 写）不入选；
/// 含 `--output` 参数的段在段级判定中整段拒绝（git diff --output=file 写文件）。
const GIT_READONLY_SUBCOMMANDS: &[&str] = &[
    "log", "status", "diff", "show", "blame", "shortlog", "describe", "rev-parse", "ls-files", "grep",
];

/// Python 系解释器：不进只读程序白名单（`python x.py` 会执行任意脚本），
/// 只对纯版本/帮助参数放行只读；`-c`、`-m`、脚本路径一律不判只读。
const PYTHON_EXECUTABLES: &[&str] = &["python", "python3", "py"];
/// Python 解释器的只读参数：仅版本查询与帮助，无文件副作用。
const PYTHON_READONLY_FLAGS: &[&str] = &["--version", "-V", "-VV", "-h", "--help"];

/// 包管理器的只读查询子命令（install/add/update 等写类不入选）。
const PNPM_READONLY_SUBCOMMANDS: &[&str] = &["ls", "list", "why", "outdated"];
const NPM_READONLY_SUBCOMMANDS: &[&str] = &["ls", "list", "outdated"];

/// 出现任一即整条命令不可判只读的结构性元素：重定向（> >> < 与 &>）、
/// 命令替换（$( 与反引号）、换行（多命令）。误伤（引号内的同名符号）方向安全——只是多走审批。
/// 目标为 /dev/null 的丢弃型重定向与 fd 复制在结构检查前剥除，见 [`DISCARD_REDIRECTS`]。
const UNSAFE_SHELL_CHARS: &[char] = &['>', '<', '$', '`', '\n', '\r'];

/// 丢弃型重定向的字面形态（长形态在前）：目标为 /dev/null 的输入/输出重定向与 fd 复制
/// （N>&M）没有文件副作用，在破坏性/网络/只读匹配之前整串剥除——否则 `> /dev/null`
/// 会误命中破坏性子串 `> /dev/`（写设备）。剥除要求目标后跟边界符（空白/`;`/`|`/`&`/串尾），
/// `>/dev/nullfoo` 这类前缀伪装不剥除、照旧按重定向拒绝；未收录形态一律保留并拒绝——方向安全。
const DISCARD_REDIRECTS: &[&str] = &[
    "2> /dev/null",
    "2>/dev/null",
    "1> /dev/null",
    "1>/dev/null",
    "&> /dev/null",
    "&>/dev/null",
    ">> /dev/null",
    ">>/dev/null",
    "> /dev/null",
    ">/dev/null",
    "< /dev/null",
    "</dev/null",
    "2>&1",
    "1>&2",
];

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
        p if PYTHON_EXECUTABLES.contains(&p) => words.all(|w| PYTHON_READONLY_FLAGS.contains(&w)),
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

/// 剥除丢弃型重定向（见 [`DISCARD_REDIRECTS`]）：命中且目标后跟边界符才剥除，
/// 替换为一个空格保持词界（不产生跨词拼接）；否则原样保留，残留的 `>`/`<`
/// 会在结构检查中拒绝。
fn strip_discard_redirects(cmd: &str) -> String {
    let mut out = cmd.to_string();
    for form in DISCARD_REDIRECTS {
        let mut from = 0;
        while let Some(pos) = out[from..].find(form) {
            let start = from + pos;
            let end = start + form.len();
            let boundary_ok = out[end..]
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || matches!(c, ';' | '|' | '&'));
            if boundary_ok {
                out.replace_range(start..end, " ");
                from = start;
            } else {
                from = end;
            }
        }
    }
    out
}

/// 整条命令是否只读（入参已经 [`strip_discard_redirects`] 剥除丢弃型重定向）：
/// 无结构性 shell 元素（重定向/命令替换/换行），且按未加引号的 `&&`/`||`/`;`
/// 与管道 `|` 切出的每一段都只读。切分是引号感知的：引号内的元字符按字面文本
/// 并入当前段（`grep "a\|b"` 不再被误切）；未加引号的裸 `&`（后台执行）与
/// 未闭合引号一律拒绝——拿不准就回落 R2 多走审批，方向安全。
fn is_readonly_command(cmd: &str) -> bool {
    if cmd.is_empty() || cmd.chars().any(|c| UNSAFE_SHELL_CHARS.contains(&c)) {
        return false;
    }
    let mut segment = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        let quoted = in_single || in_double;
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '|' if !quoted => {
                if chars.peek() == Some(&'|') {
                    chars.next(); // || 与 && 同为逻辑连接
                }
                if !is_readonly_segment(&segment) {
                    return false;
                }
                segment.clear();
            }
            ';' if !quoted => {
                if !is_readonly_segment(&segment) {
                    return false;
                }
                segment.clear();
            }
            '&' if !quoted => {
                if chars.peek() != Some(&'&') {
                    return false; // 裸 &：后台执行
                }
                chars.next();
                if !is_readonly_segment(&segment) {
                    return false;
                }
                segment.clear();
            }
            _ => segment.push(c),
        }
    }
    if in_single || in_double {
        return false; // 引号未闭合，宁可拒绝
    }
    is_readonly_segment(&segment)
}

/// 命令分类：返回（风险等级, 审批理由）。
/// 空白命令按 R2 兜底（仍需审批，不静默放行）；先剥除丢弃型重定向（/dev/null 与 fd 复制，
/// 否则 `> /dev/null` 会误命中破坏性子串 `> /dev/`），`sudo` 提权或破坏性子串命中即 R4，
/// 网络子串命中 R3，整条只读（`&&`/`||`/`;`/管道切出的每段）判 R0，其余 R2。
/// 匹配刻意从宽，不做完整 shell 语法解析。
pub fn classify_command(command: &str) -> (PolicyRisk, &'static str) {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return (PolicyRisk::R2, "执行命令");
    }
    let stripped = strip_discard_redirects(trimmed);
    let lower = stripped.to_lowercase();
    if lower.contains("sudo ") || DESTRUCTIVE_SUBSTR.iter().any(|s| lower.contains(s)) {
        return (PolicyRisk::R4, "包含破坏性命令");
    }
    if NETWORK_SUBSTR.iter().any(|s| lower.contains(s)) {
        return (PolicyRisk::R3, "命令可能访问网络");
    }
    if is_readonly_command(&stripped) {
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

    #[test]
    fn semicolon_chain_cannot_borrow_first_word_whitelist() {
        // 回归：`;` 曾不切分，首词白名单被后续任意命令「借用」，`cat x; rm y` 被判 R0 自动放行
        assert_eq!(classify_command("cat x; rm y"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("ls ; rm x"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("git log; python script.py"), (PolicyRisk::R2, "在工作区内执行命令"));
    }

    #[test]
    fn semicolon_chain_of_readonly_segments_maps_to_r0() {
        assert_eq!(classify_command("ls src-tauri/tests; grep -rn x src-tauri/tests | head"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("git log; git status"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("pwd; whoami; date"), (PolicyRisk::R0, "只读命令"));
    }

    #[test]
    fn discard_redirections_to_dev_null_map_to_r0() {
        assert_eq!(classify_command("ls src-tauri/tests 2>/dev/null"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("grep a f >/dev/null 2>&1"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("echo hi > /dev/null"), (PolicyRisk::R0, "只读命令"), "带空格形态不再误命中破坏性子串 > /dev/");
        assert_eq!(classify_command("wc -l < /dev/null"), (PolicyRisk::R0, "只读命令"));
    }

    #[test]
    fn write_redirections_stay_rejected() {
        assert_eq!(classify_command("echo hi > out.txt"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("cat x 2>/dev/null > real.txt"), (PolicyRisk::R2, "在工作区内执行命令"), "剥除丢弃重定向后残留写重定向仍拒绝");
        assert_eq!(classify_command("echo hi >/dev/nullfoo"), (PolicyRisk::R2, "在工作区内执行命令"), "前缀伪装不剥除，照旧按重定向拒绝");
        assert_eq!(classify_command("git diff --output=x HEAD > /dev/null"), (PolicyRisk::R2, "在工作区内执行命令"), "--output 段级拒绝不受剥除影响");
    }

    #[test]
    fn quoted_metacharacters_are_literal_text() {
        assert_eq!(
            classify_command("grep -n \"str_at\\|bool_at\" -n src-tauri/src/lib.rs | head -40"),
            (PolicyRisk::R0, "只读命令"),
            "grep 交替模式内的 \\| 不再被误切为管道"
        );
        assert_eq!(classify_command("echo \"a && b\""), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("echo \"x; y\""), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("grep \"a|b\" f"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("grep \"unclosed f"), (PolicyRisk::R2, "在工作区内执行命令"), "引号未闭合宁可拒绝");
    }

    #[test]
    fn plan_mode_research_command_maps_to_r0() {
        // 审计报告中的实际场景：stderr 丢弃 + `;` 复合的只读调研命令应放行
        assert_eq!(
            classify_command("ls src-tauri/tests 2>/dev/null; grep -rn \"skillsList\" src-tauri/tests 2>/dev/null | head"),
            (PolicyRisk::R0, "只读命令")
        );
    }

    #[test]
    fn sed_print_mode_stays_non_readonly() {
        // 2026-11 决策：sed 维持整程序排除（-i 写文件、e 执行命令、w 写出），计划模式靠提示词引导
        assert_eq!(classify_command("sed -n '1,80p' src-tauri/src/lib.rs"), (PolicyRisk::R2, "在工作区内执行命令"));
        assert_eq!(classify_command("sed 's/a/b/w out' f"), (PolicyRisk::R2, "在工作区内执行命令"), "w 写出脚本不得借 -n 放行");
    }

    #[test]
    fn python_version_flags_map_to_r0() {
        assert_eq!(classify_command("python --version"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("python3 -V"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("python -VV"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("py -h"), (PolicyRisk::R0, "只读命令"));
        assert_eq!(classify_command("/usr/bin/python3 --version"), (PolicyRisk::R0, "只读命令"), "绝对路径按程序名匹配");
        assert_eq!(classify_command("python --version | cat"), (PolicyRisk::R0, "只读命令"), "管道两端都只读");
        assert_eq!(classify_command("python --version && git status"), (PolicyRisk::R0, "只读命令"));
    }

    #[test]
    fn python_execution_stays_non_readonly() {
        assert_eq!(classify_command("python script.py"), (PolicyRisk::R2, "在工作区内执行命令"), "脚本执行不判只读");
        assert_eq!(classify_command("python -c \"open('x','w')\""), (PolicyRisk::R2, "在工作区内执行命令"), "-c 任意代码不判只读");
        assert_eq!(classify_command("python -m http.server"), (PolicyRisk::R2, "在工作区内执行命令"), "-m 模块执行不判只读");
        assert_eq!(classify_command("python --version --user x"), (PolicyRisk::R2, "在工作区内执行命令"), "未知参数落只读白名单外");
        assert_eq!(classify_command("python --version > out.txt"), (PolicyRisk::R2, "在工作区内执行命令"), "重定向拒绝只读");
        assert_eq!(classify_command("python --version &"), (PolicyRisk::R2, "在工作区内执行命令"), "后台执行拒绝只读");
        assert_eq!(classify_command("python -m pip install requests"), (PolicyRisk::R3, "命令可能访问网络"), "网络优先于只读");
        assert_eq!(classify_command("sudo python --version"), (PolicyRisk::R4, "包含破坏性命令"), "sudo 优先于只读");
    }
}
