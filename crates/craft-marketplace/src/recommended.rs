//! 推荐市场源候选清单（spec 第 9 节）：构建期资产 `assets/recommended-marketplaces.json`
//! 经 [`ASSET_BYTES`]（`include_bytes!`）编译期嵌入，运行时解析供推荐源常驻区块消费。
//!
//! 边界约定：
//! - **运行时不内置、不自动添加任何源**——推荐仅是候选，添加仍是用户确认动作；
//! - 资产是生成物性质约定：手改 JSON 即生效（无同步脚本，推荐源几乎不变）；
//!   本模块不在代码里硬编码任何推荐源条目；
//! - 解析失败（资产损坏）返回 [`RecommendedError`] 可读错误，不 panic——资产虽是
//!   编译期嵌入，但仍可能被手改损坏，商店页应降级为「推荐源不可用」而不是崩溃。
//!
//! 形态枚举 [`RecommendedForm`] 与 [`crate::manifest`] 的探测结果（[`crate::manifest::manifest_kind`]
//! 常量）提供映射判定 [`RecommendedForm::matches_manifest_kind`]，供推荐卡片形态徽标
//! 与实际仓库探测形态的对拍测试（票 #9 验收）。

use std::fmt;

use serde::Deserialize;

use crate::manifest::manifest_kind;

/// 构建期嵌入的推荐源资产字节（仓库根 `assets/recommended-marketplaces.json`）。
pub const ASSET_BYTES: &[u8] = include_bytes!("../../../assets/recommended-marketplaces.json");

// ---------- 错误 ----------

/// 推荐源资产损坏（JSON 不合法、字段缺失、取值非法、条目重复）。
/// 单变体携带可读中文原因（Display 直出，可经 toast 展示）。
#[derive(Debug, Clone, PartialEq)]
pub enum RecommendedError {
    /// 资产内容不满足结构约定（含具体原因摘要）。
    Invalid { reason: String },
}

impl fmt::Display for RecommendedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Invalid { reason } = self;
        write!(f, "推荐市场源资产损坏：{reason}")
    }
}

impl std::error::Error for RecommendedError {}

// ---------- 输出模型 ----------

/// 推荐源形态徽标（spec 第 9 节：市场 / 单插件 / 技能集三种）。
/// 与清单探测形态的映射见 [`RecommendedForm::matches_manifest_kind`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendedForm {
    /// 市场仓库（`.craftwork/marketplace.json` 或 `.claude-plugin/marketplace.json`）。
    Marketplace,
    /// 单插件仓库（`.claude-plugin/plugin.json` 等，自动包装成只含自身的市场）。
    Plugin,
    /// 裸技能集仓库（只有 `skills/` 目录，隐式插件）。
    Skills,
}

impl RecommendedForm {
    /// 资产 JSON 里的 form 字符串 → 枚举；未知取值返回 None（由解析层报错）。
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "marketplace" => Some(Self::Marketplace),
            "plugin" => Some(Self::Plugin),
            "skills" => Some(Self::Skills),
            _ => None,
        }
    }

    /// 形态徽标中文文案（推荐源卡片直接展示）。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Marketplace => "市场",
            Self::Plugin => "单插件",
            Self::Skills => "技能集",
        }
    }

    /// 与清单探测结果（[`manifest_kind`] 常量）是否一致：市场形态命中两种市场清单
    /// 探测位（CraftWork / Claude 同为「市场」徽标）；单插件命中形态 3；技能集命中形态 4。
    pub fn matches_manifest_kind(&self, observed: &str) -> bool {
        match self {
            Self::Marketplace => {
                observed == manifest_kind::CRAFTWORK_MARKET || observed == manifest_kind::CLAUDE_MARKET
            }
            Self::Plugin => observed == manifest_kind::SINGLE_PLUGIN,
            Self::Skills => observed == manifest_kind::BARE_SKILLS,
        }
    }
}

/// 一条推荐市场源候选（推荐源卡片数据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecommendedMarketplace {
    /// 仓库缩写 `owner/repo`（添加时展开为 GitHub URL，与市场源输入同一处理）。
    pub repo: String,
    /// 形态徽标。
    pub form: RecommendedForm,
    /// 展示名（专有名词保留原文）。
    pub display_name: String,
    /// 一句话中文描述（含形态与降级标注，如 hooks 自动激活不可用）。
    pub description: String,
}

