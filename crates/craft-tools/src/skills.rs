//! 技能（SKILL.md）发现、解析与调用（决策 D11）。
//!
//! 两层加载：run 启动时扫描技能根目录，仅把「名称 + 描述 + 来源」清单注入系统提示词
//! （`build_listing_section`）；模型调用 [`SkillInvoke`] 工具时才读取 SKILL.md 正文，
//! 以 tool result 回填。加载即重扫（无缓存），禁用/删除下一个 run 立即生效。
//!
//! 技能根目录按优先级扫描（先扫者胜，按 SKILL.md 绝对路径去重，同名先发现者生效）：
//! 用户级 `<数据目录>/skills` → `~/.agents/skills` → 工作区级 `<工作区>/.craftwork/skills`
//! → `<工作区>/.agents/skills` → 插件缓存根（技能商店已启用插件，按安装顺序）。`.agents`
//! 目录是对 Claude/agents 生态的兼容来源；受限工作区不扫工作区级技能（防投毒：清单不可见
//! 是主要防线，加载本身为 R0 只读）。手工技能先于插件技能占名。
//!
//! 插件技能（D12）：
//! - **限定名**：以 `插件名:技能名` 注入清单；裸名作别名（[`find_skill`] 同时接受两种
//!   形态，精确名优先、取优先级首个命中；与手工技能同名的裸名归手工技能）。
//! - **fail-closed**：限定名重复（两个启用插件同名，或同插件内技能同名）的全部条目
//!   互标冲突、双双拒绝解析，不做 last-write-wins。
//! - **不跟随符号链接**：插件缓存视为不可信内容，扫描用 symlink 元数据直判，防
//!   SKILL.md/目录 symlink 逃逸出缓存目录；手工技能根维持跟随（软链导入）。
//! - **commands 映射**：插件 `commands/*.md` 一层解析 frontmatter（name = 文件名）后
//!   作为插件技能暴露，origin 标注 [`SkillOrigin::Command`]。
//!
//! frontmatter 解析为自写扁平 YAML（仅顶层 `key: value`，支持 `>` / `|` 块标量折叠、
//! 成对引号剥离、剥 BOM），不引入 yaml 依赖。

use serde::Serialize;
use serde_json::{json, Value};

use craft_policy::PolicyRisk;
use craft_store::db::Db;
use craft_store::records::{get_setting, set_setting};

use crate::registry::{Tool, ToolCtx, ToolResult, ToolSpec};

/// 技能描述文件名（目录内唯一入口）。
pub const SKILL_FILE_NAME: &str = "SKILL.md";
/// 递归扫描深度上限（根为第 0 层）。
pub const MAX_SKILL_SCAN_DEPTH: usize = 8;
/// description 字符数上限，超限该技能拒绝加载（错误码 `skill_description_too_long`）。
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
/// 正文读取字节上限，超出截断并标注。
pub const MAX_SKILL_BYTES: usize = 100_000;
/// 可用技能清单字符预算，超限降级为仅名称列表（见 `build_listing_section`）。
pub const LISTING_BUDGET_CHARS: usize = 20_000;

/// settings 表键：禁用技能的 SKILL.md 绝对路径数组。总开关在 app 键 Settings.skillsEnabled。
pub const SKILLS_DISABLED_KEY: &str = "skills.disabled";

/// 扫描时跳过的目录名（大小写不敏感）：构建物与依赖目录。
const SCAN_EXCLUDED_DIRS: &[&str] = &[
    "node_modules", "dist", "build", "out", "target", "vendor", "coverage", "__pycache__", ".venv", "venv",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillScope {
    /// 用户级（数据目录 / HOME，跨会话可用）
    User,
    /// 工作区级（随仓库分发，受限工作区不扫描）
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    /// CraftWork 原生目录（`.craftwork/skills`）
    Craftwork,
    /// 外部生态兼容目录（`.agents/skills`）
    Agents,
    /// 技能商店插件缓存（不可信内容：扫描不跟随符号链接，spec 第 8 节）
    Marketplace,
}

/// 技能条目来源形态（插件域特有标注；spec 第 5 节 commands 映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillOrigin {
    /// SKILL.md 声明的技能
    Skill,
    /// 插件 `commands/*.md` 映射（仅插件域；name 取文件名）
    Command,
}

/// 单个技能的元数据：发现即解析 frontmatter，正文按需读取。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    /// frontmatter name；缺失时回退目录名。插件技能为限定名 `插件名:技能名`
    /// （诊断失败条目保留裸名，仅管理视角可见）
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub scope: SkillScope,
    pub source: SkillSource,
    /// 条目来源形态（SKILL.md / commands 映射）
    pub origin: SkillOrigin,
    /// SKILL.md 绝对路径（禁用状态与去重的主键）
    pub path: String,
    /// 技能目录绝对路径（辅助脚本/资源的基准目录）
    pub dir: String,
    /// 是否启用（settings 的 skills.disabled 过滤结果）
    pub enabled: bool,
    /// 诊断信息；非空表示不可加载（缺 frontmatter 字段、超长、同名冲突等）
    pub error: String,
}

/// 插件技能根（技能商店，D12）：已启用插件按安装顺序
/// 逐个注入扫描（由 [`load_plugin_skill_roots`] 从 skill_plugins 表读取）。
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSkillRoot {
    /// 插件名（清单 manifest name）：插件技能限定名 `插件名:技能名` 的前缀。
    /// 插件名自身约定不含冒号（含冒号的插件名会使限定名歧义，fail-closed 兜底）。
    pub name: String,
    /// 插件缓存当前版本目录绝对路径（其下 `skills/` 与 `commands/` 均纳入扫描）
    pub dir: std::path::PathBuf,
}

/// 技能根目录：扫描起点 + 归属标注。
#[derive(Debug, Clone)]
pub struct SkillRoot {
    /// 扫描起点：手工根为技能目录本身；插件根（source == Marketplace）为插件版本
    /// 目录——其下 `skills/` 与 `commands/` 由扫描分支分别展开
    pub dir: std::path::PathBuf,
    pub scope: SkillScope,
    pub source: SkillSource,
    /// 插件归属（限定名前缀与冲突标注）；空串 = 手工技能根
    pub plugin: String,
}

/// 技能根目录列表（按优先级降序）：用户级在前、工作区级在后、插件缓存根殿后
/// （按安装顺序）；受限工作区跳过工作区级。手工技能先于插件技能占名。
pub fn skill_roots(data_dir: &str, workspace: &str, restricted: bool, plugins: &[PluginSkillRoot]) -> Vec<SkillRoot> {
    skill_roots_in(&home_dir(), data_dir, workspace, restricted, plugins)
}

/// [`skill_roots`] 的 home 注入版本：测试用它隔离真实 `~/.agents/skills`（机器上
/// 可能装有任意技能，污染计数断言）。
pub fn skill_roots_in(
    home: &std::path::Path,
    data_dir: &str,
    workspace: &str,
    restricted: bool,
    plugins: &[PluginSkillRoot],
) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    if !data_dir.is_empty() {
        roots.push(SkillRoot {
            dir: std::path::Path::new(data_dir).join("skills"),
            scope: SkillScope::User,
            source: SkillSource::Craftwork,
            plugin: String::new(),
        });
    }
    if !home.as_os_str().is_empty() {
        roots.push(SkillRoot {
            dir: home.join(".agents").join("skills"),
            scope: SkillScope::User,
            source: SkillSource::Agents,
            plugin: String::new(),
        });
    }
    if !workspace.is_empty() && !restricted {
        let ws = std::path::Path::new(workspace);
        roots.push(SkillRoot {
            dir: ws.join(".craftwork").join("skills"),
            scope: SkillScope::Workspace,
            source: SkillSource::Craftwork,
            plugin: String::new(),
        });
        roots.push(SkillRoot {
            dir: ws.join(".agents").join("skills"),
            scope: SkillScope::Workspace,
            source: SkillSource::Agents,
            plugin: String::new(),
        });
    }
    // 插件缓存根殿后（spec 第 5 节优先级末位）：scope = 用户级（装在用户级缓存，
    // 与工作区无关）；传参顺序即安装顺序（先安装者占名优先）。停用插件不传即根消失。
    for p in plugins {
        roots.push(SkillRoot {
            dir: p.dir.clone(),
            scope: SkillScope::User,
            source: SkillSource::Marketplace,
            plugin: p.name.clone(),
        });
    }
    roots
}

