//! 工具框架：集中声明每个工具的静态元数据（名称/描述/Schema/风险/路径参数）；
//! 注册、策略判断与审批展示读同一份声明，新增工具只声明一次。
//! 对应 src/main/tools/registry.ts。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use craft_core::contract::DiffResult;
use craft_core::editor::diff as compute_diff;
use craft_core::platform::{backup_name, write_file_atomic, sha256_hex};
use craft_policy::command::classify_command;
use craft_policy::resolve::{relative, resolve_at, resolve_existing, ErrOutsideWorkspace};
use craft_policy::PolicyRisk;
use craft_store::db::Db;
use craft_store::records::add_backup;

/// 统一输出截断（字节级；可能切坏多字节字符——可接受取舍）。
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
pub const MAX_READ_BYTES: usize = 256 * 1024;
pub const MAX_READ_LINES: usize = 2000;

pub fn truncate_output(text: &str, limit: usize) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= limit {
        return text.to_string();
    }
    // 字节级截断后 lossy 转换（可能切坏多字节字符，与 TS 版取舍一致）
    let head = String::from_utf8_lossy(&bytes[..limit]);
    format!("{head}\n…（输出已截断，原始长度 {} 字节）", bytes.len())
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub code: String,
    pub meta: Option<Value>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: false, code: String::new(), meta: None }
    }

    pub fn ok_with_meta(output: impl Into<String>, meta: Value) -> Self {
        Self { output: output.into(), is_error: false, code: String::new(), meta: Some(meta) }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: true, code: "tool_error".into(), meta: None }
    }

    pub fn err_code(output: impl Into<String>, code: &str) -> Self {
        Self { output: output.into(), is_error: true, code: code.into(), meta: None }
    }
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub risk: PolicyRisk,
    /// 路径参数名（执行前统一解析并做越界校验）
    pub path_arg: Option<String>,
    /// 命令参数名（执行前重算风险）
    pub command_arg: Option<String>,
    /// Work 模式可用
    pub work: bool,
    pub title: String,
    pub summary_arg: Option<String>,
    pub summary_text: Option<String>,
}

/// 工具执行环境：备份出口与共享存储（Registry 构造 ctx 时携带）。
pub struct ToolEnv {
    pub backup_dir: String,
    pub db: Arc<Db>,
}

impl ToolEnv {
    /// 写前备份：目标文件当前大小 + 备份文件落盘 + backups 表登记。
    pub fn backup(&self, target: &str, data: &[u8]) -> Result<(), String> {
        if self.backup_dir.is_empty() || target.is_empty() {
            return Ok(());
        }
        let size = std::fs::metadata(target).map(|m| m.len() as i64).unwrap_or(0);
        let backup_path = std::path::Path::new(&self.backup_dir).join(backup_name(std::path::Path::new(target)));
        std::fs::create_dir_all(&self.backup_dir).map_err(|e| format!("创建备份目录失败：{e}"))?;
        set_dir_mode_0700(&self.backup_dir);
        write_file_atomic(&backup_path, data, 0o600).map_err(|e| format!("备份写入失败：{e}"))?;
        self.db
            .with(|conn| {
                add_backup(
                    conn,
                    craft_store::records::BackupRecord {
                        path: target,
                        backup_path: &backup_path.to_string_lossy(),
                        hash: &sha256_hex(data),
                        size,
                    },
                )
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// 额外路径解析（git paths 等）：仅允许主要目录。
    pub fn resolve_extra(&self, roots: &[String], rel: &str) -> Result<(String, String), ErrOutsideWorkspace> {
        let main = roots.first().cloned().unwrap_or_default();
        resolve_at(&main, rel).map(|(abs, _)| {
            let display = relative(&main, &abs);
            (abs, display)
        })
    }
}

#[cfg(unix)]
fn set_dir_mode_0700(dir: &str) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_dir: &str) {}

/// 工具调用上下文（assess 阶段准备，execute 阶段消费）。
pub struct ToolCtx {
    pub name: String,
    pub args: Value,
    /// 主路径解析结果（有 path_arg 时）
    pub target: String,
    pub display: String,
    pub workspace: String,
    pub roots: Vec<String>,
    pub target_root: String,
    /// 参数/路径校验失败原文（置位后 Execute 不再执行）
    pub err: Option<String>,
    pub env: Arc<ToolEnv>,
}

impl ToolCtx {
    pub fn str_arg(&self, key: &str) -> String {
        self.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    pub fn int_arg(&self, key: &str, fallback: i64) -> i64 {
        self.args.get(key).and_then(Value::as_i64).unwrap_or(fallback)
    }

    pub fn compute_diff(&self, old_text: &str, new_text: &str) -> DiffResult {
        compute_diff(old_text, new_text)
    }
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// 同步执行；调用方（壳层）负责放入阻塞线程池。
    fn execute(&self, ctx: &ToolCtx) -> ToolResult;
}

/// 只读评估结果。
pub struct Assessment {
    pub risk: PolicyRisk,
    pub reason: String,
    pub target: String,
    pub prepared: Option<ToolCtx>,
    pub exists: bool,
}

const IGNORED_DIRS: &[&str] = &[
    ".git", "node_modules", "dist", "bin", ".task", "vendor", "__pycache__", ".venv", "build", ".next", ".cache",
];

pub fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.contains(&name.to_lowercase().as_str())
}

pub fn human_size(bytes: i64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = -1i32;
    while value >= 1024.0 && unit < (UNITS.len() as i32 - 1) {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit as usize])
}

pub struct Registry {
    tools: HashMap<String, Box<dyn Tool>>,
    env: Arc<ToolEnv>,
}

impl Registry {
    pub fn new(backup_dir: &str, db: Arc<Db>) -> Self {
        Self {
            tools: HashMap::new(),
            env: Arc::new(ToolEnv { backup_dir: backup_dir.to_string(), db }),
        }
    }

