//! 技能（SKILL.md）发现、解析与调用（决策 D11，参考 ZCode Agent Skills 机制）。
//!
//! 两层加载：run 启动时扫描技能根目录，仅把「名称 + 描述 + 来源」清单注入系统提示词
//! （`build_listing_section`）；模型调用 [`SkillInvoke`] 工具时才读取 SKILL.md 正文，
//! 以 tool result 回填。加载即重扫（无缓存），禁用/删除下一个 run 立即生效。
//!
//! 技能根目录按优先级扫描（先扫者胜，按 SKILL.md 绝对路径去重，同名先发现者生效）：
//! 用户级 `<数据目录>/skills` → `~/.agents/skills` → 工作区级 `<工作区>/.craftwork/skills`
//! → `<工作区>/.agents/skills`。`.agents` 目录是对 Claude/agents 生态的兼容来源；
//! 受限工作区不扫工作区级技能（防投毒：清单不可见是主要防线，加载本身为 R0 只读）。
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
/// description 字符数上限，超限该技能拒绝加载（与 ZCode skill_description_too_long 对齐）。
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
/// 正文读取字节上限，超出截断并标注。
pub const MAX_SKILL_BYTES: usize = 100_000;
/// 可用技能清单字符预算，超限降级为仅名称列表（与 ZCode metadataBudget 对齐）。
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
}

/// 单个技能的元数据：发现即解析 frontmatter，正文按需读取。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMeta {
    /// frontmatter name；缺失时回退目录名
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub scope: SkillScope,
    pub source: SkillSource,
    /// SKILL.md 绝对路径（禁用状态与去重的主键）
    pub path: String,
    /// 技能目录绝对路径（辅助脚本/资源的基准目录）
    pub dir: String,
    /// 是否启用（settings 的 skills.disabled 过滤结果）
    pub enabled: bool,
    /// 诊断信息；非空表示不可加载（缺 frontmatter 字段、超长、同名冲突等）
    pub error: String,
}

/// 技能根目录：扫描起点 + 归属标注。
#[derive(Debug, Clone)]
pub struct SkillRoot {
    pub dir: std::path::PathBuf,
    pub scope: SkillScope,
    pub source: SkillSource,
}

/// 技能根目录列表（按优先级降序）：用户级在前、工作区级在后；受限工作区跳过工作区级。
pub fn skill_roots(data_dir: &str, workspace: &str, restricted: bool) -> Vec<SkillRoot> {
    skill_roots_in(&home_dir(), data_dir, workspace, restricted)
}

/// [`skill_roots`] 的 home 注入版本：测试用它隔离真实 `~/.agents/skills`（机器上
/// 可能装有任意技能，污染计数断言）。
pub fn skill_roots_in(home: &std::path::Path, data_dir: &str, workspace: &str, restricted: bool) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    if !data_dir.is_empty() {
        roots.push(SkillRoot {
            dir: std::path::Path::new(data_dir).join("skills"),
            scope: SkillScope::User,
            source: SkillSource::Craftwork,
        });
    }
    if !home.as_os_str().is_empty() {
        roots.push(SkillRoot { dir: home.join(".agents").join("skills"), scope: SkillScope::User, source: SkillSource::Agents });
    }
    if !workspace.is_empty() && !restricted {
        let ws = std::path::Path::new(workspace);
        roots.push(SkillRoot { dir: ws.join(".craftwork").join("skills"), scope: SkillScope::Workspace, source: SkillSource::Craftwork });
        roots.push(SkillRoot { dir: ws.join(".agents").join("skills"), scope: SkillScope::Workspace, source: SkillSource::Agents });
    }
    roots
}

fn home_dir() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
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

/// run 启动 / 工具加载共用的扫描入口：总开关开启时返回全部可用技能（已过滤禁用与诊断失败项）。
pub fn scan_available(db: &Db, data_dir: &str, workspace: &str, restricted: bool) -> Vec<SkillMeta> {
    scan_available_in(db, &home_dir(), data_dir, workspace, restricted)
}

/// [`scan_available`] 的 home 注入版本（测试隔离真实 `~/.agents/skills`）。
pub fn scan_available_in(db: &Db, home: &std::path::Path, data_dir: &str, workspace: &str, restricted: bool) -> Vec<SkillMeta> {
    let (enabled, disabled) = load_skills_config(db);
    if !enabled {
        return Vec::new();
    }
    let roots = skill_roots_in(home, data_dir, workspace, restricted);
    discover_skills(&roots, &disabled)
        .into_iter()
        .filter(|s| s.enabled && s.error.is_empty())
        .collect()
}

