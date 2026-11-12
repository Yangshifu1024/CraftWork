//! 行级 diff：LCS 动态规划 + 前后缀修剪 + 3 行上下文窗口合并；n*m > 4,000,000 时退化为整段删+增。
//! 相等时优先走删除，保证同输入同输出。

use crate::contract::{DiffHunk, DiffLine, DiffResult};

const MAX_LCS_CELLS: usize = 4_000_000;
const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum OpKind {
    Ctx,
    Add,
    Del,
}

#[derive(Debug, Clone)]
struct Op {
    kind: OpKind,
    text: String,
}

/// 全量行差异：剪公共前后缀后对中段求 LCS，超限中段退化为整段删+增。
fn diff_ops(old_lines: &[String], new_lines: &[String]) -> Vec<Op> {
    // 剪公共前缀
    let mut start = 0usize;
    while start < old_lines.len() && start < new_lines.len() && old_lines[start] == new_lines[start] {
        start += 1;
    }
    // 剪公共后缀
    let mut end_old = old_lines.len();
    let mut end_new = new_lines.len();
    while end_old > start && end_new > start && old_lines[end_old - 1] == new_lines[end_new - 1] {
        end_old -= 1;
        end_new -= 1;
    }
    let mut ops: Vec<Op> = Vec::new();
    for line in &old_lines[..start] {
        ops.push(Op {
            kind: OpKind::Ctx,
            text: line.clone(),
        });
    }

    let mid_old = &old_lines[start..end_old];
    let mid_new = &new_lines[start..end_new];
    match lcs_ops(mid_old, mid_new) {
        None => {
            // 退化为整段删 + 整段增
            for line in mid_old {
                ops.push(Op {
                    kind: OpKind::Del,
                    text: line.clone(),
                });
            }
            for line in mid_new {
                ops.push(Op {
                    kind: OpKind::Add,
                    text: line.clone(),
                });
            }
        }
        Some(mid_ops) => ops.extend(mid_ops),
    }

    for line in &old_lines[end_old..] {
        ops.push(Op {
            kind: OpKind::Ctx,
            text: line.clone(),
        });
    }
    ops
}

/// LCS 回溯生成操作序列；n*m 超过 MAX_LCS_CELLS 时返回 None（调用方走退化路径）。
fn lcs_ops(a: &[String], b: &[String]) -> Option<Vec<Op>> {
    let n = a.len();
    let m = b.len();
    if n == 0 {
        return Some(
            b.iter()
                .map(|text| Op {
                    kind: OpKind::Add,
                    text: text.clone(),
                })
                .collect(),
        );
    }
    if m == 0 {
        return Some(
            a.iter()
                .map(|text| Op {
                    kind: OpKind::Del,
                    text: text.clone(),
                })
                .collect(),
        );
    }
    if n.saturating_mul(m) > MAX_LCS_CELLS {
        return None;
    }

    // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度；相等时优先走删除，保证同输入同输出
    let mut dp: Vec<Vec<u32>> = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut ops: Vec<Op> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op {
                kind: OpKind::Ctx,
                text: a[i].clone(),
            });
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(Op {
                kind: OpKind::Del,
                text: a[i].clone(),
            });
            i += 1;
        } else {
            ops.push(Op {
                kind: OpKind::Add,
                text: b[j].clone(),
            });
            j += 1;
        }
    }
    while i < n {
        ops.push(Op {
            kind: OpKind::Del,
            text: a[i].clone(),
        });
        i += 1;
    }
    while j < m {
        ops.push(Op {
            kind: OpKind::Add,
            text: b[j].clone(),
        });
        j += 1;
    }
    Some(ops)
}

struct HunkBuild {
    hunks: Vec<DiffHunk>,
    additions: i64,
    deletions: i64,
}

/// 由操作序列构建 hunks：每个变更行扩 3 行上下文，重叠窗口合并。
fn build_hunks(ops: &[Op]) -> HunkBuild {
    let mut result = HunkBuild {
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
    };

    // 变更行下标集合
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.kind != OpKind::Ctx)
        .map(|(idx, _)| idx)
        .collect();
    if changed.is_empty() {
        return result;
    }

    // 合并窗口
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for &idx in &changed {
        let lo = idx.saturating_sub(CONTEXT);
        let hi = (idx + CONTEXT).min(ops.len() - 1);
        match windows.last_mut() {
            Some(last) if lo <= last.1 + 1 => last.1 = hi,
            _ => windows.push((lo, hi)),
        }
    }

    let mut old_pos: i64 = 1;
    let mut new_pos: i64 = 1;
    let line_starts: Vec<(i64, i64)> = ops
        .iter()
        .map(|op| {
            let pos = (old_pos, new_pos);
            match op.kind {
                OpKind::Ctx => {
                    old_pos += 1;
                    new_pos += 1;
                }
                OpKind::Del => old_pos += 1,
                OpKind::Add => new_pos += 1,
            }
            pos
        })
        .collect();

    for &(lo, hi) in &windows {
        let mut lines: Vec<DiffLine> = Vec::new();
        let mut hunk_old_start: i64 = 0;
        let mut hunk_new_start: i64 = 0;
        let mut hunk_old_count: i64 = 0;
        let mut hunk_new_count: i64 = 0;
        for idx in lo..=hi {
            let op = &ops[idx];
            let pos = line_starts[idx];
            let mut mark = |line: &DiffLine| {
                if hunk_old_start == 0 && line.old != 0 {
                    hunk_old_start = line.old;
                }
                if hunk_new_start == 0 && line.r#new != 0 {
                    hunk_new_start = line.r#new;
                }
            };
            match op.kind {
                OpKind::Ctx => {
                    let line = DiffLine {
                        kind: "ctx".into(),
                        old: pos.0,
                        r#new: pos.1,
                        text: op.text.clone(),
                    };
                    mark(&line);
                    lines.push(line);
                    hunk_old_count += 1;
                    hunk_new_count += 1;
                }
                OpKind::Del => {
                    let line = DiffLine {
                        kind: "del".into(),
                        old: pos.0,
                        r#new: 0,
                        text: op.text.clone(),
                    };
                    mark(&line);
                    lines.push(line);
                    hunk_old_count += 1;
                    result.deletions += 1;
                }
                OpKind::Add => {
                    let line = DiffLine {
                        kind: "add".into(),
                        old: 0,
                        r#new: pos.1,
                        text: op.text.clone(),
                    };
                    mark(&line);
                    lines.push(line);
                    hunk_new_count += 1;
                    result.additions += 1;
                }
            }
        }
        // 纯新增/纯删除块的缺失侧起始写 0
        let has_old = lines.iter().any(|l| l.kind != "add");
        let has_new = lines.iter().any(|l| l.kind != "del");
        let old_start = if has_old { hunk_old_start } else { 0 };
        let new_start = if has_new { hunk_new_start } else { 0 };
        result.hunks.push(DiffHunk {
            header: format!("@@ -{old_start},{hunk_old_count} +{new_start},{hunk_new_count} @@"),
            lines,
        });
    }
    result
}

