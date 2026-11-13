//! fs_* / apply_patch 内置工具：fs_list / fs_read / fs_glob / fs_grep /
//! fs_write / fs_edit / apply_patch。
//!
//! 路径解析与越界校验已由 Registry.assess 完成，工具拿到的 ctx.target 是解析后的
//! 绝对路径，可放心直接访问；写类工具（fs_write / fs_edit / apply_patch）统一
//! 「先备份原始内容再原子写入」。apply_patch 支持补丁预演（dry_run）与逐块应用，
//! 逐块匹配容忍行号偏移。
//!
//! # 行尾约定（决策 D26）
//!
//! **匹配与行尾无关，写入保持原文件行尾。** 模型给的 `old_string` / `new_string`
//! 与补丁文本一律只是「文本」，它用什么换行（LF 或 CRLF）不决定成败；文件侧的
//! 主导行尾（`dominant_eol`）才是落盘依据。
//!
//! 起因：Windows 上 `core.autocrlf=true` 或用户用记事本存过，工作区文件是 CRLF，
//! 而 `fs_read` 展示给模型的内容经模型复述后变成 LF——旧实现做字节级精确匹配，
//! 于是跨行替换恒失败（单行成功、多行报「没有找到要替换的内容」），apply_patch
//! 报「补丁上下文与文件内容不匹配」。注意 `.gitattributes` 的 `text=auto` 只归一
//! **索引**、不改变工作区检出，故不能靠它修这个 bug。
//!
//! 取舍：① `fs_write` 不参与转换——它的语义是「写入即内容」，且 LF 是仓库唯一
//! 入库形态，静默转 CRLF 与之相反；② 混杂行尾的文件统一为**主导**风格（既有
//! 行尾无法在「只改一处」的前提下保持原样）；③ 无换行或 CRLF 与 LF 数量相等时
//! 按 LF 处理，即不做任何转换。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch, SinkFinish};
use ignore::{WalkBuilder, WalkState};

use craft_core::platform::{is_binary, sha256_hex, write_file_atomic};
use craft_policy::PolicyRisk;

use crate::registry::{human_size, is_ignored_dir, Tool, ToolCtx, ToolResult, ToolSpec, MAX_READ_BYTES, MAX_READ_LINES};

/// `fs_read` 结果的字节上限（64 KiB）：比默认出口大一倍，但仍远小于它内部声明的
/// `MAX_READ_BYTES`（256 KiB）。
///
/// 取 64 KiB 而不是 256 KiB 的理由：读文件是 fs_read 的**功能**，需要比统一出口宽松，
/// 但一次读满 260 KiB 会吃掉小模型近半窗口。真正的修复不是把上限调大，而是**分页必须
/// 准确**——超预算时在正文末行给出可执行的 `offset=`（见 `execute`），模型按需续读。
pub const FS_READ_OUTPUT_CEILING: usize = 64 * 1024;

fn err_result(message: impl Into<String>) -> ToolResult {
    ToolResult::err(message)
}

// ---------- 行尾（D26） ----------

/// 文本的主导行尾风格。写类工具据此决定落盘形态：匹配一律在 LF 形态上做，
/// 写入时再还原成这里声明的风格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Eol {
    Crlf,
    Lf,
}

impl Eol {
    /// 供 `fs_read` 的 meta 用（界面与模型都能据它判断文件行尾）。
    fn as_str(self) -> &'static str {
        match self {
            Eol::Crlf => "crlf",
            Eol::Lf => "lf",
        }
    }
}

/// 按 CRLF 与「单独 LF」的数量比较判定主导行尾；平局或没有换行时判 LF（即不转换）。
///
/// 为什么用「数量比较」而不是「首个换行」：脚本生成或跨平台合并可能留下混杂行尾，
/// 首个换行往往是偶然的；多数派才是这个文件「看起来的样子」。
fn dominant_eol(text: &str) -> Eol {
    let crlf = text.matches("\r\n").count();
    let lf_only = text.matches('\n').count() - crlf;
    if crlf > lf_only { Eol::Crlf } else { Eol::Lf }
}

