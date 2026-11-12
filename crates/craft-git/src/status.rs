//! 工作树状态采集与渲染：对齐 `git status --short [--branch]` 的输出形态（spec 第 5 节 status）。
//!
//! 黄金对比基准是系统 git 的真实输出（票 #3 硬约束），因此渲染规则逐条对齐 git 的
//! `wt-status.c` 短格式行为：
//! - 分支头行：无 upstream `## <branch>`；有 upstream `## <branch>...<up>[ahead/behind]`；
//!   unborn HEAD `## No commits yet on <branch>`；detached `## HEAD (no branch)`。
//! - 变更行：两列状态码 `XY <path>`（X = HEAD→index 暂存态，Y = index→工作树态），
//!   同一路径的暂存与未暂存变更合并为一行；rename 行 `XY <old> -> <new>`（两侧各自 quote）。
//! - 行序：变更段在前（按目标路径字节序），未跟踪段最后（按显示路径字节序，折叠目录
//!   以 `dir/` 参与排序与显示——与 git 的排序键一致）。
//! - `core.quotepath`（默认 true）：含非 ASCII / 控制字符 / 引号 / 反斜杠的路径按 git 的
//!   C 风格引用规则转义并包双引号。
//!
//! 输出与 git 完全同构：每行以 `\n` 结尾（含最后一行），调用方（工具层）自行 trim。

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use gix::bstr::ByteSlice;
use gix::status;
use gix::status::index_worktree;
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

use crate::engine::Repository;
use crate::errors::{Error, Result};