// ---------- 解析 ----------

/// 解析构建期资产。生产入口；测试用 [`parse_recommended`] 注入字节。
pub fn recommended_marketplaces() -> Result<Vec<RecommendedMarketplace>, RecommendedError> {
    parse_recommended(ASSET_BYTES)
}

/// 从字节解析推荐源候选清单（纯函数，供资产损坏测试注入构造字节）。
/// 校验规则：JSON 数组；每条目 repo / form / displayName / description 齐全非空；
/// repo 为 `owner/repo` 缩写；form 为三个合法取值；repo 不重复。
/// 任一不满足返回 [`RecommendedError::Invalid`]（不 panic）。
pub fn parse_recommended(bytes: &[u8]) -> Result<Vec<RecommendedMarketplace>, RecommendedError> {
    let raws: Vec<RawRecommended> = serde_json::from_slice(bytes).map_err(|e| {
        RecommendedError::Invalid { reason: format!("JSON 解析失败（{e}）") }
    })?;

    let mut items = Vec::with_capacity(raws.len());
    let mut seen: Vec<&str> = Vec::with_capacity(raws.len());
    for (index, raw) in raws.iter().enumerate() {
        // repo 不走 required_field 的 trim 语义：首尾空白必须显式拒绝而不是静默吞掉
        //（"  a/b" trim 后成了合法缩写，会污染添加时的仓库展开）
        let repo_raw = raw.repo.as_deref().ok_or_else(|| RecommendedError::Invalid {
            reason: format!("第 {} 条目缺少字段 repo", index + 1),
        })?;
        let repo = repo_raw.trim();
        if repo.is_empty() {
            return Err(RecommendedError::Invalid { reason: format!("第 {} 条目字段 repo 为空白", index + 1) });
        }
        if repo != repo_raw {
            return Err(RecommendedError::Invalid {
                reason: format!("第 {} 条目 repo 含首尾空白：{repo_raw:?}", index + 1),
            });
        }
        let display_name = required_field(&raw.display_name, "displayName", index)?;
        let description = required_field(&raw.description, "description", index)?;
        validate_repo(repo, index)?;
        let form_text = required_field(&raw.form, "form", index)?;
        let form = RecommendedForm::parse(form_text).ok_or_else(|| {
            RecommendedError::Invalid {
                reason: format!("第 {} 条目 form 取值非法：{form_text:?}（合法取值 marketplace / plugin / skills）", index + 1),
            }
        })?;
        if seen.contains(&repo) {
            return Err(RecommendedError::Invalid {
                reason: format!("第 {} 条目仓库重复：{repo}", index + 1),
            });
        }
        seen.push(repo);
        items.push(RecommendedMarketplace {
            repo: repo.to_string(),
            form,
            display_name: display_name.to_string(),
            description: description.to_string(),
        });
    }
    Ok(items)
}

/// 必填字段：缺失或 trim 后为空报错（消息含字段名与条目序号）。
fn required_field<'a>(
    raw: &'a Option<String>,
    field: &str,
    index: usize,
) -> Result<&'a str, RecommendedError> {
    match raw.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => Ok(text),
        Some(_) => Err(RecommendedError::Invalid {
            reason: format!("第 {} 条目字段 {field} 为空白", index + 1),
        }),
        None => Err(RecommendedError::Invalid {
            reason: format!("第 {} 条目缺少字段 {field}", index + 1),
        }),
    }
}

/// repo 形态校验：`owner/repo` 缩写（恰好一个 `/`，两段均非空且不含空白字符）。
/// 与市场源输入的展开规则保持同一最小约束；更宽松的 URL 形态由添加入口处理，
/// 推荐资产固定用缩写形态。
fn validate_repo(repo: &str, index: usize) -> Result<(), RecommendedError> {
    let ok = match repo.split_once('/') {
        Some((owner, name)) => {
            !owner.is_empty()
                && !name.is_empty()
                && !name.contains('/')
                && !repo.chars().any(char::is_whitespace)
        }
        None => false,
    };
    if ok {
        Ok(())
    } else {
        Err(RecommendedError::Invalid {
            reason: format!("第 {} 条目 repo 不是 owner/repo 缩写：{repo:?}", index + 1),
        })
    }
}

