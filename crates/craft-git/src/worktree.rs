//! linked worktree 能力（spec 第 5 节 worktree，票 #6）：`git worktree add -b` 等价的
//! 分支创建 + 检出（含分支占用检查）、移除、未提交 / 未合并改动检测、失效管理目录
//! 清除（prune 自写——gix 0.90 无库级 prune API，spec 第 1 节已记录的真实缺口之一）。
//!
//! D9 语义对齐（D13硬约束 5）：
//! - 目录 `<数据目录>/worktrees/<会话id>`、分支 `craftwork/<会话短码>`、会话级绑定、
//!   删除级联清理的流程与目录布局由服务层（src-tauri services）保持不变，本模块只
//!   提供进程内等价的 git 原语；
//! - `add_worktree` 优先新建分支（`-b`），分支已存在（会话工作树目录丢失后重建场景）
//!   复用原分支——与迁移前服务层「先 `-b`、失败退 `worktree add <path> <branch>`」
//!   的两段式等价；
//! - 分支占用检查与 gix 的 `add_worktree` 内置冲突判定一致：同分支已在任何工作树
//!   检出时报 `CheckedOut` 冲突（等价 `git worktree add` 的 fatal）。
//!
//! 互操作（spec 第 6 节验收）：本模块产物（引用事务 + `.git/worktrees/<id>` 管理目录 +
//! 检出）与系统 git 的 worktree 元数据格式逐字段兼容（gitdir / HEAD / commondir /
//! index），互操作双向测试见 `tests/worktree.rs`。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::engine::{Repository, RepositoryKind};
use crate::errors::{Error, Result};

/// 分支创建 reflog 摘要（与 `git worktree add -b` 的默认 reflog 消息同语义）。
const BRANCH_CREATE_REFLOG: &str = "branch: Created from HEAD (craftwork worktree)";

/// `git worktree add -b <分支> <目标目录>` 等价：从 HEAD 创建分支并在目标目录检出
/// （分支不存在时创建，已存在时复用——服务层目录丢失重建场景，历史提交不丢）。
///
/// 分支占用检查内置：目标分支已在任何工作树检出时返回冲突错误（等价 git 的 fatal：
/// `'<branch>' is already used by worktree at ...`）。目标目录必须不存在或为未登记的
/// 空目录（gix 校验，失败自动清理已创建文件）。unborn HEAD（仓库无任何提交）与 bare
/// 仓库无法创建工作树，返回可读错误（对齐 git 的拒绝行为）。
pub fn add_worktree(repo_path: impl AsRef<Path>, destination: impl AsRef<Path>, branch: &str) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    add_worktree_in(&repo, destination.as_ref(), branch)
}

/// 已打开仓库版的 [`add_worktree`]（调用方需要先探测仓库形态或复用句柄时使用）。
pub fn add_worktree_in(repo: &Repository, destination: &Path, branch: &str) -> Result<()> {
    if matches!(repo.kind(), RepositoryKind::Bare) {
        return Err(Error::Internal {
            context: "创建 git 工作树失败".into(),
            detail: "快照（bare）仓库没有工作树，不能创建 linked worktree".into(),
        });
    }
    let head = repo.inner().head().map_err(|err| Error::internal("读取 HEAD 失败", err))?;
    // unborn HEAD 无提交可指向：对齐 `git worktree add -b` 的 fatal（not a valid object name）
    if let gix::head::Kind::Unborn(branch) = &head.kind {
        let name = String::from_utf8_lossy(branch.shorten()).into_owned();
        return Err(Error::Internal {
            context: "创建 git 工作树失败".into(),
            detail: format!("当前分支 '{name}' 还没有任何提交（unborn HEAD），无法从 HEAD 创建工作树分支"),
        });
    }
    let head_id = head.id().map(|id| id.detach()).ok_or_else(|| Error::Internal {
        context: "创建 git 工作树失败".into(),
        detail: "HEAD 未指向任何提交".into(),
    })?;

    let full_name = branch_full_name(branch)?;
    // 分支不存在 → ref 事务从 HEAD 创建（`-b` 语义）；已存在 → 复用（目录丢失重建场景，
    // 历史提交不丢）。占位检查交给检出一步（gix 内置 CheckedOut 冲突判定）。
    let branch_exists = repo
        .inner()
        .try_find_reference(full_name.as_ref())
        .map_err(|err| Error::internal("读取分支引用失败", err))?
        .is_some();
    if !branch_exists {
        // 分支 reflog 需要 committer 身份（与系统 git `worktree add -b` 的 reflog 写入
        // 同一要求）；身份级联缺失时报 IdentityMissing 可读错误（提交管线同一语义）。
        let identity = repo.identity()?;
        let time_text = gix::date::Time::now_local_or_utc().to_string();
        let committer = gix::actor::SignatureRef {
            name: gix::bstr::BStr::new(identity.name.as_bytes()),
            email: gix::bstr::BStr::new(identity.email.as_bytes()),
            time: time_text.as_str(),
        };
        let edit = gix::refs::transaction::RefEdit::update(
            full_name.clone(),
            gix::refs::Target::Object(head_id),
            gix::refs::transaction::PreviousValue::MustNotExist,
            BRANCH_CREATE_REFLOG,
        );
        repo.inner()
            .edit_references_as([edit], Some(committer))
            .map_err(|err| Error::internal(format!("创建分支 {branch} 失败"), err))?;
    }

    // 检出（分支占用检查在此完成：同分支已在任何工作树检出 → gix CheckedOut 冲突）
    let interrupt = AtomicBool::new(false);
    repo.inner()
        .add_worktree(destination, gix::worktree::add::Head::Attached(full_name), gix::progress::Discard, &interrupt)
        .map_err(|err| Error::internal("创建 git 工作树失败", err))?;
    normalize_worktree_gitdir_files(destination)?;
    Ok(())
}

