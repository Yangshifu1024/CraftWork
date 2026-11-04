//! fs_* / apply_patch 内置工具：读行号窗口、glob/grep 限额、写前备份、补丁预演与逐块应用。
//! 对应 src/main/tools/fs.ts。

use serde_json::{json, Value};

use craft_core::platform::{is_binary, sha256_hex, write_file_atomic};
use craft_policy::PolicyRisk;

use crate::registry::{human_size, is_ignored_dir, Tool, ToolCtx, ToolResult, ToolSpec, MAX_READ_BYTES, MAX_READ_LINES};

fn err_result(message: impl Into<String>) -> ToolResult {
    ToolResult::err(message)
}

// ---------- fs_list ----------

pub struct FsList;

impl Tool for FsList {
    fn spec(&self) -> ToolSpec {
        FsList::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let depth = ctx.int_arg("depth", 1).clamp(1, 3) as usize;
        let Ok(meta) = std::fs::metadata(&ctx.target) else {
            return err_result(format!("路径不存在：{}", ctx.display));
        };
        if !meta.is_dir() {
            return ToolResult::ok_with_meta(
                format!("{}\t{}", ctx.display, human_size(meta.len() as i64)),
                json!({ "path": ctx.target }),
            );
        }
        let mut lines: Vec<String> = Vec::new();
        walk_dir(&ctx.target, "", 1, depth, &mut lines);
        ToolResult::ok_with_meta(
            if lines.is_empty() { "（空目录）".to_string() } else { lines.join("\n") },
            json!({ "path": ctx.target }),
        )
    }
}

impl FsList {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_list".into(),
        description: "列出目录内容或查看文件信息（默认一层，最多三层）。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对工作区的路径，默认工作区根" },
                "depth": { "type": "integer", "minimum": 1, "maximum": 3, "description": "目录深度，默认 1" },
            },
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: String::new(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

fn walk_dir(dir: &str, display_dir: &str, level: usize, depth: usize, lines: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut sorted: Vec<(bool, String, String)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "." {
                return None;
            }
            let is_dir = entry.file_type().ok().map(|t| t.is_dir()).unwrap_or(false);
            // 与 TS 版一致：目录跳过隐藏与忽略表；文件跳过隐藏（保留 .env.example）
            if is_dir {
                if name.starts_with('.') || is_ignored_dir(&name) {
                    return None;
                }
            } else if name.starts_with('.') && name != ".env.example" {
                return None;
            }
            Some((is_dir, name, entry.path().to_string_lossy().to_string()))
        })
        .collect();
    // 目录优先，同类按名称排序
    sorted.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (is_dir, name, abs) in sorted {
        let display = if display_dir.is_empty() { name.clone() } else { format!("{display_dir}/{name}") };
        if is_dir {
            lines.push(format!("{}{display}/", "  ".repeat(level - 1)));
            if level < depth {
                walk_dir(&abs, &display, level + 1, depth, lines);
            }
        } else {
            let size = std::fs::metadata(&abs).map(|m| m.len() as i64).unwrap_or(0);
            lines.push(format!("{}{display}\t{}", "  ".repeat(level - 1), human_size(size)));
        }
        if lines.len() > 2000 {
            return;
        }
    }
}

// ---------- fs_read ----------

pub struct FsRead;