/// 归一为 LF：匹配前必须做，模型经 `fs_read` 看到的片段复述出来就是 LF 形态。
fn normalize_eol(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// 还原成目标行尾。LF 目标时是恒等变换（纯 LF 文件因此零行为变化）。
fn restore_eol(text: &str, eol: Eol) -> String {
    match eol {
        Eol::Crlf => text.replace('\n', "\r\n"),
        Eol::Lf => text.to_string(),
    }
}

// ---------- fs_list ----------

/// 列出目录内容或查看单个文件信息（R0 只读）。
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
        mutates: false,
        work: true,
        title: "列出目录".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

/// 递归列目录：目录优先、同类按名称排序；目录跳过隐藏与忽略表，文件跳过隐藏
/// （保留 .env.example）。输出行数超过 2000 提前止损（深层目录不再展开）。
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

/// 读取文本文件内容，带行号；支持 offset/limit 分页（R0 只读）。
pub struct FsRead;

impl Tool for FsRead {
    fn spec(&self) -> ToolSpec {
        FsRead::spec()
    }

    /// 读文件是这个工具的**功能**，需要比统一出口（32 KiB）宽松；出口再按 32 KiB
    /// 砍会静默丢正文且不给准确 offset（D29）。
    fn output_ceiling(&self) -> usize {
        FS_READ_OUTPUT_CEILING
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let Ok(meta) = std::fs::metadata(&ctx.target) else {
            return err_result(format!("文件不存在：{}", ctx.display));
        };
        if meta.is_dir() {
            return err_result(format!("{} 是目录，请使用 fs_list", ctx.display));
        }
        // 两级限制：8MB 直接拒绝（读前先判大小），MAX_READ_BYTES 截断后分页续读
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
        // 超过字节上限时只读前缀，剩余行靠 offset 分页获取
        let byte_capped = raw.len() > MAX_READ_BYTES;
        let capped: String = if byte_capped {
            String::from_utf8_lossy(&raw[..MAX_READ_BYTES]).to_string()
        } else {
            String::from_utf8_lossy(&raw).to_string()
        };
        // 行尾剥掉再展示（D26）：CRLF 文件若把 '\r' 带进输出，模型复述出去就是
        // 带 '\r' 的原文，读起来不可见却让后续匹配与替换都失真。
        // 逐行剥而不是整文件 replace，避免把正文里合法出现的 CR 也吃掉。
        let mut lines: Vec<&str> = capped.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
        if capped.ends_with('\n') {
            // 去掉结尾换行产生的空行，避免行号比实际多一行
            lines.pop();
        }
        let offset = (ctx.int_arg("offset", 1) - 1).max(0) as usize;
        let limit = ctx.int_arg("limit", MAX_READ_LINES as i64).clamp(1, MAX_READ_LINES as i64) as usize;
        if offset >= lines.len() {
            return err_result(format!("offset={} 超出文件行数（共 {} 行）", offset + 1, lines.len()));
        }
        // 字节感知分页（D29）：行数没到上限但字节到了预算时，必须在正文里给出准确的
        // offset 续读提示，而不是让上层按字节砍掉——砍掉时模型只看到「输出已截断」，
        // 不知道自己该从第几行继续，实际丢了正文却以为读完了。
        // 预算 = 工具出口上限（64 KiB）减去页脚与行号前缀的余量。
        let byte_budget = FS_READ_OUTPUT_CEILING.saturating_sub(256);
        let end_of_lines = lines.len().min(offset + limit);
        let mut out: Vec<String> = Vec::new();
        let mut used = 0usize;
        let mut stopped_at = end_of_lines;
        for (i, line) in lines.iter().enumerate().skip(offset).take(end_of_lines - offset) {
            let rendered = format!("{:>5}│ {line}", i + 1);
            // 至少输出一行：单行超预算时仍然给出内容，由出口兜底截断
            if !out.is_empty() && used + rendered.len() + 1 > byte_budget {
                stopped_at = i;
                break;
            }
            used += rendered.len() + 1;
            out.push(rendered);
        }
        let mut output = out.join("\n");
        // 两种情况都必须在正文里给出续读指令，绝不能只靠出口按字节硬切（D29）：
        // - stopped_at < lines.len()：行数或字节预算用尽，还有行没输出
        // - byte_capped：原始文件超过 MAX_READ_BYTES，`lines` 只覆盖前缀，
        //   文件后面还有内容，但行号已经到 "末尾" 了（历史行为在这里完全不提示）
        if stopped_at < lines.len() {
            output.push_str(&format!("\n…（还有 {} 行，使用 offset={} 继续读取）", lines.len() - stopped_at, stopped_at + 1));
        } else if byte_capped {
            let next = lines.len() + 1;
            output.push_str(&format!(
                "\n…（文件超过 {} 上限，本次只读到第 {} 行；使用 offset={next} 继续读取）",
                human_size(MAX_READ_BYTES as i64),
                lines.len()
            ));
        }
        // eol 随结果告知文件行尾：模型写 old_string / patch 时不必猜，也解释了
        // 为什么读到的是 LF 形态而落盘仍是原风格（D26）。
        ToolResult::ok_with_meta(output, json!({ "path": ctx.target, "eol": dominant_eol(&capped).as_str() }))
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
        mutates: false,
        work: true,
        title: "读取文件".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

// ---------- fs_glob / fs_grep ----------

/// 深度优先遍历（显式栈），对每个文件回调 (绝对路径, 相对路径)。
/// 与 walk_dir 不同：隐藏文件/目录一律跳过（含 .env.example），避免 glob/grep 误触敏感文件。
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

/// 极简 glob → 正则翻译：`**` → 任意内容、`*` → 不跨目录、`?` → 单个非目录字符，
/// 其余字符按字面量（正则元字符转义）。翻译失败返回 None（调用方按不匹配处理）。
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

/// 按 glob 模式搜索文件（R0 只读，最多返回 200 条）。
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
            // 集满 200 条即停，防止大型仓库全量列举
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
        mutates: false,
        work: true,
        title: "按模式查找文件".into(),
        summary_arg: Some("pattern".into()),
        summary_text: None,
        }
    }
}

/// fs_grep 单文件跳过的字节上限：流式搜索内存无忧，超长文件只是耗时且匹配数
/// 上限已定，读不完意义不大（旧实现的 1MB 随流式化放宽）。
const MAX_GREP_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// fs_grep 全局匹配上限（files_with_matches 模式下是文件数上限）。
const MAX_GREP_MATCHES: usize = 200;

/// fs_grep 单条结果：匹配行或上下文行，排序后统一输出。
struct GrepHit {
    rel: String,
    line: u64,
    text: String,
    /// true = 匹配行（`路径:行号: 内容`），false = 上下文行（`路径:行号- 内容`）
    is_match: bool,
}

/// fs_grep 并行搜索的共享累计（单锁保护；quit 原子标志供各线程提前终止）。
#[derive(Default)]
struct GrepState {
    hits: Vec<GrepHit>,
    /// 匹配行总数（不含上下文行）
    matched: usize,
    /// 命中文件数（files_with_matches 模式）
    files_matched: usize,
    /// 已尝试搜索的文件数
    searched: usize,
}

/// fs_grep 一次搜索的参数与共享状态（WalkParallel 各线程共享同一实例）。
struct GrepSearch {
    matcher: grep_regex::RegexMatcher,
    /// 每个匹配行前后各附带的上下文行数（0 = 不带）
    context: usize,
    /// 每个文件最多报告的匹配数（0 = 不限，对应 rg -m）
    max_count: usize,
    /// 只列命中文件路径，不输出行（对应 rg -l）
    files_only: bool,
    /// 文件名 glob 过滤（支持 * 与 **，复用 match_glob）
    include: Option<String>,
    state: Mutex<GrepState>,
    quit: AtomicBool,
}

impl GrepSearch {
    /// 搜索单个文件；返回 WalkState 供遍历器决定是否继续。
    fn search_file(&self, path: &Path, rel: &str) -> WalkState {
        {
            let mut state = self.state.lock().unwrap();
            state.searched += 1;
            if state.matched >= MAX_GREP_MATCHES || state.files_matched >= MAX_GREP_MATCHES {
                self.quit.store(true, Ordering::Relaxed);
            }
            if self.quit.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
        }
        if self.files_only {
            // 只列文件：默认 searcher 即可，首个命中即停
            let sink = FileSink { search: self, rel: rel.to_string() };
            let mut searcher = Searcher::new();
            let _ = searcher.search_path(&self.matcher, path, sink);
            return self.walk_state();
        }
        let mut builder = SearcherBuilder::new();
        builder
            .line_number(true)
            // 遇 NUL 字节即视为二进制并停止该文件（替代旧 is_binary 全量预读）
            .binary_detection(BinaryDetection::quit(0x00))
            // 默认开启的 BOM 嗅探让 UTF-16 文件自动转码后参与搜索
            .bom_sniffing(true);
        if self.context > 0 {
            builder.before_context(self.context).after_context(self.context);
        }
        let mut searcher = builder.build();
        let sink = LineSink { search: self, rel: rel.to_string(), file_matched: 0, before: Vec::new(), after: Vec::new() };
        // 读取失败按跳过处理（与遍历期 metadata 失败一致的静默取舍）
        let _ = searcher.search_path(&self.matcher, path, sink);
        self.walk_state()
    }

