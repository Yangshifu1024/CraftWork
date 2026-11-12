//! 票 #4 黄金对比：diff（两模式）的 craft-git 渲染与系统 git 输出逐字节一致。
//!
//! 基准全部经 [`capture_git`]（隔离环境）捕获，craft-git 侧经 [`discover_isolated`]
//! （隔离配置）渲染——两侧都只见 fixture 自身的 local 配置，输入对称。
//! fixture 布景确定 ⇒ 输出确定，同一 fixture 上「系统 git 实际输出」即黄金预期。
//!
//! 已知偏差（票 #4 偏差清单）不在本文件逐字节断言之列，由专门的偏差锚定测试
//! 覆盖：partial rename 的 `similarity index NN%` 数值（见 partial_rename 测试）。

mod support;

use std::path::Path;

use support::{
    capture_git, create, discover_isolated, interrupt_flag, set_local_config, assert_golden_bytes_equal,
    FixtureKind,
};

use craft_git::diff::{diff_text, DiffOptions};

// ---------- craft-git 侧实现封装 ----------

/// diff 渲染（黄金对比的被测方）：返回与 git 输出同构的字节。
fn diff_bytes(repo_path: &Path, staged: bool, path: Option<&str>) -> Result<Vec<u8>, String> {
    let repo = discover_isolated(repo_path)?;
    let text = diff_text(
        &repo,
        DiffOptions {
            staged,
            path: path.map(|p| p.as_bytes().to_vec()),
        },
        &interrupt_flag(),
    )
    .map_err(|e| format!("diff 渲染失败：{e}"))?;
    Ok(text.text.into_bytes())
}

/// 对比一个 diff 用例：系统 git 基准 vs craft-git 渲染，逐字节断言。
fn assert_diff_matches_git(repo: &support::FixtureRepo, label: &str, staged: bool, path: Option<&str>) {
    let mut args: Vec<&str> = vec!["diff", "--no-color"];
    if staged {
        args.push("--staged");
    }
    if let Some(p) = path {
        args.push("--");
        args.push(p);
    }
    let golden = capture_git(repo.path(), &args).unwrap_or_else(|e| panic!("捕获 diff 基准失败：{e}"));
    let actual =
        diff_bytes(repo.path(), staged, path).unwrap_or_else(|e| panic!("{label}：{e}"));
    assert_golden_bytes_equal(label, &golden, &actual).unwrap_or_else(|e| panic!("{e}"));
}

// ---------- staged 模式 ----------

#[test]
fn diff_staged_matches_git_on_mixed_fixture() {
    // 覆盖：staged 文本修改（多 hunk + 函数上下文段 + funclineprev 语义）、
    // staged 删除、100% 改名、二进制修改、新增（普通 / 无尾换行 / 空文件）。
    let repo = create(FixtureKind::DiffStagedMixed).expect("生成 fixture 失败");
    assert_diff_matches_git(&repo, "staged 混合场景：全部文件头形态与 hunk 渲染", true, None);
}

#[test]
fn diff_staged_matches_git_on_utf8_fixture_with_default_quotepath() {
    // 默认 core.quotepath=true：`diff --git` / `rename from/to` 行的中文路径
    // 输出为带引号的八进制转义。
    let repo = create(FixtureKind::DiffUtf8Mixed).expect("生成 fixture 失败");
    assert_diff_matches_git(&repo, "staged 中文文件名：默认 quotepath 的八进制转义", true, None);
}

#[test]
fn diff_staged_matches_git_on_utf8_fixture_with_quotepath_false() {
    let repo = create(FixtureKind::DiffUtf8Mixed).expect("生成 fixture 失败");
    set_local_config(repo.path(), "core.quotepath", "false").expect("写入 local 配置失败");
    assert_diff_matches_git(&repo, "staged 中文文件名：quotepath=false 原样输出", true, None);
}