/// 扫描全部技能根：返回全量列表（含禁用标记与诊断信息），供管理页与注入层共用。
///
/// 同名技能只有先发现（优先级高）的第一个生效；后续同名标 error「同名冲突」。
/// 仅有效技能（error 为空）占用名字，前者无效时后者可接管。
pub fn discover_skills(roots: &[SkillRoot], disabled: &[String]) -> Vec<SkillMeta> {
    let mut found: Vec<(PathRef, SkillScope, SkillSource)> = Vec::new();
    let mut seen_files: Vec<std::path::PathBuf> = Vec::new(); // canonical 去重（symlink 导入场景）
    for root in roots {
        let mut files = Vec::new();
        walk_dir(&root.dir, 0, &mut files);
        for file in files {
            let canonical = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            if seen_files.contains(&canonical) {
                continue;
            }
            seen_files.push(canonical);
            found.push((file, root.scope, root.source));
        }
    }

    let mut metas: Vec<SkillMeta> = Vec::new();
    for (path, scope, source) in found {
        let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                metas.push(invalid_meta(&dir_name, &path, &dir, scope, source, disabled, format!("SKILL.md 读取失败：{e}")));
                continue;
            }
        };
        let parsed = parse_frontmatter(&content);
        let mut error = String::new();
        let name;
        let description;
        let when_to_use;
        match &parsed.entries {
            Some(entries) => {
                name = entries.iter().find(|(k, _)| k == "name").map(|(_, v)| v.clone()).unwrap_or_default();
                description = entries.iter().find(|(k, _)| k == "description").map(|(_, v)| v.clone()).unwrap_or_default();
                when_to_use = entries.iter().find(|(k, _)| k == "when_to_use").map(|(_, v)| v.clone()).unwrap_or_default();
                if name.trim().is_empty() {
                    error = "frontmatter 缺少 name".into();
                } else if description.trim().is_empty() {
                    error = "frontmatter 缺少 description".into();
                } else if description.chars().count() > MAX_DESCRIPTION_CHARS {
                    error = format!("description 超过 {MAX_DESCRIPTION_CHARS} 字符");
                }
            }
            // 无 frontmatter：宽容处理（name 回退目录名），与 ZCode 语义一致
            None => {
                name = dir_name.clone();
                description = String::new();
                when_to_use = String::new();
            }
        }
        let mut meta = SkillMeta {
            name,
            description,
            when_to_use,
            scope,
            source,
            path: path.to_string_lossy().to_string(),
            dir: dir.to_string_lossy().to_string(),
            enabled: !disabled.iter().any(|p| p == &path.to_string_lossy()),
            error,
        };
        if !meta.error.is_empty() && meta.name.is_empty() {
            meta.name = dir_name;
        }
        metas.push(meta);
    }

    // 同名冲突：只有有效技能占用名字；被占用者标注诊断（保持发现顺序，管理页可见）
    let mut taken: Vec<String> = Vec::new();
    for meta in &mut metas {
        if meta.error.is_empty() {
            if taken.contains(&meta.name) {
                meta.error = format!("与技能「{}」同名，本技能不生效", meta.name);
            } else {
                taken.push(meta.name.clone());
            }
        }
    }
    metas
}

type PathRef = std::path::PathBuf;

fn invalid_meta(
    dir_name: &str,
    path: &std::path::Path,
    dir: &std::path::Path,
    scope: SkillScope,
    source: SkillSource,
    disabled: &[String],
    error: String,
) -> SkillMeta {
    SkillMeta {
        name: dir_name.to_string(),
        description: String::new(),
        when_to_use: String::new(),
        scope,
        source,
        path: path.to_string_lossy().to_string(),
        dir: dir.to_string_lossy().to_string(),
        enabled: !disabled.iter().any(|p| p == &path.to_string_lossy()),
        error,
    }
}

/// 递归收集目录树内全部 SKILL.md（根目录自身与子目录均可；深度上限 [`MAX_SKILL_SCAN_DEPTH`]，
/// 跳过排除目录与点目录）。目录项判定跟随符号链接（支持软链导入技能），循环由深度限制兜底。
fn walk_dir(dir: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
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
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.is_file() && name == SKILL_FILE_NAME {
            out.push(path);
        } else if meta.is_dir() && !name.starts_with('.') && !SCAN_EXCLUDED_DIRS.contains(&name.to_lowercase().as_str()) {
            walk_dir(&path, depth + 1, out);
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
/// 超出 [`LISTING_BUDGET_CHARS`] 降级为仅名称列表（与 ZCode metadataBudget 语义对齐）。
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
/// （与 ZCode Manual Command 的 prompt 改写语义一致）。
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
        let roots = skill_roots(&ctx.env.data_dir, &ctx.workspace, false);
        let available = discover_skills(&roots, &disabled);
        let Some(skill) = available.iter().find(|s| s.enabled && s.error.is_empty() && s.name == name) else {
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
            work: true,
            title: String::new(),
            summary_arg: None,
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
        vec![SkillRoot { dir: dir.to_path_buf(), scope: SkillScope::User, source: SkillSource::Craftwork }]
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
            SkillRoot { dir: user.clone(), scope: SkillScope::User, source: SkillSource::Craftwork },
            SkillRoot { dir: ws.clone(), scope: SkillScope::Workspace, source: SkillSource::Craftwork },
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
        let roots = skill_roots("/data", "/ws", true);
        assert_eq!(roots.len(), 2, "受限工作区只扫用户级两根");
        assert!(roots.iter().all(|r| r.scope == SkillScope::User));
        let roots = skill_roots("/data", "/ws", false);
        assert_eq!(roots.len(), 4);
        assert_eq!(roots[0].scope, SkillScope::User);
        assert_eq!(roots[2].scope, SkillScope::Workspace);
        assert_eq!(roots[2].source, SkillSource::Craftwork);
        assert_eq!(roots[3].source, SkillSource::Agents);
        // 空工作区：只有用户级
        assert_eq!(skill_roots("/data", "", false).len(), 2);
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
            env: Arc::new(ToolEnv {
                backup_dir: String::new(),
                data_dir: data_dir.to_string_lossy().to_string(),
                db,
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
