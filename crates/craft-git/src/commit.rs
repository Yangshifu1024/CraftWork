//! 两模式提交管线（spec 第 5 节 commit）：变更收集 → `write_blob` 逐文件 →
//! [`TreeEditor`] 构建新 tree（保留未触及路径）→ `repo.commit_as`（ref 事务 +
//! reflog 内置）→ 磁盘 index 同步写回。
//!
//! ## 两模式语义（对齐 `git add` + `git commit` / `git commit -am` 的可观察行为）
//!
//! - **路径限定**（`paths` 非空）：等价 `git add -- <paths>` + `git commit -m … -- <paths>`。
//!   指定路径的最终提交内容 = 工作树状态（可含未跟踪文件；工作树里已删除的路径
//!   进删除）；**路径外的任何变更（含 index 中已暂存的）不进本次提交且 index 不动**
//!   （git 的路径限定提交用临时 index，已实测锚定）。
//! - **`-am`**（`paths` 为空）：提交整个 index 叠加「已跟踪文件的工作树修改 / 删除」；
//!   未跟踪文件不含。index 中已有的暂存变更（含暂存新增）全部进提交（`-a` 语义：
//!   提交的就是 add 之后的 index，已实测锚定）。
//!
//! - 两模式都如实报出提交范围：`CommitOutcome.committed_files` 是本次进提交的
//!   文件数，`leftover_files` 是**被 `paths` 排除**的变更数（`-am` 模式恒 0）。
//!   路径限定提交只提交指定路径、其余变更留在工作区且索引不动（git 语义），
//!   不报这个数会让调用方以为「提交成功 = 全提交了」。
//!
//! ## 入库内容（工作树 → blob）
//!
//! 工作树文件按 `.gitattributes` 的 `text` / `eol` 与 `core.autocrlf` 经
//! `gix::filter::Pipeline` 归一行尾后再入库（`autocrlf=true` 的 Windows 检出必须
//! 归一，否则 CRLF 会原样写进 blob，历史里出现「只改一行却整篇重写」的假 diff）；
//! symlink 存链接目标路径字节，不走 filter；二进制由管线自行跳过。attribute 源用
//! `WorktreeThenIdMapping`（checking-in 语义）——本提交新增的 `.gitattributes`
//! 尚未进索引，用 `IdMapping` 会漏读、`-text` 之类的属性就不生效。
//!
//! ## 与系统 git 的已知偏差（票 #5 偏差清单）
//!
//! - 不新增签名能力；用户配置 `commit.gpgsign=true` 时 git 会签名而本管线不签名。
//! - 身份缺失直接报错（`Error::IdentityMissing`），不做 git 的「用户名 + 主机名」
//!   自动身份推断；`commit.gpgsign` 之外与 `user.useConfigOnly=true` 的 git 行为一致。
//! - 冲突（unmerged）条目与 submodule（gitlink 修改 / 新增子仓库）不纳入提交；
//!   git 会分别拒绝提交未解决冲突或写入 gitlink。冲突场景在无变更兜底下会以
//!   「nothing to commit」失败而非 git 的冲突拒绝文案。
//! - 无变更拒绝文案对齐 git 的核心拒绝句（`nothing to commit…`）；ahead/behind
//!   与 unborn 未跟踪列表等附提示行不复刻。
//! - 延迟处理型外部 filter 驱动（`filter.<name>.process` 的延迟协议）不在本期
//!   范围：`git_commit` 在应用进程内同步执行，不引入常驻驱动进程。
//!
//! ## index 同步
//!
//! 提交落库后同步更新磁盘 index（`-am` 模式刷新条目 id / mode / stat，路径限定
//! 模式覆盖指定路径条目并按需新增），随后 `remove_tree()` 失效 tree-cache 扩展
//! （gix 写 index 时 tree-cache 原样透写，不失效会让后续提交读到过期子树），
//! 再 `write()`（自带 `index.lock` 加锁）写回——保证提交后 `git status` 不把
//! 已提交内容复现出来。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gix::bstr::{BStr, ByteSlice};
use gix::hash::ObjectId;
use gix::index as gix_index;
use gix::object::tree::EntryKind;
use gix::prelude::ObjectIdExt;
use gix::status;
use gix::status::index_worktree;
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
use gix::status::tree_index::TrackRenames;

use crate::diff::entry_mode_kind;
use crate::engine::{Repository, Signature};
use crate::errors::{Error, Result};
use crate::log;