/// 读取已启用插件的技能根（技能商店 spec 第 5 节）：按安装顺序返回当前版本缓存目录。
/// 停用插件不返回（其全部技能根退出扫描，下一轮对话/下一次工具调用生效）；查询失败按
/// 空列表兜底（技能扫描不阻塞会话）。
///
/// 落点说明：skill_plugins 表的读写封装约定在 craft-marketplace（spec 第 6 节），此处是
/// 扫描链路的只读内联查询（仅 name + install_path 两列）——src-tauri 尚未依赖
/// craft-marketplace（商店服务并行接线中，本票不得动 Cargo.lock），craft-agent /
/// craft-tools 也不宜为读两列引入该依赖；商店接线完成后如需统一封装再迁移。
pub fn load_plugin_skill_roots(db: &Db) -> Vec<PluginSkillRoot> {
    db.with(|conn| {
        let mut stmt = conn
            .prepare("SELECT name, install_path FROM skill_plugins WHERE enabled = 1 ORDER BY installed_at ASC, id ASC")
            .map_err(craft_store::db::other_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PluginSkillRoot {
                    name: row.get::<_, String>(0)?,
                    dir: std::path::PathBuf::from(row.get::<_, String>(1)?),
                })
            })
            .map_err(craft_store::db::other_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(craft_store::db::other_err)
    })
    .unwrap_or_default()
}

fn home_dir() -> std::path::PathBuf {
    // 与平台层同一份主目录回退链（HOME → USERPROFILE → HOMEDRIVE+HOMEPATH），
    // 不再各写一份——技能目录在 Windows 上也依赖这条链。
    craft_core::platform::home_dir().unwrap_or_default()
}

/// 读取技能设置：(总开关, 禁用路径列表)。总开关在 app 键 Settings.skillsEnabled
/// （缺省 true）；禁用列表在 skills.disabled 键。读取失败按默认值兜底。
pub fn load_skills_config(db: &Db) -> (bool, Vec<String>) {
    let enabled = db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| v.get("skillsEnabled").and_then(Value::as_bool))
        .unwrap_or(true);
    let disabled = db
        .with(|conn| get_setting(conn, SKILLS_DISABLED_KEY))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    (enabled, disabled)
}

/// 写入禁用路径列表（UPSERT；空列表删除键，保持 settings 表干净）。
pub fn save_disabled_paths(db: &Db, paths: &[String]) -> Result<(), String> {
    let value = if paths.is_empty() {
        Value::Array(vec![])
    } else {
        json!(paths)
    };
    db.with(|conn| {
        if paths.is_empty() {
            craft_store::records::delete_setting(conn, SKILLS_DISABLED_KEY)
        } else {
            set_setting(conn, SKILLS_DISABLED_KEY, &value)
        }
    })
    .map_err(|e| e.to_string())
}

/// run 启动 / 工具加载共用的扫描入口：总开关开启时返回全部可用技能（已过滤禁用与
/// 诊断失败项）。插件根从 db 的 skill_plugins 表即时读取——停用/卸载插件即根消失。
pub fn scan_available(db: &Db, data_dir: &str, workspace: &str, restricted: bool) -> Vec<SkillMeta> {
    scan_available_in(db, &home_dir(), data_dir, workspace, restricted)
}

/// [`scan_available`] 的 home 注入版本（测试隔离真实 `~/.agents/skills`）。
pub fn scan_available_in(db: &Db, home: &std::path::Path, data_dir: &str, workspace: &str, restricted: bool) -> Vec<SkillMeta> {
    let (enabled, disabled) = load_skills_config(db);
    if !enabled {
        return Vec::new();
    }
    let plugins = load_plugin_skill_roots(db);
    let roots = skill_roots_in(home, data_dir, workspace, restricted, &plugins);
    discover_skills(&roots, &disabled)
        .into_iter()
        .filter(|s| s.enabled && s.error.is_empty())
        .collect()
}

/// 扫描全部技能根：返回全量列表（含禁用标记与诊断信息），供管理页与注入层共用。
///
/// 同名技能只有先发现（优先级高）的第一个生效；后续同名标 error「同名冲突」。
/// 仅有效技能（error 为空）占用名字，前者无效时后者可接管。
/// 插件技能先做限定名 fail-closed（[`marketplace_conflicts_fail_closed`]）再做全局占名。
pub fn discover_skills(roots: &[SkillRoot], disabled: &[String]) -> Vec<SkillMeta> {
    let mut found: Vec<(PathRef, EntryKind, SkillScope, SkillSource, String)> = Vec::new();
    let mut seen_files: Vec<std::path::PathBuf> = Vec::new(); // canonical 去重（symlink 导入场景）
    for root in roots {
        let mut entries = Vec::new();
        collect_root_entries(root, &mut entries);
        for (file, kind) in entries {
            let canonical = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            if seen_files.contains(&canonical) {
                continue;
            }
            seen_files.push(canonical);
            found.push((file, kind, root.scope, root.source, root.plugin.clone()));
        }
    }

    let mut metas: Vec<SkillMeta> = Vec::new();
    for (path, kind, scope, source, plugin) in found {
        let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        // 条目名：SKILL.md 回退目录名；command 映射 name = 文件名（spec 第 5 节）
        let raw_name = match kind {
            EntryKind::Command => path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            EntryKind::Skill => dir_name.clone(),
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                metas.push(invalid_meta(&raw_name, &path, scope, source, kind, disabled, format!("SKILL.md 读取失败：{e}")));
                continue;
            }
        };
        let parsed = parse_frontmatter(&content);
        let mut error = String::new();
        let mut name;
        let description;
        let when_to_use;
        match &parsed.entries {
            Some(entries) => {
                let fm_name = entries.iter().find(|(k, _)| k == "name").map(|(_, v)| v.clone()).unwrap_or_default();
                description = entries.iter().find(|(k, _)| k == "description").map(|(_, v)| v.clone()).unwrap_or_default();
                when_to_use = entries.iter().find(|(k, _)| k == "when_to_use").map(|(_, v)| v.clone()).unwrap_or_default();
                // command 映射无视 frontmatter name（name = 文件名，唯一标识即路径）
                name = match kind {
                    EntryKind::Command => raw_name.clone(),
                    EntryKind::Skill => fm_name,
                };
                if name.trim().is_empty() {
                    error = "frontmatter 缺少 name".into();
                } else if description.trim().is_empty() {
                    error = "frontmatter 缺少 description".into();
                } else if description.chars().count() > MAX_DESCRIPTION_CHARS {
                    error = format!("description 超过 {MAX_DESCRIPTION_CHARS} 字符");
                }
            }
            // 无 frontmatter：宽容处理（SKILL.md name 回退目录名；command 即文件名）
            None => {
                name = raw_name.clone();
                description = String::new();
                when_to_use = String::new();
            }
        }
        // 插件技能以限定名 `插件名:技能名` 注入清单（裸名解析见 find_skill 别名匹配）；
        // 手工技能保持裸名。诊断失败条目保留裸名（仅管理视角可见，不参与解析）。
        if error.is_empty() && source == SkillSource::Marketplace && !plugin.is_empty() {
            name = format!("{plugin}:{name}");
        }
        let mut meta = SkillMeta {
            name,
            description,
            when_to_use,
            scope,
            source,
            origin: match kind {
                EntryKind::Skill => SkillOrigin::Skill,
                EntryKind::Command => SkillOrigin::Command,
            },
            path: path.to_string_lossy().to_string(),
            dir: dir.to_string_lossy().to_string(),
            enabled: !disabled.iter().any(|p| p == &path.to_string_lossy()),
            error,
        };
        if !meta.error.is_empty() && meta.name.is_empty() {
            meta.name = raw_name;
        }
        metas.push(meta);
    }

    // 插件限定名同名 fail-closed 先行：被停用条目不占名，随后全局占名只处理剩余有效技能
    marketplace_conflicts_fail_closed(&mut metas);
    // 同名冲突：只有有效技能占用名字；被占用者标注诊断（保持发现顺序，管理页可见）
    let mut taken: Vec<String> = Vec::new();
    apply_name_conflicts(&mut metas, &mut taken);
    metas
}

/// 单根扫描产物条目类别：SKILL.md 技能或 command 映射（name 推导规则不同）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Skill,
    Command,
}

/// 收集单个技能根下的全部条目：手工根递归跟随符号链接（软链导入技能，行为不变）；
/// 插件根视为不可信内容——以 symlink 元数据直判，不跟随任何符号链接（防 SKILL.md /
/// 目录 symlink 逃逸出缓存目录，spec 第 8 节），且额外一层映射其下 `commands/*.md`。
fn collect_root_entries(root: &SkillRoot, out: &mut Vec<(std::path::PathBuf, EntryKind)>) {
    let marketplace = root.source == SkillSource::Marketplace;
    let skills_dir = if marketplace { root.dir.join("skills") } else { root.dir.clone() };
    walk_dir(&skills_dir, 0, !marketplace, out);
    if !marketplace {
        return;
    }
    // commands/*.md 一层映射（子目录不递归，与 Claude 生态 commands 惯例一致）；
    // 与 skills 同纪律：不跟随符号链接（symlink 条目非普通文件，天然跳过）。
    let Ok(entries) = std::fs::read_dir(root.dir.join("commands")) else {
        return; // 无 commands 目录属常态
    };
    let mut items: Vec<_> = entries.filter_map(Result::ok).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if meta.is_file() && name.to_lowercase().ends_with(".md") {
            out.push((path, EntryKind::Command));
        }
    }
}

