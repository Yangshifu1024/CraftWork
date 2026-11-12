//! 测试支撑模块：fixture 仓库矩阵生成器 + 黄金对比 helper（票 #1 核心基建）。
//!
//! 只允许在测试代码里调用系统 git（`std::process::Command`）；生产代码路径零进程调用。
//! 本模块按测试二进制分别编译（`mod support;`），部分导出项并非每个二进制都用，
//! 故整体放宽 dead_code 检查。
//!
//! ## fixture 构建策略（二选一决策：系统 git 构建）
//!
//! fixture 仓库一律用系统 git 程序化生成，理由：
//! 1. 黄金对比的基准产出方就是系统 git——两侧读同一个「git 原生写法」的对象库，
//!    排除「fixture 本身由 gix 写、与 git 有微妙差异」的干扰变量；
//! 2. spec 第 6 节的互操作验收（gix 写、git 读，反向同样成立）要求 fixture 贴近
//!    「用户机器上的真实仓库」——它们都是 git 创建的；
//! 3. 场景布景（staged 改名、二进制、未跟踪折叠）用 git 命令表达最直接。
//!
//! ## 确定性与隔离
//!
//! - 子进程环境完全隔离：`GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` 指向 `/dev/null`，
//!   杜绝用户全局配置（user.name、init.defaultBranch、core.quotepath 等）影响布景与基准；
//! - 提交身份与日期固定 → HEAD 哈希字节级确定 → `git log` 黄金输出稳定；
//! - 每次 `create` 都新建独立临时目录（tempfile），无全局状态，测试可任意并行；
//! - 依赖系统 git ≥ 2.32（`GIT_CONFIG_SYSTEM` 语义）与 `-b`（2.28+）。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// fixture 统一分支名（隔离环境下 `init.defaultBranch` 不可达，显式指定）。
pub const FIXTURE_BRANCH: &str = "master";
/// fixture 统一提交身份（author 与 committer 同值）。
pub const FIXTURE_USER_NAME: &str = "craft-fixture";
/// fixture 统一提交邮箱。
pub const FIXTURE_USER_EMAIL: &str = "fixture@craftwork.invalid";

/// fixture 矩阵：覆盖票面列出的全部场景。
///
/// 票面「干净有提交」与「无 upstream」在 git 语义上是同一形态（有提交的本地仓库
/// 天然无远程跟踪），共用 [`FixtureKind::Clean`]；upstream 场景由其对照
/// [`FixtureKind::UpstreamAhead`] 覆盖（本地领先 1，`## master...origin/master [ahead 1]`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureKind {
    /// 空仓库：`git init` 后无任何提交，HEAD 指向尚未诞生的分支（unborn HEAD）；
    /// 工作树里留一个未跟踪文件。
    Unborn,
    /// 干净仓库：单一提交，无 upstream（同时覆盖票面「干净」「无 upstream」两个场景）。
    Clean,
    /// 脏仓库：修改（staged + unstaged）+ 删除（staged + unstaged）+ staged 改名
    /// + 二进制修改 + 未跟踪（含嵌套目录折叠）+ tracked 嵌套路径。
    DirtyMixed,
    /// 浅历史：3 个提交（日期递增、消息 ASCII）。
    ShortHistory,
    /// 有 upstream：本地 bare 远程 + 已推送基线 + 本地领先 1 提交（ahead 1，未 push）。
    UpstreamAhead,
    /// UTF-8：中文文件名（含中文嵌套目录）与中文提交信息，2 个提交。
    Utf8Names,
    /// diff（--staged）专用：基线提交后纯 staged 变更（工作树与 index 一致）——
    /// staged 文本修改（多 hunk + 函数上下文段）、staged 删除、staged 100% 改名、
    /// 二进制修改、新增（普通 / 无尾换行 / 空文件）。
    DiffStagedMixed,
    /// diff（默认模式）专用：基线提交后纯工作树变更（index 不动）——未暂存文本
    /// 修改（含无尾换行文件追加）、未暂存删除、二进制修改、可执行位变化（mode-only）、
    /// 已暂存修改（不出现在默认模式输出）与未跟踪文件（不参与 diff）。
    DiffWorktreeMixed,
    /// diff（中文文件名）专用：中文路径（含中文嵌套目录）的 staged 修改与
    /// 未暂存修改，验证 `diff --git` / `---` / `+++` 行的 quotepath 转义两态。
    DiffUtf8Mixed,
}