/// git commit 的参数集（工具 `git_commit` 与门面共用）。
#[derive(Debug, Clone)]
pub struct CommitOptions {
    /// 提交信息（工具层已校验非空 / 无 `\0` / ≤2000 字节；本层不再重复校验）。
    pub message: String,
    /// 路径限定模式的路径列表（相对工作树，正斜杠分隔）；**空 = `-am` 模式**。
    pub paths: Vec<Vec<u8>>,
    /// 协作式中断标志（超时包装置位后逐项检查、尽快停止）。
    pub should_interrupt: Arc<AtomicBool>,
}

/// 提交结果：完整 hash、短 hash（`git rev-parse --short` 等价）与提交范围统计。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    /// 新提交的完整对象 id。
    pub commit_id: ObjectId,
    /// 短 hash（`core.abbrev` 自动策略，与 `rev-parse --short` 同语义）。
    pub short_hash: String,
    /// 本次进提交的文件数。
    pub committed_files: usize,
    /// 被 `paths` 排除、未进本次提交的变更数（`-am` 模式恒为 0）。
    ///
    /// 路径限定提交**只提交指定路径**，其余变更留在工作区且索引不动——这是 git 的
    /// 语义，但调用方（模型）容易以为「提交成功了 = 全提交了」，因此必须能报出这个数。
    pub leftover_files: usize,
}

/// 单个路径的最终提交动作（路径 → 动作按字节序存入 `BTreeMap`，与 git 的
/// tree 条目顺序一致）。
#[derive(Debug, Clone)]
enum Op {
    /// 工作树文件内容进提交：读文件 → `write_blob` → upsert；index 条目同步刷新
    /// （id / mode / stat；未跟踪路径为新增条目）。
    UpsertWorktree {
        /// tree 条目类型（决定 mode；可执行位变化与磁盘状态已折算在内）。
        kind: EntryKind,
    },
    /// index 中已有 blob 直接 upsert（`-am` 模式的暂存变更；blob 已在对象库，
    /// index 条目本就是该 id，无需同步）。
    UpsertIndex {
        /// index 条目的 blob id。
        id: ObjectId,
        /// tree 条目类型。
        kind: EntryKind,
    },
    /// 从 tree 移除（工作树文件已删除或 index 中已暂存删除）；index 条目同步移除。
    Remove,
}

/// 执行一次提交（两模式见模块注释）。
///
/// 成功返回新提交的完整与短 hash；无变更返回 [`Error::NothingToCommit`]（与 git
/// 空提交拒绝同构），指定路径不存在返回 [`Error::PathspecNotFound`]（等价
/// `git add` 的 fatal），身份缺失返回 [`Error::IdentityMissing`]。
pub fn commit(repo: &Repository, options: CommitOptions) -> Result<CommitOutcome> {
    // 身份级联（system → global → local）校验在前：与 git 一样，没身份就无从提交。
    let identity: Signature = repo.identity()?;
    let inner = repo.inner();
    let workdir = repo
        .workdir()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::internal("无法在裸仓库中提交", "this operation must be run in a work tree"))?;

    // pathspec 规范化：剥离 `./` 前缀；`.` / `./` / 空串归一为「匹配一切」（git 的
    // 当前目录 pathspec 语义），工具层传来的路径即相对工作树的显示路径。
    let paths: Vec<Vec<u8>> = options
        .paths
        .iter()
        .map(|p| normalize_spec(p))
        .collect();
    let path_mode = !paths.is_empty();
    let options = CommitOptions { paths, ..options };

    // HEAD 基线：unborn HEAD 无父提交、以空树为基线（对齐 git 的 root commit 路径）。
    let head_id = inner.head_id().ok().map(|id| id.detach());
    let head_tree = inner
        .head_tree_id_or_empty()
        .map_err(|err| Error::internal("读取 HEAD 树失败", err))?
        .detach();

    // index 副本：变更收集（status 平台）与提交后的同步写回用同一份，避免两次
    // 读盘之间状态漂移；unborn 仓库没有 index 文件，用空 State 占位（路径限定
    // 模式在 unborn 下创建首个提交时，条目随后写入并落盘）。
    let mut index_file = open_index(inner)?;

    // ① 变更收集：一次 status 迭代（展开未跟踪、关闭 rename 配对，见 collect_ops）。
    let collected = collect_ops(inner, &workdir, &index_file, &options, path_mode)?;
    let ops = collected.ops;
    // 指定路径的命中校验（等价 `git add` 对不存在 pathspec 的 fatal）。
    if path_mode {
        check_pathspecs_hit(&workdir, &index_file, &options.paths)?;
    }

    // ② ③ ④ 写 blob + 构建新 tree（保留未触及路径）。
    let (new_tree, blob_ids) = build_tree(inner, &workdir, &index_file, head_tree, &ops)?;

    // 与 git 一样拒绝空提交：所有动作应用后 tree 不变即无可提交内容（覆盖
    // 「工作树与 HEAD 相同」「指定路径无实际变化」等全部形态）。
    if new_tree == head_tree {
        return Err(nothing_to_commit_error(repo));
    }

    // ⑤ 提交：ref 事务（HEAD deref 到当前分支，detached 时直接更新 HEAD）+ reflog
    // 内置（`commit (initial): …` / `commit: …`，与 git 的 reflog 消息同构）。
    let time_text = gix::date::Time::now_local_or_utc().to_string();
    let sig = gix::actor::SignatureRef {
        name: BStr::new(identity.name.as_bytes()),
        email: BStr::new(identity.email.as_bytes()),
        time: time_text.as_str(),
    };
    let parents: Vec<ObjectId> = head_id.into_iter().collect();
    let commit_id = inner
        .commit_as(sig, sig, "HEAD", &options.message, new_tree, parents)
        .map_err(|err| Error::internal("创建提交失败", err))?
        .detach();

    // ⑥ index 同步 + 写回磁盘（保证后续 status 不复现已提交内容）。
    sync_index(&mut index_file, &workdir, &ops, &blob_ids)?;

    let short_hash = short_hash(repo, commit_id);
    Ok(CommitOutcome {
        commit_id,
        short_hash,
        committed_files: ops.len(),
        leftover_files: collected.excluded,
    })
}