impl Tool for FsRead {
    fn spec(&self) -> ToolSpec {
        FsRead::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let Ok(meta) = std::fs::metadata(&ctx.target) else {
            return err_result(format!("文件不存在：{}", ctx.display));
        };
        if meta.is_dir() {
            return err_result(format!("{} 是目录，请使用 fs_list", ctx.display));
        }
        if meta.len() > 8 * 1024 * 1024 {
            return err_result(format!("文件过大（{}），超过 8MB 读取上限", human_size(meta.len() as i64)));
        }
        let raw = match std::fs::read(&ctx.target) {
            Ok(raw) => raw,
            Err(err) => return err_result(format!("读取失败：{err}")),
        };
        if is_binary(&raw) {
            return ToolResult::ok_with_meta(
                format!("二进制文件 {}\n大小: {}\nSHA-256: {}", ctx.display, human_size(raw.len() as i64), sha256_hex(&raw)),
                json!({ "path": ctx.target, "binary": true }),
            );
        }
        let capped: String = if raw.len() > MAX_READ_BYTES {
            String::from_utf8_lossy(&raw[..MAX_READ_BYTES]).to_string()
        } else {
            String::from_utf8_lossy(&raw).to_string()
        };
        let mut lines: Vec<&str> = capped.split('\n').collect();
        if capped.ends_with('\n') {
            lines.pop();
        }
        let offset = (ctx.int_arg("offset", 1) - 1).max(0) as usize;
        let limit = ctx.int_arg("limit", MAX_READ_LINES as i64).clamp(1, MAX_READ_LINES as i64) as usize;
        if offset >= lines.len() {
            return err_result(format!("offset={} 超出文件行数（共 {} 行）", offset + 1, lines.len()));
        }
        let end = lines.len().min(offset + limit);
        let mut out: Vec<String> = Vec::new();
        for (i, line) in lines.iter().enumerate().skip(offset).take(end - offset) {
            out.push(format!("{:>5}│ {line}", i + 1));
        }
        let mut output = out.join("\n");
        if end < lines.len() {
            output.push_str(&format!("\n…（还有 {} 行，使用 offset={} 继续读取）", lines.len() - end, end + 1));
        }
        ToolResult::ok_with_meta(output, json!({ "path": ctx.target }))
    }
}

impl FsRead {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_read".into(),
        description: "读取文本文件内容，带行号；支持 offset/limit 分页（默认最多 2000 行）。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对工作区的文件路径" },
                "offset": { "type": "integer", "minimum": 0, "description": "起始行（从 1 计），默认 1" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 2000, "description": "读取行数，默认 2000" },
            },
            "required": ["path"],
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: String::new(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

// ---------- fs_glob / fs_grep ----------

fn walk_files(root: &str, mut cb: impl FnMut(String, String)) {
    let mut stack: Vec<(String, String)> = vec![(root.to_string(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let abs = entry.path().to_string_lossy().to_string();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            let is_dir = entry.file_type().ok().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if !is_ignored_dir(&name) {
                    stack.push((abs, rel));
                }
            } else {
                cb(abs, rel);
            }
        }
    }
}

/// glob 匹配：模式含「星号星号斜杠」时取后缀匹配 base 名与相对路径后缀；否则整路径或 base 名匹配。
fn match_glob(pattern: &str, rel: &str) -> bool {
    if let Some(marker) = pattern.find("**/") {
        let suffix = &pattern[marker + 3..];
        let base = rel.rsplit('/').next().unwrap_or(rel);
        if glob_to_regexp(suffix).map(|re| re.is_match(base)).unwrap_or(false) {
            return true;
        }
        let trimmed = suffix.strip_prefix('*').unwrap_or(suffix);
        return rel.ends_with(trimmed);
    }
    glob_to_regexp(pattern).map(|re| re.is_match(rel) || re.is_match(rel.rsplit('/').next().unwrap_or(rel))).unwrap_or(false)
}

fn glob_to_regexp(glob: &str) -> Option<regex::Regex> {
    let mut source = String::from("^");
    let mut chars = glob.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    source.push_str(".*");
                } else {
                    source.push_str("[^/]*");
                }
            }
            '?' => source.push_str("[^/]"),
            other => {
                if regex::escape(&other.to_string()).contains('\\') {
                    source.push_str(&regex::escape(&other.to_string()));
                } else {
                    source.push(other);
                }
            }
        }
    }
    source.push('$');
    regex::Regex::new(&source).ok()
}

pub struct FsGlob;

