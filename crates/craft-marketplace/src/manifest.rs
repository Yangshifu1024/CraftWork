//! 市场清单探测与解析（spec 第 2 节）：仓库快照目录 → 结构化市场模型。
//!
//! 纯函数层：输入 clone 落地后的本地仓库快照根目录，输出 [`MarketManifest`]，
//! 零网络零 git 调用。四种形态按序判定：
//!
//! 1. `.craftwork/marketplace.json`（CraftWork 一等市场清单）
//! 2. `.claude-plugin/marketplace.json`（Claude Code 市场清单，只读兼容）
//! 3. 插件清单（`.craftwork/plugin.json` > `.claude-plugin/plugin.json` > `.codex-plugin/plugin.json`）：
//!    单插件仓库，自动包装成只含自身的市场（市场 id = 插件 name）
//! 4. 裸 `skills/` 目录：裸技能集仓库，生成隐式插件（name = 仓库名，
//!    version = 调用方传入的 git sha 短码）
//! 5. 皆无 → [`ManifestError::NotSkillSource`]（「不是技能源」可读错误）
//!
//! 三种清单（CraftWork / Claude / Codex）同构解析、容忍未知字段（`featured` /
//! `pluginRoot` / `owner` 等一律忽略）；`hooks` / `agents` / `mcpServers` / `.mcp.json`
//! 只解析成布尔标注（[`ComponentAnnotations`]，供详情页「当前版本不加载」提示），
//! 不参与加载（解析层与加载层分离——解析只负责「说清仓库里有什么」）。
//!
//! 为什么 hooks 不加载：生态里的 hooks 几乎全是 Node 脚本，CraftWork 没有 Node 运行时，
//! 装了也跑不起来；含 hooks 的插件因此在详情页降级为「按需 / 斜杠激活」并明示。清单 JSON
//! 损坏或缺必填字段 name 时报 [`ManifestError`]，错误消息含文件绝对路径与原因摘要。
//!
//! 来源类型（[`EntrySource::kind`]）直接复用 [`crate::store::source_kind`] 常量，
//! 解析结果落 `skill_plugins.source_kind` 列零转换。

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::source_kind;

// ---------- 清单文件位置 ----------

/// 三类清单与配套文件的标准位置（相对仓库快照根 / 插件根目录）。
pub mod paths {
    /// CraftWork 一等市场清单。
    pub const CRAFTWORK_MARKET: &str = ".craftwork/marketplace.json";
    /// Claude Code 市场清单（只读兼容）。
    pub const CLAUDE_MARKET: &str = ".claude-plugin/marketplace.json";
    /// CraftWork 插件清单。
    pub const CRAFTWORK_PLUGIN: &str = ".craftwork/plugin.json";
    /// Claude Code 插件清单。
    pub const CLAUDE_PLUGIN: &str = ".claude-plugin/plugin.json";
    /// Codex 插件清单（只读兼容）。
    pub const CODEX_PLUGIN: &str = ".codex-plugin/plugin.json";
    /// 插件根目录旁的 MCP 配置文件（只解析存在性，不参与加载）。
    pub const DOT_MCP_JSON: &str = ".mcp.json";
}

/// 插件清单探测顺序（spec 第 2 节：`.craftwork` > `.claude-plugin` > `.codex-plugin`，
/// 先命中先用；CraftWork 不产出后两种，仅为只读兼容保留）。
const PLUGIN_MANIFEST_ORDER: [&str; 3] = [
    paths::CRAFTWORK_PLUGIN,
    paths::CLAUDE_PLUGIN,
    paths::CODEX_PLUGIN,
];

/// 市场探测形态（[`MarketManifest::manifest_kind`]；商店推荐源卡片的形态徽标直接消费）。
pub mod manifest_kind {
    /// 形态 1：CraftWork 一等市场清单。
    pub const CRAFTWORK_MARKET: &str = "craftwork-market";
    /// 形态 2：Claude Code 市场清单（只读兼容）。
    pub const CLAUDE_MARKET: &str = "claude-market";
    /// 形态 3：单插件仓库自动包装成只含自身的市场。
    pub const SINGLE_PLUGIN: &str = "single-plugin";
    /// 形态 4：裸技能集仓库（隐式插件）。
    pub const BARE_SKILLS: &str = "bare-skills";
}

// ---------- 错误 ----------

/// 清单探测与解析错误。全部变体携带可读中文消息（Display 直出，可经 toast 展示）。
#[derive(Debug, Clone, PartialEq)]
pub enum ManifestError {
    /// 形态 5：目录里没有任何可识别的清单与 skills 目录，不是技能源。
    NotSkillSource {
        /// 仓库快照根目录绝对路径。
        root: PathBuf,
    },
    /// 清单文件读取失败或 JSON 解析失败（含文件路径与原因摘要）。
    Parse {
        /// 清单文件绝对路径。
        path: PathBuf,
        /// IO 或 serde 错误摘要。
        reason: String,
    },
    /// 必填字段 name 缺失或为空白（含清单文件路径）。
    MissingName {
        /// 清单文件绝对路径。
        path: PathBuf,
    },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSkillSource { root } => write!(
                f,
                "不是技能源：{} 下没有市场清单（{}、{}）、插件清单（plugin.json）或 skills/ 目录",
                root.display(),
                paths::CRAFTWORK_MARKET,
                paths::CLAUDE_MARKET
            ),
            Self::Parse { path, reason } => write!(f, "清单解析失败：{}（{reason}）", path.display()),
            Self::MissingName { path } => write!(f, "清单缺少必填字段 name（不能为空白）：{}", path.display()),
        }
    }
}