/// pathspec 规范化：反复剥离 `./` 前缀；结果为空（`.` / `./` / 空串）表示「匹配
/// 一切」——`path_hit` 对空 spec 恒命中，与 git 的当前目录 pathspec 一致。
fn normalize_spec(spec: &[u8]) -> Vec<u8> {
    let mut rest = spec;
    while rest.starts_with(b"./") {
        rest = &rest[2..];
    }
    if rest == b"." {
        Vec::new()
    } else {
        rest.to_vec()
    }
}

// ---------- 变更收集 ----------

/// [`collect_ops`] 的产物：待应用的提交动作 + 被 pathspec 排除的变更数。
///
/// `excluded` 只统计**被 paths 排除**的变更（`-am` 模式恒为 0），用于在提交结果里
/// 如实说明「另有 N 个变更没进本次提交」——路径限定模式此前静默留下其余变更，
/// 调用方看不出「提交范围只有这些」。
struct CollectedOps {
    ops: BTreeMap<Vec<u8>, Op>,
    excluded: usize,
}

/// 一次 status 迭代收集两模式的全部提交动作。
///
/// 采集配置的行为依据：
/// - `UntrackedFiles::Files`：未跟踪文件逐个展开（路径限定模式需要文件级路径）；
///   `.gitignore` 的忽略语义由 status 的 excludes 管线处理，与 `git add` 一致。
/// - `index_worktree_rewrites(None)`：不把「工作树删除 + 未跟踪新文件」配对成
///   rename——`-am` 模式不 add 未跟踪文件（仅删除侧生效），路径限定模式按
///   `git add` 的 index 层语义就是删除 + 新增，展开处理两模式都对。
/// - `TrackRenames::Disabled`：暂存侧 rename 拆成「index 中删除来源 + 新增目标」，
///   与 add/commit 在 index 层的实际效果一致（rename 只是 status 的呈现概念）。
fn collect_ops(
    inner: &gix::Repository,
    workdir: &Path,
    index_file: &gix_index::File,
    options: &CommitOptions,
    path_mode: bool,
) -> Result<CollectedOps> {
    let iter = inner
        .status(gix::progress::Discard)
        .map_err(|err| Error::internal("初始化状态采集失败", err))?
        .untracked_files(status::UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        .tree_index_track_renames(TrackRenames::Disabled)
        .index(index_file.clone().into())
        .should_interrupt_owned(Arc::clone(&options.should_interrupt))
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(|err| Error::internal("初始化状态采集失败", err))?;

    // 两侧分开收集：gix 的 status 并行产出暂存侧与工作树侧变更（顺序不定），
    // 同一路径两侧都有动作时必须保证「暂存侧先应用、工作树侧覆盖」——与 git
    // 「先 add 工作树内容、再提交 index」的叠加顺序一致，不能依赖迭代顺序。
    let mut staged_ops: BTreeMap<Vec<u8>, Op> = BTreeMap::new();
    let mut worktree_ops: BTreeMap<Vec<u8>, Op> = BTreeMap::new();
    // 路径限定模式下被 pathspec 排除的变更数（见 [`CollectedOps::excluded`]）。
    let mut excluded = 0usize;
    for item in iter {
        let item = item.map_err(|err| Error::internal("采集工作树状态失败", err))?;
        if options.should_interrupt.load(Ordering::SeqCst) {
            // 协作式中断：超时包装置位后立即以错误返回（与 log / diff 的中断语义一致）。
            return Err(Error::internal("git commit", "操作已超时中止"));
        }
        match item {
            // 暂存侧（HEAD tree → index）：`-am` 模式的暂存变更进提交；路径限定
            // 模式只认工作树状态，但「index 中已移除的条目」两模式都要从 tree 移除
            // （工作树侧不会再报这个路径，不处理会把已暂存删除漏进提交）。
            status::Item::TreeIndex(change) => {
                use gix::diff::index::Change as IndexChange;
                match change {
                    IndexChange::Addition { location, entry_mode, id, .. } => {
                        if !path_mode {
                            staged_ops.insert(location.as_bstr().to_vec(), Op::UpsertIndex { id: id.into_owned(), kind: entry_mode_kind(entry_mode) });
                        }
                    }
                    IndexChange::Modification { location, entry_mode, id, .. } => {
                        if !path_mode {
                            staged_ops.insert(location.as_bstr().to_vec(), Op::UpsertIndex { id: id.into_owned(), kind: entry_mode_kind(entry_mode) });
                        }
                    }
                    IndexChange::Deletion { location, .. } => {
                        if !path_mode || path_hit(&options.paths, &location) {
                            staged_ops.insert(location.as_bstr().to_vec(), Op::Remove);
                        }
                    }
                    // TrackRenames::Disabled 下不产生；保守跳过。
                    IndexChange::Rewrite { .. } => {}
                }
            }
            // 未暂存侧（index → 工作树）与未跟踪。
            status::Item::IndexWorktree(worktree_item) => match worktree_item {
                index_worktree::Item::Modification { entry, rela_path, status, .. } => {
                    let path = rela_path.as_bstr().to_vec();
                    if path_mode && !path_hit(&options.paths, &path) {
                        // 路径限定提交不包含此变更；计数供结果说明「另有 N 个未包含」。
                        // 只数真正构成动作的条目（stat 刷新与冲突本来就不是变更）。
                        if !matches!(
                            status,
                            EntryStatus::NeedsUpdate(_) | EntryStatus::Conflict { .. }
                        ) && !matches!(status, EntryStatus::Change(Change::SubmoduleModification(_)))
                        {
                            excluded += 1;
                        }
                        continue;
                    }
                    match status {
                        // `git add -N`（intent-to-add）：已跟踪条目，`-am` 会把其内容
                        // stage 进提交（已实测锚定），mode 取磁盘真实状态。
                        EntryStatus::IntentToAdd => {
                            let kind = disk_entry_kind(workdir, &path);
                            worktree_ops.insert(path, Op::UpsertWorktree { kind });
                        }
                        // stat 刷新建议与冲突条目不构成提交动作（冲突场景见模块偏差）。
                        EntryStatus::NeedsUpdate(_) | EntryStatus::Conflict { .. } => {}
                        EntryStatus::Change(change) => match change {
                            Change::Removed => {
                                worktree_ops.insert(path, Op::Remove);
                            }
                            Change::Type { worktree_mode } => {
                                worktree_ops.insert(path, Op::UpsertWorktree { kind: entry_mode_kind(worktree_mode) });
                            }
                            Change::Modification { executable_bit_changed, .. } => {
                                let mut kind = entry_mode_kind(entry.mode);
                                if executable_bit_changed {
                                    kind = flip_executable(kind);
                                }
                                worktree_ops.insert(path, Op::UpsertWorktree { kind });
                            }
                            // submodule 内容变化不动 tree（偏差清单：git 会更新 gitlink）。
                            Change::SubmoduleModification(_) => {}
                        },
                    }
                }
                index_worktree::Item::DirectoryContents { entry, .. } => {
                    // 未跟踪条目：仅路径限定模式纳入（`-am` 不含新文件）；空目录与
                    // 子仓库不进提交（git add 不加空目录；子仓库见偏差清单）。
                    if !path_mode || entry.status != gix::dir::entry::Status::Untracked {
                        continue;
                    }
                    let path = entry.rela_path.as_bstr().to_vec();
                    if !path_hit(&options.paths, &path) {
                        // 未跟踪文件同样属于「被 paths 排除的变更」。
                        excluded += 1;
                        continue;
                    }
                    let kind = match entry.disk_kind {
                        Some(gix::dir::entry::Kind::Symlink) => Some(EntryKind::Link),
                        Some(gix::dir::entry::Kind::File) => Some(disk_entry_kind(workdir, &path)),
                        // 空目录（Directory）、子仓库（Repository）与未知类型跳过。
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        worktree_ops.insert(path, Op::UpsertWorktree { kind });
                    }
                }
                // rewrites 已关闭，不产生。
                index_worktree::Item::Rewrite { .. } => {}
            },
        }
    }
    // 合并：暂存侧打底，工作树侧覆盖（顺序见循环前注释）。
    let mut ops = staged_ops;
    for (path, op) in worktree_ops {
        ops.insert(path, op);
    }
    Ok(CollectedOps { ops, excluded })
}

/// pathspec 命中判定：空 spec（规范化后的「当前目录」）恒命中；否则路径 ==
/// pathspec 或位于 pathspec 目录下（复用 log 的路径过滤语义，与 `git add` 的
/// 常用目录 pathspec 行为一致）。
fn path_hit(paths: &[Vec<u8>], path: &[u8]) -> bool {
    paths
        .iter()
        .any(|spec| spec.is_empty() || log::path_matches(BStr::new(path), spec))
}

/// 指定路径的存在性校验：工作树存在（文件或目录）或 index 已有条目（前缀命中）
/// 任一成立即可；否则等价 `git add -- <path>` 的 fatal（报第一个未命中项）。
fn check_pathspecs_hit(
    workdir: &Path,
    index_file: &gix_index::File,
    paths: &[Vec<u8>],
) -> Result<()> {
    for spec in paths {
        let disk_path = gix::path::from_bstr(BStr::new(spec))
            .map_err(|err| Error::internal("路径转换失败", err))?;
        let on_disk = workdir.join(&*disk_path).exists();
        let stage = gix_index::entry::Stage::Unconflicted;
        let in_index = index_file.entry_by_path_and_stage(BStr::new(spec), stage).is_some()
            || index_file.prefixed_entries(BStr::new(spec)).is_some();
        if !on_disk && !in_index {
            return Err(Error::PathspecNotFound {
                pathspec: String::from_utf8_lossy(spec).into_owned(),
            });
        }
    }
    Ok(())
}

// ---------- blob 写入与 tree 构建 ----------

/// 遍历动作表：工作树文件读入（经 filter 管线归一）并 `write_blob`，随后在 HEAD
/// tree 的编辑器上 upsert / remove（未触及路径不动 → 保留），写出新 tree。
///
/// `index_file` 供属性查询与 filter 管线使用（`.gitattributes` 的 `text` / `eol`
/// 与 `core.autocrlf` 决定入库行尾，见 [`to_git_blob_content`]）。
///
/// 返回「工作树侧 upsert 路径 → blob id」供 index 同步复用（同一份内容不再读盘）。
fn build_tree(
    inner: &gix::Repository,
    workdir: &Path,
    index_file: &gix_index::File,
    head_tree: ObjectId,
    ops: &BTreeMap<Vec<u8>, Op>,
) -> Result<(ObjectId, BTreeMap<Vec<u8>, ObjectId>)> {
    let mut editor = inner
        .edit_tree(head_tree)
        .map_err(|err| Error::internal("加载 HEAD 树失败", err))?;
    let mut blob_ids: BTreeMap<Vec<u8>, ObjectId> = BTreeMap::new();
    // filter 管线与属性栈按需构建一次并在本次提交内复用。
    let mut pipeline: Option<gix::filter::Pipeline<'_>> = None;
    let index_state: gix_index::State = index_file.clone().into();

    for (path, op) in ops {
        match op {
            Op::UpsertWorktree { kind } => {
                let data =
                    to_git_blob_content(inner, &mut pipeline, &index_state, workdir, path, *kind)?;
                let blob_id = inner
                    .write_blob(&data)
                    .map_err(|err| Error::internal(format!("写入 blob 失败：{}", String::from_utf8_lossy(path)), err))?
                    .detach();
                editor
                    .upsert(BStr::new(path), *kind, blob_id)
                    .map_err(|err| Error::internal(format!("更新 tree 条目失败：{}", String::from_utf8_lossy(path)), err))?;
                blob_ids.insert(path.clone(), blob_id);
            }
            Op::UpsertIndex { id, kind } => {
                editor
                    .upsert(BStr::new(path), *kind, *id)
                    .map_err(|err| Error::internal(format!("更新 tree 条目失败：{}", String::from_utf8_lossy(path)), err))?;
            }
            Op::Remove => {
                editor
                    .remove(BStr::new(path))
                    .map_err(|err| Error::internal(format!("移除 tree 条目失败：{}", String::from_utf8_lossy(path)), err))?;
            }
        }
    }

    let new_tree = editor
        .write()
        .map_err(|err| Error::internal("写出新 tree 失败", err))?
        .detach();
    Ok((new_tree, blob_ids))
}

/// 读工作树文件并转成「入库形态」的 blob 内容。
///
/// symlink 存链接目标路径字节（git 的 blob 语义），不走 filter；普通文件经
/// `gix::filter::Pipeline` 的 `convert_to_git`——它按 `.gitattributes` 的 `text` /
/// `eol` 与 `core.autocrlf` 决定 CRLF → LF 归一（`autocrlf=true` 的 Windows 检出
/// 必须归一，否则 CRLF 会被原样写进 blob，历史里出现「只改一行却整篇重写」的假 diff）。
///
/// **边界**：与系统 `git add` 同源（同一套 gix filter 实现）；二进制与 `-text`
/// 属性由管线自行跳过。不做的只有「延迟处理型外部 filter 驱动」的协议部分——
/// 常规 `filter=` 驱动仍会经 `gix_filter` 执行，与 git 行为一致。
fn to_git_blob_content<'repo>(
    inner: &'repo gix::Repository,
    pipeline: &mut Option<gix::filter::Pipeline<'repo>>,
    index_state: &gix_index::State,
    workdir: &Path,
    path: &[u8],
    kind: EntryKind,
) -> Result<Vec<u8>> {
    let rel = gix::path::from_bstr(BStr::new(path))
        .map_err(|err| Error::internal("路径转换失败", err))?;
    let full = workdir.join(&*rel);
    let is_symlink = std::fs::symlink_metadata(&full)
        .map(|md| md.file_type().is_symlink())
        .unwrap_or(false);
    if kind == EntryKind::Link || is_symlink {
        let target = std::fs::read_link(&full).map_err(|err| {
            Error::internal(format!("读取符号链接失败：{}", String::from_utf8_lossy(path)), err)
        })?;
        return Ok(os_str_bytes(target.as_os_str()));
    }

    // 管线按需构建一次（属性栈与 filter 配置在本次提交内复用；每文件重建会把
    // `.gitattributes` 的 glob 解析与驱动注册重复做一遍）。
    if pipeline.is_none() {
        let attrs = inner
            .attributes_only(
                index_state,
                // 提交方向（checking in）：先读工作树的 `.gitattributes`，index 里没有
                // 或稀疏检出时再回落到索引快照。用 IdMapping 会漏掉「本提交新增的
                // `.gitattributes`」——它还没进索引，属性因此不生效。
                gix::worktree::stack::state::attributes::Source::WorktreeThenIdMapping,
            )
            .map_err(|err| Error::internal("加载 .gitattributes 失败", err))?;
        *pipeline = Some(
            gix::filter::Pipeline::new(inner, attrs.detach())
                .map_err(|err| Error::internal("初始化 filter 管线失败", err))?,
        );
    }
    let pipe = pipeline.as_mut().expect("管线已在上方构建");
    let file = std::fs::File::open(&full).map_err(|err| {
        Error::internal(format!("读取文件失败：{}", String::from_utf8_lossy(path)), err)
    })?;
    use gix::filter::plumbing::pipeline::convert::ToGitOutcome;
    let outcome = pipe
        .convert_to_git(file, &rel, index_state)
        .map_err(|err| {
            Error::internal(
                format!("EOL 归一失败：{}", String::from_utf8_lossy(path)),
                err.to_string(),
            )
        })?;
    let mut out = Vec::new();
    match outcome {
        ToGitOutcome::Unchanged(mut r) => {
            std::io::Read::read_to_end(&mut r, &mut out).map_err(|err| {
                Error::internal(format!("读取文件失败：{}", String::from_utf8_lossy(path)), err)
            })?;
        }
        ToGitOutcome::Process(mut r) => {
            std::io::Read::read_to_end(&mut r, &mut out).map_err(|err| {
                Error::internal(format!("读取文件失败：{}", String::from_utf8_lossy(path)), err)
            })?;
        }
        ToGitOutcome::Buffer(buf) => out = buf.to_vec(),
    }
    Ok(out)
}

