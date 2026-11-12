//! 双轴更新检测（spec 第 4 节，票 #5）：semver 可比则比 semver（`update-available`）；
//! 不可比则比来源 pin（git sha，`version-changed`）；都无法证明报「无更新」（[`UpdateStatus::None`]），
//! **不误报**。
//!
//! 纯函数层：零 IO 零网络——输入是已安装记录（当前版本串 + 安装时 pin 的 sha）与
//! 市场侧可用值（清单版本 + 快照 sha），输出四象限判定结果。编排级的取数（probe
//! 快照清单、读快照 HEAD、逐插件比对）在 [`crate::orchestrator`]。
//!
//! # semver 解析（自写不引 crate 的理由）
//!
//! 商店场景的版本来源（插件清单 / 市场条目）是任意文本，需要的是「宽松识别 + 保守比较」，
//! 不是完整 semver 规范：解析出 `major.minor.patch` 数字即可判定大小，不可解析自然落到
//! sha 轴（安全方向）。因此自写 [`parse_semver`]（约 30 行容错解析）而不引入 semver crate
//!——依赖面最小化，且简化语义（prerelease / build 后缀不参与比较）正是商店需要的保守性：
//! `1.2.3-beta` 与 `1.2.3` 视为同版本，不产生「更新可用」角标。
//!
//! 解析规则：去首尾空白与前导 `v` / `V`；在第一个 `-` / `+` 处截断后缀；
//! 按 `.` 分段，1~3 段、每段纯数字（缺省段补 0）；段数超过 3、含空段或非数字段 → 不可比。
//! `1.2` 与 `1.2.0` 等价、`2024.01.15` 日期形态可比。

/// semver 三元组（仅参与比较，不参与展示；版本目录名永远用原始字符串）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Semver {
    /// 三元组大小比较（`Comparable`）：更新可用判定就是逐位比大小。
    pub fn is_greater_than(&self, other: &Semver) -> bool {
        (self.major, self.minor, self.patch) > (other.major, other.minor, other.patch)
    }
}

/// 宽松解析版本串为 semver 三元组；不可解析返回 None（调用方落 sha 轴）。
/// 规则见模块注释。
pub fn parse_semver(raw: &str) -> Option<Semver> {
    let text = raw.trim();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    // prerelease（-）与 build（+）后缀不参与比较，截断即可
    let core = match text.find(['-', '+']) {
        Some(idx) => &text[..idx],
        None => text,
    };
    if core.is_empty() {
        return None;
    }
    let mut parts = [0u64; 3];
    for (count, segment) in core.split('.').enumerate() {
        if count >= 3 {
            return None; // 超过三段不是本实现的可比形态，落 sha 轴更安全
        }
        if segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        parts[count] = segment.parse::<u64>().ok()?;
    }
    Some(Semver { major: parts[0], minor: parts[1], patch: parts[2] })
}

/// 更新检测结果（spec 第 10 节商店角标的三种取值；UI 直接消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// 无更新（含「无法证明有更新」的不误报路径）。
    None,
    /// semver 可比且市场侧更高（角标 `update-available`）。
    UpdateAvailable {
        /// 市场侧可用版本原文（详情页展示）。
        available_version: String,
    },
    /// 版本串不可比但来源内容已变（sha 轴；角标 `version-changed`）。
    VersionChanged,
}