impl std::error::Error for ManifestError {}

// ---------- 输出模型 ----------

/// 市场条目来源（清单条目 `source` 字段三种形态的归一结果）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntrySource {
    /// 来源类型：[`source_kind::ENTRY_RELATIVE`](crate::store::source_kind)（插件在市场
    /// 仓库内，安装时从市场快照直接拷贝）或 `REPO`（独立仓库，按来源二次 clone）。
    /// 直接复用 store.rs 常量，落 `skill_plugins.source_kind` 列零转换。
    pub kind: String,
    /// 来源原文：相对路径（含 `./`）、`owner/repo` 缩写或 git URL。
    pub source_ref: String,
}

/// 插件组件标注（hooks / agents / MCP）：只做「声明了什么」的布尔记录，
/// 供详情页标注「包含 hooks / 代理 / MCP 配置，当前版本不加载」（spec 第 1 节降级语义）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentAnnotations {
    /// 插件声明了 hooks（自动激活不可用，技能按需或斜杠激活）。
    pub has_hooks: bool,
    /// 插件声明了子代理定义。
    pub has_agents: bool,
    /// 插件声明了 MCP 配置（plugin.json 内联 mcpServers 或旁置 .mcp.json）。
    pub has_mcp: bool,
}

/// 市场清单条目（marketplace.json 的 plugins 数组元素，或单插件 / 裸技能集的包装条目）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketEntry {
    /// 插件名（必填；与插件清单 name 一致才可安装，spec 第 2 节）。
    pub name: String,
    /// 一句话描述（可选）。
    pub description: Option<String>,
    /// 版本（可选；版本解析第二优先级，见 spec 第 4 节）。
    pub version: Option<String>,
    /// 分类（可选）。
    pub category: Option<String>,
    /// 标签（可选）。
    pub tags: Vec<String>,
    /// 条目来源（三种形态归一；缺省按 Claude 兼容惯例回退 `./<name>`）。
    pub source: EntrySource,
    /// 条目级 skills 数组（anthropics/skills 形态，strict:false）：显式技能路径列表。
    /// 安装时仅拷贝声明的技能路径；None = 未声明（装整个插件目录，扫描层自扫）。
    pub entry_skills: Option<Vec<String>>,
    /// 组件标注（条目内声明的 hooks / agents / mcpServers）。
    pub component_annotations: ComponentAnnotations,
}

/// 市场清单（四形态探测的统一输出：标准市场、单插件包装、裸技能集包装）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketManifest {
    /// 市场 id：市场清单顶层 name / 单插件仓库的插件 name / 裸技能集的仓库名。
    /// 唯一性与重复添加判定都以它为准（spec 第 2 节）。
    pub id: String,
    /// 探测形态（[`manifest_kind`] 常量）。
    pub manifest_kind: String,
    /// 市场条目（裸技能集为单条隐式插件条目；标准市场可为空数组）。
    pub entries: Vec<MarketEntry>,
    /// 清单文件绝对路径（形态 1/2 为市场清单，形态 3 为插件清单；形态 4 无清单文件为 None）。
    pub manifest_path: Option<PathBuf>,
}

/// 插件清单（plugin.json 同构解析结果；三种位置探测顺序见 [`PLUGIN_MANIFEST_ORDER`]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// 插件名（必填）。
    pub name: String,
    /// 版本（可选；版本解析首选，spec 第 4 节）。
    pub version: Option<String>,
    /// 描述（可选）。
    pub description: Option<String>,
    /// 组件标注（hooks / agents / MCP 只解析不加载）。
    pub has_hooks: bool,
    /// 是否声明了子代理定义。
    pub has_agents: bool,
    /// 是否声明了 MCP 配置。
    pub has_mcp: bool,
    /// 清单文件绝对路径（探测命中的位置）。
    pub manifest_path: PathBuf,
}

// ---------- 探测入口 ----------

/// 四形态按序探测（spec 第 2 节）：仓库快照根目录 → 市场模型。
///
/// - `repo_name`：仓库名（形态 4 隐式插件的 name 与市场 id；其余形态不消费）。
/// - `sha_hint`：git sha 短码（形态 4 隐式插件的 version；其余形态不消费，
///   形态 3 的版本走插件清单 version 字段）。调用方 clone 后取 HEAD sha 传入。
///
/// 文件存在性判定用 `is_file` / `is_dir`：路径被同名目录占位等怪异仓库按「不存在」
/// 处理，落到下一探测位，不放大成硬错误。
pub fn probe_market(repo_root: &Path, repo_name: &str, sha_hint: Option<&str>) -> Result<MarketManifest, ManifestError> {
    // 形态 1：CraftWork 一等市场清单
    let craftwork_market = repo_root.join(paths::CRAFTWORK_MARKET);
    if craftwork_market.is_file() {
        return parse_market_manifest(&craftwork_market, manifest_kind::CRAFTWORK_MARKET);
    }
    // 形态 2：Claude 市场清单（只读兼容）
    let claude_market = repo_root.join(paths::CLAUDE_MARKET);
    if claude_market.is_file() {
        return parse_market_manifest(&claude_market, manifest_kind::CLAUDE_MARKET);
    }
    // 形态 3：单插件仓库（无市场清单）→ 包装成只含自身的市场，市场 id = 插件 name
    if let Some(plugin) = probe_plugin_manifest(repo_root)? {
        return Ok(single_plugin_market(plugin));
    }
    // 形态 4：裸 skills/ 目录 → 隐式插件（name = 仓库名，version = git sha 短码）
    if repo_root.join("skills").is_dir() {
        return Ok(bare_skills_market(repo_name, sha_hint));
    }
    // 形态 5：皆无 → 不是技能源
    Err(ManifestError::NotSkillSource { root: repo_root.to_path_buf() })
}