/// 把 gix 写进工作树链接文件的 verbatim 路径改写成系统 git 可读的普通形式。
///
/// 背景：`gix_path::realpath` 的文档明确声明「returned path will be verbatim」，Windows
/// 上返回 `\\?\C:\...`。`gix` 的 `add_worktree` 把该路径原样写进两处链接文件：
/// - `<工作树>/.git`（内容 `gitdir: <主库>/.git/worktrees/<id>`）
/// - `<主库>/.git/worktrees/<id>/gitdir`（内容 `<工作树>/.git`）
///
/// 系统 git 不认 verbatim 写法，在工作树内任何命令都会报
/// `fatal: gitfile does not point to a valid repository`（实测复现）。本项目要求
/// **与系统 git 双向互操作**（spec 第 6 节），因此必须改写。Unix 上 realpath 无前缀，
/// 本函数不改变任何字节；仅当内容以 verbatim 前缀开头时才重写。
fn normalize_worktree_gitdir_files(destination: &Path) -> Result<()> {
    // 工作树的私有管理目录：由 gix 依据主库 .git/worktrees 下的登记反查。
    // 直接读工作树自己的 .git 文件拿到指向，避免依赖目录命名规则（gix 会按需加数字后缀）。
    let dot_git = destination.join(".git");
    let content = match std::fs::read_to_string(&dot_git) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::internal(format!("读取工作树链接文件失败：{}", dot_git.display()), err)),
    };
    let Some(raw) = content.trim_end().strip_prefix("gitdir: ") else {
        // 非标准内容（或已是系统 git 的目录形态）：不碰，交由互操作测试兜底
        return Ok(());
    };
    let worktree_dot_git = strip_verbatim_prefix(&dot_git.to_string_lossy());
    // 目标内容统一用正斜杠（git 链接文件的通用写法）：与 gix 写入时的
    // to_unix_separators_on_windows 一致，并顺带覆盖「无 verbatim 前缀但仍是反斜杠」的
    // Windows 短路径。两侧规范化后与原文逐字节相同则不动，保证 Unix 上零改动。
    let normalized_dot_git = worktree_dot_git.replace('\\', "/");
    let stripped_admin = strip_verbatim_prefix(raw);
    let normalized_admin = stripped_admin.replace('\\', "/");
    if normalized_admin == raw && normalized_dot_git == dot_git.to_string_lossy() {
        return Ok(());
    }
    let write = |path: &Path, value: &str, prefix: &str| -> Result<()> {
        std::fs::write(path, format!("{prefix}{value}\n"))
            .map_err(|err| Error::internal(format!("改写工作树链接文件失败：{}", path.display()), err))
    };
    write(&dot_git, &normalized_admin, "gitdir: ")?;
    // 主库侧登记文件：内容指向工作树的 .git（系统 git 读取它反查工作树位置）。
    // 定位管理目录用**剥掉 verbatim 后**的路径：带前缀的路径在非 Windows 平台不是合法
    // 路径。规范化后仍不可达时回退到原始写法，避免因路径风格差异漏改。
    let admin_dir = Path::new(&stripped_admin);
    let admin_dir = if admin_dir.is_dir() { admin_dir } else { Path::new(raw) };
    if admin_dir.is_dir() {
        write(&admin_dir.join("gitdir"), &normalized_dot_git, "")?;
    }
    Ok(())
}