/// 插件限定名同名 fail-closed（spec 第 5 节）：限定名重复（两个启用插件的 manifest
/// name 相同，或同插件内技能同名）的全部有效条目互标冲突、双双拒绝解析——限定名
/// 重复时模型无法区分加载目标，不做 last-write-wins。错误信息列出全部冲突方插件名。
fn marketplace_conflicts_fail_closed(metas: &mut [SkillMeta]) {
    // 按发现顺序收集限定名分组：(限定名, 条目索引们, 冲突方插件名们)
    let mut groups: Vec<(String, Vec<usize>, Vec<String>)> = Vec::new();
    for (idx, meta) in metas.iter().enumerate() {
        if meta.source != SkillSource::Marketplace || !meta.error.is_empty() {
            continue;
        }
        let plugin = meta.name.split_once(':').map(|(p, _)| p.to_string()).unwrap_or_default();
        match groups.iter_mut().find(|(n, _, _)| *n == meta.name) {
            Some((_, idxs, plugins)) => {
                idxs.push(idx);
                plugins.push(plugin);
            }
            None => groups.push((meta.name.clone(), vec![idx], vec![plugin])),
        }
    }
    for (_, idxs, plugins) in groups {
        if idxs.len() < 2 {
            continue;
        }
        let parties = plugins.join("、");
        for idx in idxs {
            let name = metas[idx].name.clone();
            metas[idx].error = format!("插件技能「{name}」在插件（{parties}）间同名冲突，已全部停用（fail-closed）");
        }
    }
}

/// 按请求名解析技能（skill 工具与斜杠检测共用）：限定名与裸名别名都接受——精确名
/// （手工裸名或插件限定名）优先，取优先级首个命中；无精确命中时按裸名别名匹配插件
/// 技能（与手工技能同名的裸名归手工技能，精确名优先天然成立；同名插件技能已被
/// fail-closed 停用，不会误命中）。
pub fn find_skill<'a>(skills: &'a [SkillMeta], request: &str) -> Option<&'a SkillMeta> {
    let usable = |s: &SkillMeta| s.enabled && s.error.is_empty();
    skills
        .iter()
        .find(|s| usable(s) && s.name == request)
        .or_else(|| {
            skills.iter().find(|s| {
                usable(s)
                    && s.source == SkillSource::Marketplace
                    && s.name.split_once(':').is_some_and(|(_, bare)| bare == request)
            })
        })
}

/// 同名冲突判定：只有有效技能（error 为空）占用名字，撞名者标注诊断并保持发现顺序。
///
/// `taken` 允许带入已占用的名字：管理视角按工作区分别扫描时，用户级先占名，
/// 每个工作区用自己的 `taken` 副本继续判定——不同工作区之间不互相冲突。
pub fn apply_name_conflicts(metas: &mut [SkillMeta], taken: &mut Vec<String>) {
    for meta in metas.iter_mut() {
        if meta.error.is_empty() {
            if taken.contains(&meta.name) {
                meta.error = format!("与技能「{}」同名，本技能不生效", meta.name);
            } else {
                taken.push(meta.name.clone());
            }
        }
    }
}

/// 管理视角的全量扫描：用户级扫一次（含插件缓存根）+ 每个工作区只扫其工作区级两根。
///
/// 返回 `(workspaceId, meta)`，用户级条目的 workspaceId 为空串。同名判定分两层：
/// 用户级先占名（跨会话可用者优先），各工作区各自用自己的已占名副本继续判定——
/// 不同工作区的同名技能互不冲突（它们在各自的会话上下文里分别生效）。
pub fn discover_all_scopes(
    data_dir: &str,
    workspaces: &[(String, String)],
    disabled: &[String],
    plugins: &[PluginSkillRoot],
) -> Vec<(String, SkillMeta)> {
    discover_all_scopes_in(&home_dir(), data_dir, workspaces, disabled, plugins)
}

/// [`discover_all_scopes`] 的 home 注入版本（测试隔离真实 `~/.agents/skills`）。
pub fn discover_all_scopes_in(
    home: &std::path::Path,
    data_dir: &str,
    workspaces: &[(String, String)],
    disabled: &[String],
    plugins: &[PluginSkillRoot],
) -> Vec<(String, SkillMeta)> {
    let mut out: Vec<(String, SkillMeta)> = Vec::new();
    let user_roots: Vec<SkillRoot> = skill_roots_in(home, data_dir, "", false, plugins);
    let mut user_metas = discover_skills(&user_roots, disabled);
    let mut taken: Vec<String> = Vec::new();
    apply_name_conflicts(&mut user_metas, &mut taken);
    out.extend(user_metas.into_iter().map(|m| (String::new(), m)));

    for (workspace_id, main_dir) in workspaces {
        let ws_roots: Vec<SkillRoot> = skill_roots_in(home, data_dir, main_dir, false, plugins)
            .into_iter()
            .filter(|r| r.scope == SkillScope::Workspace)
            .collect();
        if ws_roots.is_empty() {
            continue;
        }
        let mut ws_metas = discover_skills(&ws_roots, disabled);
        let mut ws_taken = taken.clone();
        apply_name_conflicts(&mut ws_metas, &mut ws_taken);
        out.extend(ws_metas.into_iter().map(|m| (workspace_id.clone(), m)));
    }
    out
}

type PathRef = std::path::PathBuf;

fn invalid_meta(
    raw_name: &str,
    path: &std::path::Path,
    scope: SkillScope,
    source: SkillSource,
    kind: EntryKind,
    disabled: &[String],
    error: String,
) -> SkillMeta {
    let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    SkillMeta {
        name: raw_name.to_string(),
        description: String::new(),
        when_to_use: String::new(),
        scope,
        source,
        origin: match kind {
            EntryKind::Skill => SkillOrigin::Skill,
            EntryKind::Command => SkillOrigin::Command,
        },
        path: path.to_string_lossy().to_string(),
        dir: dir.to_string_lossy().to_string(),
        enabled: !disabled.iter().any(|p| p == &path.to_string_lossy()),
        error,
    }
}

/// 递归收集目录树内全部 SKILL.md（根目录自身与子目录均可；深度上限 [`MAX_SKILL_SCAN_DEPTH`]，
/// 跳过排除目录与点目录）。`follow_symlinks` 决定目录项判定是否跟随符号链接：手工根
/// 跟随（支持软链导入技能），插件根不跟随（信任边界，[`collect_root_entries`] 分支控制），
/// 循环由深度限制兜底。
fn walk_dir(dir: &std::path::Path, depth: usize, follow_symlinks: bool, out: &mut Vec<(std::path::PathBuf, EntryKind)>) {
    if depth > MAX_SKILL_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // 根不存在（未创建）属正常；EACCES 等也静默跳过（技能扫描不应阻塞会话）
    };
    let mut items: Vec<_> = entries.filter_map(Result::ok).collect();
    items.sort_by_key(|e| e.file_name()); // 稳定顺序，同名冲突判定可复现
    for entry in items {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        // 跟随模式用 metadata（解析链接目标）；不跟随模式用 symlink_metadata（链接自身
        // 既非 file 也非 dir，符号链接条目天然跳过——插件域逃逸防线）
        let meta = if follow_symlinks { std::fs::metadata(&path) } else { std::fs::symlink_metadata(&path) };
        let Ok(meta) = meta else { continue };
        if meta.is_file() && name == SKILL_FILE_NAME {
            out.push((path, EntryKind::Skill));
        } else if meta.is_dir() && !name.starts_with('.') && !SCAN_EXCLUDED_DIRS.contains(&name.to_lowercase().as_str()) {
            walk_dir(&path, depth + 1, follow_symlinks, out);
        }
    }
}

/// frontmatter 解析结果：键值对（可能为 None = 无 frontmatter）与正文。
pub struct ParsedFrontmatter {
    pub entries: Option<Vec<(String, String)>>,
    pub body: String,
}

/// 解析 SKILL.md：剥 BOM，识别首行 `---` 包裹的 frontmatter，正文为其后全部内容。
/// 未闭合等异常一律按「无 frontmatter」宽容处理（正文保留原文）。
pub fn parse_frontmatter(content: &str) -> ParsedFrontmatter {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.lines();
    if lines.next().map(|l| l.trim_end()) != Some("---") {
        return ParsedFrontmatter { entries: None, body: content.to_string() };
    }
    let mut header: Vec<&str> = Vec::new();
    let mut rest_start: Option<usize> = None;
    for (i, line) in content.lines().enumerate().skip(1) {
        if line.trim_end() == "---" {
            rest_start = Some(i + 1);
            break;
        }
        header.push(line);
    }
    let Some(start) = rest_start else {
        return ParsedFrontmatter { entries: None, body: content.to_string() };
    };
    let body = content.lines().skip(start).collect::<Vec<_>>().join("\n");
    ParsedFrontmatter { entries: Some(parse_flat_yaml(&header)), body }
}