/// 采集工作树状态并渲染为 `git status --short [--branch]` 同构文本。
///
/// * `with_branch_header`：是否输出 `##` 分支头行。工具 `git_status` 与侧栏徽标用
///   `true`（对齐 `git status --short --branch`）；envcontext 摘要用 `false`（分支
///   单独采集，对齐迁移前 `git status --short` 的输出形态）。
/// * `should_interrupt`：协作式中断标志——超时包装把它置 true 后，gix 在下一个检查点
///   停止采集并以错误返回（spec 第 4 节「超时与中断用协作式取消」）。
pub fn status_text(
    repo: &Repository,
    with_branch_header: bool,
    should_interrupt: &Arc<AtomicBool>,
) -> Result<String> {
    let quotepath = quotepath_enabled(repo);

    // git 的输出是字节流（quotepath=false 时可含任意字节）；按字节组装后 lossy 成
    // String（fixture 与常见仓库均为 UTF-8，lossy 无损；非 UTF-8 场景已在票内记为偏差）。
    let mut out: Vec<u8> = Vec::new();
    if with_branch_header {
        out.extend_from_slice(branch_header(repo)?.as_bytes());
    }

    let items = collect_changes(repo, should_interrupt)?;

    // 变更段：按目标路径字节序（git 的 string_list 语义；rename 行的键是目标路径）。
    let mut keys = Vec::with_capacity(items.changes.len());
    keys.extend(items.changes.keys().cloned());
    keys.sort();
    for path in keys {
        let row = &items.changes[&path];
        out.push(row.x as u8);
        out.push(row.y as u8);
        out.push(b' ');
        match &row.rename_from {
            Some(old) => {
                // rename 行两侧路径各自引用（git quote_two 行为）。
                out.extend_from_slice(&quote_path(old, quotepath));
                out.extend_from_slice(b" -> ");
                out.extend_from_slice(&quote_path(&path, quotepath));
            }
            None => out.extend_from_slice(&quote_path(&path, quotepath)),
        }
        out.push(b'\n');
    }

    // 未跟踪段：显示名（折叠目录带尾 `/`）按字节序，恒为 `??`。
    let mut untracked = items.untracked;
    untracked.sort();
    for path in untracked {
        out.extend_from_slice(b"?? ");
        out.extend_from_slice(&quote_path(&path, quotepath));
        out.push(b'\n');
    }

    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// 渲染 `##` 分支头行（不含未跟踪/变更行）。
///
/// 单独拆出供需要分支名的调用方复用；ahead / behind 经 rev-walk 对称差计数，
/// 语义对齐 `git rev-list --count <branch> ^<upstream>`（与 `git status` 的
/// ahead / behind 同源）。
fn branch_header(repo: &Repository) -> Result<String> {
    let head = repo.inner().head().map_err(|err| Error::internal("读取 HEAD 失败", err))?;
    match head.kind {
        gix::head::Kind::Unborn(name) => {
            let branch = String::from_utf8_lossy(name.shorten()).into_owned();
            Ok(format!("## No commits yet on {branch}\n"))
        }
        gix::head::Kind::Detached { .. } => Ok("## HEAD (no branch)\n".into()),
        gix::head::Kind::Symbolic(symbolic) => {
            let full = symbolic.name.as_ref();
            let branch = String::from_utf8_lossy(full.shorten()).into_owned();
            let Some(branch_ref) = repo
                .inner()
                .try_find_reference(full)
                .map_err(|err| Error::internal("读取分支引用失败", err))?
            else {
                // 符号引用指向的分支文件不存在且未诞生（理论 unreachable）：按无 upstream 呈现。
                return Ok(format!("## {branch}\n"));
            };

            // upstream：branch.<name>.<remote/merge> 经 fetch refspec 解析出的跟踪引用。
            // git 行为（实测锚点）：跟踪引用不存在时不显示 upstream 段（`## master`）。
            let upstream = branch_ref
                .remote_tracking_ref_name(gix::remote::Direction::Fetch)
                .map(|resolved| resolved.map_err(|err| Error::internal("解析 upstream 失败", err)));
            let upstream_name = match upstream {
                Some(Ok(full_name)) => repo
                    .inner()
                    .try_find_reference(&full_name)
                    .map_err(|err| Error::internal("读取 upstream 引用失败", err))?
                    .map(|_| full_name),
                // 未配置 upstream、解析失败或引用缺失：一律按「无 upstream」渲染。
                _ => None,
            };
            let Some(upstream_name) = upstream_name else {
                return Ok(format!("## {branch}\n"));
            };
            let upstream_short = upstream_name
                .as_bstr()
                .strip_prefix(b"refs/remotes/")
                .map(|rest| String::from_utf8_lossy(rest).into_owned())
                .unwrap_or_else(|| String::from_utf8_lossy(upstream_name.as_bstr()).into_owned());

            let head_id = branch_ref
                .try_id()
                .map(|id| id.detach())
                .ok_or_else(|| Error::internal("分支引用未指向提交", "symbolic ref without commit target"))?;
            let upstream_ref = repo
                .inner()
                .find_reference(&upstream_name)
                .map_err(|err| Error::internal("读取 upstream 引用失败", err))?;
            let upstream_id = upstream_ref
                .try_id()
                .map(|id| id.detach())
                .ok_or_else(|| Error::internal("upstream 引用未指向提交", "tracking ref without commit target"))?;
            let ahead = count_walk(repo, head_id, upstream_id)?;
            let behind = count_walk(repo, upstream_id, head_id)?;
            let suffix = match (ahead, behind) {
                (0, 0) => String::new(),
                (a, 0) => format!(" [ahead {a}]"),
                (0, b) => format!(" [behind {b}]"),
                (a, b) => format!(" [ahead {a}, behind {b}]"),
            };
            Ok(format!("## {branch}...{upstream_short}{suffix}\n"))
        }
    }
}

/// `rev-list --count tips ^hidden` 等价计数：tips 可达而 hidden（含其祖先）不可达的提交数。
pub(crate) fn count_walk(repo: &Repository, tips: gix::hash::ObjectId, hidden: gix::hash::ObjectId) -> Result<usize> {
    let mut count = 0usize;
    for item in repo
        .inner()
        .rev_walk([tips])
        .with_hidden([hidden])
        .all()
        .map_err(|err| Error::internal("计算 ahead/behind 失败", err))?
    {
        item.map_err(|err| Error::internal("遍历提交历史失败", err))?;
        count += 1;
    }
    Ok(count)
}

/// 变更行的内存形态：状态码两列 + 目标路径（rename 时含来源路径）。
struct Row {
    x: char,
    y: char,
    rename_from: Option<Vec<u8>>,
}

/// 一轮状态采集的全部结果。
struct Collected {
    /// 变更段（含 rename）：目标路径 → 行。
    changes: BTreeMap<Vec<u8>, Row>,
    /// 未跟踪段的显示路径（折叠目录带尾 `/`）。
    untracked: Vec<Vec<u8>>,
}

/// 工作树是否存在任何改动（变更段或未跟踪段非空即 true；未跟踪含折叠目录）。
///
/// `git status --short` 输出非空的进程内等价判定，供 worktree 的未提交改动检测
/// （[`crate::worktree::has_uncommitted_changes`]）等「有 / 无改动」布尔问询使用，
/// 不构造渲染文本。
pub fn has_changes(repo: &Repository, should_interrupt: &Arc<AtomicBool>) -> Result<bool> {
    let collected = collect_changes(repo, should_interrupt)?;
    Ok(!collected.changes.is_empty() || !collected.untracked.is_empty())
}

/// 执行 gix status 采集（HEAD→index 与 index→工作树两路 + 未跟踪目录折叠 + rename 检测）。
///
/// rename 检测与 git 默认对齐：暂存侧按 `status.renames`/`diff.renames` 配置（gix 的
/// `TrackRenames::AsConfigured` 默认即此语义，默认开启）；未暂存侧 git 无独立配置、
/// 默认检测，故显式传入 git 默认参数的 `Rewrites`（50% 相似度、候选上限 1000、不检测 copy）。
fn collect_changes(repo: &Repository, should_interrupt: &Arc<AtomicBool>) -> Result<Collected> {
    let mut collected = Collected { changes: BTreeMap::new(), untracked: Vec::new() };

    let iter = repo
        .inner()
        .status(gix::progress::Discard)
        .map_err(|err| Error::internal("初始化状态采集失败", err))?
        .index_worktree_rewrites(gix::diff::Rewrites {
            copies: None,
            percentage: Some(0.5),
            limit: 1000,
            track_empty: false,
        })
        .should_interrupt_owned(Arc::clone(should_interrupt))
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(|err| Error::internal("初始化状态采集失败", err))?;

    for item in iter {
        let item = item.map_err(|err| Error::internal("采集工作树状态失败", err))?;
        match item {
            // 暂存侧（HEAD tree → index）。
            status::Item::TreeIndex(change) => match change {
                gix::diff::index::Change::Addition { location, .. } => {
                    set_x(&mut collected, &location, 'A', None);
                }
                gix::diff::index::Change::Deletion { location, .. } => {
                    set_x(&mut collected, &location, 'D', None);
                }
                gix::diff::index::Change::Modification { location, .. } => {
                    set_x(&mut collected, &location, 'M', None);
                }
                gix::diff::index::Change::Rewrite { source_location, location, copy, .. } => {
                    let code = if copy { 'C' } else { 'R' };
                    set_x(&mut collected, &location, code, Some(source_location.as_ref().to_vec()));
                }
            },
            // 未暂存侧（index → 工作树）与未跟踪。
            status::Item::IndexWorktree(worktree_item) => match worktree_item {
                index_worktree::Item::Modification { rela_path, status, .. } => match status {
                    EntryStatus::Conflict { summary, .. } => {
                        let (x, y) = conflict_xy(summary);
                        row_for(&mut collected, rela_path.as_bstr()).x = x;
                        row_for(&mut collected, rela_path.as_bstr()).y = y;
                    }
                    // `git add -N`（intent-to-add）：git 显示为工作树侧新增（` A`）。
                    EntryStatus::IntentToAdd => {
                        row_for(&mut collected, rela_path.as_bstr()).y = 'A';
                    }
                    // NeedsUpdate 只是 stat 刷新建议，不构成变更行。
                    EntryStatus::NeedsUpdate(_) => {}
                    EntryStatus::Change(change) => {
                        let code = match change {
                            Change::Removed => 'D',
                            Change::Type { .. } => 'T',
                            // 子模块整体视作「有修改」（与 git 的 M 呈现一致）。
                            Change::Modification { .. } | Change::SubmoduleModification(_) => 'M',
                        };
                        row_for(&mut collected, rela_path.as_bstr()).y = code;
                    }
                },
                index_worktree::Item::DirectoryContents { entry, .. } => {
                    // 目录折叠模式下整目录一个条目；git 对目录显示 `dir/`（尾斜杠参与
                    // 显示与排序）。非未跟踪（ignored 等）一律不输出。
                    if entry.status == gix::dir::entry::Status::Untracked {
                        let mut path = entry.rela_path.as_bstr().to_vec();
                        if entry.disk_kind == Some(gix::dir::entry::Kind::Directory) {
                            path.push(b'/');
                        }
                        collected.untracked.push(path);
                    }
                }
                index_worktree::Item::Rewrite { source, dirwalk_entry, copy, .. } => {
                    // 未暂存 rename：来源是 index 中被判定删除的条目，目标来自目录遍历。
                    let code = if copy { 'C' } else { 'R' };
                    set_y(&mut collected, dirwalk_entry.rela_path.as_bstr(), code, Some(source.rela_path().to_vec()));
                }
            },
        }
    }

    Ok(collected)
}

/// 未合并冲突的两列状态码（git wt-status 的 unmerged 显示规则）。
fn conflict_xy(summary: gix::status::plumbing::index_as_worktree::Conflict) -> (char, char) {
    use gix::status::plumbing::index_as_worktree::Conflict;
    match summary {
        Conflict::BothDeleted => ('D', 'D'),
        Conflict::AddedByUs => ('A', 'U'),
        Conflict::DeletedByThem => ('D', 'U'),
        Conflict::AddedByThem => ('U', 'A'),
        Conflict::DeletedByUs => ('U', 'D'),
        Conflict::BothAdded => ('A', 'A'),
        Conflict::BothModified => ('U', 'U'),
    }
}

/// 取（或建）某目标路径的行；变更段的键恒为目标路径。
fn row_for<'a>(collected: &'a mut Collected, path: &[u8]) -> &'a mut Row {
    collected.changes.entry(path.to_vec()).or_insert(Row {
        x: ' ',
        y: ' ',
        rename_from: None,
    })
}

