//! 工具框架：集中声明每个工具的静态元数据（名称/描述/Schema/风险/路径参数）。
//!
//! 调用分两阶段接缝：[`Registry::assess`] 只读评估（解析路径、命令分类、预构建
//! [`ToolCtx`]，供策略与审批展示使用，无副作用）；[`Registry::execute`] 统一执行
//! 出口（校验评估期错误、panic 隔离、输出截断）。风险等级语义（R0 读 / R1 写工作区 /
//! R2 执行 / R3 网络越界 / R4 破坏性）见 craft-policy。新增工具只声明一次，
//! 注册、策略判断与审批展示读同一份声明。

use std::collections::HashMap;
use std::sync::Mutex;
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

/// 工具输出统一截断上限（字节）：防止单次工具输出撑爆上下文窗口。
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
/// fs_read 单次读取的原始字节上限：超限文件只读前缀，剩余靠 offset 分页取。
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// fs_read 单次返回的最大行数。
pub const MAX_READ_LINES: usize = 2000;

/// 字节级截断并追加提示；可能切坏末尾多字节字符（与 TS 版一致的取舍）。
pub fn truncate_output(text: &str, limit: usize) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= limit {
        return text.to_string();
    }
    // 字节级截断后 lossy 转换（可能切坏多字节字符，与 TS 版取舍一致）
    let head = String::from_utf8_lossy(&bytes[..limit]);
    format!("{head}\n…（输出已截断，原始长度 {} 字节）", bytes.len())
}

/// 单次工具执行的结果；`code` 供上层区分错误类别（空串表示成功无码）。
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub code: String,
    /// 附加结构化数据（如解析后的路径、计数），供事件与界面使用
    pub meta: Option<Value>,
}

impl ToolResult {
    /// 成功结果，无 meta。
    pub fn ok(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: false, code: String::new(), meta: None }
    }

    /// 成功结果，附带 meta。
    pub fn ok_with_meta(output: impl Into<String>, meta: Value) -> Self {
        Self { output: output.into(), is_error: false, code: String::new(), meta: Some(meta) }
    }

    /// 失败结果，使用通用错误码 `tool_error`。
    pub fn err(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: true, code: "tool_error".into(), meta: None }
    }

    /// 失败结果，携带自定义错误码。
    pub fn err_code(output: impl Into<String>, code: &str) -> Self {
        Self { output: output.into(), is_error: true, code: code.into(), meta: None }
    }
}

/// 工具的静态声明：spec 是「唯一事实」，策略评估、审批卡片与提示词工具列表都读它。
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// 参数 JSON Schema（声明给模型的调用约束）
    pub schema: Value,
    /// 基础风险等级；参数可提升（路径越界 → R3，命令按分类重算）
    pub risk: PolicyRisk,
    /// 路径参数名（执行前统一解析并做越界校验）
    pub path_arg: Option<String>,
    /// 命令参数名（执行前重算风险）
    pub command_arg: Option<String>,
    /// Work 模式可用
    pub work: bool,
    /// 审批卡片标题；空串表示无需独立标题（按 summary 展示）
    pub title: String,
    /// 从哪个参数取审批摘要
    pub summary_arg: Option<String>,
    /// 固定摘要文本（无合适参数时使用，如 "git status"）
    pub summary_text: Option<String>,
}

/// 工具执行环境：备份出口与共享存储（Registry 构造 ctx 时携带）。
///
/// 写前备份是 Agent 写入链的安全兜底（自由档位下唯一的事后恢复手段）；
/// expectedHash 防覆盖只作用于用户编辑保存链（src-tauri services），工具链不使用。
pub struct ToolEnv {
    /// 备份根目录（`~/.craftwork/backups/`）；空串表示禁用备份（测试场景）
    pub backup_dir: String,
    /// 应用数据目录（`~/.craftwork/`）；技能等目录型资源由工具按需推导（空串 = 测试场景）
    pub data_dir: String,
    pub db: Arc<Db>,
}

impl ToolEnv {
    /// 写前备份：备份文件落盘 + backups 表登记（含原始大小与 SHA-256）。
    ///
    /// 备份目录 0700、文件 0600，与密钥文件同一保护级别，防止其他本机用户读取；
    /// backup_dir 未配置（空串）时静默跳过——纯展示类调用（todo_write、测试）无物可备份。
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

    /// 额外路径解析（git paths 等）：仅允许主要目录——Git 语义（含暂存区）只作用于主要目录仓库。
    pub fn resolve_extra(&self, roots: &[String], rel: &str) -> Result<(String, String), ErrOutsideWorkspace> {
        let main = roots.first().cloned().unwrap_or_default();
        resolve_at(&main, rel).map(|(abs, _)| {
            let display = relative(&main, &abs);
            (abs, display)
        })
    }
}

/// 备份目录权限收紧到仅当前用户（非 unix 平台无 POSIX mode 语义，空实现）。
#[cfg(unix)]
fn set_dir_mode_0700(dir: &str) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_dir: &str) {}