/// 去掉 Windows verbatim 前缀，返回系统 git 可读形式。
///
/// 两种写法都要处理：gix 写链接文件前会调用 `to_unix_separators_on_windows`，
/// 因此盘符形式落盘时是正斜杠的 `//?/C:/a/b`；管理目录路径经 `Path` 传递时可能是
/// 反斜杠的 `\\?\C:\a\b`。只认其中一种就会漏掉另一边（实测：只处理反斜杠时
/// Windows 上的 gitfile 仍是 `//?/C:/...`，系统 git 照样打不开）。
///
/// UNC 的 `\\?\UNC\server\share` / `//?/UNC/server/share` 还原为 `\\server\share`。
/// 非 verbatim 输入原样返回，保证 Unix 路径零改动。
fn strip_verbatim_prefix(path: &str) -> String {
    // 先统一判定：verbatim 前缀有两种分隔符写法
    let (rest, unc) = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        (rest, true)
    } else if let Some(rest) = path.strip_prefix("//?/UNC/") {
        (rest, true)
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        (rest, false)
    } else if let Some(rest) = path.strip_prefix("//?/") {
        (rest, false)
    } else {
        return path.to_string();
    };
    if unc {
        return format!(r"\\{rest}");
    }
    rest.to_string()
}

/// 移除一个 linked worktree（等价 `git worktree remove [--force]`）：删除检出目录并
/// 清除主仓库 `.git/worktrees/<id>` 管理目录。`force = true` 时丢弃未提交改动与
/// 子模块（迁移前服务层删除清理用 `--force`，行为对齐）；非 force 时工作树有改动
/// 或含子模块会被拒绝。
///
/// 目标按路径匹配已登记的 linked worktree（支持工作树根目录路径；解析语义与 git 一致）。
pub fn remove_worktree(repo_path: impl AsRef<Path>, target: impl AsRef<Path>, force: bool) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let force_mode = if force { gix::worktree::remove::Force::DiscardChanges } else { gix::worktree::remove::Force::Never };
    repo.inner()
        .remove_worktree(target.as_ref(), force_mode, gix::progress::Discard)
        .map_err(|err| Error::internal("移除 git 工作树失败", err))?;
    Ok(())
}

/// 删除本地分支（等价 `git branch -D`：不检查合并状态，强制删除）。
///
/// 分支不存在时静默成功（与迁移前服务层「删除分支 best-effort、失败只落审计」的
/// 宽容语义一致——重复清理场景幂等）。
pub fn delete_branch(repo_path: impl AsRef<Path>, branch: &str) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let full_name = branch_full_name(branch)?;
    let edit = gix::refs::transaction::RefEdit::new(
        full_name,
        gix::refs::transaction::Change::Delete {
            expected: gix::refs::transaction::PreviousValue::Any,
            log: gix::refs::transaction::RefLog::AndReference,
        },
    );
    repo.inner()
        .edit_references_as([edit], None)
        .map_err(|err| Error::internal(format!("删除分支 {branch} 失败"), err))?;
    Ok(())
}

/// 工作树是否有未提交改动（变更 / 未跟踪任一非空即 true；删除确认框警告的数据源）。
///
/// 判定与 `git status --short` 的非空输出一致（含未跟踪），冲突（未合并条目）同样
/// 计入——有冲突必改动。
pub fn has_uncommitted_changes(repo_path: impl AsRef<Path>) -> Result<bool> {
    let repo = Repository::discover(repo_path)?;
    let interrupt = Arc::new(AtomicBool::new(false));
    crate::status::has_changes(&repo, &interrupt)
}