    /// MCP 动态注册：同名替换。
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.spec().name, tool);
    }

    pub fn unregister(&mut self, name: &str) {
        self.tools.remove(name);
    }

    pub fn spec_for(&self, name: &str) -> Option<ToolSpec> {
        self.tools.get(name).map(|t| t.spec())
    }

    /// 按名称排序——保证提示词工具顺序稳定。
    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self.tools.values().map(|t| t.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    /// work 模式只保留 spec.work 或 mcp__ 前缀。
    pub fn specs_for_mode(&self, mode: &str) -> Vec<ToolSpec> {
        if mode != "work" {
            return self.specs();
        }
        self.specs().into_iter().filter(|s| s.work || s.name.starts_with("mcp__")).collect()
    }

    pub fn allows(&self, mode: &str, name: &str) -> bool {
        self.specs_for_mode(mode).iter().any(|s| s.name == name)
    }

    /// 只读评估：解析路径与命令分类，不产生副作用。
    pub fn assess(&self, roots: &[String], name: &str, args: &Value) -> Assessment {
        let Some(tool) = self.tools.get(name) else {
            return Assessment {
                risk: PolicyRisk::R2,
                reason: "执行命令".into(),
                target: String::new(),
                prepared: None,
                exists: false,
            };
        };
        let spec = tool.spec();
        let mut risk = spec.risk;
        let mut reason = String::new();
        let mut target = String::new();
        let mut display = String::new();
        let mut target_root = String::new();
        let mut err: Option<String> = None;

        if let Some(command_arg) = &spec.command_arg {
            let command = args.get(command_arg).and_then(Value::as_str).unwrap_or_default().to_string();
            let (cls_risk, cls_reason) = classify_command(&command);
            risk = cls_risk;
            reason = cls_reason.to_string();
            target = command;
        }

        if let Some(path_arg) = &spec.path_arg {
            let rel_raw = args.get(path_arg).and_then(Value::as_str).unwrap_or_default().trim().to_string();
            let rel_raw = if rel_raw.is_empty() { ".".to_string() } else { rel_raw };
            match resolve_existing(roots, &rel_raw) {
                Ok(resolved) => {
                    target = resolved.abs.clone();
                    display = relative(roots.first().map(String::as_str).unwrap_or_default(), &resolved.abs);
                    target_root = resolved.root;
                    // git 系工具只支持主要目录（保持「主要目录仓库」语义）
                    if name.starts_with("git_") && resolved.root_index != 0 {
                        risk = PolicyRisk::R3;
                        reason = format!("{display} 属于附加目录，Git 操作仅支持主要目录");
                    }
                }
                Err(path_err) => {
                    // 路径越界：风险提到 R3，Execute 不再执行
                    risk = PolicyRisk::R3;
                    reason = path_err.to_string();
                    err = Some(path_err.to_string());
                }
            }
        }

        let ctx = ToolCtx {
            name: name.to_string(),
            args: args.clone(),
            target,
            display,
            workspace: roots.first().cloned().unwrap_or_default(),
            roots: roots.to_vec(),
            target_root,
            err,
            env: Arc::clone(&self.env),
        };

        Assessment {
            risk,
            reason: if reason.is_empty() { reason_for_risk(spec.risk).to_string() } else { reason },
            target: ctx.target.clone(),
            prepared: Some(ctx),
            exists: true,
        }
    }

    /// 统一执行出口：err 置位直接返回错误；输出统一截断。
    pub fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let Some(tool) = self.tools.get(&ctx.name) else {
            return ToolResult::err_code(format!("未知工具：{}", ctx.name), "unknown_tool");
        };
        if let Some(err) = &ctx.err {
            return ToolResult::err_code(err.clone(), "tool_error");
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.execute(ctx)));
        match result {
            Ok(mut result) => {
                result.output = truncate_output(&result.output, MAX_OUTPUT_BYTES);
                result
            }
            Err(_) => ToolResult::err_code("工具执行异常", "tool_error"),
        }
    }
}

fn reason_for_risk(risk: PolicyRisk) -> &'static str {
    craft_policy::reason_for(risk)
}

/// 生成分页提示。
pub fn more_hint(remaining: usize) -> String {
    format!("…（还有 {remaining} 行，使用 offset 继续读取）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 输出截断_字节级并标注原始长度() {
        let short = truncate_output("hello", MAX_OUTPUT_BYTES);
        assert_eq!(short, "hello");
        let long = "x".repeat(MAX_OUTPUT_BYTES + 10);
        let truncated = truncate_output(&long, MAX_OUTPUT_BYTES);
        assert!(truncated.contains("输出已截断"));
        assert!(truncated.contains(&format!("原始长度 {} 字节", MAX_OUTPUT_BYTES + 10)));
    }

    #[test]
    fn 人可读大小() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn 忽略目录表() {
        assert!(is_ignored_dir(".git"));
        assert!(is_ignored_dir("NODE_MODULES"));
        assert!(!is_ignored_dir("src"));
    }
}