/// 工具调用上下文：assess 阶段准备，execute 阶段消费。
#[derive(Clone)]
pub struct ToolCtx {
    pub name: String,
    /// 原始参数（未经工具自行解析）
    pub args: Value,
    /// 主路径解析后的绝对路径（声明了 path_arg 时；空串表示无路径参数）
    pub target: String,
    /// 相对主要目录的展示路径（错误消息与审批摘要用）
    pub display: String,
    /// 主要目录（会话工作目录）
    pub workspace: String,
    /// 全部源文件夹（主要目录在前）
    pub roots: Vec<String>,
    /// 目标实际落到的根（存在优先解析到附加目录时与 workspace 不同）
    pub target_root: String,
    /// 评估期校验失败原文（置位后 execute 直接返回错误，不再执行工具）
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

/// 内置工具统一接口：spec 提供静态声明，execute 同步执行（不内置异步）。
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// 同步执行；调用方（壳层）负责放入阻塞线程池。
    fn execute(&self, ctx: &ToolCtx) -> ToolResult;
}

/// 只读评估结果：risk/reason 供审批流程，prepared 携带预构建上下文供 execute 复用。
pub struct Assessment {
    pub risk: PolicyRisk,
    /// 风险说明（路径越界、命令分类或基础等级的默认文案）
    pub reason: String,
    /// 评估出的目标（绝对路径或命令原文），供审批卡片展示
    pub target: String,
    /// 预构建的执行上下文；工具不存在时为 None
    pub prepared: Option<ToolCtx>,
    /// 工具是否已注册
    pub exists: bool,
}

/// 遍历与列表时默认忽略的目录名（大小写不敏感）。
const IGNORED_DIRS: &[&str] = &[
    ".git", "node_modules", "dist", "bin", ".task", "vendor", "__pycache__", ".venv", "build", ".next", ".cache",
];

pub fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.contains(&name.to_lowercase().as_str())
}

/// 字节数格式化为人类可读大小（如 2.0 KB、3.0 MB）。
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

/// 工具注册表：持有全部工具实例与共享 [`ToolEnv`]，并提供 assess / execute 接缝。
pub struct Registry {
    tools: Mutex<HashMap<String, Box<dyn Tool>>>,
    env: Arc<ToolEnv>,
}

impl Registry {
    /// backup_dir 传 `~/.craftwork/backups/`；空串则所有写前备份静默跳过。
    /// data_dir 传 `~/.craftwork/`，供技能等目录型资源推导。
    pub fn new(backup_dir: &str, data_dir: &str, db: Arc<Db>) -> Self {
        Self {
            tools: Mutex::new(HashMap::new()),
            env: Arc::new(ToolEnv { backup_dir: backup_dir.to_string(), data_dir: data_dir.to_string(), db }),
        }
    }

    /// 注册工具；MCP 动态注册同名替换。
    pub fn register(&self, tool: Box<dyn Tool>) {
        self.tools.lock().unwrap().insert(tool.spec().name, tool);
    }

    /// 注销工具（MCP 服务器断开时移除其工具）。
    pub fn unregister(&self, name: &str) {
        self.tools.lock().unwrap().remove(name);
    }

    pub fn spec_for(&self, name: &str) -> Option<ToolSpec> {
        self.tools.lock().unwrap().get(name).map(|t| t.spec())
    }

    /// 按名称排序——保证提示词工具顺序稳定。
    pub fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = self.tools.lock().unwrap().values().map(|t| t.spec()).collect();
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

    /// 工具在指定模式下是否可用（spec_for_mode 过滤的布尔版）。
    pub fn allows(&self, mode: &str, name: &str) -> bool {
        self.specs_for_mode(mode).iter().any(|s| s.name == name)
    }

    /// 只读评估：解析路径与命令分类，不产生副作用。
    pub fn assess(&self, roots: &[String], name: &str, args: &Value) -> Assessment {
        let tools_guard = self.tools.lock().unwrap();
        let Some(tool) = tools_guard.get(name) else {
            // 未知工具按 R2（执行类）保守兜底，确保仍走审批而不被静默放行
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
        drop(tools_guard);

        Assessment {
            risk,
            reason: if reason.is_empty() { reason_for_risk(spec.risk).to_string() } else { reason },
            target: ctx.target.clone(),
            prepared: Some(ctx),
            exists: true,
        }
    }

    /// 统一执行出口：err 置位直接返回错误；panic 隔离；输出统一截断。
    pub fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let tools_guard = self.tools.lock().unwrap();
        let Some(tool) = tools_guard.get(&ctx.name) else {
            return ToolResult::err_code(format!("未知工具：{}", ctx.name), "unknown_tool");
        };
        // 评估期发现路径越界/参数非法：不执行，落回统一错误
        if let Some(err) = &ctx.err {
            return ToolResult::err_code(err.clone(), "tool_error");
        }
        // 工具 panic 不允许拖垮调用线程（Agent 循环），转为普通错误结果
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
    fn truncate_output_byte_level_with_original_length() {
        let short = truncate_output("hello", MAX_OUTPUT_BYTES);
        assert_eq!(short, "hello");
        let long = "x".repeat(MAX_OUTPUT_BYTES + 10);
        let truncated = truncate_output(&long, MAX_OUTPUT_BYTES);
        assert!(truncated.contains("输出已截断"));
        assert!(truncated.contains(&format!("原始长度 {} 字节", MAX_OUTPUT_BYTES + 10)));
    }

    #[test]
    fn human_readable_size() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn ignored_dirs_table() {
        assert!(is_ignored_dir(".git"));
        assert!(is_ignored_dir("NODE_MODULES"));
        assert!(!is_ignored_dir("src"));
    }
}
