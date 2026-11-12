//! 提交间差异渲染：对齐 `git diff --no-color [--staged] [-- <path>]` 的输出形态
//! （spec 第 5 节 diff）。
//!
//! 黄金对比基准是系统 git 的真实输出（票 #4 硬约束），渲染规则对齐 git 的
//! diff.c / xdiff（xemit.c）：
//! - 两模式：`--staged` = HEAD tree vs index（unborn HEAD 用空树，对齐 git）；
//!   默认 = index vs 工作树。逐文件输出，文件间直接相连，按呈现路径字节序排序
//!   （与 git 的 diff 遍历顺序一致）。
//! - 文件级头：`diff --git a/<old> b/<new>`、`old/new mode`、`similarity index`、
//!   `rename from/to`、`new/deleted file mode`、`index <a>..<b>[ <mode>]`；
//!   文本内容用 blob 级 [`UnifiedDiff`]（上下文 3 行 + git 的 slider 启发式）渲染。
//! - hunk 头 `@@ -a[,b] +c[,d] @@` 按 git xdl_emit_hunk_hdr 规则拼装（长度为 1 时
//!   省略长度、长度为 0 时起点回退一行），gix 的 [`HunkHeader`] 的 `Display` 是
//!   恒定 `@@ -a,b +c,d @@` 格式，不能直接用。
//! - hunk 头的函数上下文段（`@@ … @@ <func>`）按 git 默认规则自实现：从 hunk
//!   上下文起点的前一行向上查找第一个「ASCII 字母 / `_` / `$`」开头的行（def_ff），
//!   截断 80 字节、去除尾部空白，且不越过上一个 hunk 已用的函数行（get_func_line
//!   的 funclineprev 语义）。无 `.gitattributes` 的 diff driver 时与 git 逐字节一致。
//! - 行内容 token 带换行（见「token 切分」），文件末行无换行时在该行后输出
//!   `\ No newline at end of file`。
//! - 路径过滤对齐 git 的常用 pathspec 形态（精确或目录前缀）；rename 对仅当
//!   两侧都被 pathspec 命中才按 rename 输出，单侧命中时拆成删除 / 新增
//!   （git diffcore 的实际行为，已实测锚定）。
//! - `index` 行的短 hash：非零侧用 `ObjectId::shorten()`（`core.abbrev` 自动
//!   策略，与 git 同语义）；缺失侧为 7 位零。工作树侧的新内容 hash 由 blob
//!   管线渲染时按 git 对象格式实时计算（零副作用，不写对象库）。
//!
//! ## token 切分
//!
//! 行级 diff 的 token 用「带换行」的行源（`ByteLines`），与 git xdiff 的行记录
//! 一致——文件末行缺换行时该行 token 与下一版本的对应行（带换行）不同，diff
//! 会如 git 一样输出整行 `-`/`+` 对。gix 的
//! [`gix::diff::blob::prepare_diff::Outcome::interned_input`] 用「去换行」切分
//! （为 diff 质量优化），与本目标不符，不使用。
//!
//! ## 已知偏差（票 #4 偏差清单）
//!
//! - partial rename（内容有差异的改名）的 `similarity index NN%` 用 gix 行级相似度
//!   换算（floor(相似度 * 100)），git 的分数算法不同，数值可能相差几点；
//!   100% rename（blob 相同）两侧一致（典型布景实测同为 71%，见黄金测试）。
//! - 用户 diff 配置（`diff.algorithm` / `diff.context`）不生效：固定 myers
//!   算法与上下文 3 行；staged 侧 rename 开关遵循 `status.renames` →
//!   `diff.renames` 配置级联（gix 的 AsConfigured）。
//! - 不支持 `.gitattributes` 的 diff driver（外部 diff 命令、funcname 正则、
//!   binary 属性覆盖），一律按默认行为处理。
//! - submodule（gitlink）与冲突文件不渲染（git 分别输出 Subproject commit 与
//!   combined diff）；`git add -N`（intent-to-add）的未暂存差异不输出。
//! - 二进制文件的 partial rename 检测缺失（gix 行级相似度无法处理二进制）；
//!   工作树侧二进制的 index 行 hash 从磁盘直读计算（filter 生效的二进制
//!   可能与 git 不同）。
//! - 裸仓库上发起默认模式 diff 直接报错（git 为 `fatal: this operation must be
//!   run in a work tree`，detail 保留原文）。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gix::bstr::{BStr, ByteSlice};
use gix::prelude::ObjectIdExt;
use gix::diff::blob::pipeline::{Mode as PipelineMode, WorktreeRoots};
use gix::diff::blob::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind};
use gix::diff::blob::{diff_with_slider_heuristics, InternedInput, UnifiedDiff};
use gix::hash::ObjectId;
use gix::object::tree::EntryKind;
use gix::status::index_worktree::RewriteSource;
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
use gix::status::tree_index::TrackRenames;

use crate::engine::Repository;
use crate::errors::{Error, Result};

/// `git diff` 的参数集（工具 `git_diff` 共用；R0 只读）。
#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    /// `true` = `--staged`（HEAD tree vs index）；`false` = 默认（index vs 工作树）。
    pub staged: bool,
    /// 路径过滤（`-- <path>`）：精确匹配或目录前缀；`None` 不过滤。
    pub path: Option<Vec<u8>>,
}

/// [`diff_text`] 的结果：渲染文本 + 过滤前后的文件数。
///
/// `changed_files` 是收集到的变更总数（未过滤），`shown_files` 是 pathspec 过滤后
/// 实际渲染的文件数。二者不等且 `text` 为空时，说明路径参数一个都没命中。
#[derive(Debug, Clone, Default)]
pub struct DiffText {
    /// 与 `git diff --no-color` 同构的文本（无差异时为空串）。
    pub text: String,
    /// 收集到的变更总数（pathspec 过滤前）。
    pub changed_files: usize,
    /// 实际渲染进 `text` 的文件数（pathspec 过滤后）。
    pub shown_files: usize,
}