/// 生成的 fixture 仓库。
pub struct FixtureRepo {
    /// 持有临时根目录生命周期（drop 时整体清理）；路径本身经 `root_path` 取用，字段只为保活。
    #[allow(dead_code)]
    root: tempfile::TempDir,
    /// 工作仓库路径（`<root>/work`）。
    work: PathBuf,
    /// bare 远程路径（仅 [`FixtureKind::UpstreamAhead`] 有值，`<root>/remote.git`）。
    remote: Option<PathBuf>,
    /// HEAD 提交数（unborn 仓库为 0）。
    head_commits: usize,
    /// 是否配置了 upstream（origin 跟踪分支）。
    has_upstream: bool,
}

impl FixtureRepo {
    /// 工作仓库路径（所有黄金对比命令的工作目录）。
    pub fn path(&self) -> &Path {
        &self.work
    }

    /// bare 远程路径（`UpstreamAhead` 专用）。
    pub fn remote_path(&self) -> Option<&Path> {
        self.remote.as_deref()
    }

    /// HEAD 提交数（unborn 仓库为 0）。
    pub fn head_commits(&self) -> usize {
        self.head_commits
    }

    /// 是否配置了 upstream。
    pub fn has_upstream(&self) -> bool {
        self.has_upstream
    }

    /// 统一分支名。
    pub fn branch(&self) -> &'static str {
        FIXTURE_BRANCH
    }
}

