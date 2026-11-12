//! 提交历史遍历与渲染：对齐 `git log --oneline [--decorate] [-n N] [-- path]` 的输出形态
//! （spec 第 5 节 log）。
//!
//! 黄金对比基准是系统 git 的真实输出（票 #3 硬约束），渲染规则对齐 git 的 log-tree.c：
//! - 行格式：`<短hash> (<装饰, 装饰>) <首行标题>`；无装饰时省略括号段。
//! - 装饰：HEAD 最先（指向分支时 `HEAD -> <branch>`，分离态 `HEAD`），其余引用按完整
//!   引用名字典序（本地分支去 `refs/heads/`、远程跟踪去 `refs/remotes/`、标签加 `tag: `）。
//! - 排序：按提交时间新到旧（`git log` 默认顺序）。
//! - 短 hash：`core.abbrev` 自动策略下小仓库为 7 位。
//! - 路径过滤：提交相对第一父的 tree-diff 触及路径即显示（空历史起点用空树）。
//!
//! 已知与 git 的偏差（票内记录）：merge 提交的历史简化按「与第一父比较」近似实现，
//! 与 git 的 parent-selection 简化在含 merge 的历史上可能有顺序/数量差异（fixture
//! 矩阵与当前工具消费场景均为线性历史）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gix::bstr::BStr;
use gix::hash::ObjectId;
use gix::revision::walk::Sorting;
use gix::refs::{Category, FullNameRef};
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::engine::Repository;
use crate::errors::{Error, Result};

/// `git log` 的参数集（工具 `git_log` 与 envcontext 摘要共用）。
#[derive(Debug, Clone)]
pub struct LogOptions {
    /// 输出的提交条数上限（`-n N`；工具层 schema 限制 1~100）。
    pub limit: usize,
    /// 路径过滤（`-- <path>`）：只保留触及该路径的提交；`None` 不过滤。
    pub path: Option<Vec<u8>>,
    /// 是否渲染 `--decorate` 装饰（envcontext 摘要迁移前不带装饰，用 `false`）。
    pub decorate: bool,
}