/// 渲染两模式 diff 为 `git diff --no-color [--staged] [-- <path>]` 同构文本。
///
/// 输出与 git 完全同构：每行以 `\n` 结尾（含最后一行），调用方自行 trim。
/// `should_interrupt` 为协作式中断标志（超时包装置位后逐变更检查、尽快停止）。
///
/// 返回 [`DiffText`] 而非裸字符串：`changed_files` / `shown_files` 让调用方能区分
/// 「仓库里确实没有改动」与「有改动但被 pathspec 全滤掉」——后者多半是路径参数
/// 写错，静默渲染成空差异会让这类缺陷无从排查（曾发生：Windows 反斜杠展示路径
/// 与 `.` 整仓语义都曾让 `git_diff` 永远返回「没有差异」）。
pub fn diff_text(
    repo: &Repository,
    options: DiffOptions,
    should_interrupt: &Arc<AtomicBool>,
) -> Result<DiffText> {
    // 默认模式必须有工作树（裸仓库上 git 为 fatal，detail 保留 git 原文）。
    let workdir = repo.workdir().map(Path::to_path_buf);
    if !options.staged && workdir.is_none() {
        return Err(Error::internal(
            "无法在裸仓库中执行",
            "this operation must be run in a work tree",
        ));
    }
    let workdir = workdir.unwrap_or_default();

    let changes = if options.staged {
        collect_staged_changes(repo, should_interrupt)?
    } else {
        collect_worktree_changes(repo, should_interrupt)?
    };
    let changed_files = changes.len();

    // 呈现路径（rename 单侧命中时拆开后的路径）字节序 = git 的 diff 输出顺序。
    let mut presented = BTreeMap::new();
    for change in changes {
        for change in apply_path_filter(change, options.path.as_deref()) {
            presented.insert(change.new_path.clone(), change);
        }
    }

    let quotepath = crate::status::quotepath_enabled(repo);
    let mut out: Vec<u8> = Vec::new();
    for change in presented.values() {
        render_file_change(repo, change, &workdir, quotepath, options.staged, &mut out)?;
    }
    Ok(DiffText {
        text: String::from_utf8_lossy(&out).into_owned(),
        changed_files,
        shown_files: presented.len(),
    })
}

// ---------- 变更收集 ----------

/// 单文件变更（渲染的最小单位；字段齐全后与收集来源无关）。
///
/// * Addition：`old_*` 全 `None`；Deletion：`new_*` 全 `None`。
/// * rename/copy：`change_kind = Rename`，`old_path` 为来源、`new_path` 为目标。
/// * 工作树侧的内容变化：`new_id` 为 `None`（内容在磁盘上，hash 由 blob 渲染时
///   实时计算）；staged 侧全部 id 都已知。
#[derive(Clone)]
struct FileChange {
    change_kind: ChangeKind,
    old_path: Vec<u8>,
    new_path: Vec<u8>,
    old_id: Option<ObjectId>,
    new_id: Option<ObjectId>,
    old_kind: Option<EntryKind>,
    new_kind: Option<EntryKind>,
    copy: bool,
}

/// 变更形态（渲染分支的事实源；id 是否已知与形态无关——工作树侧的内容
/// 变化 `new_id` 恒为 `None`，hash 由渲染实时计算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    /// 新增（`/dev/null` → 内容）。
    Addition,
    /// 删除（内容 → `/dev/null`）。
    Deletion,
    /// 修改（含 mode-only / 类型变化；`old_path == new_path`）。
    Modification,
    /// 改名或复制（`copy = true` 时按 git 的 `copy from/to` 渲染）。
    Rename,
}

/// `--staged`：HEAD tree vs index（unborn HEAD 用空树，等价 git 行为）。
///
/// rename 检测用 [`TrackRenames::AsConfigured`]（`status.renames` → `diff.renames`
/// → 默认开启），与 git 的配置级联一致。
fn collect_staged_changes(
    repo: &Repository,
    should_interrupt: &Arc<AtomicBool>,
) -> Result<Vec<FileChange>> {
    let inner = repo.inner();
    let tree_id = inner
        .head_tree_id_or_empty()
        .map_err(|err| Error::internal("读取 HEAD 树失败", err))?;
    let index = inner
        .index_or_empty()
        .map_err(|err| Error::internal("读取 index 失败", err))?;

    let mut changes = Vec::new();
    let outcome = inner
        .tree_index_status(
            &tree_id,
            &index,
            None,
            TrackRenames::AsConfigured,
            |change, _tree_index, _worktree_index| {
                // 协作式中断：超时包装置位后停止收集（回调返回 Break）。
                if should_interrupt.load(Ordering::SeqCst) {
                    return Ok(std::ops::ControlFlow::Break(()));
                }
                changes.push(file_change_from_index(change));
                Ok(std::ops::ControlFlow::Continue(()))
            },
        )
        .map_err(|err| Error::internal("比较 HEAD 树与 index 失败", err))?;
    drop(outcome);
    // 收集中断后按错误返回，保持「超时 = 失败」语义（与 log 的中断行为一致）。
    if should_interrupt.load(Ordering::SeqCst) {
        return Err(Error::internal("git diff", "操作已超时中止"));
    }
    Ok(changes)
}