/// 行级差异入口：任一侧超过 20000 行时返回截断标记。
pub fn diff(old_text: &str, new_text: &str) -> DiffResult {
    let normalized_old = old_text.replace("\r\n", "\n");
    let normalized_new = new_text.replace("\r\n", "\n");
    let mut old_lines: Vec<&str> = normalized_old.split('\n').collect();
    let mut new_lines: Vec<&str> = normalized_new.split('\n').collect();
    // 尾部空行来自「以换行结尾」的切分伪影，与 TS 版 splitLines 的 pop 语义一致
    if old_lines.last() == Some(&"") {
        old_lines.pop();
    }
    if new_lines.last() == Some(&"") {
        new_lines.pop();
    }
    let old_lines: Vec<String> = old_lines.into_iter().map(str::to_string).collect();
    let new_lines: Vec<String> = new_lines.into_iter().map(str::to_string).collect();

    if old_lines.len() > 20000 || new_lines.len() > 20000 {
        return DiffResult {
            additions: new_lines.len() as i64,
            deletions: old_lines.len() as i64,
            hunks: vec![DiffHunk {
                header: "@@ 截断 @@".into(),
                lines: vec![DiffLine {
                    kind: "ctx".into(),
                    old: 0,
                    r#new: 0,
                    text: "…（差异过大已截断，仅显示统计）".into(),
                }],
            }],
            truncated: true,
        };
    }
    let ops = diff_ops(&old_lines, &new_lines);
    let built = build_hunks(&ops);
    DiffResult {
        additions: built.additions,
        deletions: built.deletions,
        hunks: built.hunks,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_changes_empty_hunks() {
        let result = diff("a\nb\nc", "a\nb\nc");
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 0);
        assert!(result.hunks.is_empty());
        assert!(!result.truncated);
    }

    #[test]
    fn single_line_change_expands_context() {
        let result = diff("a\nb\nc\nd\ne", "a\nb\nX\nd\ne");
        assert_eq!(result.additions, 1);
        assert_eq!(result.deletions, 1);
        assert_eq!(result.hunks.len(), 1);
        let hunk = &result.hunks[0];
        // 变更行 + 前后各 3 行上下文：a,b,c,X,d,e 共 6 行
        assert_eq!(hunk.lines.len(), 6);
        assert!(hunk.header.contains("-1,5"));
        assert!(hunk.header.contains("+1,5"));
    }

    #[test]
    fn pure_addition_old_side_starts_at_zero() {
        let result = diff("a", "a\nb\nc");
        assert_eq!(result.additions, 2);
        assert_eq!(result.deletions, 0);
        // 新内容整体在一行之后追加：单 hunk
        assert_eq!(result.hunks.len(), 1);
    }

    #[test]
    fn crlf_normalized() {
        let result = diff("a\r\nb", "a\nb");
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 0);
    }

    #[test]
    fn separated_changes_produce_multiple_hunks() {
        // 首尾修改，中间隔 13 行：两处变更的上下文窗口不再重叠
        let old_text = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np";
        let new_text = "X\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\nY";
        let result = diff(old_text, new_text);
        assert_eq!(result.hunks.len(), 2);
        assert_eq!(result.additions, 2);
        assert_eq!(result.deletions, 2);
    }

    #[test]
    fn leading_trailing_changes_merge_single_hunk_when_windows_overlap() {
        let result = diff("a\nb\nc\nd\ne\nf\ng\nh", "X\nb\nc\nd\ne\nf\ng\nY");
        assert_eq!(result.hunks.len(), 1);
        assert_eq!(result.additions, 2);
        assert_eq!(result.deletions, 2);
    }

    #[test]
    fn oversize_input_truncated() {
        let big = "x\n".repeat(20001);
        let result = diff(&big, "y");
        assert!(result.truncated);
        assert_eq!(result.deletions, 20001);
        assert_eq!(result.additions, 1);
    }

    #[test]
    fn lcs_oversize_degenerates_to_wholesale_delete_add() {
        // n*m > 4_000_000：构造 3000 x 2000 的完全不同文本（6M 超限），
        // 但行数不超 20000，走退化路径
        let old_text: String = (0..3000).map(|i| format!("old{i}\n")).collect();
        let new_text: String = (0..2000).map(|i| format!("new{i}\n")).collect();
        let result = diff(&old_text, &new_text);
        assert_eq!(result.deletions, 3000);
        assert_eq!(result.additions, 2000);
        assert!(!result.truncated);
    }
}