/// 扁平 YAML 解析：仅顶层 `key: value`，支持 `>` / `|` 块标量与成对引号剥离。
/// 不支持的语法（嵌套、列表）按空值忽略——技能 frontmatter 只用扁平字段。
pub fn parse_flat_yaml(lines: &[&str]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let Some((key, value)) = split_key_value(trimmed) else {
            i += 1;
            continue;
        };
        let value = value.trim();
        // 块标量：`>` 折叠（单换行折叠为空格、空行为段落换行）；`|` 保留换行
        let style = value.trim_end_matches(['-', '+']);
        if value.len() == style.len() && (style == ">" || style == "|") {
            let key_indent = line.len() - line.trim_start().len();
            let mut block: Vec<&str> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let l = lines[j];
                if l.trim().is_empty() {
                    block.push("");
                    j += 1;
                    continue;
                }
                let indent = l.len() - l.trim_start().len();
                if indent <= key_indent {
                    break;
                }
                block.push(l.trim());
                j += 1;
            }
            while block.last() == Some(&"") {
                block.pop();
            }
            let joined = if style == ">" { fold_block(&block) } else { block.join("\n") };
            out.push((key.to_string(), joined));
            i = j;
        } else {
            out.push((key.to_string(), unquote(value)));
            i += 1;
        }
    }
    out
}

/// 折叠块标量：连续非空行以空格连接，空行折叠为换行（YAML folded 语义）。
fn fold_block(block: &[&str]) -> String {
    let mut out = String::new();
    let mut pending_break = false;
    for line in block {
        if line.is_empty() {
            if !out.is_empty() {
                pending_break = true;
            }
        } else {
            if pending_break {
                out.push('\n');
                pending_break = false;
            } else if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(line);
        }
    }
    out
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(':')?;
    let key = line[..idx].trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return None;
    }
    Some((key, line[idx + 1..].trim()))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// 读取技能正文：剥 frontmatter、替换技能目录变量、超长截断。
pub fn load_skill_body(path: &str, dir: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("技能文件读取失败：{e}"))?;
    let truncated = bytes.len() > MAX_SKILL_BYTES;
    let mut raw: &[u8] = &bytes[..];
    if truncated {
        // 按 UTF-8 字符边界回退截断，避免切坏多字节字符
        let mut end = MAX_SKILL_BYTES;
        while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
        raw = &bytes[..end];
    }
    let content = String::from_utf8_lossy(raw);
    let parsed = parse_frontmatter(&content);
    let mut body = parsed.body;
    if truncated {
        body.push_str("\n\n[技能内容已截断]");
    }
    for var in ["${CLAUDE_SKILL_DIR}", "${CRAFTWORK_SKILL_DIR}"] {
        body = body.replace(var, dir);
    }
    Ok(body)
}

/// 渲染系统提示词的「可用技能」段：空清单返回 None（整段不注入）；
/// 超出 [`LISTING_BUDGET_CHARS`] 降级为仅名称列表（保清单整体可用优先于保描述完整）。
pub fn build_listing_section(skills: &[SkillMeta]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let scope_label = |s: &SkillMeta| if s.scope == SkillScope::User { "用户级" } else { "工作区" };
    let full: Vec<String> = skills
        .iter()
        .map(|s| {
            let mut line = format!("- {}：{}", s.name, s.description);
            if !s.when_to_use.is_empty() {
                line.push_str(&format!("（适用：{}）", s.when_to_use));
            }
            line.push_str(&format!("（{}）", scope_label(s)));
            line
        })
        .collect();
    let header = "以下技能可用 skill 工具加载；需要时调用 skill 工具（name 填技能名）读取完整内容后再按内容执行；不要猜测清单之外的技能名。";
    let section = format!("# 可用技能\n{header}\n{}", full.join("\n"));
    if section.chars().count() <= LISTING_BUDGET_CHARS {
        return Some(section);
    }
    let names = skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("、");
    Some(format!("# 可用技能\n可用技能（描述过长已省略）：{names}。需要时用 skill 工具加载后再执行。"))
}

/// 解析斜杠技能调用文本：`/名称 [参数]`。名称需与技能清单精确匹配（由调用方完成），
/// 本函数只做文本形态解析：返回 (技能名, 参数原文)；非斜杠形态返回 None。
pub fn parse_slash_invocation(text: &str) -> Option<(String, String)> {
    let text = text.trim_start();
    let rest = text.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let (name, args) = match rest.find(char::is_whitespace) {
        Some(idx) => (&rest[..idx], rest[idx..].trim()),
        None => (rest, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), args.to_string()))
}

/// 斜杠显式调用的引导 prompt：落库与前端显示保留原文（`/名称 参数`），
/// 模型历史使用本函数产物——模型经 skill 工具拉取技能正文后再执行请求
/// （斜杠显式调用统一改写成这段引导 prompt，落库仍保留原文）。
pub fn manual_skill_prompt(name: &str, args: &str) -> String {
    let args_block = if args.trim().is_empty() { "（无附加参数）".to_string() } else { args.trim().to_string() };
    format!(
        "用户通过斜杠命令显式调用技能「{name}」。\n请先调用 skill 工具（name=\"{name}\"）加载技能内容，再严格按技能内容执行以下请求：\n\n{args_block}"
    )
}

/// 从斜杠调用原文构造引导 prompt：原文形态非法时参数按空处理（name 由调用方校验过）。
pub fn manual_skill_prompt_from_text(name: &str, raw_text: &str) -> String {
    let args = parse_slash_invocation(raw_text).map(|(_, args)| args).unwrap_or_default();
    manual_skill_prompt(name, &args)
}

/// skill 内置工具（R0，Work 模式可用）：按名称从技能清单加载 SKILL.md 正文。
///
/// 不声明 path_arg——按名字加载而非按路径，天然规避任意文件读取；技能目录由扫描
/// 结果给出，不信任模型传入的路径。加载即重扫：禁用/删除立即生效。
pub struct SkillInvoke;

impl SkillInvoke {
    pub const NAME: &'static str = "skill";
}

impl Tool for SkillInvoke {
    fn spec(&self) -> ToolSpec {
        SkillInvoke::spec()
    }

    /// 技能正文是用户装的指令包，`load_skill_body` 已按 [`MAX_SKILL_BYTES`]（100 KB）
    /// 裁过并追加了自己的截断标记。出口若仍按 32 KiB 砍，不仅丢掉三分之二正文，
    /// 还会把那份标记切掉——技能「能加载」和「能读全」必须是同一件事（D29）。
    fn output_ceiling(&self) -> usize {
        MAX_SKILL_BYTES + 4 * 1024
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let name = ctx.str_arg("name").trim().to_string();
        if name.is_empty() {
            return ToolResult::err("name 不能为空");
        }
        let (enabled, disabled) = load_skills_config(&ctx.env.db);
        if !enabled {
            return ToolResult::err_code("技能功能已在设置中关闭", "skill_disabled");
        }
        // 工具侧不判受限：清单注入层已对受限工作区隐藏工作区级技能，加载本身为 R0 只读，
        // 安全边界在技能内容驱动的后续工具调用（各自走审批）。
        // 插件根即时重读（加载即重扫语义）：停用/卸载插件的下一次调用即不命中。
        let plugins = load_plugin_skill_roots(&ctx.env.db);
        let roots = skill_roots(&ctx.env.data_dir, &ctx.workspace, false, &plugins);
        let available = discover_skills(&roots, &disabled);
        // 限定名与裸名别名都接受（清单注入的是限定名；裸名在无同名手工技能时同样可用）
        let Some(skill) = find_skill(&available, &name) else {
            return ToolResult::err_code(format!("技能不存在或不可用：{name}。请从系统提示词的可用技能清单中选择确切名称。"), "skill_not_found");
        };
        match load_skill_body(&skill.path, &skill.dir) {
            Ok(body) => ToolResult::ok_with_meta(
                format!(
                    "<skill_content name=\"{}\">\n{}\n\n技能基础目录：{}\n正文中的相对路径均相对于该目录，辅助脚本经 shell 工具执行时仍走审批。\n</skill_content>",
                    skill.name, body, skill.dir
                ),
                json!({ "name": skill.name, "path": skill.path }),
            ),
            Err(e) => ToolResult::err_code(e, "skill_read_failed"),
        }
    }
}