    fn walk_state(&self) -> WalkState {
        if self.quit.load(Ordering::Relaxed) {
            WalkState::Quit
        } else {
            WalkState::Continue
        }
    }
}

/// 行模式 Sink：收集匹配行与上下文行。
///
/// 上下文冲刷逻辑：Before 行在 matched 回调时已积累，随匹配行一起输出；
/// After 行属于上一个匹配，在下一个匹配（行号顺序自然衔接）或文件结束
/// （finish）时冲刷。重叠窗口的行由 grep-searcher 保证不重复回调。
struct LineSink<'a> {
    search: &'a GrepSearch,
    rel: String,
    file_matched: usize,
    before: Vec<(u64, String)>,
    after: Vec<(u64, String)>,
}

impl LineSink<'_> {
    fn push_hit(&self, state: &mut GrepState, line: u64, text: String, is_match: bool) {
        state.hits.push(GrepHit { rel: self.rel.clone(), line, text, is_match });
    }
}

impl Sink for LineSink<'_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let line_no = mat.line_number().unwrap_or(0);
        // 单行模式下 bytes 即完整一行（含行终止符）
        let text = String::from_utf8_lossy(mat.bytes()).trim().to_string();
        let quit = {
            let mut state = self.search.state.lock().unwrap();
            for (line, text) in std::mem::take(&mut self.after) {
                self.push_hit(&mut state, line, text, false);
            }
            for (line, text) in std::mem::take(&mut self.before) {
                self.push_hit(&mut state, line, text, false);
            }
            self.push_hit(&mut state, line_no, text, true);
            state.matched += 1;
            state.matched >= MAX_GREP_MATCHES
        };
        if quit {
            self.search.quit.store(true, Ordering::Relaxed);
            return Ok(false);
        }
        self.file_matched += 1;
        if self.search.max_count > 0 && self.file_matched >= self.search.max_count {
            return Ok(false);
        }
        Ok(true)
    }

    fn context(&mut self, _searcher: &Searcher, ctx: &SinkContext<'_>) -> Result<bool, io::Error> {
        let item = (ctx.line_number().unwrap_or(0), String::from_utf8_lossy(ctx.bytes()).trim().to_string());
        match ctx.kind() {
            SinkContextKind::Before => self.before.push(item),
            _ => self.after.push(item),
        }
        Ok(true)
    }

    fn finish(&mut self, _searcher: &Searcher, _finish: &SinkFinish) -> Result<(), io::Error> {
        // 文件结束：冲刷最后一个匹配的 after 上下文
        if !self.after.is_empty() {
            let mut state = self.search.state.lock().unwrap();
            for (line, text) in std::mem::take(&mut self.after) {
                self.push_hit(&mut state, line, text, false);
            }
        }
        Ok(())
    }
}

/// 只列文件模式 Sink：首个命中即登记并停该文件。
struct FileSink<'a> {
    search: &'a GrepSearch,
    rel: String,
}

impl Sink for FileSink<'_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, _mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let quit = {
            let mut state = self.search.state.lock().unwrap();
            state.files_matched += 1;
            state.hits.push(GrepHit { rel: self.rel.clone(), line: 0, text: String::new(), is_match: true });
            state.files_matched >= MAX_GREP_MATCHES
        };
        if quit {
            self.search.quit.store(true, Ordering::Relaxed);
        }
        Ok(false)
    }
}

/// 在工作区内搜索文本，默认子串匹配、可选正则（R0 只读，最多 200 条）。
///
/// 遍历用 ripgrep 的 ignore crate（并行 + .gitignore 尊重 + 隐藏文件跳过），
/// 搜索用 grep-searcher/grep-regex（流式逐行 + 字面量快速路径 + 二进制检测）。
pub struct FsGrep;

impl FsGrep {
    /// 从参数构建共享搜索状态；pattern 编译失败返回错误文本。
    ///
    /// `ToolResult` 随输出预算申报（D29）变大后触发了 clippy 的 `result_large_err`；
    /// 这里把错误当返回值直接交给调用方，不存储、不跨线程传递，装箱只会多一次分配。
    #[allow(clippy::result_large_err)]
    fn prepare(ctx: &ToolCtx, pattern: &str) -> Result<Arc<GrepSearch>, ToolResult> {
        let use_regex = ctx.args.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let ignore_case = ctx.args.get("ignore_case").and_then(Value::as_bool).unwrap_or(false);
        // 字面量经转义后统一走 regex 引擎编译，自动获得 memmem 字面量快速路径
        let source = if use_regex { pattern.to_string() } else { regex::escape(pattern) };
        let matcher = match RegexMatcherBuilder::new().case_insensitive(ignore_case).build(&source) {
            Ok(m) => m,
            Err(err) => {
                let hint = if use_regex { "正则无效" } else { "搜索内容无法编译" };
                return Err(err_result(format!("{hint}：{err}")));
            }
        };
        Ok(Arc::new(GrepSearch {
            matcher,
            context: ctx.int_arg("context", 0).clamp(0, 10) as usize,
            max_count: ctx.int_arg("max_count", 0).clamp(0, 10_000) as usize,
            files_only: ctx.args.get("files_with_matches").and_then(Value::as_bool).unwrap_or(false),
            include: { let s = ctx.str_arg("include"); if s.is_empty() { None } else { Some(s) } },
            state: Mutex::new(GrepState::default()),
            quit: AtomicBool::new(false),
        }))
    }
}