/// 按探测顺序查找插件清单（`.craftwork` > `.claude-plugin` > `.codex-plugin`）。
/// 三个位置都没有时返回 `Ok(None)`（调用方继续下一形态）；命中但解析失败时传播错误。
pub fn probe_plugin_manifest(plugin_root: &Path) -> Result<Option<PluginManifest>, ManifestError> {
    for rel in PLUGIN_MANIFEST_ORDER {
        let path = plugin_root.join(rel);
        if !path.is_file() {
            continue;
        }
        let raw: RawPlugin = read_manifest(&path)?;
        let name = required_name(raw.name, &path)?;
        return Ok(Some(PluginManifest {
            has_hooks: raw.hooks.is_some(),
            has_agents: raw.agents.is_some(),
            has_mcp: raw.mcp_servers.is_some() || plugin_root.join(paths::DOT_MCP_JSON).is_file(),
            name,
            version: non_blank(raw.version),
            description: raw.description,
            manifest_path: path,
        }));
    }
    Ok(None)
}

// ---------- 解析 ----------

/// 解析市场清单文件为市场模型（形态 1/2 共用；两种格式同构，仅形态标注不同）。
fn parse_market_manifest(path: &Path, kind: &str) -> Result<MarketManifest, ManifestError> {
    let raw: RawMarketplace = read_manifest(path)?;
    let id = required_name(raw.name, path)?;
    let mut entries = Vec::with_capacity(raw.plugins.as_ref().map_or(0, Vec::len));
    for entry in raw.plugins.unwrap_or_default() {
        entries.push(entry_from_raw(entry, path)?);
    }
    Ok(MarketManifest {
        id,
        manifest_kind: kind.to_string(),
        entries,
        manifest_path: Some(path.to_path_buf()),
    })
}

/// 形态 3 包装：单插件仓库 → 只含自身的市场（id = 插件 name，条目来源 = 仓库根 `./`）。
fn single_plugin_market(plugin: PluginManifest) -> MarketManifest {
    MarketManifest {
        id: plugin.name.clone(),
        manifest_kind: manifest_kind::SINGLE_PLUGIN.to_string(),
        manifest_path: Some(plugin.manifest_path.clone()),
        entries: vec![MarketEntry {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            version: plugin.version.clone(),
            category: None,
            tags: Vec::new(),
            source: EntrySource { kind: source_kind::ENTRY_RELATIVE.to_string(), source_ref: "./".to_string() },
            entry_skills: None,
            component_annotations: ComponentAnnotations {
                has_hooks: plugin.has_hooks,
                has_agents: plugin.has_agents,
                has_mcp: plugin.has_mcp,
            },
        }],
    }
}

/// 形态 4 包装：裸技能集仓库 → 单条隐式插件条目（name = 仓库名，version = git sha 短码）。
/// 条目来源同样是仓库根 `./`；技能 discovery 由扫描层负责（spec 第 5 节），清单层不列技能。
fn bare_skills_market(repo_name: &str, sha_hint: Option<&str>) -> MarketManifest {
    MarketManifest {
        id: repo_name.to_string(),
        manifest_kind: manifest_kind::BARE_SKILLS.to_string(),
        manifest_path: None,
        entries: vec![MarketEntry {
            name: repo_name.to_string(),
            description: None,
            version: sha_hint.map(str::to_string),
            category: None,
            tags: Vec::new(),
            source: EntrySource { kind: source_kind::ENTRY_RELATIVE.to_string(), source_ref: "./".to_string() },
            entry_skills: None,
            component_annotations: ComponentAnnotations::default(),
        }],
    }
}

/// 条目原始 JSON → 输出模型（name 必填、source 三形态归一、未知字段已在反序列化时丢弃）。
fn entry_from_raw(raw: RawEntry, manifest_path: &Path) -> Result<MarketEntry, ManifestError> {
    let name = required_name(raw.name, manifest_path)?;
    let source = match raw.source.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => classify_source(s),
        // Claude 兼容惯例：条目省略 source 时按相对路径回退到 ./<name>
        _ => EntrySource { kind: source_kind::ENTRY_RELATIVE.to_string(), source_ref: format!("./{name}") },
    };
    Ok(MarketEntry {
        name,
        description: raw.description,
        version: non_blank(raw.version),
        category: raw.category,
        tags: raw.tags.unwrap_or_default(),
        source,
        entry_skills: raw.skills,
        component_annotations: ComponentAnnotations {
            has_hooks: raw.hooks.is_some(),
            has_agents: raw.agents.is_some(),
            has_mcp: raw.mcp_servers.is_some(),
        },
    })
}