/// 遍历从 HEAD 起的历史并渲染为 `git log --oneline [--decorate] [-n N] [-- path]` 同构文本。
///
/// 输出每行以 `\n` 结尾（含最后一行），调用方自行 trim。unborn HEAD（仓库刚 init、
/// 无提交）返回 [`Error::UnbornHead`]，其文案与系统 git 的 fatal 输出逐字一致。
/// `should_interrupt` 为协作式中断标志（超时包装置位后逐提交检查、尽快停止）。
pub fn log_text(
    repo: &Repository,
    options: LogOptions,
    should_interrupt: &Arc<AtomicBool>,
) -> Result<String> {
    let head = repo.inner().head().map_err(|err| Error::internal("读取 HEAD 失败", err))?;
    let mut decorations = BTreeMap::<ObjectId, Vec<String>>::new();
    // HEAD 的装饰信息（refs 遍历后再装配，避免借用冲突）：
    // - `head_token`：HEAD 对应的装饰 token（`HEAD -> <branch>` 或分离态 `HEAD`）；
    // - `head_absorbed_branch`：符号指向分支时被合并掉的分支显示名（跳过其独立 token）。
    let mut head_token: Option<(ObjectId, String)> = None;
    let mut head_absorbed_branch: Option<String> = None;

    let tip = match &head.kind {
        gix::head::Kind::Unborn(name) => {
            let branch = String::from_utf8_lossy(name.shorten()).into_owned();
            return Err(Error::UnbornHead { branch });
        }
        gix::head::Kind::Detached { target, .. } => {
            if options.decorate {
                head_token = Some((*target, "HEAD".to_string()));
            }
            *target
        }
        gix::head::Kind::Symbolic(symbolic) => {
            let full = symbolic.name.as_ref();
            let referent = repo
                .inner()
                .try_find_reference(full)
                .map_err(|err| Error::internal("读取分支引用失败", err))?;
            let Some(id) = referent.and_then(|reference| reference.try_id().map(|id| id.detach())) else {
                // 符号引用存在但不指向提交：与 unborn 同态（HEAD 尚未诞生）。
                let branch = String::from_utf8_lossy(full.shorten()).into_owned();
                return Err(Error::UnbornHead { branch });
            };
            if options.decorate {
                let branch = branch_short(full);
                head_token = Some((id, format!("HEAD -> {branch}")));
                head_absorbed_branch = Some(branch);
            }
            id
        }
    };

    if options.decorate {
        // 装饰装配对齐 git 的 decoration 链表语义（load_ref_decorations + 头插）：
        // refs 按完整引用名字典序遍历并头插 → 显示顺序为「字典序逆序」；HEAD 最后
        // 头插、占据链头（显示在最前）。实测锚点：`(HEAD -> master, origin/master)`、
        // 标准克隆 `(HEAD -> main, origin/main, origin/HEAD)`。
        let refs = repo
            .inner()
            .references()
            .map_err(|err| Error::internal("遍历引用失败", err))?;
        let iter = refs.all().map_err(|err| Error::internal("遍历引用失败", err))?;
        for reference in iter {
            let mut reference = reference.map_err(|err| Error::internal("读取引用失败", err))?;
            let full = reference.name();
            let Some(display) = decoration_display(full) else {
                continue;
            };
            // 标签对象 peel 到提交（git 的装饰挂在提交上）。
            let Ok(id) = reference.peel_to_id() else {
                continue;
            };
            decorations.entry(id.detach()).or_default().insert(0, display);
        }

        // HEAD 符号指向分支时，同提交上该分支的独立 token 被合并进 `HEAD -> <branch>`
        // （git 的 current_pointed_by_HEAD 行为：跳过分支条目，不单独显示）。
        if let Some(branch) = head_absorbed_branch
            && let Some(list) = decorations.get_mut(&tip)
        {
            list.retain(|token| *token != branch);
        }
        if let Some((id, token)) = head_token {
            // entry 兜底：分离 HEAD 且无任何其他引用时，`(HEAD)` 也应显示。
            decorations.entry(id).or_default().insert(0, token);
        }
    }

    let path = options.path.as_deref();
    let mut out = String::new();
    let mut shown = 0usize;
    for info in repo
        .inner()
        .rev_walk([tip])
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
        .all()
        .map_err(|err| Error::internal("初始化提交遍历失败", err))?
    {
        // 超时等价物：超时包装置位中断标志后，逐提交检查、立即停止。
        if should_interrupt.load(Ordering::SeqCst) {
            return Err(Error::internal("git log", "操作已超时中止"));
        }
        let info = info.map_err(|err| Error::internal("遍历提交历史失败", err))?;
        if let Some(path) = path
            && !commit_touches_path(repo.inner(), info.id, path)?
        {
            continue;
        }
        let short = info
            .id()
            .shorten()
            .map_err(|err| Error::internal("计算短哈希失败", err))?
            .to_string();
        // 提交对象先绑定再取标题：title 借用 commit 数据，不能让临时值先于借用消亡。
        let commit = info
            .object()
            .map_err(|err| Error::internal("读取提交对象失败", err))?;
        let title = commit
            .message()
            .map_err(|err| Error::internal("解析提交信息失败", err))?
            .title;
        match decorations.get(&info.id) {
            Some(list) => out.push_str(&format!("{short} ({}) {}\n", list.join(", "), String::from_utf8_lossy(title))),
            None => out.push_str(&format!("{short} {}\n", String::from_utf8_lossy(title))),
        }
        shown += 1;
        if shown >= options.limit {
            break;
        }
    }
    Ok(out)
}

/// 分支短名（`refs/heads/master` → `master`）。
fn branch_short(name: &FullNameRef) -> String {
    String::from_utf8_lossy(name.shorten()).into_owned()
}

/// 引用的装饰显示名（git log-tree.c 规则）；不参与装饰的引用返回 `None`。
///
/// 本地分支去 `refs/heads/`、远程跟踪去 `refs/remotes/`（含 `origin/HEAD` 这类特殊
/// 跟踪引用）、标签加 `tag: ` 前缀；其余引用（stash、notes 等）显示完整引用名。
fn decoration_display(name: &FullNameRef) -> Option<String> {
    match name.category_and_short_name() {
        Some((Category::LocalBranch, short)) => {
            Some(String::from_utf8_lossy(short).into_owned())
        }
        Some((Category::RemoteBranch, short)) => {
            Some(String::from_utf8_lossy(short).into_owned())
        }
        Some((Category::Tag, short)) => Some(format!("tag: {}", String::from_utf8_lossy(short))),
        _ => Some(String::from_utf8_lossy(name.as_bstr()).into_owned()),
    }
}