/// 双轴比较（spec 第 4 节）：判断已安装插件在市场刷新后是否有更新。
///
/// - `installed_version`：已安装记录的版本指针（= 版本目录名）；
/// - `installed_sha`：安装时 pin 的 git sha 短码（无 pin 为 None）；
/// - `available_version`：市场侧版本解析结果（插件清单 version > 条目 version；两者都缺为 None）；
/// - `available_sha`：市场侧来源 sha（entry-relative 条目 = 快照 HEAD 短码；repo 条目
///   不二次 clone 拿不到，传 None——sha 轴无法证明时按 None 不误报）。
///
/// 判定顺序（四象限，票 #5 验收）：
/// 1. 两侧版本都可解析为 semver 且市场侧更高 → [`UpdateStatus::UpdateAvailable`]；
/// 2. semver 相等且两侧 sha 都在场且不同 → [`UpdateStatus::VersionChanged`]；
/// 3. 任一侧版本不可解析 → 只看 sha 轴（两侧都在场且不同才报 VersionChanged）；
/// 4. 其余（市场侧更低、证据缺失）→ [`UpdateStatus::None`] 不误报。
///
/// 第 2 条是双轴检测的关键补充：上游改了内容没 bump 版本（同版本号重新发布）时
/// semver 轴看不出变化，由 sha 轴兜住，对应升级编排的「同版本原地重装」路径。
pub fn compare_for_update(
    installed_version: &str,
    installed_sha: Option<&str>,
    available_version: Option<&str>,
    available_sha: Option<&str>,
) -> UpdateStatus {
    // 轴一：semver 可比（两侧都能解析）
    if let (Some(installed), Some(available)) =
        (parse_semver(installed_version), available_version.and_then(parse_semver))
    {
        if available.is_greater_than(&installed) {
            return UpdateStatus::UpdateAvailable {
                available_version: available_version.unwrap_or_default().trim().to_string(),
            };
        }
        if available == installed {
            // 同版本：sha 轴补充判定「内容变了吗」（语义见第 2 条判定）
            if let (Some(installed), Some(available)) = (installed_sha, available_sha)
                && installed != available
            {
                return UpdateStatus::VersionChanged;
            }
            return UpdateStatus::None;
        }
        // 市场侧更低：不误报（semver 轴已给出结论，不再看 sha）
        return UpdateStatus::None;
    }
    // 轴二：版本不可比（缺字段或解析失败）→ 比来源 pin
    if let (Some(installed), Some(available)) = (installed_sha, available_sha)
        && installed != available
    {
        return UpdateStatus::VersionChanged;
    }
    UpdateStatus::None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- parse_semver：合法形态 ----------

    #[test]
    fn parse_semver_accepts_canonical_and_prefix_forms() {
        let v = parse_semver("1.2.3").unwrap();
        assert_eq!(v, Semver { major: 1, minor: 2, patch: 3 });
        // 前导 v / V 与空白
        assert_eq!(parse_semver("v2.0.0"), parse_semver("2.0.0"));
        assert_eq!(parse_semver("V3.4.5"), parse_semver("3.4.5"));
        assert_eq!(parse_semver("  1.2.3  "), parse_semver("1.2.3"));
    }

    #[test]
    fn parse_semver_tolerates_missing_segments_and_suffixes() {
        // 缺省段补 0
        assert_eq!(parse_semver("2.1"), Some(Semver { major: 2, minor: 1, patch: 0 }));
        assert_eq!(parse_semver("5"), Some(Semver { major: 5, minor: 0, patch: 0 }));
        // prerelease / build 后缀截断不参与比较
        assert_eq!(parse_semver("1.2.3-beta.1"), Some(Semver { major: 1, minor: 2, patch: 3 }));
        assert_eq!(parse_semver("1.2.3+build.7"), Some(Semver { major: 1, minor: 2, patch: 3 }));
        assert_eq!(parse_semver("v1.0.0-rc-2"), Some(Semver { major: 1, minor: 0, patch: 0 }));
        // 日期形态可比
        assert_eq!(parse_semver("2024.01.15"), Some(Semver { major: 2024, minor: 1, patch: 15 }));
    }

    #[test]
    fn parse_semver_rejects_non_comparable_forms() {
        // 全部不可解析（落 sha 轴，安全方向）
        for bad in [
            "", "   ", "abc", "v", "1.2.3.4", "1..2", ".5", "1.x", "x.2.3", "1.2.", "-1.2.3",
            "①.2.3", "1 2 3",
        ] {
            assert!(parse_semver(bad).is_none(), "{bad:?} 应不可解析");
        }
    }

    #[test]
    fn semver_comparison_is_lexicographic_by_segments() {
        let old = Semver { major: 1, minor: 9, patch: 9 };
        assert!(Semver { major: 2, minor: 0, patch: 0 }.is_greater_than(&old));
        assert!(Semver { major: 1, minor: 10, patch: 0 }.is_greater_than(&old));
        assert!(Semver { major: 1, minor: 9, patch: 10 }.is_greater_than(&old));
        assert!(!Semver { major: 1, minor: 9, patch: 9 }.is_greater_than(&old), "相等不算更高");
        assert!(!Semver { major: 1, minor: 9, patch: 8 }.is_greater_than(&old), "更低不算更高");
        assert!(!Semver { major: 0, minor: 99, patch: 99 }.is_greater_than(&old), "major 优先");
    }

    // ---------- compare_for_update 四象限（票 #5 验收） ----------

    #[test]
    fn quadrant_semver_bump_reports_update_available() {
        let status = compare_for_update("1.0.0", Some("aaa1111"), Some("1.1.0"), Some("bbb2222"));
        assert_eq!(
            status,
            UpdateStatus::UpdateAvailable { available_version: "1.1.0".to_string() },
            "semver 可比且更高 → update-available"
        );
    }

    #[test]
    fn quadrant_same_semver_changed_sha_reports_version_changed() {
        let status = compare_for_update("1.0.0", Some("aaa1111"), Some("1.0.0"), Some("bbb2222"));
        assert_eq!(status, UpdateStatus::VersionChanged, "semver 相同但 sha 变 → version-changed");
    }

    #[test]
    fn quadrant_no_change_reports_none() {
        let status = compare_for_update("1.0.0", Some("aaa1111"), Some("1.0.0"), Some("aaa1111"));
        assert_eq!(status, UpdateStatus::None, "semver 与 sha 均未变 → none");
    }

    #[test]
    fn quadrant_sha_only_versions_compare_by_sha_axis() {
        // 无版本字段（版本目录名 = sha 短码）→ 只能比 sha
        let changed = compare_for_update("aaa1111", Some("aaa1111"), None, Some("bbb2222"));
        assert_eq!(changed, UpdateStatus::VersionChanged, "sha 变 → version-changed");
        let same = compare_for_update("aaa1111", Some("aaa1111"), None, Some("aaa1111"));
        assert_eq!(same, UpdateStatus::None, "sha 相同 → none 不误报");
    }

    // ---------- 不误报边界 ----------

    #[test]
    fn lower_or_equal_semver_does_not_misreport() {
        let lower = compare_for_update("2.0.0", Some("aaa"), Some("1.9.9"), Some("bbb"));
        assert_eq!(lower, UpdateStatus::None, "市场侧版本更低不误报（sha 轴不再补充判定）");
        let equal = compare_for_update("2.0.0", Some("aaa"), Some("v2.0.0"), Some("aaa"));
        assert_eq!(equal, UpdateStatus::None, "同版本（含 v 前缀等价）且 sha 相同不误报");
        let equal_no_sha = compare_for_update("2.0.0", Some("aaa"), Some("2.0.0"), None);
        assert_eq!(equal_no_sha, UpdateStatus::None, "同版本但市场侧无 sha（repo 条目）不误报");
    }

    #[test]
    fn mixed_parsability_falls_through_to_sha_axis() {
        // 已装版本是 sha 短码（不可解析），市场侧突然给出 semver：semver 轴无法比 → sha 轴
        let status = compare_for_update("aaa1111", Some("aaa1111"), Some("2.0.0"), Some("aaa1111"));
        assert_eq!(status, UpdateStatus::None, "sha 未变 → none（semver 轴证据不足不裁决）");
        let changed = compare_for_update("aaa1111", Some("aaa1111"), Some("2.0.0"), Some("bbb2222"));
        assert_eq!(changed, UpdateStatus::VersionChanged, "sha 变 → version-changed");
    }

    #[test]
    fn missing_evidence_never_misreports() {
        // 市场侧版本与 sha 双缺
        let status = compare_for_update("1.0.0", Some("aaa"), None, None);
        assert_eq!(status, UpdateStatus::None);
        // 已装无 pin（老数据）+ 市场侧只有 sha
        let status = compare_for_update("dev", None, None, Some("bbb"));
        assert_eq!(status, UpdateStatus::None, "一侧缺 sha 无法证明 → 不误报");
        // prerelease 后缀不产生 semver 伪更新：1.2.3-beta 与 1.2.3 同版本（sha 同则 none）
        let status = compare_for_update("1.2.3-beta", Some("a"), Some("1.2.3"), Some("a"));
        assert_eq!(status, UpdateStatus::None, "后缀不参与比较，同版本且 sha 相同不报更新");
        // 同版本但 sha 变化：四象限之一「semver 相同 sha 变化」→ version-changed
        let status = compare_for_update("1.2.3-beta", Some("a"), Some("1.2.3"), Some("b"));
        assert_eq!(status, UpdateStatus::VersionChanged, "同版本号重新发布由 sha 轴兜住");
    }

    #[test]
    fn available_version_text_is_preserved_verbatim() {
        let status = compare_for_update("1.0.0", None, Some("  v9.9.9  "), None);
        assert_eq!(
            status,
            UpdateStatus::UpdateAvailable { available_version: "v9.9.9".to_string() },
            "展示文本 trim 但保留原文（含 v 前缀）"
        );
    }
}