impl Tool for FsGlob {
    fn spec(&self) -> ToolSpec {
        FsGlob::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let pattern = ctx.str_arg("pattern");
        if pattern.is_empty() {
            return err_result("pattern 不能为空");
        }
        let mut matches: Vec<String> = Vec::new();
        walk_files(&ctx.target, |_abs, rel| {
            if matches.len() < 200 && match_glob(&pattern, &rel) {
                matches.push(rel);
            }
        });
        matches.sort();
        let count = matches.len();
        ToolResult::ok_with_meta(
            if matches.is_empty() { "没有匹配的文件".to_string() } else { matches.join("\n") },
            json!({ "path": ctx.target, "count": count }),
        )
    }
}

impl FsGlob {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_glob".into(),
        description: "按 glob 模式搜索文件（如 *.ts、src/**/*.go），最多返回 200 条。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "glob 模式" },
                "path": { "type": "string", "description": "搜索根目录，默认工作区根" },
            },
            "required": ["pattern"],
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: String::new(),
        summary_arg: Some("pattern".into()),
        summary_text: None,
        }
    }
}

pub struct FsGrep;

impl Tool for FsGrep {
    fn spec(&self) -> ToolSpec {
        FsGrep::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let pattern = ctx.str_arg("pattern");
        if pattern.is_empty() {
            return err_result("pattern 不能为空");
        }
        let use_regex = ctx.args.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let regex = if use_regex {
            match regex::Regex::new(&pattern) {
                Ok(re) => Some(re),
                Err(err) => return err_result(format!("正则无效：{err}")),
            }
        } else {
            None
        };
        let mut results: Vec<String> = Vec::new();
        walk_files(&ctx.target, |abs, rel| {
            if results.len() >= 200 {
                return;
            }
            let Ok(meta) = std::fs::metadata(&abs) else { return };
            if meta.len() > 1024 * 1024 {
                return;
            }
            let Ok(raw) = std::fs::read(&abs) else { return };
            if is_binary(&raw) {
                return;
            }
            let text = String::from_utf8_lossy(&raw);
            for (i, line) in text.split('\n').enumerate() {
                let hit = match &regex {
                    Some(re) => re.is_match(line),
                    None => line.contains(&pattern),
                };
                if hit {
                    results.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                    if results.len() >= 200 {
                        return;
                    }
                }
            }
        });
        let count = results.len();
        ToolResult::ok_with_meta(
            if results.is_empty() { "没有匹配的内容".to_string() } else { results.join("\n") },
            json!({ "path": ctx.target, "count": count }),
        )
    }
}

impl FsGrep {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_grep".into(),
        description: "在工作区内搜索文本（默认子串匹配，可选正则），最多返回 200 条。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "搜索内容" },
                "path": { "type": "string", "description": "搜索根目录，默认工作区根" },
                "regex": { "type": "boolean", "description": "按正则解释 pattern，默认 false" },
            },
            "required": ["pattern"],
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: String::new(),
        summary_arg: Some("pattern".into()),
        summary_text: None,
        }
    }
}

// ---------- fs_write / fs_edit ----------

pub struct FsWrite;

impl Tool for FsWrite {
    fn spec(&self) -> ToolSpec {
        FsWrite::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let content = ctx.str_arg("content");
        let mut had_existing = false;
        if let Ok(existing) = std::fs::read(&ctx.target) {
            had_existing = true;
            if let Err(err) = ctx.env.backup(&ctx.target, &existing) {
                return err_result(err);
            }
        }
        if let Err(err) = std::path::Path::new(&ctx.target)
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
        {
            return err_result(format!("写入失败：{err}"));
        }
        if let Err(err) = write_file_atomic(std::path::Path::new(&ctx.target), content.as_bytes(), 0o644) {
            return err_result(format!("写入失败：{err}"));
        }
        let size = content.len();
        ToolResult::ok_with_meta(
            format!("已写入 {}（{}）", ctx.display, human_size(size as i64)),
            json!({ "path": ctx.target, "bytes": size, "backup": had_existing }),
        )
    }
}

impl FsWrite {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_write".into(),
        description: "写入（或创建）文件；已存在文件会先自动备份。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对工作区的文件路径" },
                "content": { "type": "string", "description": "完整文件内容" },
            },
            "required": ["path", "content"],
        }),
        risk: PolicyRisk::R1,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: "写入文件".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