/// index 侧变更 → [`FileChange`]（Addition / Deletion / Modification / Rewrite）。
fn file_change_from_index(change: gix::diff::index::ChangeRef<'_, '_>) -> FileChange {
    use gix::diff::index::ChangeRef as IndexChange;
    match change {
        IndexChange::Addition { location, entry_mode, id, .. } => FileChange {
            change_kind: ChangeKind::Addition,
            old_path: location.as_ref().to_vec(),
            new_path: location.as_ref().to_vec(),
            old_id: None,
            new_id: Some(id.into_owned()),
            old_kind: None,
            new_kind: Some(entry_mode_kind(entry_mode)),
            copy: false,
        },
        IndexChange::Deletion { location, entry_mode, id, .. } => FileChange {
            change_kind: ChangeKind::Deletion,
            old_path: location.as_ref().to_vec(),
            new_path: location.as_ref().to_vec(),
            old_id: Some(id.into_owned()),
            new_id: None,
            old_kind: Some(entry_mode_kind(entry_mode)),
            new_kind: None,
            copy: false,
        },
        IndexChange::Modification {
            location,
            previous_entry_mode,
            previous_id,
            entry_mode,
            id,
            ..
        } => FileChange {
            change_kind: ChangeKind::Modification,
            old_path: location.as_ref().to_vec(),
            new_path: location.as_ref().to_vec(),
            old_id: Some(previous_id.into_owned()),
            new_id: Some(id.into_owned()),
            old_kind: Some(entry_mode_kind(previous_entry_mode)),
            new_kind: Some(entry_mode_kind(entry_mode)),
            copy: false,
        },
        IndexChange::Rewrite {
            source_location,
            source_entry_mode,
            source_id,
            location,
            entry_mode,
            id,
            copy,
            ..
        } => FileChange {
            change_kind: ChangeKind::Rename,
            old_path: source_location.as_ref().to_vec(),
            new_path: location.as_ref().to_vec(),
            old_id: Some(source_id.into_owned()),
            new_id: Some(id.into_owned()),
            old_kind: Some(entry_mode_kind(source_entry_mode)),
            new_kind: Some(entry_mode_kind(entry_mode)),
            copy,
        },
    }
}

/// 默认模式：index vs 工作树。
///
/// **不检测 rename**（实测锚定的 git 行为）：`git diff` 的比较集合只含 index
/// 条目，未跟踪文件不参与 diff，因此「工作树删除 + 未跟踪新文件」不会像
/// `git status` 那样配对成 rename（status.rs 的未暂存 rename 检测服务于
/// status 输出，与 diff 的集合语义不同），只显示删除。
fn collect_worktree_changes(
    repo: &Repository,
    should_interrupt: &Arc<AtomicBool>,
) -> Result<Vec<FileChange>> {
    let iter = repo
        .inner()
        .status(gix::progress::Discard)
        .map_err(|err| Error::internal("初始化状态采集失败", err))?
        // 关闭 rewrite 跟踪（None）：对齐 `git diff` 不做未暂存 rename 的事实。
        .index_worktree_rewrites(None)
        .should_interrupt_owned(Arc::clone(should_interrupt))
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(|err| Error::internal("初始化状态采集失败", err))?;

    let mut changes = Vec::new();
    for item in iter {
        let item = item.map_err(|err| Error::internal("采集工作树状态失败", err))?;
        match item {
            // index↔工作树迭代只产 IndexWorktree 项（TreeIndex 属 tree-diff 迭代）。
            gix::status::Item::TreeIndex(_) => {}
            gix::status::Item::IndexWorktree(item) => match item {
                gix::status::index_worktree::Item::Modification { entry, rela_path, status, .. } => match status {
                // `git add -N`（intent-to-add）与冲突文件的未暂存差异不输出
                // （git 分别输出 new file 形态与 combined diff，偏差清单）。
                EntryStatus::IntentToAdd | EntryStatus::Conflict { .. } => {}
                // NeedsUpdate 只是 stat 刷新建议，不构成变更。
                EntryStatus::NeedsUpdate(_) => {}
                EntryStatus::Change(change) => {
                    let path: &[u8] = rela_path.as_ref();
                    match change {
                        Change::Removed => changes.push(FileChange {
                            change_kind: ChangeKind::Deletion,
                            old_path: path.to_vec(),
                            new_path: path.to_vec(),
                            old_id: Some(entry.id),
                            new_id: None,
                            old_kind: Some(entry_mode_kind(entry.mode)),
                            new_kind: None,
                            copy: false,
                        }),
                        // 类型变化按「mode 变化 + 内容 diff」处理（git 的呈现形态）。
                        Change::Type { worktree_mode } => changes.push(FileChange {
                            change_kind: ChangeKind::Modification,
                            old_path: path.to_vec(),
                            new_path: path.to_vec(),
                            old_id: Some(entry.id),
                            new_id: None,
                            old_kind: Some(entry_mode_kind(entry.mode)),
                            new_kind: Some(entry_mode_kind(worktree_mode)),
                            copy: false,
                        }),
                        // 内容变化（可能伴随可执行位变化）；子模块不渲染（偏差清单）。
                        Change::Modification { executable_bit_changed, .. } => {
                            let old_kind = entry_mode_kind(entry.mode);
                            let new_kind = if executable_bit_changed {
                                toggle_executable(old_kind)
                            } else {
                                old_kind
                            };
                            changes.push(FileChange {
                                change_kind: ChangeKind::Modification,
                                old_path: path.to_vec(),
                                new_path: path.to_vec(),
                                old_id: Some(entry.id),
                                new_id: None,
                                old_kind: Some(old_kind),
                                new_kind: Some(new_kind),
                                copy: false,
                            });
                        }
                        // 子模块整体视作修改不渲染（偏差清单）。
                        Change::SubmoduleModification(_) => {}
                    }
                }
                }
                gix::status::index_worktree::Item::DirectoryContents { .. } => {
                    // 未跟踪 / 忽略条目不参与 diff（git 行为：未跟踪文件不出现在 diff 中）。
                }
                gix::status::index_worktree::Item::Rewrite { source, dirwalk_entry, dirwalk_entry_id, copy, .. } => {
                let (source_entry, source_rela_path) = match source {
                    RewriteSource::RewriteFromIndex { source_entry, source_rela_path, .. } => {
                        (source_entry, source_rela_path)
                    }
                    // copy 来源在目录遍历（`Rewrites::copies = None` 时不会出现）。
                    RewriteSource::CopyFromDirectoryEntry { .. } => continue,
                };
                // 工作树侧 kind：dirwalk 的 Kind 不区分可执行位，用磁盘元数据补齐
                // （仅影响 index 行的 mode 显示）。
                let new_kind = match dirwalk_entry.disk_kind {
                    Some(gix::dir::entry::Kind::Symlink) => Some(EntryKind::Link),
                    Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Repository) => {
                        Some(worktree_file_kind(repo, dirwalk_entry.rela_path.as_bytes()))
                    }
                    _ => Some(EntryKind::Blob),
                };
                changes.push(FileChange {
                    change_kind: ChangeKind::Rename,
                    old_path: source_rela_path.as_bytes().to_vec(),
                    new_path: dirwalk_entry.rela_path.as_bytes().to_vec(),
                    old_id: Some(source_entry.id),
                    new_id: Some(dirwalk_entry_id),
                    old_kind: Some(entry_mode_kind(source_entry.mode)),
                    new_kind,
                    copy,
                });
                }
            }
        }
    }
    Ok(changes)
}