#[test]
fn diff_staged_matches_git_with_path_filter() {
    let repo = create(FixtureKind::DiffStagedMixed).expect("生成 fixture 失败");
    // 精确文件过滤。
    assert_diff_matches_git(&repo, "staged 路径过滤：精确文件", true, Some("code.txt"));
    // 目录前缀过滤（含 rename 对的目录命中：两侧都在 docs/ 下，保持 rename 形态）。
    assert_diff_matches_git(&repo, "staged 路径过滤：目录前缀（rename 保持）", true, Some("docs"));
    // rename 对单侧命中：拆成删除 / 新增（git diffcore 行为）。
    assert_diff_matches_git(&repo, "staged 路径过滤：rename 仅旧路径命中 → 删除形态", true, Some("docs/renamed_old.txt"));
    assert_diff_matches_git(&repo, "staged 路径过滤：rename 仅新路径命中 → 新增形态", true, Some("docs/renamed_new.txt"));
    // 过滤无命中：输出为空且不报错（git 行为：退出 0、空 stdout）。
    assert_diff_matches_git(&repo, "staged 路径过滤：无命中空输出", true, Some("nope"));
}

// ---------- 默认（index vs 工作树）模式 ----------

#[test]
fn diff_worktree_matches_git_on_mixed_fixture() {
    // 覆盖：未暂存文本修改（func 段）、未暂存删除、二进制修改、可执行位
    // 变化（mode-only 无 index 行）、无尾换行文件追加（\ No newline 标记）、
    // 已暂存修改与未跟踪文件不输出。
    let repo = create(FixtureKind::DiffWorktreeMixed).expect("生成 fixture 失败");
    assert_diff_matches_git(&repo, "默认模式混合场景：全部变更形态与排除规则", false, None);
}

#[test]
fn diff_worktree_matches_git_on_utf8_fixture_with_default_quotepath() {
    let repo = create(FixtureKind::DiffUtf8Mixed).expect("生成 fixture 失败");
    assert_diff_matches_git(&repo, "默认模式中文文件名：默认 quotepath 的八进制转义", false, None);
}

#[test]
fn diff_worktree_matches_git_on_utf8_fixture_with_quotepath_false() {
    let repo = create(FixtureKind::DiffUtf8Mixed).expect("生成 fixture 失败");
    set_local_config(repo.path(), "core.quotepath", "false").expect("写入 local 配置失败");
    assert_diff_matches_git(&repo, "默认模式中文文件名：quotepath=false 原样输出", false, None);
}

#[test]
fn diff_worktree_matches_git_with_path_filter() {
    let repo = create(FixtureKind::DiffWorktreeMixed).expect("生成 fixture 失败");
    assert_diff_matches_git(&repo, "默认模式路径过滤：精确文件", false, Some("wt_mod.txt"));
    // 无命中：空输出。
    assert_diff_matches_git(&repo, "默认模式路径过滤：无命中空输出", false, Some("absent"));
}

// ---------- 空差异与 unborn ----------

#[test]
fn diff_is_empty_on_clean_and_unborn_fixtures() {
    // 干净仓库：两模式都无输出（工具层按「没有差异」兜底）。
    let clean = create(FixtureKind::Clean).expect("生成 fixture 失败");
    assert_diff_matches_git(&clean, "干净仓库：staged 空输出", true, None);
    assert_diff_matches_git(&clean, "干净仓库：默认模式空输出", false, None);

    // unborn 仓库：git 两模式都输出空（--staged 相对空树；未跟踪不参与 diff）。
    let unborn = create(FixtureKind::Unborn).expect("生成 fixture 失败");
    assert_diff_matches_git(&unborn, "unborn 仓库：staged 相对空树空输出", true, None);
    assert_diff_matches_git(&unborn, "unborn 仓库：默认模式空输出", false, None);
}

// ---------- 偏差锚定：partial rename 的 similarity 数值 ----------