pub struct FsEdit;

impl Tool for FsEdit {
    fn spec(&self) -> ToolSpec {
        FsEdit::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let old_str = ctx.str_arg("old_string");
        let new_str = ctx.str_arg("new_string");
        if old_str.is_empty() {
            return err_result("old_string 不能为空");
        }
        let raw = match std::fs::read(&ctx.target) {
            Ok(raw) => raw,
            Err(err) => return err_result(format!("读取失败：{err}")),
        };
        let text = String::from_utf8_lossy(&raw).to_string();
        let count = text.matches(&old_str).count();
        if count == 0 {
            return err_result("没有找到要替换的内容；请先读取文件确认原文");
        }
        if count > 1 {
            return err_result(format!("匹配到 {count} 处相同内容，请提供更长的唯一片段"));
        }
        if let Err(err) = ctx.env.backup(&ctx.target, &raw) {
            return err_result(err);
        }
        let replaced = text.replacen(&old_str, &new_str, 1);
        if let Err(err) = write_file_atomic(std::path::Path::new(&ctx.target), replaced.as_bytes(), 0o644) {
            return err_result(format!("写入失败：{err}"));
        }
        let preview_old: String = old_str.chars().take(40).collect();
        let preview_new: String = new_str.chars().take(40).collect();
        ToolResult::ok_with_meta(
            format!("已替换 {} 中的 1 处内容（{preview_old} → {preview_new}）", ctx.display),
            json!({ "path": ctx.target }),
        )
    }
}

impl FsEdit {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_edit".into(),
        description: "精确替换文件中的唯一文本片段；old_string 必须在文件中恰好出现一次。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对工作区的文件路径" },
                "old_string": { "type": "string", "description": "要替换的原文（必须唯一）" },
                "new_string": { "type": "string", "description": "替换后的内容" },
            },
            "required": ["path", "old_string", "new_string"],
        }),
        risk: PolicyRisk::R1,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: "修改文件".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

// ---------- apply_patch ----------

#[derive(Debug, Clone)]
pub struct PatchHunk {
    pub old_start: i64,
    pub old_count: i64,
    pub new_start: i64,
    pub new_count: i64,
    pub lines: Vec<(char, String)>, // tag: ' ' | '-' | '+'
}