/// 写入暂存侧状态码；rename 行同时记录来源路径（git 显示 `old -> new`）。
fn set_x(collected: &mut Collected, path: &[u8], code: char, rename_from: Option<Vec<u8>>) {
    let row = row_for(collected, path);
    row.x = code;
    if rename_from.is_some() && row.rename_from.is_none() {
        row.rename_from = rename_from;
    }
}

/// 写入未暂存侧状态码（未暂存 rename 的目标是来自目录遍历的新路径）。
fn set_y(collected: &mut Collected, path: &[u8], code: char, rename_from: Option<Vec<u8>>) {
    let row = row_for(collected, path);
    row.y = code;
    if rename_from.is_some() && row.rename_from.is_none() {
        row.rename_from = rename_from;
    }
}

/// 读取 `core.quotepath`（git 默认 true：非 ASCII 路径转义）。
///
/// 显式 `false`/`no`/`off`/`0` 才关闭；其余值（含未配置）按 git 默认开启处理。
pub(crate) fn quotepath_enabled(repo: &Repository) -> bool {
    match repo.config_string("core.quotepath") {
        Some(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0"
        ),
        None => true,
    }
}

/// git 的 C 风格路径引用（`quote_c_style`）：需要转义时输出带双引号的形式。
///
/// 触发条件：控制字符、DEL、`"`、`\`，以及（quotepath 开启时）一切非 ASCII 字节。
/// 常见控制字符用字母缩写（`\a \b \t \n \v \f \r`），其余按三位八进制转义——
/// 与 git quote.c 的 cq_lookup 行为一致（锚点：中文「草」= `\350\215\211`）。
/// 供 status 与 diff 的路径渲染共用（`diff --git` / `---` / `+++` /
/// `Binary files` / `rename from/to` 行）。
pub(crate) fn quote_path(path: &[u8], quotepath: bool) -> Vec<u8> {
    let needs_quote = path
        .iter()
        .any(|&b| b < 0x20 || b == 0x7f || b == b'"' || b == b'\\' || (quotepath && b >= 0x80));
    if !needs_quote {
        return path.to_vec();
    }
    let mut out = Vec::with_capacity(path.len() + 2);
    out.push(b'"');
    for &b in path {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            _ if b < 0x20 || b == 0x7f || (quotepath && b >= 0x80) => {
                // 三位八进制（与 git 输出逐字节一致）。
                out.push(b'\\');
                out.extend_from_slice(format!("{b:03o}").as_bytes());
            }
            _ => out.push(b),
        }
    }
    out.push(b'"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_path_plain_ascii_stays_raw() {
        assert_eq!(quote_path(b"src/main.rs", true), b"src/main.rs");
        // 空格不触发引用（git 行为：含空格路径在 --short 下不加引号）。
        assert_eq!(quote_path(b"my file.txt", true), b"my file.txt");
    }

    #[test]
    fn quote_path_escapes_non_ascii_when_quotepath_on() {
        // 「草稿」的 UTF-8 字节序列 → \350\215\211\347\250\277（foundation 锚点取「草」一段）。
        let path = "草稿.md".as_bytes();
        assert_eq!(
            quote_path(path, true),
            "\"\\350\\215\\211\\347\\250\\277.md\"".as_bytes()
        );
        assert_eq!(quote_path(path, false), path);
    }

    #[test]
    fn quote_path_escapes_quote_backslash_and_control_chars() {
        assert_eq!(quote_path(b"a\"b", true), b"\"a\\\"b\"");
        assert_eq!(quote_path(b"a\\b", true), b"\"a\\\\b\"");
        assert_eq!(quote_path(b"a\tb", true), b"\"a\\tb\"");
        assert_eq!(quote_path(b"a\x01b", true), b"\"a\\001b\"");
        assert_eq!(quote_path(b"a\x7fb", false), b"\"a\\177b\"");
    }

    #[test]
    fn conflict_codes_match_git_unmerged_display() {
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::BothModified), ('U', 'U'));
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::BothAdded), ('A', 'A'));
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::BothDeleted), ('D', 'D'));
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::AddedByUs), ('A', 'U'));
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::AddedByThem), ('U', 'A'));
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::DeletedByUs), ('U', 'D'));
        assert_eq!(conflict_xy(gix::status::plumbing::index_as_worktree::Conflict::DeletedByThem), ('D', 'U'));
    }
}