impl SkillInvoke {
    pub fn spec() -> ToolSpec {
        ToolSpec {
            name: SkillInvoke::NAME.into(),
            description: "加载指定技能的完整内容（SKILL.md 正文与辅助资源说明）。技能是用户安装的指令包；名称必须来自系统提示词的可用技能清单，不要猜测。".into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "技能名（来自可用技能清单的确切名称）" },
                    "args": { "type": "string", "description": "可选：调用技能时携带的参数原文" },
                },
                "required": ["name"],
            }),
            risk: PolicyRisk::R0,
            path_arg: None,
            command_arg: None,
            mutates: false,
            work: true,
            title: "加载技能".into(),
            summary_arg: Some("name".into()),
            summary_text: Some("读取技能".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolEnv;
    use std::sync::Arc;

    /// 建临时技能目录并写入 SKILL.md，返回根目录路径。
    fn make_skill(root: &std::path::Path, name: &str, frontmatter: Option<&str>, body: &str) -> std::path::PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut content = String::new();
        if let Some(fm) = frontmatter {
            content.push_str("---\n");
            content.push_str(fm);
            content.push_str("\n---\n");
        }
        content.push_str(body);
        std::fs::write(dir.join(SKILL_FILE_NAME), content).unwrap();
        dir
    }

    fn root_of(dir: &std::path::Path) -> Vec<SkillRoot> {
        vec![SkillRoot { dir: dir.to_path_buf(), scope: SkillScope::User, source: SkillSource::Craftwork, plugin: String::new() }]
    }

    /// 插件技能根 helper（测试用）：插件名 + 版本目录。
    fn plugin_root_of(name: &str, dir: &std::path::Path) -> PluginSkillRoot {
        PluginSkillRoot { name: name.to_string(), dir: dir.to_path_buf() }
    }

    /// 插件测试的隔离根列表：home 注入不存在的路径，隔离真实 `~/.agents/skills`
    /// （机器上可能装有任意技能，污染计数与裸名唯一性断言）。
    fn isolated_roots(plugins: &[PluginSkillRoot]) -> Vec<SkillRoot> {
        skill_roots_in(std::path::Path::new("/nonexistent-cw-skills-home"), "", "", false, plugins)
    }

    /// 造市场 + 插件行（测试自用）：craft-tools 无 craft-marketplace 依赖（本票不得动
    /// Cargo.lock），直接 SQL 造行；列与 craft-store 迁移 V14 的 skill_plugins 表对应。
    fn insert_plugin_row(db: &Db, id: &str, name: &str, install_path: &std::path::Path, enabled: bool, installed_at: i64) {
        let conn = db.lock_conn();
        conn.execute(
            "INSERT OR IGNORE INTO marketplaces (id, source, repo_url, created_at) VALUES ('mk', 'mk', 'https://example.com/mk', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            &format!(
                "INSERT INTO skill_plugins (id, marketplace_id, name, current_version, versions, install_path, source_kind, source_ref, enabled, installed_at, updated_at) \
                 VALUES ('{id}', 'mk', '{name}', '1.0.0', '[\"1.0.0\"]', '{}', 'entry-relative', './x', {}, {installed_at}, {installed_at})",
                install_path.to_string_lossy().replace('\'', "''"),
                enabled as i64
            ),
            [],
        )
        .unwrap();
    }

    // ---------- frontmatter 解析 ----------

    #[test]
    fn frontmatter_plain_keys_quotes_and_comments() {
        let parsed = parse_frontmatter("---\n# 注释\nname: demo\ndescription: \"带引号 描述\"\nwhen_to_use: '单引号'\n---\n正文");
        let entries = parsed.entries.unwrap();
        assert_eq!(entries[0], ("name".into(), "demo".into()));
        assert_eq!(entries[1], ("description".into(), "带引号 描述".into()));
        assert_eq!(entries[2], ("when_to_use".into(), "单引号".into()));
        assert_eq!(parsed.body, "正文");
    }

    #[test]
    fn frontmatter_block_scalars_folded_and_literal() {
        let fm = "---\ndescription: >\n  第一段续行一\n  续行二\n\n  第二段\nwhen_to_use: |\n  行一\n  行二\n---\n正文";
        let parsed = parse_frontmatter(fm);
        let entries = parsed.entries.unwrap();
        assert_eq!(entries[0], ("description".into(), "第一段续行一 续行二\n第二段".into()));
        assert_eq!(entries[1], ("when_to_use".into(), "行一\n行二".into()));
    }

    #[test]
    fn frontmatter_missing_or_unclosed_falls_back_to_body() {
        let no_fm = parse_frontmatter("没有 frontmatter 的正文");
        assert!(no_fm.entries.is_none());
        assert_eq!(no_fm.body, "没有 frontmatter 的正文");
        let unclosed = parse_frontmatter("---\nname: demo\n没有闭合");
        assert!(unclosed.entries.is_none());
        // BOM 剥除
        let bom = parse_frontmatter("\u{feff}---\nname: demo\n---\n正文");
        assert_eq!(bom.entries.unwrap()[0], ("name".into(), "demo".into()));
    }

    #[test]
    fn slash_invocation_parsing() {
        assert_eq!(parse_slash_invocation("/demo"), Some(("demo".into(), "".into())));
        assert_eq!(parse_slash_invocation("/demo 帮我做 X"), Some(("demo".into(), "帮我做 X".into())));
        assert_eq!(parse_slash_invocation("  /demo\n第二行参数"), Some(("demo".into(), "第二行参数".into())));
        assert_eq!(parse_slash_invocation("普通消息 /demo"), None);
        assert_eq!(parse_slash_invocation("/"), None);
    }

    // ---------- 发现与扫描 ----------

    #[test]
    fn discover_priority_and_duplicate_name_conflict() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let user = tmp.join("user");
        let ws = tmp.join("ws");
        make_skill(&user, "demo", Some("name: demo\ndescription: 用户级"), "用户正文");
        make_skill(&ws, "demo", Some("name: demo\ndescription: 工作区级"), "工作区正文");
        make_skill(&ws, "other", Some("name: other\ndescription: 其他"), "正文");

        let roots = vec![
            SkillRoot { dir: user.clone(), scope: SkillScope::User, source: SkillSource::Craftwork, plugin: String::new() },
            SkillRoot { dir: ws.clone(), scope: SkillScope::Workspace, source: SkillSource::Craftwork, plugin: String::new() },
        ];
        let all = discover_skills(&roots, &[]);
        let demo: Vec<_> = all.iter().filter(|s| s.name == "demo").collect();
        assert_eq!(demo.len(), 2);
        // 先发现者（用户级）无诊断生效，后者标冲突
        assert_eq!(demo[0].scope, SkillScope::User);
        assert!(demo[0].error.is_empty());
        assert!(demo[1].error.contains("同名"));
        assert!(all.iter().any(|s| s.name == "other" && s.error.is_empty()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 管理视角：两个工作区各有同名技能时互不冲突；用户级同名者优先占名。
    #[test]
    fn discover_all_scopes_groups_by_workspace_without_cross_conflict() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-all-scopes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let data_dir = tmp.join("data");
        let home = tmp.join("home");
        let ws_a = tmp.join("ws-a");
        let ws_b = tmp.join("ws-b");
        make_skill(&data_dir.join("skills"), "shared", Some("name: shared\ndescription: 用户级"), "x");
        make_skill(&ws_a.join(".craftwork").join("skills"), "demo", Some("name: demo\ndescription: A 的"), "x");
        make_skill(&ws_b.join(".craftwork").join("skills"), "demo", Some("name: demo\ndescription: B 的"), "x");

        let workspaces = vec![
            ("ws-a".to_string(), ws_a.to_string_lossy().to_string()),
            ("ws-b".to_string(), ws_b.to_string_lossy().to_string()),
        ];
        let all = discover_all_scopes_in(&home, &data_dir.to_string_lossy(), &workspaces, &[], &[]);

        let user: Vec<_> = all.iter().filter(|(id, _)| id.is_empty()).collect();
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].1.name, "shared");
        assert_eq!(user[0].1.scope, SkillScope::User);

        let demo: Vec<_> = all.iter().filter(|(_, s)| s.name == "demo").collect();
        assert_eq!(demo.len(), 2, "两个工作区的同名技能都应出现");
        assert!(demo.iter().all(|(_, s)| s.error.is_empty()), "不同工作区之间不判同名冲突");
        assert_eq!(demo[0].0, "ws-a");
        assert_eq!(demo[1].0, "ws-b");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 用户级技能先占名：只有与该用户级技能同名的工作区级条目被标冲突。
    #[test]
    fn discover_all_scopes_marks_user_conflict_only_in_colliding_workspace() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-scope-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let data_dir = tmp.join("data");
        let home = tmp.join("home");
        let ws_a = tmp.join("ws-a");
        let ws_b = tmp.join("ws-b");
        make_skill(&data_dir.join("skills"), "clash", Some("name: clash\ndescription: 用户级"), "x");
        make_skill(&ws_a.join(".craftwork").join("skills"), "clash", Some("name: clash\ndescription: A 的"), "x");
        make_skill(&ws_b.join(".craftwork").join("skills"), "only-b", Some("name: only-b\ndescription: B 的"), "x");

        let workspaces = vec![
            ("ws-a".to_string(), ws_a.to_string_lossy().to_string()),
            ("ws-b".to_string(), ws_b.to_string_lossy().to_string()),
        ];
        let all = discover_all_scopes_in(&home, &data_dir.to_string_lossy(), &workspaces, &[], &[]);

        let a_clash = all.iter().find(|(id, s)| id == "ws-a" && s.name == "clash").expect("A 的 clash");
        assert!(a_clash.1.error.contains("同名"), "与用户级同名的工作区级技能标冲突");
        let b_only = all.iter().find(|(id, s)| id == "ws-b" && s.name == "only-b").expect("B 的 only-b");
        assert!(b_only.1.error.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_skips_excluded_and_hidden_dirs_respects_depth() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-depth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        make_skill(&tmp, "plain", Some("name: plain\ndescription: 平铺"), "x");
        make_skill(&tmp.join("group").join("nested"), "deep", Some("name: deep\ndescription: 分组目录"), "x");
        make_skill(&tmp.join("node_modules"), "junk", Some("name: junk\ndescription: 依赖目录"), "x");
        make_skill(&tmp.join(".hidden"), "hide", Some("name: hide\ndescription: 点目录"), "x");
        // 深度越限：10 层嵌套
        let mut deep_dir = tmp.clone();
        for i in 0..12 {
            deep_dir = deep_dir.join(format!("l{i}"));
        }
        make_skill(&deep_dir, "toodeep", Some("name: toodeep\ndescription: 太深"), "x");

        let all = discover_skills(&root_of(&tmp), &[]);
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"plain"));
        assert!(names.contains(&"deep"), "分组子目录应被递归发现");
        assert!(!names.contains(&"junk"));
        assert!(!names.contains(&"hide"));
        assert!(!names.contains(&"toodeep"), "超过深度上限的不扫描");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discover_marks_disabled_and_invalid() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-invalid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let ok = make_skill(&tmp, "ok", Some("name: ok\ndescription: 正常"), "x");
        let long_desc = format!("name: bad\ndescription: {}", "长".repeat(MAX_DESCRIPTION_CHARS + 1));
        let bad = make_skill(&tmp, "bad", Some(&long_desc), "x");
        let no_desc = make_skill(&tmp, "nodesc", Some("name: nodesc"), "x");
        let plain = make_skill(&tmp, "plain", None, "无 frontmatter 容忍");

        let disabled_path = ok.join(SKILL_FILE_NAME).to_string_lossy().to_string();
        let all = discover_skills(
            &root_of(&tmp),
            &[
                disabled_path.clone(),
                bad.join(SKILL_FILE_NAME).to_string_lossy().to_string(),
                plain.join(SKILL_FILE_NAME).to_string_lossy().to_string(),
            ],
        );
        let find = |n: &str| all.iter().find(|s| s.name == n).unwrap();
        assert!(!find("ok").enabled);
        assert!(find("ok").error.is_empty());
        assert!(find("bad").error.contains("1024"));
        assert!(find("nodesc").error.contains("description"));
        assert!(!find("plain").enabled, "禁用按 SKILL.md 路径匹配（无 frontmatter 也一样）");
        assert!(find("plain").error.is_empty());
        assert!(find("plain").description.is_empty());
        let _ = std::fs::remove_dir_all(&bad);
        let _ = std::fs::remove_dir_all(&no_desc);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn skill_roots_respect_restricted_workspace() {
        let roots = skill_roots("/data", "/ws", true, &[]);
        assert_eq!(roots.len(), 2, "受限工作区只扫用户级两根");
        assert!(roots.iter().all(|r| r.scope == SkillScope::User));
        let roots = skill_roots("/data", "/ws", false, &[]);
        assert_eq!(roots.len(), 4);
        assert_eq!(roots[0].scope, SkillScope::User);
        assert_eq!(roots[2].scope, SkillScope::Workspace);
        assert_eq!(roots[2].source, SkillSource::Craftwork);
        assert_eq!(roots[3].source, SkillSource::Agents);
        // 空工作区：只有用户级
        assert_eq!(skill_roots("/data", "", false, &[]).len(), 2);
        // 插件缓存根殿后（传参顺序 = 安装顺序）、scope 用户级、source marketplace
        let plugins = vec![
            plugin_root_of("p1", std::path::Path::new("/cache/a")),
            plugin_root_of("p2", std::path::Path::new("/cache/b")),
        ];
        let with_plugins = skill_roots("/data", "", false, &plugins);
        assert_eq!(with_plugins.len(), 4);
        assert_eq!(with_plugins[2].source, SkillSource::Marketplace);
        assert_eq!(with_plugins[2].scope, SkillScope::User);
        assert_eq!(with_plugins[2].plugin, "p1");
        assert_eq!(with_plugins[3].dir, std::path::Path::new("/cache/b"));
        // 受限工作区不裁插件根（插件装在用户级缓存，与工作区无关）
        assert_eq!(skill_roots("/data", "/ws", true, &plugins).len(), 4);
    }

    // ---------- 插件技能根（技能商店，D12） ----------

    /// 插件技能以限定名 `插件名:技能名` 发现与注入；限定名与裸名别名都能解析到同一技能。
    #[test]
    fn marketplace_skills_qualified_name_and_bare_alias_resolution() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugin_dir = tmp.join("cache").join("foo").join("1.0.0");
        make_skill(&plugin_dir.join("skills"), "demo", Some("name: demo\ndescription: 插件技能"), "插件正文");

        let roots = isolated_roots(&[plugin_root_of("foo", &plugin_dir)]);
        let all = discover_skills(&roots, &[]);
        assert_eq!(all.len(), 1);
        let meta = &all[0];
        assert_eq!(meta.name, "foo:demo", "插件技能以限定名注入");
        assert_eq!(meta.source, SkillSource::Marketplace);
        assert_eq!(meta.origin, SkillOrigin::Skill);
        assert_eq!(meta.scope, SkillScope::User);
        assert!(meta.error.is_empty());

        // 限定名与裸名都解析到同一技能（无同名手工技能时裸名别名可用）
        assert_eq!(find_skill(&all, "foo:demo").map(|s| s.path.as_str()), Some(meta.path.as_str()));
        assert_eq!(find_skill(&all, "demo").map(|s| s.path.as_str()), Some(meta.path.as_str()));
        // 正文加载经限定名条目正常
        assert_eq!(load_skill_body(&meta.path, &meta.dir).unwrap(), "插件正文");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 手工技能先占名：与手工技能同名的插件裸名别名归手工技能，插件技能限定名仍可用，
    /// 两者互标不冲突（限定名与裸名是两个解析键）。
    #[test]
    fn manual_skills_take_bare_name_before_plugin_alias() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-manual-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let manual = tmp.join("manual");
        make_skill(&manual, "demo", Some("name: demo\ndescription: 手工技能"), "手工正文");
        let plugin_dir = tmp.join("cache").join("foo").join("1.0.0");
        make_skill(&plugin_dir.join("skills"), "demo", Some("name: demo\ndescription: 插件技能"), "插件正文");

        let mut roots = root_of(&manual);
        roots.extend(isolated_roots(&[plugin_root_of("foo", &plugin_dir)]));
        let all = discover_skills(&roots, &[]);
        let manual_meta = all.iter().find(|s| s.source == SkillSource::Craftwork).unwrap();
        let plugin_meta = all.iter().find(|s| s.source == SkillSource::Marketplace).unwrap();
        // 限定名不同 → 不冲突，两者都有效（手工先占名只影响裸名解析归必）
        assert!(manual_meta.error.is_empty(), "实际 {}", manual_meta.error);
        assert_eq!(plugin_meta.name, "foo:demo");
        assert!(plugin_meta.error.is_empty(), "实际 {}", plugin_meta.error);
        // 裸名解析归手工（精确名优先），限定名解析归插件
        assert_eq!(find_skill(&all, "demo").unwrap().path, manual_meta.path);
        assert_eq!(find_skill(&all, "foo:demo").unwrap().path, plugin_meta.path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 两个启用插件同名（manifest name 相同）：限定名重复 fail-closed，双双拒绝解析
    /// （限定名与裸名都不可用），错误信息说明冲突；无冲突技能不受影响。
    #[test]
    fn marketplace_same_name_plugins_fail_closed() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-clash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let a_dir = tmp.join("cache-a");
        let b_dir = tmp.join("cache-b");
        make_skill(&a_dir.join("skills"), "demo", Some("name: demo\ndescription: A 的"), "x");
        make_skill(&b_dir.join("skills"), "demo", Some("name: demo\ndescription: B 的"), "x");
        make_skill(&a_dir.join("skills"), "other", Some("name: other\ndescription: A 独有"), "x");

        let roots = isolated_roots(&[plugin_root_of("foo", &a_dir), plugin_root_of("foo", &b_dir)]);
        let all = discover_skills(&roots, &[]);
        let clashes: Vec<_> = all.iter().filter(|s| s.name == "foo:demo").collect();
        assert_eq!(clashes.len(), 2, "两条同名限定名条目都出现在列表");
        for meta in &clashes {
            assert!(meta.error.contains("同名冲突") && meta.error.contains("foo"), "互标冲突且说明双方：{}", meta.error);
        }
        // 双双拒绝解析：不做 last-write-wins
        assert!(find_skill(&all, "foo:demo").is_none());
        assert!(find_skill(&all, "demo").is_none());
        // 无冲突技能正常可用
        assert!(find_skill(&all, "foo:other").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 同一插件内两个同名技能：限定名相同即解析歧义，同样 fail-closed 双双停用。
    #[test]
    fn marketplace_duplicate_skill_in_same_plugin_fail_closed() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-dup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugin_dir = tmp.join("cache").join("foo");
        make_skill(&plugin_dir.join("skills"), "one", Some("name: dup\ndescription: 一"), "x");
        make_skill(&plugin_dir.join("skills"), "two", Some("name: dup\ndescription: 二"), "x");

        let roots = isolated_roots(&[plugin_root_of("foo", &plugin_dir)]);
        let all = discover_skills(&roots, &[]);
        let clashes: Vec<_> = all.iter().filter(|s| s.name == "foo:dup").collect();
        assert_eq!(clashes.len(), 2);
        assert!(clashes.iter().all(|s| s.error.contains("同名冲突")), "同插件内同名同样 fail-closed");
        assert!(find_skill(&all, "foo:dup").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 插件域扫描不跟随符号链接（信任边界）：skills 内指向缓存外的目录 / 文件 symlink
    /// 都不穿透；手工根的 symlink 维持跟随（既有软链导入行为不变的对照）。
    #[test]
    #[cfg(unix)]
    fn marketplace_scan_does_not_follow_symlinks() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        // 逃逸目标：插件缓存外的真技能
        let outside = tmp.join("outside");
        make_skill(&outside, "escape", Some("name: escape\ndescription: 缓存外"), "逃逸正文");
        let plugin_dir = tmp.join("cache").join("foo").join("1.0.0");
        let skills = plugin_dir.join("skills");
        make_skill(&skills, "inside", Some("name: inside\ndescription: 缓存内"), "x");
        // 目录 symlink：skills/link-dir → 缓存外技能目录
        std::os::unix::fs::symlink(outside.join("escape"), skills.join("link-dir")).unwrap();
        // 文件 symlink：skills/inner/SKILL.md → 缓存外 SKILL.md
        std::fs::create_dir_all(skills.join("inner")).unwrap();
        std::os::unix::fs::symlink(outside.join("escape").join(SKILL_FILE_NAME), skills.join("inner").join(SKILL_FILE_NAME)).unwrap();

        let roots = isolated_roots(&[plugin_root_of("foo", &plugin_dir)]);
        let names: Vec<String> = discover_skills(&roots, &[]).iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"foo:inside".to_string()), "缓存内真实技能正常发现：{names:?}");
        assert!(
            !names.iter().any(|n| n.ends_with("escape")),
            "目录与文件 symlink 逃逸都不穿透：{names:?}"
        );

        // 对照：手工根内 symlink 维持跟随（软链导入技能）
        let manual = tmp.join("manual");
        std::fs::create_dir_all(&manual).unwrap();
        std::os::unix::fs::symlink(outside.join("escape"), manual.join("linked")).unwrap();
        let manual_names: Vec<String> = discover_skills(&root_of(&manual), &[]).iter().map(|s| s.name.clone()).collect();
        assert!(manual_names.contains(&"escape".to_string()), "手工根 symlink 行为不变：{manual_names:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Windows 等价（无真机，CI 的 windows runner 是唯一验证场）。
    ///
    /// 为什么不用 junction：std 的 `FileType::is_symlink` 对 junction（mount point）
    /// 返回 false，扫描器无法用与 Unix 相同的语义识别 junction——junction 逃逸的
    /// 拦截属 D7 实机走查的已知限制，本测试只验证真正的 NTFS symlink（需要
    /// 管理员或开发者模式特权；无特权环境打印原因后跳过，不算失败）。
    #[test]
    #[cfg(windows)]
    fn marketplace_scan_does_not_follow_symlinks_windows() {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let outside = tmp.join("outside");
        make_skill(&outside, "escape", Some("name: escape\ndescription: 缓存外"), "逃逸正文");
        let plugin_dir = tmp.join("cache").join("foo").join("1.0.0");
        let skills = plugin_dir.join("skills");
        make_skill(&skills, "inside", Some("name: inside\ndescription: 缓存内"), "x");
        std::fs::create_dir_all(skills.join("inner")).unwrap();
        // 无特权环境（非管理员且未开开发者模式）跳过：这是环境限制，不是产品缺陷
        if let Err(err) = symlink_dir(outside.join("escape"), skills.join("link-dir")) {
            eprintln!("跳过：创建目录 symlink 失败（需要特权）：{err}");
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }
        if let Err(err) = symlink_file(outside.join("escape").join(SKILL_FILE_NAME), skills.join("inner").join(SKILL_FILE_NAME)) {
            eprintln!("跳过：创建文件 symlink 失败（需要特权）：{err}");
            let _ = std::fs::remove_dir_all(&tmp);
            return;
        }

        let roots = isolated_roots(&[plugin_root_of("foo", &plugin_dir)]);
        let names: Vec<String> = discover_skills(&roots, &[]).iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"foo:inside".to_string()), "缓存内真实技能正常发现：{names:?}");
        assert!(
            !names.iter().any(|n| n.ends_with("escape")),
            "目录与文件 symlink 逃逸都不穿透：{names:?}"
        );

        // 对照：手工根内 symlink 维持跟随（软链导入技能）
        let manual = tmp.join("manual");
        std::fs::create_dir_all(&manual).unwrap();
        symlink_dir(outside.join("escape"), manual.join("linked")).unwrap();
        let manual_names: Vec<String> = discover_skills(&root_of(&manual), &[]).iter().map(|s| s.name.clone()).collect();
        assert!(manual_names.contains(&"escape".to_string()), "手工根 symlink 行为不变：{manual_names:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// commands/*.md 一层映射为插件技能：origin 标注 command、name = 文件名（限定名注入）、
    /// frontmatter description 解析、无 frontmatter 宽容；子目录不递归、非 .md 不映射；
    /// 裸名解析与正文加载（目录变量以命令文件所在目录为基准）正常。
    #[test]
    fn commands_mapped_as_marketplace_skills() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-cmds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let plugin_dir = tmp.join("cache").join("foo").join("1.0.0");
        std::fs::create_dir_all(plugin_dir.join("commands")).unwrap();
        std::fs::write(plugin_dir.join("commands").join("review.md"), "---\ndescription: 审查代码\n---\n审查正文 ${CLAUDE_SKILL_DIR}").unwrap();
        std::fs::write(plugin_dir.join("commands").join("note.md"), "无 frontmatter 的 command").unwrap();
        std::fs::create_dir_all(plugin_dir.join("commands").join("nested")).unwrap();
        std::fs::write(plugin_dir.join("commands").join("nested").join("deep.md"), "x").unwrap();
        std::fs::write(plugin_dir.join("commands").join("ignore.txt"), "x").unwrap();
        make_skill(&plugin_dir.join("skills"), "demo", Some("name: demo\ndescription: 技能"), "x");

        let roots = isolated_roots(&[plugin_root_of("foo", &plugin_dir)]);
        let all = discover_skills(&roots, &[]);
        let commands: Vec<_> = all.iter().filter(|s| s.origin == SkillOrigin::Command).collect();
        assert_eq!(commands.len(), 2, "恰好 review + note 两条：{:?}", all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>());
        let review = all.iter().find(|s| s.name == "foo:review").expect("review 映射存在");
        assert_eq!(review.origin, SkillOrigin::Command);
        assert_eq!(review.description, "审查代码");
        assert!(review.error.is_empty());
        let note = all.iter().find(|s| s.name == "foo:note").expect("note 映射存在");
        assert!(note.error.is_empty() && note.description.is_empty(), "无 frontmatter 宽容处理");
        assert!(all.iter().all(|s| s.name != "foo:deep"), "commands 子目录不递归");
        assert!(all.iter().all(|s| s.name != "foo:ignore"), "非 .md 文件不映射");

        // 裸名解析 + 正文加载（目录变量以命令文件所在目录为基准替换）
        let picked = find_skill(&all, "review").unwrap();
        let body = load_skill_body(&picked.path, &picked.dir).unwrap();
        assert!(body.contains("审查正文"));
        assert!(!body.contains("${CLAUDE_SKILL_DIR}"));
        assert_eq!(picked.dir, plugin_dir.join("commands").to_string_lossy().to_string());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// load_plugin_skill_roots 只返回启用插件（按安装顺序）；停用插件的根退出扫描，
    /// 其技能从清单消失。
    #[test]
    fn plugin_disabled_roots_dropped_from_scan() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-disabled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let a_dir = tmp.join("a");
        let b_dir = tmp.join("b");
        make_skill(&a_dir.join("skills"), "alpha", Some("name: alpha\ndescription: A"), "x");
        make_skill(&b_dir.join("skills"), "beta", Some("name: beta\ndescription: B"), "x");
        let db = Arc::new(Db::open(&tmp.join("t.db")).unwrap());
        insert_plugin_row(&db, "alpha@mk", "alpha", &a_dir, true, 100);
        insert_plugin_row(&db, "beta@mk", "beta", &b_dir, false, 200);

        let roots = load_plugin_skill_roots(&db);
        assert_eq!(roots, vec![plugin_root_of("alpha", &a_dir)], "停用插件不返回，启用按安装顺序");

        let all = discover_skills(&isolated_roots(&roots), &[]);
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha:alpha"));
        assert!(!names.contains(&"beta:beta"), "停用插件的技能根退出扫描：{names:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// skill 工具对插件技能的限定名与裸名请求都能加载（插件根经 skill_plugins 表注入）；
    /// 停用插件后下一次调用即不命中（加载即重扫）。
    #[test]
    fn skill_invoke_resolves_marketplace_qualified_and_bare_names() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-mkt-invoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let plugin_dir = tmp.join("cache").join("foo").join("1.0.0");
        let dir = make_skill(&plugin_dir.join("skills"), "demo", Some("name: demo\ndescription: 插件"), "插件正文");
        let db = Arc::new(Db::open(&tmp.join("t.db")).unwrap());
        insert_plugin_row(&db, "foo@mk", "foo", &plugin_dir, true, 100);
        let mut ctx = make_tool_ctx(&data_dir, "", Arc::clone(&db));

        ctx.args = json!({ "name": "foo:demo" });
        let ok = SkillInvoke.execute(&ctx);
        assert!(!ok.is_error, "限定名加载：{}", ok.output);
        assert!(ok.output.contains("插件正文"));
        assert!(ok.output.contains(&format!("技能基础目录：{}", dir.to_string_lossy())));

        ctx.args = json!({ "name": "demo" });
        assert!(!SkillInvoke.execute(&ctx).is_error, "裸名别名命中插件技能");

        ctx.args = json!({ "name": "nope" });
        assert_eq!(SkillInvoke.execute(&ctx).code, "skill_not_found");

        // 停用插件（表行 enabled=0）后根消失，下一次调用即不命中
        db.lock_conn().execute("UPDATE skill_plugins SET enabled = 0", []).unwrap();
        ctx.args = json!({ "name": "foo:demo" });
        assert_eq!(SkillInvoke.execute(&ctx).code, "skill_not_found", "停用插件下一调用即生效");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ---------- 正文加载与清单渲染 ----------

    #[test]
    fn body_strips_frontmatter_and_expands_dir_variable() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-body-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let dir = make_skill(&tmp, "demo", Some("name: demo\ndescription: x"), "脚本位于 ${CLAUDE_SKILL_DIR}/scripts 与 ${CRAFTWORK_SKILL_DIR}/assets");
        let path = dir.join(SKILL_FILE_NAME);
        let body = load_skill_body(path.to_string_lossy().as_ref(), dir.to_string_lossy().as_ref()).unwrap();
        assert!(!body.contains("name: demo"), "frontmatter 应被剥除");
        assert!(!body.contains("${CLAUDE_SKILL_DIR}"));
        assert!(body.contains(&format!("{}/scripts", dir.to_string_lossy())));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn body_truncates_over_limit_with_marker() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-trunc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let big = "字".repeat(80_000); // 多字节字符，验证按字符边界截断
        let dir = make_skill(&tmp, "big", Some("name: big\ndescription: x"), &big);
        let path = dir.join(SKILL_FILE_NAME);
        let body = load_skill_body(path.to_string_lossy().as_ref(), dir.to_string_lossy().as_ref()).unwrap();
        assert!(body.contains("[技能内容已截断]"));
        assert!(body.chars().count() < MAX_SKILL_BYTES, "截断后不应超过字节上限对应的字符规模");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn listing_budget_degrades_to_names_only() {
        let mk = |name: &str| SkillMeta {
            name: name.into(),
            description: "很长的描述 ".repeat(50),
            when_to_use: String::new(),
            scope: SkillScope::User,
            source: SkillSource::Craftwork,
            origin: SkillOrigin::Skill,
            path: format!("/{name}/SKILL.md"),
            dir: format!("/{name}"),
            enabled: true,
            error: String::new(),
        };
        assert!(build_listing_section(&[]).is_none());
        let few = vec![mk("demo")];
        let section = build_listing_section(&few).unwrap();
        assert!(section.contains("- demo："));
        assert!(section.contains("用户级"));
        // 大量技能撑爆预算 → 仅名称
        let many: Vec<SkillMeta> = (0..200).map(|i| mk(&format!("s{i}"))).collect();
        let degraded = build_listing_section(&many).unwrap();
        assert!(degraded.contains("描述过长已省略"));
        assert!(degraded.contains("s0、s1"));
        assert!(!degraded.contains("很长的描述"));
    }

    // ---------- SkillInvoke 工具接缝 ----------

    fn make_tool_ctx(data_dir: &std::path::Path, workspace: &str, db: Arc<Db>) -> ToolCtx {
        ToolCtx {
            name: "skill".into(),
            args: json!({}),
            target: String::new(),
            display: String::new(),
            workspace: workspace.into(),
            roots: vec![],
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: Arc::new(ToolEnv {
                backup_dir: String::new(),
                data_dir: data_dir.to_string_lossy().to_string(),
                db,
                shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()),
                proxy_config: crate::registry::direct_proxy_config(),
            }),
        }
    }

    #[test]
    fn skill_invoke_loads_body_and_reports_missing() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let data_dir = tmp.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let dir = make_skill(&data_dir.join("skills"), "demo", Some("name: demo\ndescription: 演示"), "演示正文 ${CRAFTWORK_SKILL_DIR}");
        let db = Arc::new(Db::open(&tmp.join("t.db")).unwrap());
        let mut ctx = make_tool_ctx(&data_dir, "", db);
        ctx.args = json!({ "name": "demo" });
        let result = SkillInvoke.execute(&ctx);
        assert!(!result.is_error);
        assert!(result.output.contains("<skill_content name=\"demo\">"));
        assert!(result.output.contains(&format!("技能基础目录：{}", dir.to_string_lossy())));
        assert!(result.output.contains("演示正文"));

        ctx.args = json!({ "name": "nope" });
        let result = SkillInvoke.execute(&ctx);
        assert!(result.is_error);
        assert_eq!(result.code, "skill_not_found");

        ctx.args = json!({});
        assert!(SkillInvoke.execute(&ctx).is_error);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn skill_invoke_respects_master_switch_and_disabled() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-switch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let data_dir = tmp.join("data");
        let skills = data_dir.join("skills");
        let dir = make_skill(&skills, "demo", Some("name: demo\ndescription: 演示"), "正文");
        let db = Arc::new(Db::open(&tmp.join("t.db")).unwrap());
        let mut ctx = make_tool_ctx(&data_dir, "", Arc::clone(&db));
        ctx.args = json!({ "name": "demo" });
        assert!(!SkillInvoke.execute(&ctx).is_error);

        // 总开关关闭（app 键 Settings.skillsEnabled）
        db.with(|conn| set_setting(conn, "app", &json!({ "skillsEnabled": false }))).unwrap();
        let result = SkillInvoke.execute(&ctx);
        assert_eq!(result.code, "skill_disabled");

        // 总开关恢复但该技能被禁用
        db.with(|conn| set_setting(conn, "app", &json!({ "skillsEnabled": true }))).unwrap();
        db.with(|conn| set_setting(conn, SKILLS_DISABLED_KEY, &json!([dir.join(SKILL_FILE_NAME).to_string_lossy()]))).unwrap();
        let result = SkillInvoke.execute(&ctx);
        assert_eq!(result.code, "skill_not_found", "禁用技能按不存在处理");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_available_filters_switch_and_disabled() {
        let tmp = std::env::temp_dir().join(format!("cw-skills-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let data_dir = tmp.join("data");
        let skills = data_dir.join("skills");
        let a = make_skill(&skills, "a", Some("name: a\ndescription: 甲"), "x");
        make_skill(&skills, "b", Some("name: b\ndescription: 乙"), "x");
        let db = Arc::new(Db::open(&tmp.join("t.db")).unwrap());
        let data_dir_str = data_dir.to_string_lossy().to_string();
        // home 注入为不存在的路径：隔离真实 ~/.agents/skills（机器上可能装有任意技能）
        let isolated_home = std::path::Path::new("/nonexistent-cw-skills-home");
        assert_eq!(scan_available_in(&db, isolated_home, &data_dir_str, "", false).len(), 2);

        db.with(|conn| set_setting(conn, SKILLS_DISABLED_KEY, &json!([a.join(SKILL_FILE_NAME).to_string_lossy()]))).unwrap();
        let available = scan_available_in(&db, isolated_home, &data_dir_str, "", false);
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].name, "b");
        assert_eq!(available[0].scope, SkillScope::User);

        db.with(|conn| set_setting(conn, "app", &json!({ "skillsEnabled": false }))).unwrap();
        assert!(scan_available_in(&db, isolated_home, &data_dir_str, "", false).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