// ---------- index 同步 ----------

/// 提交后把动作表同步进磁盘 index 并写回。
///
/// - 工作树侧 upsert：已有条目刷新 id / mode / stat（`-am` 的 stat 刷新落在这一步）；
///   未跟踪路径新增条目（最后统一排序恢复查找不变量）。
/// - Remove：移除对应条目（未暂存删除）。
/// - 暂存侧（`UpsertIndex`）与 index 无交集，跳过。
/// - 写回前 `remove_tree()` 失效 tree-cache（gix 写 index 原样透写该扩展，条目
///   变更后必须失效，否则后续提交可能读到过期子树），`write()` 自带锁保护。
fn sync_index(
    index_file: &mut gix_index::File,
    workdir: &Path,
    ops: &BTreeMap<Vec<u8>, Op>,
    blob_ids: &BTreeMap<Vec<u8>, ObjectId>,
) -> Result<()> {
    let stage = gix_index::entry::Stage::Unconflicted;
    let mut pushed = false;
    for (path, op) in ops {
        match op {
            Op::UpsertWorktree { kind } => {
                let Some(id) = blob_ids.get(path) else {
                    return Err(Error::internal(
                        "index 同步失败",
                        format!("缺少 blob id：{}", String::from_utf8_lossy(path)),
                    ));
                };
                let mode = index_mode_from_kind(*kind);
                let stat = stat_from_disk(workdir, path)?;
                match index_file.entry_index_by_path_and_stage(BStr::new(path), stage) {
                    Some(idx) => {
                        let entry = index_file
                            .entries_mut()
                            .get_mut(idx)
                            .ok_or_else(|| Error::internal("index 同步失败", "条目索引越界"))?;
                        entry.id = *id;
                        entry.mode = mode;
                        entry.stat = stat;
                        // intent-to-add 条目内容落地后清除标记（git add 的行为）。
                        entry.flags.remove(gix_index::entry::Flags::INTENT_TO_ADD);
                    }
                    None => {
                        index_file.dangerously_push_entry(stat, *id, gix_index::entry::Flags::empty(), mode, BStr::new(path));
                        pushed = true;
                    }
                }
            }
            Op::Remove => {
                if let Some(idx) = index_file.entry_index_by_path_and_stage(BStr::new(path), stage) {
                    index_file.remove_entry_at_index(idx);
                }
            }
            Op::UpsertIndex { .. } => {}
        }
    }
    if pushed {
        index_file.sort_entries();
    }
    index_file.remove_tree();
    index_file
        .write(gix_index::write::Options::default())
        .map_err(|err| Error::internal("写回 index 失败", err))?;
    Ok(())
}