pub fn parse_patch(patch: &str) -> Result<Vec<PatchHunk>, String> {
    let normalized = patch.replace("\r\n", "\n");
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop(); // 补丁常以换行结尾
    }
    let mut hunks: Vec<PatchHunk> = Vec::new();
    let mut current: Option<PatchHunk> = None;
    let header_re = regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();

    for line in lines {
        if let Some(caps) = header_re.captures(line) {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            current = Some(PatchHunk {
                old_start: caps[1].parse().unwrap_or(1),
                old_count: caps.get(2).map(|m| m.as_str().parse().unwrap_or(1)).unwrap_or(1),
                new_start: caps[3].parse().unwrap_or(1),
                new_count: caps.get(4).map(|m| m.as_str().parse().unwrap_or(1)).unwrap_or(1),
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = current.as_mut() else { continue }; // @@ 之前的内容忽略
        if line.starts_with('\\') {
            continue; // "\ No newline" 提示行
        }
        if line.is_empty() {
            hunk.lines.push((' ', String::new()));
            continue;
        }
        let mut chars = line.chars();
        let tag = chars.next().unwrap_or(' ');
        if tag == ' ' || tag == '-' || tag == '+' {
            hunk.lines.push((tag, chars.as_str().to_string()));
        } else {
            hunk.lines.push((' ', line.to_string()));
        }
    }
    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }
    // 校验每块行数与声明一致
    for (i, hunk) in hunks.iter().enumerate() {
        let del = hunk.lines.iter().filter(|(tag, _)| *tag != '+').count() as i64;
        let add = hunk.lines.iter().filter(|(tag, _)| *tag != '-').count() as i64;
        if del != hunk.old_count || add != hunk.new_count {
            return Err(format!("第 {} 个补丁块的旧/新内容行数不符", i + 1));
        }
    }
    Ok(hunks)
}

/// 应用单个 hunk：优先按修正位置匹配，找不到从文件头重搜（容忍行号偏移）；纯新增块直接插入。
fn apply_hunk(file_lines: &mut Vec<String>, hunk: &PatchHunk, delta: i64) -> Result<i64, String> {
    let old_lines: Vec<String> = hunk.lines.iter().filter(|(tag, _)| *tag != '+').map(|(_, t)| t.clone()).collect();
    let new_lines: Vec<String> = hunk.lines.iter().filter(|(tag, _)| *tag != '-').map(|(_, t)| t.clone()).collect();

    if old_lines.is_empty() {
        // 纯新增块：在 old_start-1+delta 处插入
        let at = (hunk.old_start - 1 + delta).clamp(0, file_lines.len() as i64) as usize;
        file_lines.splice(at..at, new_lines.iter().cloned());
        return Ok(new_lines.len() as i64);
    }

    let matches_at = |at: usize| -> bool {
        if at + old_lines.len() > file_lines.len() {
            return false;
        }
        old_lines.iter().enumerate().all(|(i, line)| &file_lines[at + i] == line)
    };

    let mut pos: Option<usize> = None;
    let preferred = hunk.old_start - 1 + delta;
    if preferred >= 0 && (preferred as usize) <= file_lines.len() && matches_at(preferred as usize) {
        pos = Some(preferred as usize);
    } else {
        for at in 0..=(file_lines.len().saturating_sub(old_lines.len())) {
            if matches_at(at) {
                pos = Some(at);
                break;
            }
        }
    }
    let Some(pos) = pos else {
        return Err("补丁上下文与文件内容不匹配，无法应用".into());
    };
    file_lines.splice(pos..pos + old_lines.len(), new_lines.iter().cloned());
    Ok(new_lines.len() as i64 - old_lines.len() as i64)
}

pub struct ApplyPatch;

impl Tool for ApplyPatch {
    fn spec(&self) -> ToolSpec {
        ApplyPatch::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let patch_text = ctx.str_arg("patch");
        if patch_text.trim().is_empty() {
            return err_result("patch 不能为空");
        }
        match std::fs::metadata(&ctx.target) {
            Ok(meta) if meta.is_dir() => return err_result(format!("{} 是目录", ctx.display)),
            Ok(_) => {}
            Err(_) => return err_result(format!("apply_patch 只修改已有文件，目标不存在：{}", ctx.display)),
        }

        let hunks = match parse_patch(&patch_text) {
            Ok(hunks) => hunks,
            Err(err) => return err_result(err),
        };
        if hunks.is_empty() {
            return err_result("补丁中没有可应用的内容块");
        }

        // 选择要应用的块（从 1 起）
        let selected: Vec<i64> = ctx
            .args
            .get("hunks")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        let mut unique_selected = selected.clone();
        unique_selected.sort();
        unique_selected.dedup();
        if unique_selected.iter().any(|n| *n < 1 || *n > hunks.len() as i64) {
            return err_result(format!("hunks 序号超出范围（共 {} 块）", hunks.len()));
        }
        let chosen: Vec<&PatchHunk> = if unique_selected.is_empty() {
            hunks.iter().collect()
        } else {
            unique_selected.iter().map(|n| &hunks[(n - 1) as usize]).collect()
        };

        let original = match std::fs::read(&ctx.target) {
            Ok(raw) => raw,
            Err(err) => return err_result(format!("读取失败：{err}")),
        };
        let text = String::from_utf8_lossy(&original).to_string();
        let mut file_lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let mut delta: i64 = 0;
        let mut added: i64 = 0;
        let mut removed: i64 = 0;
        for hunk in &chosen {
            let old_len = hunk.lines.iter().filter(|(tag, _)| *tag != '+').count() as i64;
            let new_len = hunk.lines.iter().filter(|(tag, _)| *tag != '-').count() as i64;
            match apply_hunk(&mut file_lines, hunk, delta) {
                Ok(d) => delta += d,
                Err(err) => return err_result(err),
            }
            added += new_len - new_len.min(old_len) + (new_len - old_len).max(0);
            removed += (old_len - new_len).max(0);
        }

        if ctx.args.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
            return ToolResult::ok(format!(
                "补丁预演通过：将应用 {} 个补丁块到 {}。未写入文件。",
                chosen.len(),
                ctx.display
            ));
        }

        if let Err(err) = ctx.env.backup(&ctx.target, &original) {
            return err_result(err);
        }
        if let Err(err) = write_file_atomic(std::path::Path::new(&ctx.target), file_lines.join("\n").as_bytes(), 0o644) {
            return err_result(format!("写入失败：{err}"));
        }
        ToolResult::ok_with_meta(
            format!("已应用 {} 个补丁块到 {}（+{added} −{removed} 行）。", chosen.len(), ctx.display),
            json!({ "path": ctx.target }),
        )
    }
}