/// 生成指定场景的 fixture 仓库。
///
/// 幂等：输入只有 `kind`；同样布景产生字节级一致的对象库（固定身份 + 固定日期）。
/// 可并行：独立临时目录，无共享状态。
pub fn create(kind: FixtureKind) -> Result<FixtureRepo, String> {
    let root = tempfile::tempdir().map_err(|e| format!("创建临时目录失败：{e}"))?;
    // 先取走根路径（TempDir 随后会被移进 FixtureRepo，之后不能再用其方法）。
    let root_path = root.path().to_path_buf();
    let work = root_path.join("work");
    std::fs::create_dir(&work).map_err(|e| format!("创建工作目录失败：{e}"))?;

    let mut repo = FixtureRepo {
        root,
        work: work.clone(),
        remote: None,
        head_commits: 0,
        has_upstream: false,
    };

    match kind {
        FixtureKind::Unborn => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(&work, "src/main.rs", "fn main() {}\n")?;
        }
        FixtureKind::Clean => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(&work, "README.md", "# fixture\n\nclean baseline\n")?;
            write_text(&work, "src/lib.rs", "pub const BASE: &str = \"clean\";\n")?;
            write_text(&work, "nested/deep/file.txt", "deep tracked file\n")?;
            commit_all(&work, "initial commit", 0)?;
            repo.head_commits = 1;
        }
        FixtureKind::DirtyMixed => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(&work, "README.md", "# fixture\n\nclean baseline\n")?;
            write_text(&work, "src/lib.rs", "pub const BASE: &str = \"clean\";\n")?;
            write_text(&work, "docs/a.md", "doc a\n")?;
            write_text(&work, "extra/stale.txt", "stale content\n")?;
            write_text(&work, "nested/deep/file.txt", "deep tracked file\n")?;
            write_bytes(&work, "assets/logo.png", &base_png_bytes())?;
            commit_all(&work, "base state", 0)?;

            // staged 修改：改动后 add。
            write_text(&work, "README.md", "# fixture\n\nmodified line added\n")?;
            git_fixture(&work, &["add", "README.md"])?;
            // unstaged 修改：只改工作树。
            write_text(&work, "src/lib.rs", "pub const BASE: &str = \"dirty\";\n")?;
            // staged 删除。
            git_fixture(&work, &["rm", "nested/deep/file.txt"])?;
            // unstaged 删除：直接移除工作树文件。
            std::fs::remove_file(work.join("extra/stale.txt"))
                .map_err(|e| format!("删除 extra/stale.txt 失败：{e}"))?;
            // staged 改名（git mv 保证 index 中为 rename 对）。
            git_fixture(&work, &["mv", "docs/a.md", "docs/b.md"])?;
            // 二进制修改（unstaged）：换一组不同长度字节。
            write_bytes(&work, "assets/logo.png", &modified_png_bytes())?;
            // 未跟踪：根级文件 + 嵌套目录（`git status --short` 会折叠为 `?? more/`）。
            write_text(&work, "scratch.txt", "untracked scratch\n")?;
            write_text(&work, "more/deep/scratch.txt", "untracked nested scratch\n")?;
            repo.head_commits = 1;
        }
        FixtureKind::ShortHistory => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            for (seq, (file, message)) in [
                ("f1.txt", "commit one"),
                ("f2.txt", "commit two"),
                ("f3.txt", "commit three"),
            ]
            .into_iter()
            .enumerate()
            {
                write_text(&work, file, &format!("content of {file}\n"))?;
                commit_all(&work, message, seq)?;
            }
            repo.head_commits = 3;
        }
        FixtureKind::UpstreamAhead => {
            let remote = root_path.join("remote.git");
            git_fixture(&root_path, &["init", "--bare", "-b", FIXTURE_BRANCH, "remote.git"])?;
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(&work, "README.md", "# fixture\n\nsynced with origin\n")?;
            commit_all(&work, "pushed baseline", 0)?;
            let remote_abs = remote.to_string_lossy().into_owned();
            git_fixture(&work, &["remote", "add", "origin", &remote_abs])?;
            git_fixture(&work, &["push", "-u", "origin", FIXTURE_BRANCH])?;
            // 本地再提交一次（不 push）→ ahead 1。
            write_text(&work, "local-only.txt", "ahead by one\n")?;
            commit_all(&work, "local ahead commit", 1)?;
            repo.remote = Some(remote);
            repo.head_commits = 2;
            repo.has_upstream = true;
        }
        FixtureKind::Utf8Names => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(&work, "设计文档/需求说明.md", "# 需求说明\n\n第一批需求条目\n")?;
            write_text(&work, "笔记.txt", "随手记录的第一条笔记\n")?;
            commit_all(&work, "初始提交：项目骨架说明", 0)?;
            write_text(&work, "设计文档/需求说明.md", "# 需求说明\n\n第一批需求条目\n第二批补充细节\n")?;
            commit_all(&work, "更新：补充设计细节", 1)?;
            // 留一个未跟踪的中文文件：status 输出中必有路径行，用于锚定
            // core.quotepath 对中文路径的转义 / 原样输出行为。
            write_text(&work, "草稿.md", "未提交的中文草稿\n")?;
            repo.head_commits = 2;
        }
        FixtureKind::DiffStagedMixed => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            // 多 hunk + 函数上下文的文本（行首字母行触发 func 段；第二处变更上方
            // 无可匹配行，验证「不越过上一 hunk 函数行」的 funclineprev 语义）。
            write_text(
                &work,
                "code.txt",
                "fn_alpha() {\n\
                 \x20   alpha_1\n\
                 \x20   alpha_2\n\
                 \x20   alpha_3\n\
                 \x20   alpha_4\n\
                 \x20   alpha_5\n\
                 \x20   alpha_6\n\
                 CHANGED_HERE_V1\n\
                 \x20   beta_1\n\
                 \x20   beta_2\n\
                 \x20   beta_3\n\
                 \x20   beta_4\n\
                 \x20   beta_5\n\
                 \x20   beta_6\n\
                 \x20   beta_7\n\
                 \x20   beta_8\n\
                 \x20   beta_9\n\
                 \x20   beta_10\n\
                 \x20   beta_11\n\
                 CHANGED_TOO_V1\n\
                 \x20   gamma_1\n\
                 \x20   gamma_2\n",
            )?;
            write_text(&work, "docs/doomed.txt", "first line\nsecond line\nthird line\n")?;
            write_text(&work, "docs/renamed_old.txt", "rename source line one\nline two\nline three\nline four\n")?;
            write_bytes(&work, "assets/pic.bin", &base_png_bytes())?;
            commit_all(&work, "diff staged baseline", 0)?;

            // staged 修改（两处内容变更，产生两个 hunk）。
            write_text(
                &work,
                "code.txt",
                "fn_alpha() {\n\
                 \x20   alpha_1\n\
                 \x20   alpha_2\n\
                 \x20   alpha_3\n\
                 \x20   alpha_4\n\
                 \x20   alpha_5\n\
                 \x20   alpha_6\n\
                 CHANGED_HERE_V2\n\
                 \x20   beta_1\n\
                 \x20   beta_2\n\
                 \x20   beta_3\n\
                 \x20   beta_4\n\
                 \x20   beta_5\n\
                 \x20   beta_6\n\
                 \x20   beta_7\n\
                 \x20   beta_8\n\
                 \x20   beta_9\n\
                 \x20   beta_10\n\
                 \x20   beta_11\n\
                 CHANGED_TOO_V2\n\
                 \x20   gamma_1\n\
                 \x20   gamma_2\n",
            )?;
            git_fixture(&work, &["add", "code.txt"])?;
            // staged 删除。
            git_fixture(&work, &["rm", "docs/doomed.txt"])?;
            // staged 100% 改名（git mv 保证 index 中为 rename 对、内容不变）。
            git_fixture(&work, &["mv", "docs/renamed_old.txt", "docs/renamed_new.txt"])?;
            // staged 二进制修改。
            write_bytes(&work, "assets/pic.bin", &modified_png_bytes())?;
            git_fixture(&work, &["add", "assets/pic.bin"])?;
            // staged 新增：普通文本 / 无尾换行 / 空文件。
            write_text(&work, "added.txt", "brand new content\nsecond new line\n")?;
            git_fixture(&work, &["add", "added.txt"])?;
            write_text(&work, "added_no_nl.txt", "no trailing newline")?;
            git_fixture(&work, &["add", "added_no_nl.txt"])?;
            write_text(&work, "added_empty.txt", "")?;
            git_fixture(&work, &["add", "added_empty.txt"])?;
            repo.head_commits = 1;
        }
        FixtureKind::DiffWorktreeMixed => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(
                &work,
                "wt_mod.txt",
                "fn_worktree() {\n\
                 \x20   w1\n\
                 \x20   w2\n\
                 \x20   w3\n\
                 \x20   w4\n",
            )?;
            write_text(&work, "wt_del.txt", "will be removed\nsecond line\n")?;
            write_bytes(&work, "wt_bin.bin", &base_png_bytes())?;
            write_text(&work, "wt_no_nl.txt", "tail line")?;
            write_text(&work, "wt_mode.sh", "echo unchanged content\n")?;
            write_text(&work, "wt_staged_mod.txt", "staged only change\n")?;
            commit_all(&work, "diff worktree baseline", 0)?;

            // 未暂存修改：中段改动（func 段）+ 可执行位变化（mode-only）。
            write_text(
                &work,
                "wt_mod.txt",
                "fn_worktree() {\n\
                 \x20   w1\n\
                 \x20   w2 CHANGED\n\
                 \x20   w3\n\
                 \x20   w4\n",
            )?;
            // 可执行位变化（git 无 chmod 子命令，直接改文件系统权限）。
            // POSIX mode 是 unix 语义：Windows 无此 API（曾让 CI 的 windows job 编译失败），
            // 跳过可执行位用例——文件模式变化的 diff 语义在 macOS/Linux 覆盖即可。
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let path = work.join("wt_mode.sh");
                let mut perm = std::fs::metadata(&path).map_err(|e| format!("读取 wt_mode.sh 元数据失败：{e}"))?.permissions();
                perm.set_mode(perm.mode() | 0o111);
                std::fs::set_permissions(&path, perm).map_err(|e| format!("改 wt_mode.sh 权限失败：{e}"))?;
            }
            // 未暂存删除：直接移除工作树文件。
            std::fs::remove_file(work.join("wt_del.txt")).map_err(|e| format!("删除 wt_del.txt 失败：{e}"))?;
            // 未暂存二进制修改。
            write_bytes(&work, "wt_bin.bin", &modified_png_bytes())?;
            // 无尾换行文件末尾追加行（末行失去「无换行」状态 + 新增行）。
            write_text(&work, "wt_no_nl.txt", "tail line\nappended with newline\n")?;
            // 已暂存修改（默认模式不输出）。
            write_text(&work, "wt_staged_mod.txt", "staged only change\nstaged extra line\n")?;
            git_fixture(&work, &["add", "wt_staged_mod.txt"])?;
            // 未跟踪文件（默认模式不输出）。
            write_text(&work, "wt_untracked.txt", "untracked must not appear\n")?;
            repo.head_commits = 1;
        }
        FixtureKind::DiffUtf8Mixed => {
            git_fixture(&work, &["init", "-b", FIXTURE_BRANCH])?;
            write_text(&work, "笔记/计划.txt", "第一行计划\n第二行计划\n第三行计划\n")?;
            write_text(&work, "设计文档/说明.md", "原始说明第一行\n原始说明第二行\n")?;
            commit_all(&work, "中文 diff 基线", 0)?;

            // staged 修改（工作树随后保持与 index 一致）。
            write_text(&work, "笔记/计划.txt", "第一行计划\n第二行计划已更新\n第三行计划\n")?;
            git_fixture(&work, &["add", "笔记/计划.txt"])?;
            // 未暂存修改。
            write_text(&work, "设计文档/说明.md", "原始说明第一行\n说明第二行改动了\n")?;
            repo.head_commits = 1;
        }
    }

    // 仓库级身份：worktree 系测试经 gix 创建分支需要 user.name/user.email；
    // CI runner 没有全局 git 身份（实测 IdentityMissing 挂掉全部 worktree 用例，
    // 本地有全局配置所以只在 CI 失败）。须在 git init 之后写（未 init 的目录
    // git config 会失败），放在 match 各分支完成之后统一生效。
    set_local_config(&work, "user.name", "CraftWork Test")?;
    set_local_config(&work, "user.email", "test@craftwork.invalid")?;

    Ok(repo)
}