// ---------- 小工具 ----------

/// 打开 index 副本：优先读磁盘 index；不存在（unborn）时以空 State 挂到 index
/// 路径上（与 gix 的 `index_or_empty` 同构），提交后可写回落盘。
fn open_index(inner: &gix::Repository) -> Result<gix_index::File> {
    inner.open_index().or_else(|_| {
        Ok(gix_index::File::from_state(
            gix_index::State::new(inner.object_hash()),
            inner.index_path(),
        ))
    })
}

/// 可执行位翻转（`Change::Modification` 的 executable_bit_changed 场景）。
fn flip_executable(kind: EntryKind) -> EntryKind {
    match kind {
        EntryKind::Blob => EntryKind::BlobExecutable,
        EntryKind::BlobExecutable => EntryKind::Blob,
        other => other,
    }
}

/// 读磁盘文件（或符号链接）的 tree 条目类型（可执行位生效；symlink → Link）。
fn disk_entry_kind(workdir: &Path, path: &[u8]) -> EntryKind {
    let Ok(rel) = gix::path::from_bstr(BStr::new(path)) else {
        return EntryKind::Blob;
    };
    let Ok(meta) = std::fs::symlink_metadata(workdir.join(&*rel)) else {
        return EntryKind::Blob;
    };
    if meta.file_type().is_symlink() {
        return EntryKind::Link;
    }
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

/// tree 条目类型 → index 条目 mode（index 写入用）。
fn index_mode_from_kind(kind: EntryKind) -> gix_index::entry::Mode {
    match kind {
        EntryKind::Blob => gix_index::entry::Mode::FILE,
        EntryKind::BlobExecutable => gix_index::entry::Mode::FILE_EXECUTABLE,
        EntryKind::Link => gix_index::entry::Mode::SYMLINK,
        EntryKind::Commit => gix_index::entry::Mode::COMMIT,
        EntryKind::Tree => gix_index::entry::Mode::DIR,
    }
}

/// 读磁盘 metadata 构造 index 条目 stat（git add 语义：完整 stat 使后续 status
/// 可靠地走「未变更」短路）。
fn stat_from_disk(workdir: &Path, path: &[u8]) -> Result<gix_index::entry::Stat> {
    let rel = gix::path::from_bstr(BStr::new(path))
        .map_err(|err| Error::internal("路径转换失败", err))?;
    let meta = std::fs::symlink_metadata(workdir.join(&*rel)).map_err(|err| {
        Error::internal(format!("读取文件状态失败：{}", String::from_utf8_lossy(path)), err)
    })?;
    Ok(stat_from_metadata(&meta))
}

/// `std::fs::Metadata` → index 条目 stat。
///
/// unix 全字段填充；非 unix 仅 mtime / size（该平台无 dev/ino/uid/gid 语义）。
/// 字段按 u32 截断（git 本身同样截断；超过 u32 的 mtime / size 用饱和处理）。
fn stat_from_metadata(meta: &std::fs::Metadata) -> gix_index::entry::Stat {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let time = |secs: i64, nsecs: i64| gix_index::entry::stat::Time {
            secs: secs.clamp(0, u32::MAX as i64) as u32,
            nsecs: nsecs.clamp(0, u32::MAX as i64) as u32,
        };
        gix_index::entry::Stat {
            mtime: time(meta.mtime(), meta.mtime_nsec()),
            ctime: time(meta.ctime(), meta.ctime_nsec()),
            dev: meta.dev() as u32,
            ino: meta.ino() as u32,
            uid: meta.uid(),
            gid: meta.gid(),
            size: meta.len().min(u32::MAX as u64) as u32,
        }
    }
    #[cfg(not(unix))]
    {
        let time = meta.modified().ok().and_then(|t| {
            let duration = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            Some(gix_index::entry::stat::Time {
                secs: duration.as_secs().min(u32::MAX as u64) as u32,
                nsecs: duration.subsec_nanos(),
            })
        });
        gix_index::entry::Stat {
            mtime: time.unwrap_or_default(),
            ctime: time.unwrap_or_default(),
            dev: 0,
            ino: 0,
            uid: 0,
            gid: 0,
            size: meta.len().min(u32::MAX as u64) as u32,
        }
    }
}