/// 读工作树文件的可执行位（rename 目标的 mode 显示与提交管线的新文件 mode 判定共用）。
pub(crate) fn worktree_file_kind(repo: &Repository, rela_path: &[u8]) -> EntryKind {
    let Some(workdir) = repo.workdir() else {
        return EntryKind::Blob;
    };
    let Ok(rel) = gix::path::from_bstr(BStr::new(rela_path)) else {
        return EntryKind::Blob;
    };
    let path = workdir.join(&*rel);
    let Ok(meta) = path.symlink_metadata() else {
        return EntryKind::Blob;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 != 0 {
            EntryKind::BlobExecutable
        } else {
            EntryKind::Blob
        }
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        EntryKind::Blob
    }
}

/// 路径过滤（对齐 git 常用 pathspec + rename 单侧命中拆分）。
///
/// rename 对仅在两侧都命中时保持 rename 形态；仅旧路径命中拆成删除、仅新路径
/// 命中拆成新增（git diffcore 的实际行为，已实测锚定）。
fn apply_path_filter(change: FileChange, path: Option<&[u8]>) -> Vec<FileChange> {
    let Some(path) = path else { return vec![change] };
    let old_matches = crate::log::path_matches(BStr::new(&change.old_path), path);
    let new_matches = crate::log::path_matches(BStr::new(&change.new_path), path);
    if change.change_kind != ChangeKind::Rename {
        return old_matches.then_some(change).into_iter().collect();
    }
    if old_matches && new_matches {
        return vec![change];
    }
    if old_matches {
        // 拆成删除（rename 的来源侧；呈现路径为旧路径）。
        vec![FileChange {
            change_kind: ChangeKind::Deletion,
            new_path: change.old_path.clone(),
            new_id: None,
            new_kind: None,
            copy: false,
            ..change
        }]
    } else if new_matches {
        // 拆成新增（rename 的目标侧；呈现路径为新路径）。
        vec![FileChange {
            change_kind: ChangeKind::Addition,
            old_path: change.new_path.clone(),
            old_id: None,
            old_kind: None,
            copy: false,
            ..change
        }]
    } else {
        Vec::new()
    }
}

// ---------- 文件级渲染 ----------