/// 工作树分支相对主仓库 HEAD 是否有未合并提交（删除确认框「含未合并的提交」警告的
/// 数据源）：工作树分支 tip 可达而主仓库 HEAD 不可达的提交数 > 0 即 true。
///
/// 主仓库 unborn HEAD（还没有任何提交）时，工作树分支的全部提交都算未合并；工作树
/// HEAD 未 born（分支无提交）或 detached（分离头取当前提交）按实际指向判定。
pub fn has_unmerged_commits(main_repo_path: impl AsRef<Path>, worktree_path: impl AsRef<Path>) -> Result<bool> {
    let main = Repository::open(main_repo_path)?;
    let worktree = Repository::open(worktree_path)?;
    let branch_tip = worktree
        .inner()
        .head()
        .map_err(|err| Error::internal("读取工作树 HEAD 失败", err))?
        .id()
        .map(|id| id.detach());
    let Some(branch_tip) = branch_tip else {
        // 工作树分支没有任何提交：谈不上未合并
        return Ok(false);
    };
    let main_head = main
        .inner()
        .head()
        .map_err(|err| Error::internal("读取主仓库 HEAD 失败", err))?
        .id()
        .map(|id| id.detach());
    let Some(main_head) = main_head else {
        // 主仓库还没有任何提交：分支提交全部未合并
        return Ok(true);
    };
    Ok(crate::status::count_walk(&main, branch_tip, main_head)? > 0)
}

/// 清除失效的 linked worktree 管理目录（等价 `git worktree prune`；gix 0.90 无库级
/// prune API，spec 第 1 节记录的真实缺口，按 git 的判定规则自写）：
/// 扫描 `<common_dir>/worktrees/<id>/gitdir` 记录的工作树侧 git 路径，目标不存在
/// （工作树目录被手动删除等）即整目录清除。管理目录不完整（gitdir 文件缺失）同样
/// 视为失效。返回被清除的管理目录名列表（供审计与日志）。
pub fn prune(repo_path: impl AsRef<Path>) -> Result<Vec<String>> {
    let repo = Repository::open(repo_path)?;
    let admin_root = repo.common_dir().join("worktrees");
    let Ok(entries) = std::fs::read_dir(&admin_root) else {
        // 无 worktrees 目录（从未创建过 linked worktree）：无需清理
        return Ok(Vec::new());
    };
    let mut pruned = Vec::new();
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().to_string();
        let admin_dir = entry.path();
        if !admin_dir.is_dir() {
            continue;
        }
        let gitdir_file = admin_dir.join("gitdir");
        let dead = match std::fs::read_to_string(&gitdir_file) {
            Ok(content) => {
                let gitdir = PathBuf::from(content.trim());
                !gitdir.exists()
            }
            Err(_) => true,
        };
        if dead {
            if let Err(err) = std::fs::remove_dir_all(&admin_dir) {
                return Err(Error::internal(
                    format!("清除失效工作树管理目录失败：{}", admin_dir.display()),
                    err,
                ));
            }
            pruned.push(id);
        }
    }
    Ok(pruned)
}

/// 分支短名 → 完整引用名（`refs/heads/<branch>`）；非法分支名返回可读错误。
fn branch_full_name(branch: &str) -> Result<gix::refs::FullName> {
    gix::refs::FullName::try_from(format!("refs/heads/{branch}"))
        .map_err(|err| Error::internal(format!("分支名不合法：{branch}"), err))
}