/// 条目 source 三形态归一（spec 第 2 节）：
/// 含 `://` → git URL（REPO）；`./` / `../` 前缀 → 相对路径（ENTRY_RELATIVE）；
/// 恰好一段 `/` 分隔的两段非空文本 → `owner/repo` 缩写（REPO）；其余（无斜杠目录名、
/// 多段路径、以 `/` 开头的仓库内路径）→ 相对路径（ENTRY_RELATIVE）。
fn classify_source(raw: &str) -> EntrySource {
    // `./` / `../` 前缀先行拦截（"." 段不是合法 owner 名，避免 "./dir" 被误判为缩写）
    let repo = raw.contains("://")
        || (!raw.starts_with("./") && !raw.starts_with("../") && is_owner_repo_shorthand(raw));
    let kind = if repo { source_kind::REPO } else { source_kind::ENTRY_RELATIVE };
    EntrySource { kind: kind.to_string(), source_ref: raw.to_string() }
}

/// `owner/repo` 缩写判定：恰好一个 `/`，两段均非空。
fn is_owner_repo_shorthand(raw: &str) -> bool {
    match raw.split_once('/') {
        Some((owner, repo)) => !owner.is_empty() && !repo.is_empty() && !repo.contains('/'),
        None => false,
    }
}

/// 必填 name：缺失或 trim 后为空白报 [`ManifestError::MissingName`]（含清单路径）。
fn required_name(raw: Option<String>, path: &Path) -> Result<String, ManifestError> {
    match raw.map(|n| n.trim().to_string()) {
        Some(name) if !name.is_empty() => Ok(name),
        _ => Err(ManifestError::MissingName { path: path.to_path_buf() }),
    }
}

/// 可选文本字段：空白串视为未填（版本解析回退链不受空串干扰，spec 第 4 节）。
fn non_blank(raw: Option<String>) -> Option<String> {
    match raw {
        Some(v) if v.trim().is_empty() => None,
        other => other,
    }
}