/// OsStr → 字节（symlink 目标写入 blob 用；非 unix 经 lossy 文本转换）。
#[cfg(unix)]
fn os_str_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_str_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

/// 短 hash（`core.abbrev` 自动策略；hash 尚无歧义上下文时与 `rev-parse --short`
/// 一致；极端情况下 shorten 失败回退 7 位）。
fn short_hash(repo: &Repository, id: ObjectId) -> String {
    id.attach(repo.inner())
        .shorten()
        .map(|prefix| prefix.to_string())
        .unwrap_or_else(|_| id.to_hex_with_len(7).to_string())
}

/// 无变更拒绝错误：文案与系统 git 的空提交拒绝输出同构（核心拒绝句逐字对齐；
/// ahead/behind 等附提示行不复刻，见模块偏差）。
fn nothing_to_commit_error(repo: &Repository) -> Error {
    let inner = repo.inner();
    let message = match inner.head() {
        Ok(head) => match head.kind {
            gix::head::Kind::Unborn(name) => format!(
                "On branch {}\n\nNo commits yet\n\nnothing to commit (create/copy files and use \"git add\" to track)",
                String::from_utf8_lossy(name.shorten()),
            ),
            gix::head::Kind::Detached { target, .. } => format!(
                "HEAD detached at {}\nnothing to commit, working tree clean",
                short_hash(repo, target),
            ),
            gix::head::Kind::Symbolic(reference) => format!(
                "On branch {}\nnothing to commit, working tree clean",
                String::from_utf8_lossy(reference.name.shorten()),
            ),
        },
        // HEAD 读取失败按通用文案兜底（不阻断「拒绝空提交」这一核心语义）。
        Err(_) => "nothing to commit, working tree clean".to_string(),
    };
    Error::NothingToCommit { message }
}