impl ApplyPatch {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "apply_patch".into(),
        description: "对已存在的文件应用统一 diff（unified diff）补丁；支持 dry_run 预演与按块（hunks，从 1 起）选择性应用。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "目标文件（相对工作区）" },
                "patch": { "type": "string", "description": "统一 diff 补丁文本" },
                "dry_run": { "type": "boolean", "description": "只校验不写入，默认 false" },
                "hunks": { "type": "array", "items": { "type": "integer" }, "description": "要应用的补丁块序号（从 1 起），默认全部" },
            },
            "required": ["path", "patch"],
        }),
        risk: PolicyRisk::R1,
        path_arg: Some("path".into()),
        command_arg: None,
        work: true,
        title: "应用补丁".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob匹配_通配与双星() {
        assert!(match_glob("*.ts", "src/main.ts"));
        assert!(match_glob("**/*.go", "src/a/b.go"));
        assert!(!match_glob("*.ts", "src/main.go"));
        assert!(match_glob("main.rs", "src/main.rs"));
    }

    #[test]
    fn 补丁解析_行数校验() {
        let patch = "@@ -1,2 +1,2 @@\n-a\n+b\n c\n";
        let hunks = parse_patch(patch).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 3);
        assert!(parse_patch("@@ -1,3 +1,2 @@\n-a\n+b").is_err(), "行数不符必须报错");
    }

    #[test]
    fn 补丁应用_修正位置与行号偏移回退() {
        let mut file: Vec<String> = "one\ntwo\nthree\nfour".split('\n').map(str::to_string).collect();
        let hunk = parse_patch("@@ -2,1 +2,1 @@\n-two\n+TWO").unwrap().remove(0);
        assert_eq!(apply_hunk(&mut file, &hunk, 0).unwrap(), 0);
        assert_eq!(file.join("\n"), "one\nTWO\nthree\nfour");

        // 行号偏移：声明位置不对，从头搜索命中
        let mut file: Vec<String> = "x\ny\nz\nthree".split('\n').map(str::to_string).collect();
        let hunk = parse_patch("@@ -9,1 +9,1 @@\n-three\n+3").unwrap().remove(0);
        assert_eq!(apply_hunk(&mut file, &hunk, 0).unwrap(), 0);
        assert_eq!(file.join("\n"), "x\ny\nz\n3");

        // 纯新增块
        let mut file: Vec<String> = "a\nb".split('\n').map(str::to_string).collect();
        let hunk = parse_patch("@@ -2,0 +2,1 @@\n+inserted").unwrap().remove(0);
        assert_eq!(apply_hunk(&mut file, &hunk, 0).unwrap(), 1);
        assert_eq!(file.join("\n"), "a\ninserted\nb");
    }

    #[test]
    fn 补丁应用_上下文不匹配报错() {
        let mut file: Vec<String> = "a\nb".split('\n').map(str::to_string).collect();
        let hunk = parse_patch("@@ -1,1 +1,1 @@\n-nothing-matches\n+new").unwrap().remove(0);
        assert!(apply_hunk(&mut file, &hunk, 0).is_err());
    }
}