/// 读文件并反序列化为 `T`。读取失败与 JSON 解析失败统一报 [`ManifestError::Parse`]，
/// 消息含文件绝对路径与原因摘要（serde 错误自带行列号）。
fn read_manifest<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, ManifestError> {
    let text = std::fs::read_to_string(path).map_err(|e| ManifestError::Parse {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|e| ManifestError::Parse { path: path.to_path_buf(), reason: e.to_string() })
}

// ---------- 原始 JSON 结构（容忍未知字段） ----------

/// serde 默认忽略未知字段，`featured` / `pluginRoot` / `owner` 等顶层字段天然容忍；
/// 可选数组一律走 Option 再落默认值，以容忍显式 `null`。
#[derive(Debug, Default, Deserialize)]
struct RawMarketplace {
    /// 市场名（必填；缺失或空白在 [`parse_market_manifest`] 报错）。
    #[serde(default)]
    name: Option<String>,
    /// 市场条目数组（Claude / CraftWork 同构；缺失或 null 视为空市场）。
    #[serde(default)]
    plugins: Option<Vec<RawEntry>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEntry {
    /// 条目名（必填；缺失或空白在 [`entry_from_raw`] 报错）。
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 条目来源原文（三种形态，见 [`classify_source`]；缺省回退 `./<name>`）。
    #[serde(default)]
    source: Option<String>,
    /// 条目级 skills 数组（anthropics/skills 形态；null 视为未声明）。
    #[serde(default)]
    skills: Option<Vec<String>>,
    /// hooks 声明（对象 / 数组 / 字符串路径均可，只判存在性）。
    #[serde(default)]
    hooks: Option<serde_json::Value>,
    /// agents 声明（只判存在性）。
    #[serde(default)]
    agents: Option<serde_json::Value>,
    /// 内联 MCP 配置（只判存在性；旁置 `.mcp.json` 在 [`probe_plugin_manifest`] 判定）。
    #[serde(rename = "mcpServers", default)]
    mcp_servers: Option<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPlugin {
    /// 插件名（必填；缺失或空白在 [`probe_plugin_manifest`] 报错）。
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    hooks: Option<serde_json::Value>,
    #[serde(default)]
    agents: Option<serde_json::Value>,
    #[serde(rename = "mcpServers", default)]
    mcp_servers: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// 在临时目录内写最小目录树（相对路径 + 文本内容），返回根路径。
    /// fixture 用程序化构造而非提交 golden 目录树：六类形态每棵树仅 1~2 个文本文件，
    /// 程序化构造更紧凑，且与 store.rs 测试自建临时目录的风格一致。
    fn write_tree(root: &Path, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("创建 fixture 目录失败");
            }
            std::fs::write(&path, content).expect("写入 fixture 文件失败");
        }
    }

    fn entry_named<'a>(manifest: &'a MarketManifest, name: &str) -> &'a MarketEntry {
        manifest
            .entries
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("市场 {} 缺少条目 {name}", manifest.id))
    }

    fn relative(kind_ref: &str, path_ref: &str) -> EntrySource {
        EntrySource { kind: kind_ref.to_string(), source_ref: path_ref.to_string() }
    }

    // ---------- 四形态判定顺序 ----------

    #[test]
    fn probe_prefers_craftwork_marketplace_over_all_other_forms() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[
                // 形态 1/2/3/4 的探测物全部在场，必须命中形态 1
                (paths::CRAFTWORK_MARKET, r#"{"name":"demo","plugins":[{"name":"alpha","source":"./x"}]}"#),
                (paths::CLAUDE_MARKET, r#"{"name":"claude-demo","plugins":[]}"#),
                (paths::CLAUDE_PLUGIN, r#"{"name":"solo"}"#),
                ("skills/alpha/SKILL.md", "---\nname: alpha\n---\n正文"),
            ],
        );
        let market = probe_market(dir.path(), "demo-repo", Some("abc1234")).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::CRAFTWORK_MARKET);
        assert_eq!(market.id, "demo");
        assert_eq!(market.entries.len(), 1);
        assert_eq!(market.manifest_path.as_deref(), Some(dir.path().join(paths::CRAFTWORK_MARKET).as_path()));
    }

    #[test]
    fn probe_prefers_claude_marketplace_over_plugin_and_bare_skills() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[
                // 形态 2/3/4 同时在场，必须命中形态 2
                (paths::CLAUDE_MARKET, r#"{"name":"mattpocock-skills","plugins":[{"name":"engineering","source":"./"}]}"#),
                (paths::CLAUDE_PLUGIN, r#"{"name":"solo"}"#),
                ("skills/alpha/SKILL.md", "---\nname: alpha\n---\n正文"),
            ],
        );
        let market = probe_market(dir.path(), "mattpocock-skills", None).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::CLAUDE_MARKET);
        assert_eq!(market.id, "mattpocock-skills");
    }

    #[test]
    fn probe_prefers_plugin_manifest_over_bare_skills_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[
                // 形态 3/4 同时在场，必须命中形态 3
                (paths::CLAUDE_PLUGIN, r#"{"name":"superpowers","version":"2.0.0"}"#),
                ("skills/alpha/SKILL.md", "---\nname: alpha\n---\n正文"),
            ],
        );
        let market = probe_market(dir.path(), "superpowers", Some("abc1234")).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::SINGLE_PLUGIN);
        assert_eq!(market.id, "superpowers", "单插件仓库的市场 id = 插件 name，与仓库名无关");
        assert_eq!(market.entries.len(), 1);
        assert_eq!(market.entries[0].version.as_deref(), Some("2.0.0"), "形态 3 版本来自插件清单");
    }

    #[test]
    fn probe_bare_skills_dir_becomes_implicit_plugin_with_repo_name_and_sha() {
        let dir = tempfile::tempdir().unwrap();
        // openspec 形态：只有裸 skills/ 目录，没有任何清单
        write_tree(
            dir.path(),
            &[
                ("skills/openspec/SKILL.md", "---\nname: openspec\n---\n正文"),
                ("skills/archive/SKILL.md", "---\nname: archive\n---\n正文"),
            ],
        );
        let market = probe_market(dir.path(), "openspec", Some("9d3f2a1")).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::BARE_SKILLS);
        assert_eq!(market.id, "openspec", "形态 4 市场 id = 仓库名");
        assert_eq!(market.manifest_path, None, "裸技能集没有清单文件");
        assert_eq!(market.entries.len(), 1);
        let entry = &market.entries[0];
        assert_eq!(entry.name, "openspec", "隐式插件 name = 仓库名");
        assert_eq!(entry.version.as_deref(), Some("9d3f2a1"), "隐式插件 version = 传入的 git sha 短码");
        assert_eq!(entry.source, relative(source_kind::ENTRY_RELATIVE, "./"));
        assert_eq!(entry.entry_skills, None, "裸技能集不声明条目级 skills");
        assert_eq!(entry.component_annotations, ComponentAnnotations::default());
        // 不传 sha 短码时版本为空（编排层自行回退）
        let market = probe_market(dir.path(), "openspec", None).unwrap();
        assert_eq!(market.entries[0].version, None);
    }

    #[test]
    fn probe_rejects_directory_without_manifest_or_skills() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[("README.md", "# 只是一个普通仓库")]);
        let err = probe_market(dir.path(), "plain", None).unwrap_err();
        assert!(matches!(err, ManifestError::NotSkillSource { .. }), "实际 {err:?}");
        assert!(err.to_string().contains("不是技能源"), "错误消息应可读，实际 {err}");
        assert!(err.to_string().contains(dir.path().to_str().unwrap()), "错误消息应含仓库根路径");
    }

    // ---------- 形态 3：插件清单三位置探测 ----------

    #[test]
    fn probe_wraps_single_codex_plugin_when_claude_manifest_absent() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CODEX_PLUGIN, r#"{"name":"codex-only","description":"Codex 插件"}"#)]);
        let market = probe_market(dir.path(), "repo", None).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::SINGLE_PLUGIN);
        assert_eq!(market.id, "codex-only");
        let entry = &market.entries[0];
        assert_eq!(entry.source, relative(source_kind::ENTRY_RELATIVE, "./"), "单插件条目来源 = 仓库根 ./");
        assert_eq!(entry.description.as_deref(), Some("Codex 插件"));
        assert_eq!(market.manifest_path.as_deref(), Some(dir.path().join(paths::CODEX_PLUGIN).as_path()));
    }

    #[test]
    fn plugin_manifest_probes_craftwork_then_claude_then_codex() {
        let dir = tempfile::tempdir().unwrap();
        // .craftwork 与 .claude-plugin 同时在场：先命中 .craftwork
        write_tree(
            dir.path(),
            &[
                (paths::CRAFTWORK_PLUGIN, r#"{"name":"craftwork-first"}"#),
                (paths::CLAUDE_PLUGIN, r#"{"name":"claude-second"}"#),
                (paths::CODEX_PLUGIN, r#"{"name":"codex-third"}"#),
            ],
        );
        let found = probe_plugin_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(found.name, "craftwork-first");

        // 移除 .craftwork 后：命中 .claude-plugin
        std::fs::remove_file(dir.path().join(paths::CRAFTWORK_PLUGIN)).unwrap();
        let found = probe_plugin_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(found.name, "claude-second");

        // 只剩 .codex-plugin：命中 codex
        std::fs::remove_file(dir.path().join(paths::CLAUDE_PLUGIN)).unwrap();
        let found = probe_plugin_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(found.name, "codex-third");

        // 全部移除：None（继续下一形态探测）
        std::fs::remove_file(dir.path().join(paths::CODEX_PLUGIN)).unwrap();
        assert_eq!(probe_plugin_manifest(dir.path()).unwrap(), None);
    }

    // ---------- 形态 2：市场清单条目解析 ----------

    #[test]
    fn probe_parses_mattpocock_market_with_self_referencing_relative_source() {
        let dir = tempfile::tempdir().unwrap();
        // mattpocock 形态：市场清单 + 条目 source "./"（插件自指市场仓库根）
        write_tree(
            dir.path(),
            &[(
                paths::CLAUDE_MARKET,
                r#"{"name":"mattpocock-skills","plugins":[
                    {"name":"engineering","description":"工程方法论技能","source":"./"},
                    {"name":"productivity","description":"生产力技能","source":"./"}
                ]}"#,
            )],
        );
        let market = probe_market(dir.path(), "mattpocock-skills", None).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::CLAUDE_MARKET);
        assert_eq!(market.entries.len(), 2);
        for name in ["engineering", "productivity"] {
            let entry = entry_named(&market, name);
            assert_eq!(entry.source, relative(source_kind::ENTRY_RELATIVE, "./"), "source \"./\" 标 entry-relative");
            assert_eq!(entry.entry_skills, None);
            assert_eq!(entry.component_annotations, ComponentAnnotations::default());
        }
    }

    #[test]
    fn probe_parses_entry_level_skills_array_of_anthropics_form() {
        let dir = tempfile::tempdir().unwrap();
        // anthropics/skills 形态：条目用显式 skills 路径数组声明归属（strict:false 模式）
        write_tree(
            dir.path(),
            &[(
                paths::CLAUDE_MARKET,
                r#"{"name":"anthropics-skills","owner":{"name":"Anthropic"},"plugins":[
                    {"name":"document-skills","description":"文档处理技能","source":"./",
                     "skills":["document/docx","document/pdf","document/pptx","document/xlsx"]}
                ]}"#,
            )],
        );
        let market = probe_market(dir.path(), "anthropics-skills", None).unwrap();
        let entry = entry_named(&market, "document-skills");
        assert_eq!(
            entry.entry_skills,
            Some(vec!["document/docx".into(), "document/pdf".into(), "document/pptx".into(), "document/xlsx".into()]),
            "条目级 skills 数组解析为显式技能路径列表"
        );
        // 顶层 owner 对象等未知字段被容忍忽略，不影响解析
        assert_eq!(market.id, "anthropics-skills");
    }

    #[test]
    fn probe_classifies_all_three_entry_source_shapes_in_rampstackco_market() {
        let dir = tempfile::tempdir().unwrap();
        // rampstackco 形态（形态 2）：一个清单内同时出现三种 source 形态
        write_tree(
            dir.path(),
            &[(
                paths::CLAUDE_MARKET,
                r#"{"name":"rampstackco-claude-skills","plugins":[
                    {"name":"in-tree","source":"./skills/brand-design"},
                    {"name":"shorthand","source":"some-owner/some-repo"},
                    {"name":"git-url","source":"https://github.com/some-owner/some-repo.git"}
                ]}"#,
            )],
        );
        let market = probe_market(dir.path(), "rampstackco-claude-skills", None).unwrap();
        assert_eq!(entry_named(&market, "in-tree").source, relative(source_kind::ENTRY_RELATIVE, "./skills/brand-design"));
        assert_eq!(entry_named(&market, "shorthand").source, relative(source_kind::REPO, "some-owner/some-repo"));
        assert_eq!(entry_named(&market, "git-url").source, relative(source_kind::REPO, "https://github.com/some-owner/some-repo.git"));
    }

    #[test]
    fn classify_source_covers_shape_boundaries() {
        // git URL（含 ://）
        assert_eq!(classify_source("https://example.com/o/r"), relative(source_kind::REPO, "https://example.com/o/r"));
        // owner/repo 缩写（恰好一段 /）
        assert_eq!(classify_source("owner/repo"), relative(source_kind::REPO, "owner/repo"));
        // 相对路径边界：./ 与 ../ 前缀、无斜杠目录名、多段路径、仓库内绝对路径
        assert_eq!(classify_source("./dir"), relative(source_kind::ENTRY_RELATIVE, "./dir"));
        assert_eq!(classify_source("../up"), relative(source_kind::ENTRY_RELATIVE, "../up"), "../ 只有一个 / 但不是缩写");
        assert_eq!(classify_source("plain-dir"), relative(source_kind::ENTRY_RELATIVE, "plain-dir"));
        assert_eq!(classify_source("a/b/c"), relative(source_kind::ENTRY_RELATIVE, "a/b/c"));
        assert_eq!(classify_source("/abs/in/repo"), relative(source_kind::ENTRY_RELATIVE, "/abs/in/repo"));
        // 空段（"/x"、"o/"、"​/r" 形态）不算缩写，归相对路径
        assert_eq!(classify_source("/x"), relative(source_kind::ENTRY_RELATIVE, "/x"));
        assert_eq!(classify_source("o/"), relative(source_kind::ENTRY_RELATIVE, "o/"));
    }

    #[test]
    fn entry_source_defaults_to_relative_path_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[(paths::CLAUDE_MARKET, r#"{"name":"demo","plugins":[{"name":"no-source","description":"无 source 条目"}]}"#)],
        );
        let market = probe_market(dir.path(), "demo", None).unwrap();
        let entry = entry_named(&market, "no-source");
        assert_eq!(entry.source, relative(source_kind::ENTRY_RELATIVE, "./no-source"), "缺省 source 按 Claude 兼容惯例回退 ./<name>");
    }

    // ---------- 容忍性 ----------

    #[test]
    fn probe_tolerates_unknown_fields_and_null_values() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[(
                paths::CLAUDE_MARKET,
                r#"{"name":"demo","featured":["x"],"pluginRoot":"./plugins","metadata":{"anything":1},
                    "plugins":[
                    {"name":"tolerant","author":{"name":"某人"},"homepage":"https://example.com",
                     "license":"MIT","category":"testing","tags":["a","b"],
                     "skills":null,"tags2":1,"hooks":null}
                ]}"#,
            )],
        );
        let market = probe_market(dir.path(), "demo", None).unwrap();
        assert_eq!(market.entries.len(), 1, "featured / pluginRoot / metadata 等未知字段被忽略");
        let entry = entry_named(&market, "tolerant");
        assert_eq!(entry.category.as_deref(), Some("testing"));
        assert_eq!(entry.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(entry.entry_skills, None, "null skills 视为未声明");
        assert!(!entry.component_annotations.has_hooks, "null hooks 不算声明");
    }

    #[test]
    fn probe_accepts_empty_plugins_array_and_missing_plugins_field() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_MARKET, r#"{"name":"empty","plugins":[]}"#)]);
        let market = probe_market(dir.path(), "empty", None).unwrap();
        assert_eq!(market.entries.len(), 0, "空市场合法（0 条目）");

        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_MARKET, r#"{"name":"no-field"}"#)]);
        let market = probe_market(dir.path(), "no-field", None).unwrap();
        assert_eq!(market.entries.len(), 0, "缺 plugins 字段同为空市场");

        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_MARKET, r#"{"name":"null-field","plugins":null}"#)]);
        let market = probe_market(dir.path(), "null-field", None).unwrap();
        assert_eq!(market.entries.len(), 0, "plugins: null 同为空市场");
    }

    #[test]
    fn probe_handles_multilingual_and_special_character_manifests() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[(
                paths::CLAUDE_MARKET,
                r#"{"name":"中文市场","plugins":[
                    {"name":"技能-✨","description":"包含 emoji 与中文的描述（括号）、引号\"与斜杠/","version":"1.0.0",
                     "source":"./目录名","tags":["中文标签","design ✨"]}
                ]}"#,
            )],
        );
        let market = probe_market(dir.path(), "chinese", None).unwrap();
        assert_eq!(market.id, "中文市场");
        let entry = &market.entries[0];
        assert_eq!(entry.name, "技能-✨");
        let description = entry.description.as_deref().unwrap_or_default();
        assert!(description.contains("（括号）"), "中文括号完整保留：{description}");
        assert!(description.contains('"'), "JSON 转义引号正确还原：{description}");
        assert!(description.ends_with("斜杠/"), "结尾斜杠完整保留：{description}");
        assert_eq!(entry.source, relative(source_kind::ENTRY_RELATIVE, "./目录名"));
        assert_eq!(entry.tags, vec!["中文标签".to_string(), "design ✨".to_string()]);
    }

    // ---------- 插件清单组件标注 ----------

    #[test]
    fn probe_ponytail_style_plugin_with_hooks_annotation() {
        let dir = tempfile::tempdir().unwrap();
        // ponytail 形态：单插件清单带 hooks 字段（字符串路径形态），无市场清单、无 skills 之外的干扰
        write_tree(
            dir.path(),
            &[
                (
                    paths::CLAUDE_PLUGIN,
                    r#"{"name":"ponytail","version":"5.3.2","description":"单 prompt 工作流","hooks":"./hooks/hooks.json"}"#,
                ),
                ("skills/ponytail/SKILL.md", "---\nname: ponytail\n---\n正文"),
            ],
        );
        let market = probe_market(dir.path(), "ponytail", None).unwrap();
        assert_eq!(market.manifest_kind, manifest_kind::SINGLE_PLUGIN);
        let entry = &market.entries[0];
        assert_eq!(entry.name, "ponytail");
        assert_eq!(entry.version.as_deref(), Some("5.3.2"));
        assert!(entry.component_annotations.has_hooks, "hooks 字段存在即标注（不参与加载）");
        assert!(!entry.component_annotations.has_agents);
        assert!(!entry.component_annotations.has_mcp);
    }

    #[test]
    fn plugin_manifest_annotations_cover_agents_mcp_field_and_dot_mcp_json() {
        let dir = tempfile::tempdir().unwrap();
        // agents + 内联 mcpServers
        write_tree(
            dir.path(),
            &[(paths::CRAFTWORK_PLUGIN, r#"{"name":"annotated","agents":{"researcher":{}},"mcpServers":{"fs":{"command":"x"}}}"#)],
        );
        let plugin = probe_plugin_manifest(dir.path()).unwrap().unwrap();
        assert_eq!(plugin.name, "annotated");
        assert!(plugin.has_agents, "agents 声明被解析保留（不加载）");
        assert!(plugin.has_mcp, "内联 mcpServers 被解析保留（不加载）");
        assert!(!plugin.has_hooks);

        // 旁置 .mcp.json 文件（无内联 mcpServers）同样触发 has_mcp
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[(paths::CRAFTWORK_PLUGIN, r#"{"name":"sidecar-mcp"}"#), (paths::DOT_MCP_JSON, r#"{"mcpServers":{}}"#)],
        );
        let plugin = probe_plugin_manifest(dir.path()).unwrap().unwrap();
        assert!(plugin.has_mcp, "插件根 .mcp.json 存在即标注");
        assert!(!plugin.has_hooks);
        assert!(!plugin.has_agents);
    }

    // ---------- 错误路径 ----------

    #[test]
    fn probe_bad_marketplace_json_reports_path_and_reason() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_MARKET, "{ 不是 JSON")]);
        let err = probe_market(dir.path(), "broken", None).unwrap_err();
        let expected = dir.path().join(paths::CLAUDE_MARKET);
        assert!(
            matches!(&err, ManifestError::Parse { path, .. } if path == &expected),
            "应报 Parse 且含清单路径，实际 {err:?}"
        );
        assert!(err.to_string().contains(&expected.to_string_lossy().to_string()), "错误消息含文件路径：{err}");
        assert!(err.to_string().contains("清单解析失败"), "错误消息可读：{err}");
    }

    #[test]
    fn probe_bad_plugin_json_propagates_from_single_plugin_form() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_PLUGIN, r#"{"name":"x""#)]);
        let err = probe_market(dir.path(), "broken", None).unwrap_err();
        assert!(matches!(err, ManifestError::Parse { .. }), "形态 3 清单损坏应显式报错，实际 {err:?}");
    }

    #[test]
    fn probe_missing_market_name_reports_manifest_path() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_MARKET, r#"{"plugins":[]}"#)]);
        let err = probe_market(dir.path(), "no-name", None).unwrap_err();
        assert!(matches!(&err, ManifestError::MissingName { .. }), "实际 {err:?}");
        assert!(err.to_string().contains(&dir.path().join(paths::CLAUDE_MARKET).to_string_lossy().to_string()));
    }

    #[test]
    fn probe_missing_entry_name_and_blank_name_report_manifest_path() {
        // 条目缺 name
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[(paths::CLAUDE_MARKET, r#"{"name":"demo","plugins":[{"description":"没有名字"}]}"#)],
        );
        let err = probe_market(dir.path(), "demo", None).unwrap_err();
        assert!(matches!(&err, ManifestError::MissingName { .. }), "实际 {err:?}");
        assert!(err.to_string().contains("name"), "错误消息指明缺失字段：{err}");

        // 条目 name 为空白串同样拒绝
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CLAUDE_MARKET, r#"{"name":"demo","plugins":[{"name":"   "}]}"#)]);
        let err = probe_market(dir.path(), "demo", None).unwrap_err();
        assert!(matches!(err, ManifestError::MissingName { .. }), "实际 {err:?}");

        // 插件清单缺 name 同理
        let dir = tempfile::tempdir().unwrap();
        write_tree(dir.path(), &[(paths::CODEX_PLUGIN, r#"{"version":"1.0.0"}"#)]);
        let err = probe_market(dir.path(), "demo", None).unwrap_err();
        assert!(matches!(err, ManifestError::MissingName { .. }), "实际 {err:?}");
    }

    #[test]
    fn blank_version_and_source_treated_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        write_tree(
            dir.path(),
            &[(paths::CLAUDE_MARKET, r#"{"name":"demo","plugins":[{"name":"blank-fields","version":"  ","source":"  "}]}"#)],
        );
        let market = probe_market(dir.path(), "demo", None).unwrap();
        let entry = entry_named(&market, "blank-fields");
        assert_eq!(entry.version, None, "空白版本视为未填，不阻断版本回退链");
        assert_eq!(entry.source, relative(source_kind::ENTRY_RELATIVE, "./blank-fields"), "空白 source 视为缺省，走回退");
    }
}