impl Tool for FsGrep {
    fn spec(&self) -> ToolSpec {
        FsGrep::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let pattern = ctx.str_arg("pattern");
        if pattern.is_empty() {
            return err_result("pattern 不能为空");
        }
        let search = match FsGrep::prepare(ctx, &pattern) {
            Ok(s) => s,
            Err(result) => return result,
        };
        let root = PathBuf::from(&ctx.target);
        WalkBuilder::new(&ctx.target)
            // 隐藏文件与目录一律跳过（含 .env），与 glob/列表工具同一安全语义
            .hidden(true)
            .git_ignore(true)
            .ignore(true)
            // 工作区常是仓库子目录，父级 ignore 规则也要生效
            .parents(true)
            // 非 git 目录也应用 ignore 规则（配合 IGNORED_DIRS 兜底生成物目录）
            .require_git(false)
            .filter_entry(|e| e.depth() == 0 || !is_ignored_dir(&e.file_name().to_string_lossy()))
            .build_parallel()
            .run(|| {
                let search = Arc::clone(&search);
                let root = root.clone();
                Box::new(move |entry| {
                    if search.quit.load(Ordering::Relaxed) {
                        return WalkState::Quit;
                    }
                    let Ok(entry) = entry else { return WalkState::Continue };
                    // 只搜文件（symlink 默认不跟随）；目录由遍历器自行下钻
                    if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                        return WalkState::Continue;
                    }
                    if entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_GREP_FILE_BYTES {
                        return WalkState::Continue;
                    }
                    // 相对搜索根的展示路径；根本身就是文件时退化为文件名
                    let Ok(rel_path) = entry.path().strip_prefix(&root) else {
                        return WalkState::Continue;
                    };
                    let rel = if rel_path.as_os_str().is_empty() {
                        entry.file_name().to_string_lossy().to_string()
                    } else {
                        rel_path.to_string_lossy().to_string()
                    };
                    if let Some(pattern) = &search.include
                        && !match_glob(pattern, &rel)
                    {
                        return WalkState::Continue;
                    }
                    search.search_file(entry.path(), &rel)
                })
            });
        let quit = search.quit.load(Ordering::Relaxed);
        let files_only = search.files_only;
        // 锁内快照取走结果集（并行遍历已结束，不依赖 Arc 引用计数归一）
        let (mut hits, matched, files_matched, searched) = {
            let mut state = search.state.lock().unwrap();
            let hits = std::mem::take(&mut state.hits);
            (hits, state.matched, state.files_matched, state.searched)
        };
        // 并行到达顺序不确定：按路径字典序 + 行号数值序排序，输出稳定可读
        hits.sort_by(|a, b| a.rel.cmp(&b.rel).then(a.line.cmp(&b.line)));
        let lines: Vec<String> = hits
            .iter()
            .map(|h| {
                if files_only {
                    h.rel.clone()
                } else if h.is_match {
                    format!("{}:{}: {}", h.rel, h.line, h.text)
                } else {
                    // 上下文行用「-」替代「:」，与 rg 的输出约定一致，便于区分匹配与上下文
                    format!("{}:{}- {}", h.rel, h.line, h.text)
                }
            })
            .collect();
        let count = if files_only { files_matched } else { matched };
        let empty_text = if files_only { "没有匹配的文件" } else { "没有匹配的内容" };
        ToolResult::ok_with_meta(
            if lines.is_empty() { empty_text.to_string() } else { lines.join("\n") },
            json!({
                "path": ctx.target,
                "count": count,
                "truncated": quit,
                "searched_files": searched,
            }),
        )
    }
}

impl FsGrep {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "fs_grep".into(),
        description: "在工作区内搜索文本（默认子串匹配，可选正则）。遵循 .gitignore、跳过隐藏与二进制文件，并行搜索，最多返回 200 条匹配行。".into(),
        schema: json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "搜索内容" },
                "path": { "type": "string", "description": "搜索根目录，默认工作区根" },
                "regex": { "type": "boolean", "description": "按正则解释 pattern，默认 false" },
                "ignore_case": { "type": "boolean", "description": "大小写不敏感匹配，默认 false" },
                "include": { "type": "string", "description": "只搜索文件名匹配 glob 的文件（如 \"*.rs\"、\"src/**/*.go\"）" },
                "context": { "type": "integer", "minimum": 0, "maximum": 10, "description": "每个匹配行前后各附带 N 行上下文，默认 0" },
                "max_count": { "type": "integer", "minimum": 1, "description": "每个文件最多报告的匹配数" },
                "files_with_matches": { "type": "boolean", "description": "只列出含匹配的文件路径，不输出行内容，默认 false" },
            },
            "required": ["pattern"],
        }),
        risk: PolicyRisk::R0,
        path_arg: Some("path".into()),
        command_arg: None,
        mutates: false,
        work: true,
        title: "搜索内容".into(),
        summary_arg: Some("pattern".into()),
        summary_text: None,
        }
    }
}

// ---------- fs_write / fs_edit ----------

/// 写入（或创建）文件（R1 写工作区）；已存在文件先自动备份。
pub struct FsWrite;

impl Tool for FsWrite {
    fn spec(&self) -> ToolSpec {
        FsWrite::spec()
    }

    fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let content = ctx.str_arg("content");
        // 原字节留给 diff 的 old 侧：重新读一次会在「读取后、写入前」的窗口里拿到
        // 已改内容，与计算 diff 的内容不再一致。
        let mut existing_bytes: Option<Vec<u8>> = None;
        if let Ok(existing) = std::fs::read(&ctx.target) {
            existing_bytes = Some(existing);
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
        let mut meta = json!({ "path": ctx.target, "bytes": size });
        // 二进制原文件没有行语义：整体重写不构成「一行行改了什么」，不产 diff
        // （界面回到纯文本输出，D21）。
        let old_text = match existing_bytes.as_deref() {
            Some(raw) if !is_binary(raw) => Some(String::from_utf8_lossy(raw).to_string()),
            Some(_) => None,
            // 新建文件：old 侧按空串处理，整体计为新增
            None => Some(String::new()),
        };
        if let Some(old) = old_text {
            ctx.attach_diff(&mut meta, &old, &content);
        }
        ToolResult::ok_with_meta(format!("已写入 {}（{}）", ctx.display, human_size(size as i64)), meta)
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
        mutates: true,
        work: true,
        title: "写入文件".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

/// 精确替换文件中的唯一文本片段（R1 写工作区）；old_string 必须恰好出现一次，
/// 零次或多于一次都拒绝，防止误改。
///
/// 匹配在 LF 形态上做、写盘还原原文件行尾（D26，见模块头「行尾约定」）。
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
        // 行尾与匹配无关（D26）：双方都归一到 LF。文件侧主导行尾另行记住，
        // 写盘时还原——否则「只改一处」会把整个文件的行尾翻掉。
        let eol = dominant_eol(&text);
        let flat = normalize_eol(&text);
        let old_flat = normalize_eol(&old_str);
        let new_flat = normalize_eol(&new_str);
        // 唯一性校验：匹配多处时模型补长片段重试，避免改错位置。
        // 在归一形态上数，防止「LF 形态只出现一次、CRLF 形态另有一处」被误判为唯一。
        let count = flat.matches(&old_flat).count();
        if count == 0 {
            return err_result(format!(
                "没有找到要替换的内容（文件行尾为 {}）；请先读取文件确认原文；old_string 只写正文文本，不必匹配行尾符",
                eol.as_str()
            ));
        }
        if count > 1 {
            return err_result(format!("匹配到 {count} 处相同内容，请提供更长的唯一片段"));
        }
        // 差异按「归一后 → 归一后」算，与落盘内容同源（界面不显示整文件行尾翻转）
        let replaced_flat = flat.replacen(&old_flat, &new_flat, 1);
        let replaced = restore_eol(&replaced_flat, eol);
        if let Err(err) = write_file_atomic(std::path::Path::new(&ctx.target), replaced.as_bytes(), 0o644) {
            return err_result(format!("写入失败：{err}"));
        }
        let preview_old: String = old_flat.chars().take(40).collect();
        let preview_new: String = new_flat.chars().take(40).collect();
        let mut meta = json!({ "path": ctx.target, "eol": eol.as_str() });
        // 行级差异挂到结果上：界面工具卡展开即可看到「哪几行被替换」，
        // 不必再点开输出正文（那只有首 40 字的预览）。两侧都用归一形态：
        // `editor::diff` 本来就先归一 CRLF，这里省掉一次多余拆分，
        // 也让「文件行尾风格」不混进差异。
        ctx.attach_diff(&mut meta, &flat, &replaced_flat);
        ToolResult::ok_with_meta(
            format!("已替换 {} 中的 1 处内容（{preview_old} → {preview_new}）", ctx.display),
            meta,
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
        mutates: true,
        work: true,
        title: "修改文件".into(),
        summary_arg: Some("path".into()),
        summary_text: None,
        }
    }
}