// ---------- 黄金对比 helper ----------

/// 黄金对比用例：同一操作的双实现绑定。
///
/// - `git_args`：系统 git 命令参数（基准产出方），在 fixture 工作目录执行，
///   输出作为黄金基准；
/// - `actual`：craft-git 侧的同义实现（被测方），返回与 git 输出逐字节可比的内容。
///   本票用「二次系统 git 调用」占位验证链路端到端可用；M3~M5 票把 `actual`
///   换成 status / log / diff 的 craft-git 渲染实现，`run_golden_case` 不变。
pub struct GoldenCase {
    /// 用例名（断言失败消息中标识来源）。
    pub label: &'static str,
    /// 系统 git 命令参数（不含 `git` 本身）。
    pub git_args: &'static [&'static str],
    /// craft-git 侧实现：入参为工作仓库路径。
    pub actual: fn(&Path) -> Result<Vec<u8>, String>,
}

/// 执行一个黄金对比用例：跑系统 git 拿基准，跑 actual 拿实现输出，逐字节断言。
pub fn run_golden_case(repo: &FixtureRepo, case: &GoldenCase) -> Result<(), String> {
    let golden = capture_git(repo.path(), case.git_args)?;
    let actual = (case.actual)(repo.path())?;
    assert_golden_bytes_equal(case.label, &golden, &actual)
}