/// 资产原始 JSON 结构（字段全部可选：缺失在 [`required_field`] 给出带字段名的可读错误，
/// 而不是 serde 一次性报「missing field」；未知字段默认忽略）。
#[derive(Debug, Default, Deserialize)]
struct RawRecommended {
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    form: Option<String>,
    #[serde(rename = "displayName", default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::manifest::{paths, probe_market};

    /// 构造坏资产字节（非法 JSON / 缺字段 / 重复仓库等场景共用）。
    fn parse_str(text: &str) -> Result<Vec<RecommendedMarketplace>, RecommendedError> {
        parse_recommended(text.as_bytes())
    }

    // ---------- 资产解析（票 #9：schema 校验） ----------

    #[test]
    fn recommended_asset_parses_with_five_entries_and_non_empty_text() {
        let items = recommended_marketplaces().expect("内置推荐源资产必须可解析");
        assert_eq!(items.len(), 5, "首批五条推荐源");
        let repos: Vec<&str> = items.iter().map(|i| i.repo.as_str()).collect();
        assert_eq!(
            repos,
            vec![
                "obra/superpowers",
                "mattpocock/skills",
                "DietrichGebert/ponytail",
                "anthropics/skills",
                "rampstackco/claude-skills",
            ],
            "五条目仓库与顺序固定（票 #9）"
        );
        for item in &items {
            assert!(!item.display_name.is_empty(), "{} 展示名非空", item.repo);
            assert!(
                item.description.contains('（') || item.description.chars().count() > 8,
                "{} 描述应为有内容的中文句子：{}",
                item.repo,
                item.description
            );
        }
    }

    #[test]
    fn recommended_asset_form_annotations_match_ticket() {
        let items = recommended_marketplaces().unwrap();
        let form_of = |repo: &str| {
            items
                .iter()
                .find(|i| i.repo == repo)
                .unwrap_or_else(|| panic!("缺少推荐源 {repo}"))
                .form
        };
        assert_eq!(form_of("obra/superpowers"), RecommendedForm::Plugin, "superpowers 是单插件仓库");
        assert_eq!(form_of("mattpocock/skills"), RecommendedForm::Marketplace);
        assert_eq!(form_of("DietrichGebert/ponytail"), RecommendedForm::Marketplace);
        assert_eq!(form_of("anthropics/skills"), RecommendedForm::Marketplace);
        assert_eq!(form_of("rampstackco/claude-skills"), RecommendedForm::Marketplace);
    }

    // ---------- form 枚举与 manifest 探测形态映射（票 #9 验收） ----------