/// 渲染单个文件的 patch（头拼装 + blob diff），写入 `out`。
fn render_file_change(
    repo: &Repository,
    change: &FileChange,
    workdir: &Path,
    quotepath: bool,
    staged: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    // blob 级渲染先做：hunk 是否为空、是否二进制、工作树侧新内容 hash 都影响
    // 头部的行拼装。
    let blob = render_blob(repo, change, workdir, staged)?;

    out.extend_from_slice(b"diff --git ");
    out.extend_from_slice(&quote_prefixed_path(b"a/", &change.old_path, quotepath));
    out.extend_from_slice(b" ");
    out.extend_from_slice(&quote_prefixed_path(b"b/", &change.new_path, quotepath));
    out.push(b'\n');

    // mode 变化（含类型变化）：`old mode` / `new mode` 两行。
    let mode_changed = change.old_kind.is_some()
        && change.new_kind.is_some()
        && change.old_kind != change.new_kind;
    if mode_changed {
        emit_mode_line(out, b"old mode", change.old_kind);
        emit_mode_line(out, b"new mode", change.new_kind);
    }

    match change.change_kind {
        ChangeKind::Rename => {
            // rename/copy：similarity（100% = blob 相同；partial 用 gix 行级相似度，
            // 数值与 git 的分数算法可能不同——票内偏差）。
            let similarity = match (change.old_id, change.new_id) {
                (Some(old), Some(new)) if old == new => 1.0,
                _ => blob.similarity.unwrap_or(0.0),
            };
            out.extend_from_slice(
                format!("similarity index {}%\n", (similarity * 100.0) as u32).as_bytes(),
            );
            let verb: &[u8] = if change.copy { b"copy" } else { b"rename" };
            out.extend_from_slice(verb);
            out.extend_from_slice(b" from ");
            out.extend_from_slice(&crate::status::quote_path(&change.old_path, quotepath));
            out.push(b'\n');
            out.extend_from_slice(verb);
            out.extend_from_slice(b" to ");
            out.extend_from_slice(&crate::status::quote_path(&change.new_path, quotepath));
            out.push(b'\n');
        }
        ChangeKind::Addition => emit_mode_line(out, b"new file mode", change.new_kind),
        ChangeKind::Deletion => emit_mode_line(out, b"deleted file mode", change.old_kind),
        ChangeKind::Modification => {}
    }

    // index 行：内容有差异（或一侧缺失）时输出；100% rename（blob 相同）无
    // index 行（git 行为：仅 similarity + from/to）；mode-only（内容相同）也
    // 不输出。mode 尾巴仅在两侧 kind 都存在且相等时显示（新增 / 删除 / mode
    // 变化时都不带）。
    let identical_ids = matches!((change.old_id, change.new_id), (Some(a), Some(b)) if a == b);
    let one_way = matches!(change.change_kind, ChangeKind::Addition | ChangeKind::Deletion);
    let content_changed = match (change.old_id, change.new_id) {
        (Some(a), Some(b)) => a != b,
        // 工作树侧内容变化：hash 实时算，是否真的变了以渲染结果为准。
        (Some(_), None) => blob.is_binary || !blob.hunks.is_empty(),
        _ => true,
    };
    if one_way || (!identical_ids && content_changed) {
        let zero = "0000000";
        let old_hash = match change.old_id {
            Some(id) => short_hash(repo, id)?,
            None => zero.to_string(),
        };
        let new_hash = match change.new_id.or(blob.new_hash) {
            Some(id) => short_hash(repo, id)?,
            None => zero.to_string(),
        };
        out.extend_from_slice(format!("index {old_hash}..{new_hash}").as_bytes());
        if change.old_kind.is_some() && change.new_kind.is_some() && !mode_changed {
            out.push(b' ');
            out.extend_from_slice(entry_kind_str(change.new_kind));
        }
        out.push(b'\n');
    }

    // `---`/`+++`/`Binary files` 行的两侧存在性以「该侧 kind 是否存在」为准
    //（与 id 无关：工作树侧内容在磁盘上，id 恒为 None）。
    let old_exists = change.old_kind.is_some();
    let new_exists = change.new_kind.is_some();
    if blob.is_binary {
        // `Binary files <a> and <b> differ`（缺失侧为 /dev/null）。
        out.extend_from_slice(b"Binary files ");
        emit_diff_label(out, b"a/", &change.old_path, old_exists, quotepath);
        out.extend_from_slice(b" and ");
        emit_diff_label(out, b"b/", &change.new_path, new_exists, quotepath);
        out.extend_from_slice(b" differ\n");
        return Ok(());
    }
    if blob.hunks.is_empty() {
        // 无内容差异（mode-only / 新增空文件）：不出 `---`/`+++` 与 hunk。
        return Ok(());
    }
    out.extend_from_slice(b"--- ");
    emit_diff_label(out, b"a/", &change.old_path, old_exists, quotepath);
    out.push(b'\n');
    out.extend_from_slice(b"+++ ");
    emit_diff_label(out, b"b/", &change.new_path, new_exists, quotepath);
    out.push(b'\n');
    out.extend_from_slice(&blob.hunks);
    Ok(())
}

/// `---`/`+++`/`Binary files` 行的一侧标签：存在时 `<prefix><quoted>`，缺失时 `/dev/null`。
///
/// 引号包住 `a/` 前缀 + 路径整体（git 的 quote_c_style 作用在拼接后的完整
/// 路径上，实测锚点：`"a/\344\272\214...bin"`）；`rename from/to` 行的引号
/// 只包路径本身（prefix 在引号外，另见 `render_file_change`）。
fn emit_diff_label(out: &mut Vec<u8>, prefix: &[u8], path: &[u8], exists: bool, quotepath: bool) {
    if exists {
        out.extend_from_slice(&quote_prefixed_path(prefix, path, quotepath));
    } else {
        out.extend_from_slice(b"/dev/null");
    }
}

/// 引号包住「前缀 + 路径」整体：需要转义时输出 `"<prefix><escaped>"`，
/// 否则原样拼接（git 对 `a/<path>` 的引用语义）。
fn quote_prefixed_path(prefix: &[u8], path: &[u8], quotepath: bool) -> Vec<u8> {
    let mut joined = Vec::with_capacity(prefix.len() + path.len());
    joined.extend_from_slice(prefix);
    joined.extend_from_slice(path);
    crate::status::quote_path(&joined, quotepath)
}

/// 渲染 mode 行（`old mode 100644` 等）。
fn emit_mode_line(out: &mut Vec<u8>, label: &[u8], kind: Option<EntryKind>) {
    out.extend_from_slice(label);
    out.push(b' ');
    out.extend_from_slice(entry_kind_str(kind));
    out.push(b'\n');
}

/// mode 的八进制显示（`100644` / `100755` / `120000` / `160000`；缺失兜底 `000000`）。
fn entry_kind_str(kind: Option<EntryKind>) -> &'static [u8] {
    match kind {
        Some(kind) => kind.as_octal_str(),
        None => b"000000",
    }
}

/// 短 hash（`core.abbrev` 自动策略，gix 的 shorten 与 git 同语义）。
///
/// 工作树侧实时计算的 blob hash 不在对象库中（`shorten` 需查库确认无歧义），
/// 此时按 git 的默认缩写长度回退为前 7 位（fixture 与常规仓库规模下
/// `core.abbrev=auto` 与 7 一致；超大仓库可能出现 1 位长度差，见偏差清单）。
fn short_hash(repo: &Repository, id: ObjectId) -> Result<String> {
    id.attach(repo.inner())
        .shorten()
        .map(|prefix| prefix.to_string())
        .or_else(|_| Ok(id.to_hex_with_len(7).to_string()))
}