/// 捕获系统 git 命令输出（字节级，黄金基准）。
///
/// 与布景相同的隔离环境（杜绝用户全局配置漂移）；stdout 原样返回，
/// 由调用方决定按 UTF-8 文本还是字节处理。命令失败（非零退出）返回含 stderr 的错误。
pub fn capture_git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = run_git_raw(dir, args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {args:?} 失败（退出码 {:?}）：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// 以隔离配置打开 fixture 仓库（M3 status/log 渲染专用）。
///
/// 黄金基准经 [`capture_git`] 完全隔离宿主机全局配置，craft-git 侧若用普通
/// `discover` 会并入用户真实 `~/.gitconfig`（core.quotepath、core.abbrev、
/// status.showUntrackedFiles 等都影响渲染），两侧输入不对称；隔离打开后两侧
/// 都只见 fixture 自身的 local 配置，对比才成立。
pub fn discover_isolated(path: &Path) -> Result<craft_git::Repository, String> {
    craft_git::Repository::discover_isolated(path).map_err(|e| format!("隔离打开仓库失败：{e}"))
}

/// 写 fixture 仓库的 local 配置（测 `core.quotepath` 等格式开关的两态）。
///
/// local 级优先级最高，两侧（隔离的 git 与隔离打开的 gix）都读得到，对比对称。
pub fn set_local_config(dir: &Path, key: &str, value: &str) -> Result<(), String> {
    git_fixture(dir, &["config", key, value])?;
    Ok(())
}

/// 删除 fixture 仓库的 local 配置项。create() 现在统一写入仓库级身份，
/// 「身份缺失」类用例用它还原无身份前提。
pub fn remove_local_config(dir: &Path, key: &str) -> Result<(), String> {
    git_fixture(dir, &["config", "--unset", key])?;
    Ok(())
}

/// 新建未置位的中断标志（status/log 渲染调用需要传）。
pub fn interrupt_flag() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// 以**继承测试进程环境**的方式跑 git 并捕获输出。
///
/// 用途：与 gix 进程内读取做「同环境」对照——gix 的 config 快照读取的是测试进程
/// 的真实环境（无 GIT_CONFIG_GLOBAL 覆盖），这类对照（如全局 user.name）必须让
/// git 子进程读到同一环境，否则两侧来源不同、断言失真。布景与黄金基准一律用
/// [`capture_git`]（隔离环境），不要混用。
pub fn capture_git_inherited(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("启动系统 git 失败：{e}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {args:?} 失败（退出码 {:?}）：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// 布景用 git 调用：断言成功，返回 stdout 文本（一般布景命令 stdout 为空）。
pub fn git_fixture(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = run_git_raw(dir, args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "布景命令 git {args:?} 失败（退出码 {:?}）：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// 逐字节对比黄金基准与实现输出。
///
/// 相等通过；不等时返回的错误包含用例名、首个偏差的字节偏移、两侧字节长度
/// 与两侧内容的 lossy 文本预览，方便直接定位格式差异。
pub fn assert_golden_bytes_equal(label: &str, golden: &[u8], actual: &[u8]) -> Result<(), String> {
    if golden == actual {
        return Ok(());
    }
    let first_diff = golden
        .iter()
        .zip(actual.iter())
        .position(|(g, a)| g != a)
        .unwrap_or_else(|| golden.len().min(actual.len()));
    Err(format!(
        "黄金对比失败 [{label}]：输出不一致（首个偏差在字节偏移 {first_diff}）\n\
         基准（{} 字节）：{:?}\n\
         实现（{} 字节）：{:?}",
        golden.len(),
        String::from_utf8_lossy(golden),
        actual.len(),
        String::from_utf8_lossy(actual),
    ))
}

// ---------- 内部工具 ----------

/// 布景与捕获共用的 git 进程启动器：完全隔离的环境。
///
/// - `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` = `/dev/null`：屏蔽用户全局与系统配置；
/// - 固定 author / committer 身份：提交哈希确定；
/// - `TZ=UTC`、`LC_ALL=C`：时间与消息输出稳定。
///
/// 环境变量经 `Command::env` 传给子进程（进程级隔离），不触碰本测试进程的全局环境。
fn run_git_raw(dir: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", FIXTURE_USER_NAME)
        .env("GIT_AUTHOR_EMAIL", FIXTURE_USER_EMAIL)
        .env("GIT_COMMITTER_NAME", FIXTURE_USER_NAME)
        .env("GIT_COMMITTER_EMAIL", FIXTURE_USER_EMAIL)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("启动系统 git 失败（测试依赖 git ≥ 2.32）：{e}"))
}

/// 布景提交：add 全部 + 固定日期提交（日期按 `seq` 递增一天，保证哈希与顺序确定）。
///
/// 公开给集成测试使用（如 transport 测试在 clone 后向 fixture 追加提交，验证 fetch 增量）。
pub fn commit_all(dir: &Path, message: &str, seq: usize) -> Result<(), String> {
    git_fixture(dir, &["add", "-A"])?;
    let date = format!("2026-01-{:02}T00:00:00+00:00", 10 + seq);
    let output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", FIXTURE_USER_NAME)
        .env("GIT_AUTHOR_EMAIL", FIXTURE_USER_EMAIL)
        .env("GIT_COMMITTER_NAME", FIXTURE_USER_NAME)
        .env("GIT_COMMITTER_EMAIL", FIXTURE_USER_EMAIL)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("启动系统 git 失败：{e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "提交 {message:?} 失败（退出码 {:?}）：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// 写入 UTF-8 文本文件（自动创建父目录）。
fn write_text(dir: &Path, rel_path: &str, contents: &str) -> Result<(), String> {
    write_bytes(dir, rel_path, contents.as_bytes())
}

/// 写入字节文件（自动创建父目录）。
fn write_bytes(dir: &Path, rel_path: &str, bytes: &[u8]) -> Result<(), String> {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录 {parent:?} 失败：{e}"))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("写入 {rel_path} 失败：{e}"))
}

/// 基底二进制内容：PNG 魔数 + 占位字节。
fn base_png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend(0u8..=15);
    bytes
}

/// 修改后的二进制内容：长度不同且字节不同，保证二进制修改可检出。
fn modified_png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend((20u8..=40).rev());
    bytes
}