// ---------- apply_patch ----------

/// 一个补丁块（unified diff 的 @@ hunk）：起止行号 + 带 tag 的内容行。
#[derive(Debug, Clone)]
pub struct PatchHunk {
    pub old_start: i64,
    pub old_count: i64,
    pub new_start: i64,
    pub new_count: i64,
    pub lines: Vec<(char, String)>, // tag: ' ' | '-' | '+'
}

/// 解析统一 diff 文本为补丁块序列；忽略 @@ 之前的文件头，并校验每块的
/// 旧/新内容行数与 @@ 声明一致（不符即整体报错，不做部分应用）。
///
/// 块头必须是完整的 `@@ -旧起点,旧行数 +新起点,新行数 @@`（行数为 1 时可省略 `,1`）。
/// 以 `@@` 开头却不合本格式的行就地报错，而不是静默跳过——放过它会让后续内容行
/// 全部被当作「@@ 之前的文件头」丢掉，最终只报一句「没有可应用的内容块」，模型
/// 无从知道是块头写错了（实测：某会话连续 4 次因此失败，随后转向 shell 脚本改文件）。
pub fn parse_patch(patch: &str) -> Result<Vec<PatchHunk>, String> {
    let normalized = patch.replace("\r\n", "\n");
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop(); // 补丁常以换行结尾
    }
    let mut hunks: Vec<PatchHunk> = Vec::new();
    let mut current: Option<PatchHunk> = None;
    let header_re = regex::Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();

    for (idx, line) in lines.iter().enumerate() {
        let line = *line;
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
        // 以 @@ 开头却不匹配上面正则 = 块头不合规（典型是裸 `@@`）。必须就地报错：
        // 放过它会让后续内容行全部被当作「@@ 之前的文件头」丢掉，模型只会收到
        // 「没有可应用的内容块」，无从定位是块头格式问题。
        if line.starts_with("@@") {
            return Err(format!(
                "第 {} 行的补丁块头不合规；块头需写成 @@ -旧起点,旧行数 +新起点,新行数 @@（例：@@ -12,3 +12,4 @@）；请修正补丁后重试，不要改用 Python/shell 脚本整体重写文件",
                idx + 1
            ));
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
            return Err(format!("第 {} 个补丁块的旧/新内容行数不符；请修正补丁后重试，不要改用 Python/shell 脚本整体重写文件", i + 1));
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
        return Err("补丁上下文与文件内容不匹配，无法应用；请重新读取文件并修正补丁后重试，不要改用 Python/shell 脚本整体重写文件".into());
    };
    file_lines.splice(pos..pos + old_lines.len(), new_lines.iter().cloned());
    Ok(new_lines.len() as i64 - old_lines.len() as i64)
}

/// 对已存在的文件应用统一 diff 补丁（R1 写工作区）；只改已有文件，不创建新文件。
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
            return err_result("补丁中没有可应用的内容块；块头需写成 @@ -旧起点,旧行数 +新起点,新行数 @@（例：@@ -12,3 +12,4 @@）；请修正补丁后重试，不要改用 Python/shell 脚本整体重写文件");
        }

        // 选择要应用的块（从 1 起）；未指定 hunks 时应用全部
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
        // 行尾与补丁匹配无关（D26）：补丁侧已由 parse_patch 归一为 LF，文件侧同样归一，
        // 否则每行尾部残留 '\r'，逐行比较恒不相等。主导行尾另行记住，写盘时还原。
        let eol = dominant_eol(&text);
        let flat = normalize_eol(&text);
        let mut file_lines: Vec<String> = flat.split('\n').map(str::to_string).collect();
        // delta 累计已应用块造成的行数偏移，供后续块修正匹配位置
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

        // 预演只校验可应用性，不写盘
        if ctx.args.get("dry_run").and_then(Value::as_bool).unwrap_or(false) {
            return ToolResult::ok(format!(
                "补丁预演通过：将应用 {} 个补丁块到 {}。未写入文件。",
                chosen.len(),
                ctx.display
            ));
        }

        // 全部块应用成功后才原子写入（任一块失败则文件保持原样）
        let updated = file_lines.join("\n");
        if let Err(err) = write_file_atomic(std::path::Path::new(&ctx.target), restore_eol(&updated, eol).as_bytes(), 0o644) {
            return err_result(format!("写入失败：{err}"));
        }
        let mut meta = json!({ "path": ctx.target, "eol": eol.as_str() });
        // 差异按「应用前原文 → 应用后全文」计算，而不是拼补丁原文：补丁的行号可能带偏移，
        // 实际落盘的内容以重算为准，界面展示的才是真正改了什么。两侧都用归一形态，
        // 与 fs_edit 一致：文件行尾风格不混进差异。
        ctx.attach_diff(&mut meta, &flat, &updated);
        ToolResult::ok_with_meta(
            format!("已应用 {} 个补丁块到 {}（+{added} −{removed} 行）。", chosen.len(), ctx.display),
            meta,
        )
    }
}