// ---------- blob 级渲染 ----------

/// 单文件 blob diff 的渲染结果。
struct BlobRender {
    /// hunk 主体（含 hunk 头；空 = 无内容差异）。
    hunks: Vec<u8>,
    /// 任一侧被判定为二进制（git 的 8000 字节内含 NUL 规则，由 gix pipeline 判定）。
    is_binary: bool,
    /// 行级相似度（partial rename 头部用；与 git 的分数算法存在偏差）。
    similarity: Option<f32>,
    /// 工作树侧新内容的 git 对象 hash（`index` 行显示用；id 已知的场景为 `None`）。
    new_hash: Option<ObjectId>,
}

/// gitlink / 子树不可 blob diff（submodule 不渲染，偏差清单）。
fn is_unrenderable(kind: Option<EntryKind>) -> bool {
    matches!(kind, Some(EntryKind::Commit | EntryKind::Tree))
}

/// 用 gix 的 blob diff 管线渲染一个文件的 hunk 主体。
///
/// * 工作树模式下新侧从磁盘读（`new_root` = workdir，含 ToGit 转换与 CRLF
///   过滤）；staged 模式两侧都从对象库读。
/// * 缺失侧用 null id 占位（gix pipeline 对 null id 产出 Missing 数据 = 空内容，
///   永不查对象库）。
/// * 工作树侧（`new_id = None`）渲染完成后按 git 对象格式对转换后的新内容做
///   hash，供 `index` 行显示（零副作用，不写对象库）。
fn render_blob(
    repo: &Repository,
    change: &FileChange,
    workdir: &Path,
    staged: bool,
) -> Result<BlobRender> {
    if is_unrenderable(change.old_kind) || is_unrenderable(change.new_kind) {
        return Ok(BlobRender { hunks: Vec::new(), is_binary: false, similarity: None, new_hash: None });
    }

    let roots = if staged {
        WorktreeRoots::default()
    } else {
        WorktreeRoots {
            old_root: None,
            new_root: Some(workdir.to_path_buf()),
        }
    };
    let mut cache = repo
        .inner()
        .diff_resource_cache(PipelineMode::ToGit, roots)
        .map_err(|err| Error::internal("初始化 diff 管线失败", err))?;
    let objects = &repo.inner().objects;

    let set_resource = |cache: &mut gix::diff::blob::Platform,
                            id: Option<ObjectId>,
                            kind: Option<EntryKind>,
                            path: &[u8],
                            side: gix::diff::blob::ResourceKind|
     -> Result<()> {
        // 缺失侧（/dev/null）也必须 set：null id 会被 pipeline 判定为 Missing
        // 数据（空内容），否则 prepare_diff 以「资源未设置」失败。
        cache
            .set_resource(
                id.unwrap_or_else(|| ObjectId::null(repo.inner().object_hash())),
                kind.unwrap_or(EntryKind::Blob),
                BStr::new(path),
                side,
                objects,
            )
            .map_err(|err| Error::internal("装配 diff 资源失败", err.to_string()))?;
        Ok(())
    };
    set_resource(
        &mut cache,
        change.old_id,
        change.old_kind,
        &change.old_path,
        gix::diff::blob::ResourceKind::OldOrSource,
    )?;
    set_resource(
        &mut cache,
        change.new_id,
        change.new_kind,
        &change.new_path,
        gix::diff::blob::ResourceKind::NewOrDestination,
    )?;

    let prepared = cache
        .prepare_diff()
        .map_err(|err| Error::internal("准备 blob diff 失败", err.to_string()))?;
    match prepared.operation {
        gix::diff::blob::platform::prepare_diff::Operation::SourceOrDestinationIsBinary => {
            // 二进制数据不进 pipeline 缓冲（只剩 size），工作树侧的 index 行
            // hash 从磁盘直读原始字节计算（无 CRLF / filter 的二进制与 git
            // 一致；filter 生效的二进制可能出现差异，见偏差清单）。
            let new_hash = match change.new_id {
                Some(id) => Some(id),
                None if !staged => hash_worktree_file(repo, workdir, &change.new_path),
                None => None,
            };
            Ok(BlobRender {
                hunks: Vec::new(),
                is_binary: true,
                similarity: None,
                new_hash,
            })
        }
        gix::diff::blob::platform::prepare_diff::Operation::InternalDiff { algorithm } => {
            // token 带换行（与 git 的行记录一致，模块注释「token 切分」）。
            let input = InternedInput::new(prepared.old.intern_source(), prepared.new.intern_source());
            let diff = diff_with_slider_heuristics(algorithm, &input);

            // partial rename 的行级相似度（gix rewrites tracker 同一公式：
            // (旧字节长 - 被删字节) / max(旧长, 新长)）。
            let similarity = if change.change_kind == ChangeKind::Rename && change.new_id != change.old_id {
                let old_len = prepared.old.data.as_slice().map(<[u8]>::len).unwrap_or(0);
                let new_len = prepared.new.data.as_slice().map(<[u8]>::len).unwrap_or(0);
                let removed: usize = diff
                    .hunks()
                    .flat_map(|hunk| {
                        &input.before[hunk.before.start as usize..hunk.before.end as usize]
                    })
                    .map(|token| input.interner[*token].len())
                    .sum();
                (old_len > 0).then(|| (old_len - removed) as f32 / old_len.max(new_len) as f32)
            } else {
                None
            };

            // 工作树侧新内容的 git 对象 hash（git 对 index 行实时计算同款 hash）。
            let new_hash = change.new_id.or_else(|| {
                prepared.new.data.as_slice().map(|data| hash_blob(repo, data))
            });

            let sink = GitHunkSink {
                out: Vec::new(),
                input: &input,
                funcline_prev: None,
            };
            let sink = UnifiedDiff::new(&diff, &input, sink, ContextSize::symmetrical(3))
                .consume()
                .map_err(|err| Error::internal("渲染 hunk 失败", err))?;
            Ok(BlobRender { hunks: sink.out, is_binary: false, similarity, new_hash })
        }
        // `skip_internal_diff_if_external_is_configured = false`，不会出现。
        gix::diff::blob::platform::prepare_diff::Operation::ExternalCommand { .. } => Err(Error::internal(
            "blob diff",
            "外部 diff 命令路径未启用",
        )),
    }
}