/// 提交是否触及 `path`：相对第一父（根提交相对空树）的 tree-diff 中存在路径匹配的变更。
///
/// 路径匹配语义对齐 git pathspec 的常用形态：精确文件匹配或目录前缀匹配（`dir` 覆盖
/// `dir/下任意`）。merge 提交按第一父比较（见模块注释的偏差记录）。
fn commit_touches_path(repo: &gix::Repository, commit: ObjectId, path: &[u8]) -> Result<bool> {
    let commit = repo
        .find_commit(commit)
        .map_err(|err| Error::internal("读取提交对象失败", err))?;
    let tree = commit
        .tree()
        .map_err(|err| Error::internal("读取提交树失败", err))?;
    // 路径过滤不做 rename 跟踪（git log 默认语义；`--follow` 才跟随重命名）。
    let diff_options = gix::diff::Options::default().with_rewrites(None);
    let first_parent_tree = commit
        .parent_ids()
        .next()
        .map(|parent| {
            repo.find_commit(parent)
                .and_then(|parent| parent.tree())
                .map_err(|err| Error::internal("读取父提交树失败", err))
        })
        .transpose()?;
    let changes = repo
        .diff_tree_to_tree(
            first_parent_tree.as_ref(),
            Some(&tree),
            Some(diff_options),
        )
        .map_err(|err| Error::internal("比较提交树失败", err))?;
    Ok(changes.iter().any(|change| path_matches(change.location(), path)))
}

/// git pathspec 常用语义：路径相等，或该路径是 `path` 目录下的条目。
///
/// 归一三条：git 路径一律正斜杠，Windows 上 `craft-policy::relative` 产出的展示
/// 路径是反斜杠，必须在入口换成 `/`；结尾分隔符不影响目录语义（`docs/` 与 `docs`
/// 等价——git 的目录 pathspec 也这么处理）；`"."` / `""` 表示「当前目录」即不过滤
/// （工具层未传 path 时 registry 用 `.` 表示整仓）。
pub(crate) fn path_matches(location: &BStr, path: &[u8]) -> bool {
    if path == b"." || path.is_empty() {
        return true;
    }
    let bytes: &[u8] = location.as_ref();
    let normalized: Vec<u8> = path.iter().map(|&b| if b == b'\\' { b'/' } else { b }).collect();
    let mut path = normalized.as_slice();
    // 末尾连续 `/` 去掉；全被去掉（`/`、`//`）时按不过滤处理，避免空前缀误匹配整棵树。
    while let [rest @ .., b'/'] = path {
        path = rest;
    }
    if path.is_empty() {
        return true;
    }
    bytes == path || (bytes.starts_with(path) && bytes.get(path.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_matches_exact_file_or_directory_prefix() {
        let location = BStr::new(b"src/lib.rs");
        assert!(path_matches(location, b"src/lib.rs"));
        assert!(path_matches(location, b"src"));
        assert!(!path_matches(location, b"src/lib"));
        // `srcx` 不能靠前缀误匹配 `src`
        assert!(!path_matches(location, b"srcx"));
        assert!(!path_matches(location, b"docs"));
    }

    /// Windows 上 `craft-policy::relative` 产出反斜杠展示路径；pathspec 必须在入口
    /// 归一为正斜杠，否则子目录路径永远不命中（`git_diff` 静默返回「没有差异」）。
    #[test]
    fn path_matches_normalizes_backslash_separator() {
        let location = BStr::new(b"docs/arch.md");
        assert!(path_matches(location, b"docs\\arch.md"));
        assert!(path_matches(location, b"docs"));
        assert!(path_matches(location, b"docs\\"));
        // 大小写与内容仍逐字节比较，归一不引入模糊匹配
        assert!(!path_matches(location, b"Docs\\arch.md"));
        assert!(!path_matches(location, b"docs\\other.md"));
    }

    /// `"."` 与空 pathspec 表示「当前目录」= 不过滤（工具层未传 path 时 registry
    /// 用 `.` 表示整仓，曾导致全仓 diff 变空）。
    #[test]
    fn path_matches_dot_means_no_filter() {
        for path in [&b"."[..], &b""[..]] {
            assert!(path_matches(BStr::new(b"src/lib.rs"), path));
            assert!(path_matches(BStr::new(b"docs/arch.md"), path));
        }
    }

    #[test]
    fn decoration_names_follow_git_rules() {
        let branch = gix_refs_full("refs/heads/master");
        assert_eq!(decoration_display(branch.as_ref()).as_deref(), Some("master"));
        let remote = gix_refs_full("refs/remotes/origin/master");
        assert_eq!(decoration_display(remote.as_ref()).as_deref(), Some("origin/master"));
        let tag = gix_refs_full("refs/tags/v1.0");
        assert_eq!(decoration_display(tag.as_ref()).as_deref(), Some("tag: v1.0"));
        let stash = gix_refs_full("refs/stash");
        assert_eq!(decoration_display(stash.as_ref()).as_deref(), Some("refs/stash"));
    }

    /// 构造完整引用名（校验合法性后经 `AsRef` 取 `FullNameRef`）。
    fn gix_refs_full(name: &'static str) -> gix::refs::FullName {
        gix::refs::FullName::try_from(name).expect("引用名合法")
    }
}