impl ApplyPatch {
    pub fn spec() -> ToolSpec {
        ToolSpec {
        name: "apply_patch".into(),
        description: "对已存在的文件应用统一 diff（unified diff）补丁；支持 dry_run 预演与按块（hunks，从 1 起）选择性应用。块头必须写成 @@ -旧起点,旧行数 +新起点,新行数 @@（例：@@ -12,3 +12,4 @@），裸 @@ 不被接受。".into(),
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
        mutates: true,
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
    use crate::registry::ToolEnv;

    /// 搭一个最小 fs_grep 测试场：临时数据目录写入 files（支持子目录）、
    /// 独立临时目录放测试 db，返回 (数据目录, db 目录, ctx)。
    /// db 单独存放避免被 fs_grep 自身搜到；测试结束调用方清理两个目录。
    fn grep_setup(files: &[(&str, &str)], args: Value) -> (PathBuf, PathBuf, ToolCtx) {
        let uniq = crate::test_util::unique_temp_dir("grep");
        let root = uniq.join("data");
        std::fs::create_dir_all(&root).unwrap();
        for (path, content) in files {
            let p = root.join(path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(p, content).unwrap();
        }
        let db_dir = uniq.join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = Arc::new(craft_store::db::Db::open(&db_dir.join("t.db")).unwrap());
        let ctx = ToolCtx {
            name: "fs_grep".into(),
            args,
            target: root.to_string_lossy().to_string(),
            display: String::new(),
            workspace: root.to_string_lossy().to_string(),
            roots: vec![root.to_string_lossy().to_string()],
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: Arc::new(ToolEnv { data_dir: String::new(), db, shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() }),
        };
        (root, db_dir, ctx)
    }

    fn grep_args(pattern: &str) -> Value {
        json!({ "pattern": pattern })
    }

    #[test]
    fn grep_substring_matches_with_line_numbers() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.txt", "hello world\nbye HELLO\nhi again\n")],
            grep_args("hello"),
        );
        let result = FsGrep.execute(&ctx);
        assert!(!result.is_error);
        assert!(result.output.contains("a.txt:1: hello world"), "got: {}", result.output);
        assert!(!result.output.contains("HELLO"), "大小写敏感不应命中: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_regex_mode_matches_pattern() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.txt", "hello world\nbye\n")],
            json!({ "pattern": "h.l+o", "regex": true }),
        );
        let result = FsGrep.execute(&ctx);
        assert!(result.output.contains("a.txt:1: hello world"), "got: {}", result.output);
        // 无效正则返回错误
        let (_, _, mut bad) = grep_setup(&[], json!({ "pattern": "a(", "regex": true }));
        bad.target = root.to_string_lossy().to_string();
        let err = FsGrep.execute(&bad);
        assert!(err.is_error);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_ignore_case_matches_case_insensitive() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.txt", "HELLO world\nbye\n")],
            json!({ "pattern": "hello", "ignore_case": true }),
        );
        let result = FsGrep.execute(&ctx);
        assert!(result.output.contains("a.txt:1: HELLO world"), "got: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_include_filters_by_glob() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.rs", "fn target_fn() {}\n"), ("b.ts", "const target = 1;\n")],
            json!({ "pattern": "target", "include": "*.rs" }),
        );
        let result = FsGrep.execute(&ctx);
        assert!(result.output.contains("a.rs:"), "got: {}", result.output);
        assert!(!result.output.contains("b.ts"), "include 应过滤非 rs 文件: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_context_returns_surrounding_lines() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.txt", "line1\nline2\nmatch line\nline4\nline5\n")],
            json!({ "pattern": "match line", "context": 1 }),
        );
        let result = FsGrep.execute(&ctx);
        assert!(result.output.contains("a.txt:2- line2"), "before 上下文: {}", result.output);
        assert!(result.output.contains("a.txt:3: match line"), "匹配行: {}", result.output);
        assert!(result.output.contains("a.txt:4- line4"), "after 上下文: {}", result.output);
        assert!(!result.output.contains("line1"), "窗口外不应出现: {}", result.output);
        assert!(!result.output.contains("line5"), "窗口外不应出现: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_max_count_limits_per_file_matches() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.txt", "x\nx\nx\n")],
            json!({ "pattern": "x", "max_count": 2 }),
        );
        let result = FsGrep.execute(&ctx);
        assert_eq!(result.output.lines().count(), 2, "每文件上限: {}", result.output);
        assert_eq!(result.meta.as_ref().unwrap()["count"], 2);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_files_with_matches_lists_paths_only() {
        let (root, db_dir, ctx) = grep_setup(
            &[("a.rs", "needle\n"), ("b.rs", "nothing\n")],
            json!({ "pattern": "needle", "files_with_matches": true }),
        );
        let result = FsGrep.execute(&ctx);
        assert_eq!(result.output, "a.rs", "只列文件路径: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_skips_hidden_files_and_ignored_dirs() {
        let (root, db_dir, ctx) = grep_setup(
            &[
                (".secret.txt", "needle\n"),
                ("node_modules/pkg.js", "needle\n"),
                ("ok.txt", "needle\n"),
            ],
            grep_args("needle"),
        );
        let result = FsGrep.execute(&ctx);
        assert_eq!(result.output, "ok.txt:1: needle", "隐藏文件与忽略目录应跳过: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_respects_gitignore() {
        let (root, db_dir, ctx) = grep_setup(
            &[
                ("generated.log", "needle\n"),
                ("keep.txt", "needle\n"),
                (".gitignore", "generated.log\n"),
            ],
            grep_args("needle"),
        );
        let result = FsGrep.execute(&ctx);
        assert!(result.output.contains("keep.txt"), "got: {}", result.output);
        assert!(!result.output.contains("generated.log"), ".gitignore 应生效: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_stops_at_global_match_limit() {
        let big = "x\n".repeat(220);
        let (root, db_dir, ctx) = grep_setup(&[("big.txt", &big)], grep_args("x"));
        let result = FsGrep.execute(&ctx);
        let meta = result.meta.as_ref().unwrap();
        assert_eq!(meta["count"], 200, "全局上限 200");
        assert_eq!(meta["truncated"], true, "达到上限应标记 truncated");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn grep_skips_binary_files() {
        let uniq = crate::test_util::unique_temp_dir("grep-bin");
        let root = uniq.join("data");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("bin.dat"), b"craft\x00needle\n").unwrap();
        std::fs::write(root.join("text.txt"), b"needle\n").unwrap();
        let db_dir = uniq.join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = Arc::new(craft_store::db::Db::open(&db_dir.join("t.db")).unwrap());
        let ctx = ToolCtx {
            name: "fs_grep".into(),
            args: grep_args("needle"),
            target: root.to_string_lossy().to_string(),
            display: String::new(),
            workspace: root.to_string_lossy().to_string(),
            roots: vec![root.to_string_lossy().to_string()],
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: Arc::new(ToolEnv { data_dir: String::new(), db, shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() }),
        };
        let result = FsGrep.execute(&ctx);
        assert!(result.output.contains("text.txt"), "got: {}", result.output);
        assert!(!result.output.contains("bin.dat"), "含 NUL 的二进制文件应跳过: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn glob_match_wildcard_and_double_star() {
        assert!(match_glob("*.ts", "src/main.ts"));
        assert!(match_glob("**/*.go", "src/a/b.go"));
        assert!(!match_glob("*.ts", "src/main.go"));
        assert!(match_glob("main.rs", "src/main.rs"));
    }

    #[test]
    fn parse_patch_line_count_validation() {
        let patch = "@@ -1,2 +1,2 @@\n-a\n+b\n c\n";
        let hunks = parse_patch(patch).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 3);
        assert!(parse_patch("@@ -1,3 +1,2 @@\n-a\n+b").is_err(), "行数不符必须报错");
    }

    /// 裸 `@@`（无行号范围）必须报出块头格式指引，而不是落到「没有可应用的内容块」。
    /// 回归自实测：模型连续 4 次写成裸 `@@`，只收到空块错误，无从纠正，最终转向 shell 脚本。
    #[test]
    fn parse_patch_bare_hunk_header_reports_format_hint() {
        let err = parse_patch("@@\n-old\n+new\n").unwrap_err();
        assert!(err.contains("块头不合规"), "got: {err}");
        assert!(err.contains("@@ -旧起点,旧行数 +新起点,新行数 @@"), "缺少格式指引: {err}");
        assert!(err.contains("第 1 行"), "缺少行号定位: {err}");
        assert!(!err.contains("没有可应用的内容块"), "不应退化为空块错误: {err}");
        // 护栏必须与另两条失败分支一致
        assert!(err.contains("不要改用 Python/shell 脚本整体重写文件"), "缺少 D10 拦截语: {err}");
    }

    /// 空块分支（补丁完全没有 @@ 时）同样要带格式指引与 D10 拦截语。
    #[test]
    fn apply_patch_empty_hunks_error_carries_script_rewrite_guardrail() {
        let (root, db_dir, ctx) = write_setup("empty-hunks", Some("one\ntwo\n"), json!({ "patch": "no hunks here\n" }));
        let result = ApplyPatch.execute(&ctx);
        assert!(result.is_error, "应失败: {}", result.output);
        assert!(result.output.contains("没有可应用的内容块"), "got: {}", result.output);
        assert!(result.output.contains("@@ -旧起点,旧行数 +新起点,新行数 @@"), "缺少格式指引: {}", result.output);
        assert!(result.output.contains("不要改用 Python/shell 脚本整体重写文件"), "缺少 D10 拦截语: {}", result.output);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn apply_hunk_preferred_position_and_offset_fallback() {
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
    fn apply_hunk_context_mismatch_error() {
        let mut file: Vec<String> = "a\nb".split('\n').map(str::to_string).collect();
        let hunk = parse_patch("@@ -1,1 +1,1 @@\n-nothing-matches\n+new").unwrap().remove(0);
        assert!(apply_hunk(&mut file, &hunk, 0).is_err());
    }

    /// 写类工具的公共测试场：临时工作区（可选预置文件）+ 独立临时 db + 指向目标文件的 ctx。
    /// db 单独存放，避免被工具自身遍历到；调用方负责清理返回的两个目录。
    fn write_setup(tag: &str, initial: Option<&str>, args: Value) -> (PathBuf, PathBuf, ToolCtx) {
        let uniq = crate::test_util::unique_temp_dir(tag);
        let root = uniq.join("data");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("t.txt");
        if let Some(text) = initial {
            std::fs::write(&target, text).unwrap();
        }
        let db_dir = uniq.join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = Arc::new(craft_store::db::Db::open(&db_dir.join("t.db")).unwrap());
        let ctx = ToolCtx {
            name: "fs_write".into(),
            args,
            target: target.to_string_lossy().to_string(),
            display: "t.txt".into(),
            workspace: root.to_string_lossy().to_string(),
            roots: vec![root.to_string_lossy().to_string()],
            target_root: root.to_string_lossy().to_string(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: Arc::new(ToolEnv { data_dir: String::new(), db, shell_tasks: Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: crate::registry::direct_proxy_config() }),
        };
        (root, db_dir, ctx)
    }

    #[test]
    fn fs_edit_meta_carries_line_diff() {
        let (root, db_dir, ctx) = write_setup(
            "diff-edit",
            Some("alpha\nbeta\ngamma\n"),
            json!({ "old_string": "beta", "new_string": "BETA" }),
        );
        let result = FsEdit.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        let diff = &result.meta.as_ref().unwrap()["diff"];
        assert_eq!(diff["additions"], 1, "diff: {diff}");
        assert_eq!(diff["deletions"], 1, "diff: {diff}");
        assert_eq!(diff["truncated"], false);
        let hunks = diff["hunks"].as_array().unwrap();
        assert_eq!(hunks.len(), 1);
        let kinds: Vec<&str> = hunks[0]["lines"].as_array().unwrap().iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"add") && kinds.contains(&"del"), "应含增删行: {kinds:?}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn fs_write_new_file_diff_all_additions() {
        let (root, db_dir, ctx) = write_setup("diff-new", None, json!({ "content": "one\ntwo\n" }));
        let result = FsWrite.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        let diff = &result.meta.as_ref().unwrap()["diff"];
        assert_eq!(diff["additions"], 2, "新建文件整体计为新增: {diff}");
        assert_eq!(diff["deletions"], 0, "diff: {diff}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn fs_write_binary_existing_skips_diff() {
        let (root, db_dir, ctx) = write_setup("diff-bin", Some("craft\u{0}binary"), json!({ "content": "plain text\n" }));
        let result = FsWrite.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        let meta = result.meta.as_ref().unwrap();
        assert!(meta.get("diff").is_none(), "二进制原文件不应产 diff: {meta}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn apply_patch_meta_diff_reflects_applied_content() {
        let (root, db_dir, ctx) = write_setup(
            "diff-patch",
            Some("one\ntwo\nthree\n"),
            json!({ "patch": "@@ -2,1 +2,1 @@\n-two\n+TWO\n" }),
        );
        let result = ApplyPatch.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        let diff = &result.meta.as_ref().unwrap()["diff"];
        assert_eq!(diff["additions"], 1, "diff: {diff}");
        assert_eq!(diff["deletions"], 1, "diff: {diff}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    // ---------- 行尾无关匹配（D26） ----------

    /// CRLF 文件的读回字节（断言落盘行尾用）。
    fn read_bytes(ctx: &ToolCtx) -> String {
        String::from_utf8_lossy(&std::fs::read(&ctx.target).unwrap()).to_string()
    }

    fn assert_all_crlf(ctx: &ToolCtx) {
        let text = read_bytes(ctx);
        assert_eq!(text.matches("\r\n").count(), text.matches('\n').count(), "落盘应全为 CRLF: {text:?}");
    }

    /// 模型经 fs_read 看到的是 LF 形态，故 old_string 通常也是 LF——CRLF 文件必须照样命中。
    #[test]
    fn fs_edit_matches_lf_snippet_in_crlf_file() {
        let (root, db_dir, ctx) = write_setup(
            "eol-edit-match",
            Some("alpha\r\nbeta\r\ngamma\r\n"),
            json!({ "old_string": "beta\ngamma", "new_string": "BETA\nGAMMA" }),
        );
        let result = FsEdit.execute(&ctx);
        assert!(!result.is_error, "CRLF 文件应能用 LF 片段匹配: {}", result.output);
        assert_eq!(read_bytes(&ctx), "alpha\r\nBETA\r\nGAMMA\r\n");
        assert_eq!(result.meta.as_ref().unwrap()["eol"], "crlf");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// 替换不得把整个文件的行尾翻成 LF（那会让「只改一处」变成全文重写）。
    #[test]
    fn fs_edit_preserves_crlf_line_endings() {
        let (root, db_dir, ctx) = write_setup(
            "eol-edit-keep",
            Some("one\r\ntwo\r\nthree\r\n"),
            json!({ "old_string": "two", "new_string": "TWO" }),
        );
        let result = FsEdit.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        let text = read_bytes(&ctx);
        assert!(text.contains("TWO"), "应完成替换: {text:?}");
        assert!(!text.contains("TWO\n"), "替换处不应变成 LF: {text:?}");
        assert_all_crlf(&ctx);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// 纯 LF 文件走归一化再还原是恒等变换，必须是零行为变化。
    #[test]
    fn fs_edit_lf_file_unchanged_by_normalization() {
        let (root, db_dir, ctx) = write_setup(
            "eol-edit-lf",
            Some("alpha\nbeta\ngamma\n"),
            json!({ "old_string": "beta\ngamma", "new_string": "BETA\nGAMMA" }),
        );
        let result = FsEdit.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert_eq!(read_bytes(&ctx), "alpha\nBETA\nGAMMA\n", "纯 LF 文件不应被引入 CR");
        assert_eq!(result.meta.as_ref().unwrap()["eol"], "lf");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// 两处「行尾不同、正文相同」的内容归一后必须判为多处，不能当成唯一片段改错位置。
    #[test]
    fn fs_edit_rejects_multiple_matches_after_normalization() {
        let (root, db_dir, ctx) = write_setup(
            "eol-edit-dup",
            Some("dup\nkeep\r\ndup\r\n"),
            json!({ "old_string": "dup", "new_string": "DUP" }),
        );
        let result = FsEdit.execute(&ctx);
        assert!(result.is_error, "混合行尾下的重复内容仍应被拒绝: {}", result.output);
        assert!(result.output.contains("匹配到 2 处"), "{}", result.output);
        assert_eq!(read_bytes(&ctx), "dup\nkeep\r\ndup\r\n", "拒绝时不得落盘");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// 补丁侧是 LF，CRLF 文件侧也必须能匹配上上下文。
    #[test]
    fn apply_patch_applies_to_crlf_file() {
        let (root, db_dir, ctx) = write_setup(
            "eol-patch-apply",
            Some("one\r\ntwo\r\nthree\r\n"),
            json!({ "patch": "@@ -2,1 +2,1 @@\n-two\n+TWO\n" }),
        );
        let result = ApplyPatch.execute(&ctx);
        assert!(!result.is_error, "CRLF 文件应能吃下 LF 补丁: {}", result.output);
        assert_eq!(read_bytes(&ctx), "one\r\nTWO\r\nthree\r\n");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// 补丁落盘同样保持原文件行尾。
    #[test]
    fn apply_patch_preserves_crlf_line_endings() {
        let (root, db_dir, ctx) = write_setup(
            "eol-patch-keep",
            Some("a\r\nb\r\nc\r\n"),
            json!({ "patch": "@@ -1,3 +1,4 @@\n a\n b\n+inserted\n c\n" }),
        );
        let result = ApplyPatch.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        let text = read_bytes(&ctx);
        assert_eq!(text, "a\r\nb\r\ninserted\r\nc\r\n", "应插入且保持 CRLF");
        assert_eq!(result.meta.as_ref().unwrap()["eol"], "crlf");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// fs_read 不得把 '\r' 带进输出（否则模型复述出去的内容天然带 CR），并告知文件行尾。
    #[test]
    fn fs_read_strips_carriage_return_and_reports_crlf() {
        let (root, db_dir, ctx) = write_setup("eol-read", Some("alpha\r\nbeta\r\n"), json!({}));
        let result = FsRead.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(!result.output.contains('\r'), "输出不应含 CR: {:?}", result.output);
        assert!(result.output.contains("alpha"), "{}", result.output);
        assert_eq!(result.meta.as_ref().unwrap()["eol"], "crlf");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    #[test]
    fn dominant_eol_prefers_majority_and_defaults_to_lf() {
        assert_eq!(dominant_eol("a\r\nb\r\n"), Eol::Crlf);
        assert_eq!(dominant_eol("a\nb\n"), Eol::Lf);
        // 无换行与平局按 LF：不做转换是最安全的一侧
        assert_eq!(dominant_eol("no newline"), Eol::Lf);
        assert_eq!(dominant_eol("a\r\nb\n"), Eol::Lf);
        // 混杂但 CRLF 占多数：判 CRLF
        assert_eq!(dominant_eol("a\r\nb\r\nc\n"), Eol::Crlf);
    }

    /// 回归守卫（D29）：文件超过 MAX_READ_BYTES 时，`lines` 只覆盖前缀，
    /// 历史行为是**完全不提续读**（行号显示到「末尾」，模型以为读完了，实际文件还有内容）。
    /// 现在必须在正文末行给出准确的 offset。
    #[test]
    fn fs_read_hints_next_offset_when_file_exceeds_byte_cap() {
        // 每行 100 字节 × 3000 行 ≈ 300 KB，超过 256 KB 的读取上限
        let body: String = (1..=3000).map(|i| format!("line-{i:04}-{}\n", "x".repeat(88))).collect();
        assert!(body.len() > MAX_READ_BYTES, "测试数据必须超过读取上限");
        let (root, db_dir, ctx) = write_setup("read-byte-cap", Some(&body), json!({}));
        let result = FsRead.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        // 不得落到出口兜底那句
        assert!(
            !result.output.contains("输出已截断"),
            "不应落到出口兜底: {}",
            &result.output[result.output.len().saturating_sub(200)..]
        );
        let hint = result.output.lines().last().unwrap_or_default();
        assert!(hint.contains("使用 offset="), "末行应是续读提示: {hint}");
        let next: usize = hint
            .split("offset=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .expect("解析 offset");
        // 续读起点必须落在截断点之后、且不超过总行数
        assert!(next > 1 && next <= 3000, "offset 越界: {next}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }

    /// 小文件不受影响：未触及字节预算时输出里不应出现任何分页/截断提示。
    #[test]
    fn fs_read_small_file_has_no_pagination_hint() {
        let body: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        let (root, db_dir, ctx) = write_setup("read-small-no-hint", Some(&body), json!({}));
        let result = FsRead.execute(&ctx);
        assert!(!result.is_error, "{}", result.output);
        assert!(!result.output.contains("offset="), "不应有续读提示: {}", result.output);
        assert_eq!(result.output.lines().count(), 10);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&db_dir);
    }
}