/// 按 git 对象格式计算 blob hash（`blob <len>\0<data>`；零副作用，不写对象库）。
fn hash_blob(repo: &Repository, data: &[u8]) -> ObjectId {
    let mut hasher = gix::hash::hasher(repo.inner().object_hash());
    hasher.update(format!("blob {}\0", data.len()).as_bytes());
    hasher.update(data);
    hasher.try_finalize().unwrap_or_else(|_| ObjectId::null(repo.inner().object_hash()))
}

/// 直读工作树文件并按 git 对象格式计算 hash（二进制 index 行显示用）。
fn hash_worktree_file(repo: &Repository, workdir: &Path, rela_path: &[u8]) -> Option<ObjectId> {
    let rel = gix::path::from_bstr(BStr::new(rela_path)).ok()?;
    let data = std::fs::read(workdir.join(&*rel)).ok()?;
    Some(hash_blob(repo, &data))
}

/// git 的 hunk 渲染 sink：hunk 头按 `xdl_emit_hunk_hdr` 规则 + 函数上下文段 +
/// `\ No newline at end of file`。
struct GitHunkSink<'a> {
    out: Vec<u8>,
    input: &'a InternedInput<&'a [u8]>,
    /// 上一个 hunk 的函数行（git get_func_line 的 funclineprev：不向上越过它）。
    funcline_prev: Option<usize>,
}

impl ConsumeHunk for GitHunkSink<'_> {
    type Out = Self;

    fn consume_hunk(&mut self, header: gix::diff::blob::unified_diff::HunkHeader, lines: &[(DiffLineKind, &[u8])]) -> std::io::Result<()> {
        // 函数上下文段：从 hunk 上下文起点的前一行向上查找（git get_func_line），
        // 不越过上一个 hunk 已用的函数行；找不到时省略段。
        let context_start = header.before_hunk_start.saturating_sub(1) as isize;
        let limit = self.funcline_prev.map_or(-1, |prev| prev as isize);
        let mut func_line: Option<&[u8]> = None;
        let mut cursor = context_start - 1;
        while cursor != limit && cursor >= 0 {
            let line = self.input.interner[self.input.before[cursor as usize]];
            if let Some(func) = default_func_line(line) {
                func_line = Some(func);
                self.funcline_prev = Some(cursor as usize);
                break;
            }
            cursor -= 1;
        }

        // hunk 头：`@@ -<s1>[,<c1>] +<s2>[,<c2>] @@[ <func>]`；git 规则：长度为 1
        // 时省略长度、长度为 0 时起点回退一行（`c ? s : s - 1`）。
        let (s1, c1, s2, c2) = (
            header.before_hunk_start,
            header.before_hunk_len,
            header.after_hunk_start,
            header.after_hunk_len,
        );
        self.out.extend_from_slice(b"@@ -");
        emit_hunk_number(&mut self.out, c1, s1);
        if c1 != 1 {
            self.out.push(b',');
            emit_hunk_number(&mut self.out, c1, c1);
        }
        self.out.extend_from_slice(b" +");
        emit_hunk_number(&mut self.out, c2, s2);
        if c2 != 1 {
            self.out.push(b',');
            emit_hunk_number(&mut self.out, c2, c2);
        }
        self.out.extend_from_slice(b" @@");
        if let Some(func) = func_line {
            self.out.push(b' ');
            self.out.extend_from_slice(func);
        }
        self.out.push(b'\n');

        // 行：前缀 + 内容（token 自带换行）；末行无换行时补 git 的标记行。
        for &(kind, content) in lines {
            self.out.push(kind.to_prefix() as u8);
            self.out.extend_from_slice(content);
            if !content.ends_with(b"\n") {
                self.out.extend_from_slice(b"\n\\ No newline at end of file\n");
            }
        }
        Ok(())
    }

    fn finish(self) -> Self::Out {
        self
    }
}

/// hunk 头数字：长度为 0 时起点回退一行（git 的 `c ? s : s - 1`）。
fn emit_hunk_number(out: &mut Vec<u8>, count: u32, start: u32) {
    // gix 的 header 起点恒为 1-based（>= 1），`start - 1` 实际不会下溢；
    // saturating 只作防御（0 不会在真实输出中出现）。
    let value = if count != 0 { start } else { start.saturating_sub(1) };
    out.extend_from_slice(value.to_string().as_bytes());
}

/// git 默认 funcname 判定（xdiff-interface 的 def_ff）：首字符为 ASCII 字母、
/// `_` 或 `$` 的行可作函数上下文；截断 80 字节后去除尾部空白。
fn default_func_line(line: &[u8]) -> Option<&[u8]> {
    let first = *line.first()?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return None;
    }
    let mut len = line.len().min(80);
    while len > 0 && line[len - 1].is_ascii_whitespace() {
        len -= 1;
    }
    Some(&line[..len])
}