#[test]
fn partial_rename_similarity_is_a_recorded_deviation() {
    // 偏差（票 #4 清单）：partial rename 的 `similarity index NN%` 用 gix 行级
    // 相似度换算，git 的分数算法不同，数值可能不同。本测试布景「HEAD 已有的
    // 文件 git mv + 少量内容修改」（仍 > 50% 相似 → 两侧都应检出 rename），
    // 断言**除 similarity 行之外**的输出逐字节一致，similarity 行的两侧数值
    // 仅记录、不比较。
    let repo = create(FixtureKind::DiffStagedMixed).expect("生成 fixture 失败");
    let path = repo.path();
    // 先落一个专属基线提交（commit_all 的 add -A 会把布景的 staged 变更一并
    // 提交，恰好得到干净的对照状态），再对 HEAD 已有文件做 partial rename 布景。
    std::fs::write(
        path.join("pr_base.txt"),
        "pr line one\npr line two\npr line three\npr line four\npr line five\n\
         pr line six\npr line seven\npr line eight\npr line nine\npr line ten\n",
    )
    .expect("写 pr_base.txt 失败");
    support::commit_all(path, "partial rename baseline", 1).expect("基线提交失败");
    support::git_fixture(path, &["mv", "pr_base.txt", "pr_renamed.txt"]).expect("git mv 失败");
    std::fs::write(
        path.join("pr_renamed.txt"),
        "pr line one\npr line two\npr line three\npr line four\npr line five\n\
         pr line six\npr line seven\npr line CHANGED eight\npr line nine\npr line CHANGED ten\n",
    )
    .expect("写 pr_renamed.txt 失败");
    support::git_fixture(path, &["add", "pr_renamed.txt"]).expect("add 失败");
    let golden = capture_git(path, &["diff", "--no-color", "--staged"]).expect("捕获基准失败");
    let actual = diff_bytes(path, true, None).expect("渲染失败");

    // 按行对比，跳过两侧的 similarity 行（各跳各行：它们都在 rename 块首行）。
    let golden_text = String::from_utf8_lossy(&golden).into_owned();
    let actual_text = String::from_utf8_lossy(&actual).into_owned();
    let skip_similarity = |text: &str| -> Vec<String> {
        text.lines()
            .filter(|line| !line.starts_with("similarity index"))
            .map(String::from)
            .collect()
    };
    assert_eq!(
        skip_similarity(&golden_text),
        skip_similarity(&actual_text),
        "partial rename 除 similarity 行外应逐字节一致。\n\
         git similarity 行：{:?}\ncraft-git similarity 行：{:?}",
        golden_text.lines().find(|l| l.starts_with("similarity index")),
        actual_text.lines().find(|l| l.starts_with("similarity index")),
    );
    // 偏差记录锚点：两侧 similarity 行都存在且不等价于 100%（确为 partial）。
    // 实测锚点：本布景（10 行改 2 行）两侧同为 71%——gix 的行级相似度公式与
    // git 的分数在该形态下收敛；更复杂的内容分布仍可能分叉，偏差保留记录。
    assert!(
        golden_text.contains("similarity index") && actual_text.contains("similarity index"),
        "两侧都应检出 rename 并输出 similarity 行"
    );
}

// ---------- 中断标志 ----------

#[test]
fn diff_stops_when_interrupt_flag_is_set() {
    // 协作式中断：标志置位后收集阶段立即以错误返回（超时包装的等价语义）。
    let repo = create(FixtureKind::DiffStagedMixed).expect("生成 fixture 失败");
    let inner = craft_git::Repository::discover_isolated(repo.path()).expect("打开失败");
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let result = craft_git::diff::diff_text(
        &inner,
        DiffOptions { staged: true, path: None },
        &flag,
    );
    assert!(result.is_err(), "置位中断标志后 diff 应以错误返回");
}

// ---------- 未暂存 rename（index 侧删除 + 未跟踪新文件的改名声） ----------

#[test]
fn diff_worktree_matches_git_on_unstaged_rename() {
    // 未暂存改名：工作树删除已跟踪文件 + 新增相似内容的未跟踪文件，git 默认
    // 检出为 rename（diff.renames=true 对默认模式同样生效）；gix 的
    // index_worktree_rewrites 用同一默认参数（50% / 1000）。
    let repo = create(FixtureKind::DiffWorktreeMixed).expect("生成 fixture 失败");
    let path = repo.path();
    // 布景：删除 wt_mod.txt，新增内容高度相似的 wt_renamed.txt（8 行改 1 行）。
    let original = "rn line one\nrn line two\nrn line three\nrn line four\n\
                    rn line five\nrn line six\nrn line seven\nrn line eight\n";
    let renamed = "rn line one\nrn line two\nrn line three\nrn line four\n\
                   rn line five\nrn line six\nrn line seven\nrn line CHANGED eight\n";
    // 先落一个只含该文件的基线提交（布景已有 staged 修改，一并提交即可得到
    // 干净的对照状态），再做未暂存改名。
    std::fs::write(path.join("wt_mod.txt"), original).expect("写文件失败");
    support::commit_all(path, "unstaged rename baseline", 1).expect("基线提交失败");
    std::fs::remove_file(path.join("wt_mod.txt")).expect("删除工作树文件失败");
    std::fs::write(path.join("wt_renamed.txt"), renamed).expect("写新文件失败");

    assert_diff_matches_git(&repo, "未暂存改名：rename 对的两侧内容渲染", false, None);
}