    /// 用 manifest 探测构造指定形态的仓库快照并返回探测结果形态串。
    fn probe_kind(root: &Path, files: &[(&str, &str)], repo_name: &str) -> String {
        for (rel, content) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
        }
        probe_market(root, repo_name, Some("abc1234")).unwrap().manifest_kind
    }

    #[test]
    fn forms_map_to_actual_probe_results_of_three_shapes() {
        // 形态 3：单插件仓库（obra/superpowers 形态）→ Plugin 徽标命中
        let dir = tempfile::tempdir().unwrap();
        let kind = probe_kind(dir.path(), &[(paths::CLAUDE_PLUGIN, r#"{"name":"superpowers"}"#)], "superpowers");
        assert_eq!(kind, manifest_kind::SINGLE_PLUGIN);
        assert!(RecommendedForm::Plugin.matches_manifest_kind(&kind));

        // 形态 2：市场清单（mattpocock / anthropics / rampstackco 形态）→ Marketplace 徽标命中
        let dir = tempfile::tempdir().unwrap();
        let kind = probe_kind(
            dir.path(),
            &[(paths::CLAUDE_MARKET, r#"{"name":"skills","plugins":[{"name":"engineering","source":"./"}]}"#)],
            "skills",
        );
        assert_eq!(kind, manifest_kind::CLAUDE_MARKET);
        assert!(RecommendedForm::Marketplace.matches_manifest_kind(&kind));

        // 形态 1：CraftWork 一等市场清单 → 同样命中 Marketplace 徽标
        let dir = tempfile::tempdir().unwrap();
        let kind = probe_kind(
            dir.path(),
            &[(paths::CRAFTWORK_MARKET, r#"{"name":"craft-market","plugins":[]}"#)],
            "craft-market",
        );
        assert_eq!(kind, manifest_kind::CRAFTWORK_MARKET);
        assert!(RecommendedForm::Marketplace.matches_manifest_kind(&kind));

        // 形态 4：裸技能集（隐式插件）→ Skills 徽标命中
        let dir = tempfile::tempdir().unwrap();
        let kind = probe_kind(dir.path(), &[("skills/demo/SKILL.md", "---\nname: demo\n---\n正文")], "bare-repo");
        assert_eq!(kind, manifest_kind::BARE_SKILLS);
        assert!(RecommendedForm::Skills.matches_manifest_kind(&kind));
    }

    #[test]
    fn form_badge_matching_is_exclusive_across_shapes() {
        // 徽标与形态一一对应，不互相误命中（推荐卡片标注错误即测试暴露）
        assert!(!RecommendedForm::Plugin.matches_manifest_kind(manifest_kind::CLAUDE_MARKET));
        assert!(!RecommendedForm::Marketplace.matches_manifest_kind(manifest_kind::SINGLE_PLUGIN));
        assert!(!RecommendedForm::Skills.matches_manifest_kind(manifest_kind::CLAUDE_MARKET));
        assert!(!RecommendedForm::Marketplace.matches_manifest_kind(manifest_kind::BARE_SKILLS));
    }

    #[test]
    fn form_labels_cover_three_badges() {
        assert_eq!(RecommendedForm::Marketplace.label(), "市场");
        assert_eq!(RecommendedForm::Plugin.label(), "单插件");
        assert_eq!(RecommendedForm::Skills.label(), "技能集");
    }

    // ---------- 资产损坏：可读错误不 panic（票 #9 + spec 第 9 节） ----------

    #[test]
    fn corrupted_asset_returns_readable_error_instead_of_panicking() {
        // 非 JSON
        let err = parse_str("这不是 JSON").unwrap_err();
        assert!(matches!(err, RecommendedError::Invalid { .. }));
        assert!(err.to_string().contains("资产损坏"), "错误消息可读：{err}");
        assert!(err.to_string().contains("JSON 解析失败"));

        // 数组元素缺字段（repo 缺失）
        let err = parse_str(r#"[{"form":"plugin","displayName":"x","description":"y"}]"#).unwrap_err();
        assert!(err.to_string().contains("repo"), "缺字段错误应点名字段：{err}");

        // 空白 description
        let err = parse_str(r#"[{"repo":"a/b","form":"plugin","displayName":"x","description":"  "}]"#).unwrap_err();
        assert!(err.to_string().contains("description"), "实际 {err}");

        // form 取值非法
        let err = parse_str(r#"[{"repo":"a/b","form":"bundle","displayName":"x","description":"y"}]"#).unwrap_err();
        assert!(err.to_string().contains("form 取值非法"), "实际 {err}");

        // repo 不是 owner/repo 缩写（含首尾空白：必须显式拒绝而不是 trim 后静默放行）
        for (bad, keyword) in [
            ("no-slash", "owner/repo"),
            ("a/b/c", "owner/repo"),
            ("  a/b", "空白"),
            ("a b/c", "owner/repo"),
            ("https://github.com/a/b", "owner/repo"),
        ] {
            let asset = format!(r#"[{{"repo":"{bad}","form":"plugin","displayName":"x","description":"y"}}]"#);
            let err = parse_str(&asset).unwrap_err();
            assert!(err.to_string().contains(keyword), "repo={bad:?} 应被拒且消息含 {keyword:?}：{err}");
        }

        // 仓库重复
        let err = parse_str(
            r#"[
                {"repo":"a/b","form":"plugin","displayName":"x","description":"y"},
                {"repo":"a/b","form":"marketplace","displayName":"z","description":"w"}
            ]"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("重复"), "实际 {err}");

        // 顶层不是数组
        let err = parse_str(r#"{"repo":"a/b"}"#).unwrap_err();
        assert!(matches!(err, RecommendedError::Invalid { .. }));

        // 空数组合法（0 条目——资产被清空时推荐区不崩溃）
        assert_eq!(parse_str("[]").unwrap(), Vec::new());
    }
}