// ---------- 小工具 ----------

/// index 条目 mode → tree entry kind（diff 渲染与提交管线共用）。
pub(crate) fn entry_mode_kind(mode: gix::index::entry::Mode) -> EntryKind {
    mode.to_tree_entry_mode()
        .map(EntryKind::from)
        .unwrap_or(EntryKind::Blob)
}

/// 可执行位翻转（`100644` ↔ `100755`；其余 kind 原样）。
fn toggle_executable(kind: EntryKind) -> EntryKind {
    match kind {
        EntryKind::Blob => EntryKind::BlobExecutable,
        EntryKind::BlobExecutable => EntryKind::Blob,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_header_numbers_follow_git_rules() {
        // 长度 0 时起点回退一行（git 的 `c ? s : s - 1`）：`@@ -0,0 …`。
        let mut out = Vec::new();
        emit_hunk_number(&mut out, 0, 1);
        assert_eq!(out, b"0");
        let mut out = Vec::new();
        emit_hunk_number(&mut out, 0, 0);
        assert_eq!(out, b"0"); // saturating 场景不会出现（start >= 1），兜底为 0 前的防御由调用方保证
        let mut out = Vec::new();
        emit_hunk_number(&mut out, 1, 1);
        assert_eq!(out, b"1");
        let mut out = Vec::new();
        emit_hunk_number(&mut out, 3, 7);
        assert_eq!(out, b"7");
    }

    #[test]
    fn default_func_line_matches_git_def_ff() {
        assert_eq!(default_func_line(b"fn main() {\n"), Some(&b"fn main() {"[..]));
        // 首字符非字母 / _ / $：不匹配。
        assert_eq!(default_func_line(b"    indented();\n"), None);
        assert_eq!(default_func_line(b"}\n"), None);
        assert_eq!(default_func_line(b""), None);
        assert_eq!(default_func_line(b"_private();\n"), Some(&b"_private();"[..]));
        assert_eq!(default_func_line(b"$id = 1;\n"), Some(&b"$id = 1;"[..]));
        // 尾部空白去除（含换行）。
        assert_eq!(default_func_line(b"func  \n"), Some(&b"func"[..]));
    }

    #[test]
    fn exec_toggle_only_affects_blobs() {
        assert_eq!(toggle_executable(EntryKind::Blob), EntryKind::BlobExecutable);
        assert_eq!(toggle_executable(EntryKind::BlobExecutable), EntryKind::Blob);
        assert_eq!(toggle_executable(EntryKind::Link), EntryKind::Link);
    }

    #[test]
    fn unrenderable_kinds_cover_gitlinks_and_trees() {
        assert!(is_unrenderable(Some(EntryKind::Commit)));
        assert!(is_unrenderable(Some(EntryKind::Tree)));
        assert!(!is_unrenderable(Some(EntryKind::Blob)));
        assert!(!is_unrenderable(None));
    }

    #[test]
    fn path_filter_splits_rename_on_partial_match() {
        let rename = FileChange {
            change_kind: ChangeKind::Rename,
            old_path: b"docs/a.md".to_vec(),
            new_path: b"docs/b.md".to_vec(),
            old_id: Some(ObjectId::null(gix::hash::Kind::Sha1)),
            new_id: Some(ObjectId::null(gix::hash::Kind::Sha1)),
            old_kind: Some(EntryKind::Blob),
            new_kind: Some(EntryKind::Blob),
            copy: false,
        };
        // 两侧都命中：保持 rename。
        assert_eq!(apply_path_filter(rename.clone(), Some(b"docs")).len(), 1);
        // 都不命中：丢弃。
        assert!(apply_path_filter(rename.clone(), Some(b"src")).is_empty());
        // 仅旧路径命中：拆成删除（呈现路径 = 旧路径）。
        let split = apply_path_filter(rename.clone(), Some(b"docs/a.md"));
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].change_kind, ChangeKind::Deletion);
        assert!(split[0].new_id.is_none());
        assert_eq!(split[0].new_path, b"docs/a.md");
        // 仅新路径命中：拆成新增（呈现路径 = 新路径）。
        let split = apply_path_filter(rename, Some(b"docs/b.md"));
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].change_kind, ChangeKind::Addition);
        assert!(split[0].old_id.is_none());
        assert_eq!(split[0].new_path, b"docs/b.md");
    }

    /// `apply_path_filter` 的路径过滤必须接受 Windows 反斜杠展示路径与 `.`
    /// （整仓）——否则工具层拿不到任何变更，只能渲染成「没有差异」。
    #[test]
    fn path_filter_accepts_backslash_and_dot() {
        let change = FileChange {
            change_kind: ChangeKind::Modification,
            old_path: b"docs/arch.md".to_vec(),
            new_path: b"docs/arch.md".to_vec(),
            old_id: Some(ObjectId::null(gix::hash::Kind::Sha1)),
            new_id: Some(ObjectId::null(gix::hash::Kind::Sha1)),
            old_kind: Some(EntryKind::Blob),
            new_kind: Some(EntryKind::Blob),
            copy: false,
        };
        // Windows 展示路径（反斜杠）与目录前缀都要命中。
        assert_eq!(apply_path_filter(change.clone(), Some(b"docs\\arch.md")).len(), 1);
        assert_eq!(apply_path_filter(change.clone(), Some(b"docs")).len(), 1);
        // 「当前目录」= 不过滤。
        assert_eq!(apply_path_filter(change.clone(), Some(b".")).len(), 1);
        // 真正的非命中仍被丢弃（归一不引入模糊匹配）。
        assert!(apply_path_filter(change, Some(b"src")).is_empty());
    }
}