// ---------- 测试 ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_full_name_rejects_illegal_names() {
        // 含 `..` 或非法组件的分支名在 FullName 解析层被拒（防引用路径穿越）
        assert!(branch_full_name("craftwork/abc123").is_ok());
        assert!(branch_full_name("../escape").is_err());
        assert!(branch_full_name("").is_err());
    }

    /// verbatim 前缀剥离：Windows 上 gix 写出的两种写法都必须还原成系统 git 可读形式。
    /// gix 落盘前会做正反斜杠转换，所以盘符形式可能是 `//?/C:/...`；管理目录路径经
    /// `Path` 传递时是 `\\?\C:\...`。两种都要处理，否则 Windows 上 gitfile 仍是 verbatim。
    /// 非 verbatim 输入原样返回，保证 Unix 路径零改动。
    #[test]
    fn strip_verbatim_prefix_handles_both_separator_forms() {
        // 反斜杠写法（Path 传入的形态）
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\a\b"), r"C:\a\b");
        // 正斜杠写法（gix 经 to_unix_separators_on_windows 落盘的形态）
        assert_eq!(strip_verbatim_prefix("//?/C:/a/b"), "C:/a/b");
        // UNC 两种写法
        assert_eq!(strip_verbatim_prefix(r"\\?\UNC\server\share\x"), r"\\server\share\x");
        assert_eq!(strip_verbatim_prefix("//?/UNC/server/share/x"), r"\\server/share/x");
        // 非 verbatim 输入不变
        assert_eq!(strip_verbatim_prefix("/tmp/a/b"), "/tmp/a/b");
        assert_eq!(strip_verbatim_prefix(r"C:\a\b"), r"C:\a\b");
    }

    /// 改写逻辑的平台无关验证：伪造一处「gix 写出 verbatim 路径」的链接文件，
    /// 调用改写函数后必须变成系统 git 可读的普通路径形态。
    /// macOS 上 gix 不会产生 verbatim 前缀，因此这条用例是 Windows 修复在 Unix 上
    /// 唯一可执行的回归保护（真实 Windows 行为由 CI 的 windows runner 覆盖）。
    #[test]
    fn normalize_rewrites_verbatim_links_to_plain_form() {
        // 唯一目录：纳秒时间戳在 macOS 上会撞名（见 craft-core::testutil 注释）
        let root = craft_core::testutil::unique_temp_dir("wt-gitfile");
        // 布局：<root>/admin（主库 .git/worktrees/<id>）与 <root>/wt（工作树）
        let admin = root.join("admin");
        let worktree = root.join("wt");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        let dot_git = worktree.join(".git");
        // 按 gix 的真实落盘形态伪造：正斜杠 + `//?/` 前缀（实测 Windows 日志确认）
        let verbatim_admin = format!("//?/{}", admin.to_string_lossy().replace('\\', "/"));
        std::fs::write(&dot_git, format!("gitdir: {verbatim_admin}\n")).unwrap();
        let verbatim_dot_git = format!("//?/{}", dot_git.to_string_lossy().replace('\\', "/"));
        std::fs::write(admin.join("gitdir"), format!("{verbatim_dot_git}\n")).unwrap();

        normalize_worktree_gitdir_files(&worktree).expect("改写失败");

        let after = std::fs::read_to_string(&dot_git).unwrap();
        assert!(!after.contains("//?/") && !after.contains(r"\\?\"), "不应残留 verbatim 前缀：{after}");
        assert!(after.starts_with("gitdir: "), "格式保持 gitdir: 前缀：{after}");
        // 指向的管理目录必须真实存在（否则系统 git 仍会报错）
        let target = after.trim_start_matches("gitdir: ").trim_end();
        assert!(Path::new(target).is_dir(), "改写后指向的管理目录应可解析：{target}");

        let reverse = std::fs::read_to_string(admin.join("gitdir")).unwrap();
        assert!(!reverse.contains("//?/") && !reverse.contains(r"\\?\"), "不应残留 verbatim 前缀：{reverse}");
        assert!(reverse.trim().ends_with(".git"), "应指向工作树 .git：{reverse}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 非 verbatim 内容：改写只保证「不引入 verbatim 且统一正斜杠」，不要求逐字节
    /// 等于原文——Windows 上原文用反斜杠，统一成正斜杠正是 git 链接文件的通用写法。
    #[test]
    fn normalize_keeps_gitfile_when_already_plain() {
        let root = craft_core::testutil::unique_temp_dir("wt-gitfile-noop");
        let admin = root.join("admin");
        let worktree = root.join("wt");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        let dot_git = worktree.join(".git");
        std::fs::write(&dot_git, format!("gitdir: {}\n", admin.to_string_lossy())).unwrap();
        std::fs::write(admin.join("gitdir"), format!("{}\n", dot_git.to_string_lossy())).unwrap();

        normalize_worktree_gitdir_files(&worktree).expect("改写失败");

        let after = std::fs::read_to_string(&dot_git).unwrap();
        assert!(!after.contains("//?/") && !after.contains(r"\\?\"), "不应出现 verbatim 前缀：{after}");
        assert!(!after.contains('\\'), "应统一正斜杠：{after}");
        std::fs::remove_dir_all(&root).ok();
    }
}
