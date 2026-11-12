//! 工具框架：集中声明每个工具的静态元数据（名称/描述/Schema/风险/路径参数）。
//!
//! 调用分两阶段接缝：[`Registry::assess`] 只读评估（解析路径、命令分类、预构建
//! [`ToolCtx`]，供策略与审批展示使用，无副作用）；[`Registry::execute`] 统一执行
//! 出口（校验评估期错误、panic 隔离、输出按预算裁决）。风险等级语义（R0 读 / R1 写工作区 /
//! R2 执行 / R3 网络越界 / R4 破坏性）见 craft-policy。新增工具只声明一次，
//! 注册、策略判断与审批展示读同一份声明。
//!
//! ## 输出预算（D29）
//!
//! 出口的字节上限**按工具可调**（[`Tool::output_ceiling`]，默认 [`MAX_OUTPUT_BYTES`]），
//! 且区分「谁截的」：
//!
//! - 工具自己按功能上限截过（`fs_read` 的 256 KiB / 2000 行、`skill` 的 100 KB、
//!   `bash_output` 的 64 KiB 请求量），就在 [`ToolResult::truncation`] 里**申报**丢弃量
//!   与文案，出口不再二次截断——只要不超 `output_ceiling`。
//! - 没申报的按 `output_ceiling` 截断，并追加唯一的「原始长度」提示。
//!
//! 为什么要有这套申报：历史上内外两层各自无条件按 32 KiB 截断，内层追加的标记把长度
//! 顶过上限、又被外层切掉，外层再拿「截断后的长度」当原始长度报给模型；同时
//! `fs_read` 声明的 2000 行被静默砍到约 1000 行且不给准确 offset。丢弃量必须由**截断方**
//! 算准并往上带，不能由上层反推。

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::Arc;

use serde_json::Value;

use craft_core::contract::DiffResult;
use craft_core::editor::diff as compute_diff;
use craft_core::platform::{backup_name, write_file_atomic, sha256_hex};
use craft_policy::command::classify_command;
use craft_policy::resolve::{relative, resolve_at, resolve_existing_with_temp, ErrOutsideWorkspace};
use craft_policy::PolicyRisk;
use craft_store::db::Db;
use craft_store::records::add_backup;

/// 工具输出统一截断上限（字节）：防止单次工具输出撑爆上下文窗口。
///
/// 这是**默认**值；个别工具按自己的功能边界覆盖它（见 [`Tool::output_ceiling`]，D29）。
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
/// 工具结果 `meta.diff` 的序列化预算（字节）：行级差异随事件落库，一次巨大改动不能把
/// 事件表撑大。超限时只保留增删统计、置空 hunks（见 `ToolCtx::attach_diff`）。
pub const MAX_DIFF_META_BYTES: usize = 64 * 1024;
/// fs_read 单次读取的原始字节上限：超限文件只读前缀，剩余靠 offset 分页取。
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// fs_read 单次返回的最大行数。
pub const MAX_READ_LINES: usize = 2000;

/// 一次截断的结果：保留的正文 + 被丢弃的字节数。
///
/// `dropped_bytes == 0` 表示未截断。丢弃量必须由截断方算出来并往上带，不能由上层
/// 拿「截断后的长度」反推——那正是历史上「原始长度」报成 32793 的原因（D29）。
#[derive(Debug, Clone, PartialEq)]
pub struct Truncated {
    pub text: String,
    pub dropped_bytes: usize,
}

/// 按字节上限保留前缀，切点回退到 UTF-8 字符边界。
///
/// 回退规则：切点落在多字节字符中间时向前退到该字符首字节。历史上这里用
/// `from_utf8_lossy` 硬切，末尾会替出 U+FFFD；对模型来说正文结尾出现乱码字符比少一个
/// 字符更糟（会让它以为原文如此）。
pub fn keep_head(text: &str, limit: usize) -> Truncated {
    let bytes = text.as_bytes();
    if bytes.len() <= limit {
        return Truncated { text: text.to_string(), dropped_bytes: 0 };
    }
    let mut end = limit;
    // UTF-8 续字节恒为 10xxxxxx；从切点往回退到字符首字节
    while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    Truncated { text: text[..end].to_string(), dropped_bytes: bytes.len() - end }
}

/// 字节级截断并追加提示（保留给单点调用方与既有测试的入口）。
///
/// 新代码优先用 [`keep_head`] + [`TruncationInfo`]：把「丢了多少字节」交给上层统一渲染，
/// 避免内外两层各写一份标记文案（见 `Registry::execute` 的分流）。
pub fn truncate_output(text: &str, limit: usize) -> String {
    let truncated = keep_head(text, limit);
    if truncated.dropped_bytes == 0 {
        return truncated.text;
    }
    let original = text.len();
    format!("{}\n…（输出已截断，原始长度 {original} 字节）", truncated.text)
}

/// 工具自己申报的截断事实：谁截的谁申报，执行出口据此不再二次截断。
///
/// 为什么需要这个申报：`Registry::execute` 与各工具各有一个字节上限，历史上两边都
/// 无条件截断，于是内层追加的标记被外层切掉、外层报的「原始长度」是内层截断后的长度。
/// 申报把「我已经截过了、这是真实丢弃量」变成 interface 的一部分（D29）。
#[derive(Debug, Clone, PartialEq)]
pub struct TruncationInfo {
    /// 被丢弃的字节数（工具自己算出来的真实值）
    pub dropped_bytes: usize,
    /// 面向模型的说明（产生方负责唯一文案）
    pub note: String,
}

impl TruncationInfo {
    /// 由截断结果构造；未截断时返回 None。
    pub fn from_truncated(truncated: &Truncated, note: impl Into<String>) -> Option<Self> {
        (truncated.dropped_bytes > 0).then(|| Self { dropped_bytes: truncated.dropped_bytes, note: note.into() })
    }
}

/// 单次工具执行的结果；`code` 供上层区分错误类别（空串表示成功无码）。
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub code: String,
    /// 附加结构化数据（如解析后的路径、计数），供事件与界面使用
    pub meta: Option<Value>,
    /// 工具返回的图片等媒体（目前只有 MCP 工具会产生）。
    ///
    /// 为什么与 `output` 分开：`output` 是模型可见的文本，会被 32KB 截断；
    /// 图片一旦被当成文本塞进去就会撑爆上下文且完全不可读。媒体走独立通道：
    /// Runner 落盘到附件目录、挂到 tool 消息的 `attachments`，模型侧由
    /// `Attachment` 携带 base64（各适配器按协议能力投影），界面侧懒加载显示。
    pub media: Vec<ToolMedia>,
    /// 本工具自己已实施的截断申报（`None` = 未截断）。
    ///
    /// 设置后 `Registry::execute` 不再二次截断正文——标记文案由产生方负责一份。
    /// 见 [`TruncationInfo`] 与 D29。
    pub truncation: Option<TruncationInfo>,
}

/// 工具产生的媒体项（base64 内容 + MIME）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolMedia {
    /// 展示用文件名（如 `mcp-image-1.png`）
    pub name: String,
    pub mime: String,
    /// base64 编码的内容（不重复编解码：MCP 协议本身即 base64）
    pub base64: String,
    /// 原始字节数（预算判定与界面展示用）
    pub size: usize,
}

impl ToolResult {
    /// 成功结果，无 meta、无媒体。
    pub fn ok(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: false, code: String::new(), meta: None, media: Vec::new(), truncation: None }
    }

    /// 成功结果，附带 meta。
    pub fn ok_with_meta(output: impl Into<String>, meta: Value) -> Self {
        Self { output: output.into(), is_error: false, code: String::new(), meta: Some(meta), media: Vec::new(), truncation: None }
    }

    /// 失败结果，使用通用错误码 `tool_error`。
    pub fn err(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: true, code: "tool_error".into(), meta: None, media: Vec::new(), truncation: None }
    }

    /// 失败结果，携带自定义错误码。
    pub fn err_code(output: impl Into<String>, code: &str) -> Self {
        Self { output: output.into(), is_error: true, code: code.into(), meta: None, media: Vec::new(), truncation: None }
    }

    /// 附加媒体（链式，构造后调用）。
    pub fn with_media(mut self, media: Vec<ToolMedia>) -> Self {
        self.media = media;
        self
    }

    /// 申报本工具已做的截断（链式，构造后调用）。`None` 不改变任何东西。
    pub fn with_truncation(mut self, truncation: Option<TruncationInfo>) -> Self {
        if truncation.is_some() {
            self.truncation = truncation;
        }
        self
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
    /// 是否具备写语义（会改动工作区、文件系统或外部状态）。计划模式的硬门卡靠它判定，
    /// 不看风险等级——`fs_write` / `fs_edit` / `apply_patch` 都是 R1（单文件改动风险不高），
    /// 按等级判定会把计划模式下最该拦的文件写入漏掉。
    /// 门卡只在分级档启用（自由档 = 放行一切操作，D8 2026-11-10 修订），判定方是 Runner；
    /// 本字段同时也是计划软闸门的写类调用计数依据。
    /// 未标注者一律 false，但 Registry 对未知工具按「可能写」保守兜底（见 Registry::is_mutating）。
    pub mutates: bool,
    /// Work 模式可用
    pub work: bool,
    /// 人类可读标题（中文短语，如「修改文件」「执行命令」）：对话流的工具行、审批卡片
    /// 标题与桌面通知都用它；空串表示回退为工具名（MCP 动态工具没有内置标题）。
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
    /// 后台命令任务注册表（`shell_exec` 的 run_in_background 与超时转后台共用）。
    /// 与 Registry 同寿命：壳层装配一份，工具经 ctx.env 取用——不做全局单例，
    /// 否则测试之间会互相污染任务。
    pub shell_tasks: Arc<crate::shell_tasks::ShellTasks>,
    /// 出站代理配置解析器（D14：出站请求一律经 craft-core::network 统一解析，
    /// 代理与 UA 不绕过）。每次发起出站请求时求值——改设置立即生效；测试场景传
    /// 直连闭包（`direct_proxy_config`）。
    pub proxy_config: Arc<dyn Fn() -> craft_core::network::ProxyConfig + Send + Sync>,
}

/// 出站代理解析器（从 DB settings 读取，D14）：壳层装配 ToolEnv 与 Registry 生产
/// 构造共用同一读取语义——无持久化设置回落 system 模式；custom 地址非法按直连兜底
/// （禁代理比走错代理安全）。每次求值即时读库：改设置下一轮生效。
pub fn db_proxy_config(db: &Arc<Db>) -> Arc<dyn Fn() -> craft_core::network::ProxyConfig + Send + Sync> {
    let db = Arc::clone(db);
    Arc::new(move || {
        let parsed: Option<(String, String)> = craft_store::records::get_setting(&db.lock_conn(), "app")
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value::<craft_core::contract::Settings>(v).ok())
            .map(|s| (s.network_proxy_mode, s.network_proxy_url));
        let (mode, url) = parsed.unwrap_or_else(|| ("system".into(), String::new()));
        craft_core::network::ProxyConfig::from_settings(&mode, &url).unwrap_or(craft_core::network::ProxyConfig {
            mode: craft_core::network::ProxyMode::None,
            effective_url: None,
        })
    })
}

/// 测试用的直连代理解析器：显式 None 模式（清空环境探测，出站确定性不依赖本机代理）。
pub fn direct_proxy_config() -> Arc<dyn Fn() -> craft_core::network::ProxyConfig + Send + Sync> {
    Arc::new(|| craft_core::network::ProxyConfig { mode: craft_core::network::ProxyMode::None, effective_url: None })
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
    ///
    /// 展示路径直接取 `resolve_at` 的产出（相对**解析后真实根**的相对路径），
    /// 不能拿词法根再走一遍 `relative`：根经符号链接解析后形态不同（macOS 的
    /// `/var` → `/private/var`），前缀错位会让 `relative` 回落成绝对路径——
    /// git paths 的路径限定提交因此全部失配，报「nothing to commit」。
    pub fn resolve_extra(&self, roots: &[String], rel: &str) -> Result<(String, String), ErrOutsideWorkspace> {
        let main = roots.first().cloned().unwrap_or_default();
        resolve_at(&main, rel)
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
    /// 本次调用是否处于计划模式（由 Runner 在准备 ctx 时填入）。
    /// 工具层不持有计划态，但这个 bool 让少数工具能做「计划态专属」的自我校验：
    /// todo_write 据此要求待办首项为「退出计划模式」（计划模式下的第一条待办就该是它）。
    pub plan_active: bool,
    /// 会话 id（后台命令任务按会话归属：`bash_output` / `kill_shell` 只认本会话的任务，
    /// 任务输出目录也按它切分）。空串表示无会话上下文（测试 / 内部调用）。
    pub thread_id: String,
    pub env: Arc<ToolEnv>,
}

impl ToolCtx {
    pub fn str_arg(&self, key: &str) -> String {
        self.args.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    /// 后台任务归属键：会话 id；无会话上下文时用工作区名兜底（测试与内部调用）。
    pub fn thread_scope_id(&self) -> String {
        if !self.thread_id.is_empty() {
            return self.thread_id.clone();
        }
        let leaf = std::path::Path::new(&self.workspace)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if leaf.is_empty() {
            "default".to_string()
        } else {
            format!("ws-{leaf}")
        }
    }

    pub fn int_arg(&self, key: &str, fallback: i64) -> i64 {
        self.args.get(key).and_then(Value::as_i64).unwrap_or(fallback)
    }

    pub fn compute_diff(&self, old_text: &str, new_text: &str) -> DiffResult {
        compute_diff(old_text, new_text)
    }

    /// 把行级差异挂到工具结果的 `meta.diff` 上（界面工具卡展开时渲染，决策 D21）。
    ///
    /// 三个取舍都写在这里，调用方不必各自判断：
    /// - 新旧文本相等时不写字段——界面据此区分「没有变更」与「变更统计为 0」；
    /// - 序列化超过 [`MAX_DIFF_META_BYTES`] 时降级为统计形态（清空 hunks 并置 `truncated`），
    ///   差异本体不落事件表；
    /// - 写入失败（理论上不会发生）静默跳过，diff 是展示增强，不能让它把工具结果变成错误。
    pub fn attach_diff(&self, meta: &mut Value, old_text: &str, new_text: &str) {
        if old_text == new_text {
            return;
        }
        let diff = self.compute_diff(old_text, new_text);
        let Ok(value) = serde_json::to_value(&diff) else {
            return;
        };
        // 预算按序列化后的字节算：diff 的紧凑 JSON 再序列化一次只为量体积，
        // 比按行数估算可靠（长行与深缩进都会让行数严重低估实际体积）。
        let over_budget = serde_json::to_string(&value).map(|text| text.len() > MAX_DIFF_META_BYTES).unwrap_or(false);
        meta["diff"] = if over_budget {
            // 降级为统计形态：界面显示「差异过大，仅显示统计」，行内容不落库。
            serde_json::json!({
                "additions": diff.additions,
                "deletions": diff.deletions,
                "hunks": [],
                "truncated": true,
            })
        } else {
            value
        };
    }
}

/// 内置工具统一接口：spec 提供静态声明，execute 同步执行（不内置异步）。
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// 同步执行；调用方（壳层）负责放入阻塞线程池。
    fn execute(&self, ctx: &ToolCtx) -> ToolResult;

    /// 审批卡「原因」的覆盖值（默认 `None` = 用风险等级的通用文案）。
    ///
    /// 内置工具的风险等级已能自解释（「修改文件」），但 MCP 这类动态工具需要说明
    /// 「经哪个外部服务器执行」，否则用户看到的只是笼统的「执行命令」。
    fn assess_reason(&self) -> Option<String> {
        None
    }

    /// 审批授权目标的覆盖值（默认 `None` = 用路径/命令解析出的 target）。
    ///
    /// 授权缓存（`Grants`）按 `tool|target` 建键，target 为空时无法缓存 session /
    /// workspace 级授权——MCP 工具没有路径参数，必须自带一个稳定的目标标识。
    fn grant_target(&self, _ctx: &ToolCtx) -> Option<String> {
        None
    }

    /// 本工具结果的字节上限（执行出口据此裁决，默认 [`MAX_OUTPUT_BYTES`]）。
    ///
    /// 为什么按工具分：统一 32 KiB 会把「工具自己已经按功能上限裁过」的结果再砍一刀
    /// （`fs_read` 声明 256 KiB / 2000 行、`skill` 声明 100 KB、`bash_output` 可要 64 KB），
    /// 砍掉时还不带准确的分页提示。工具自己申报的能力边界由本方法表达，出口只做兜底
    /// 校验（D29）。
    ///
    /// 做成 trait 方法而不是 `ToolSpec` 字段：`ToolSpec` 有近百处结构体字面量构造点，
    /// 加必填字段会让每处都改一行，而真正需要非默认值的只有三个工具。
    fn output_ceiling(&self) -> usize {
        MAX_OUTPUT_BYTES
    }
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
    /// Arc 而非 Box：execute 必须能把工具句柄克隆出锁再执行（shell_exec 可能同步等待
    /// 子进程分钟级，持锁执行会让并发 run 的所有工具评估与执行全部冻结）。
    tools: Mutex<HashMap<String, Arc<dyn Tool>>>,
    env: Arc<ToolEnv>,
}

impl Registry {
    /// backup_dir 传 `~/.craftwork/backups/`；空串则所有写前备份静默跳过。
    /// data_dir 传 `~/.craftwork/`，供技能等目录型资源推导。
    ///
    /// 后台任务注册表由本处新建（测试与独立装配用）。生产装配先建一个
    /// `Arc<ShellTasks>`，用 [`Registry::new_with_shell_tasks`] 注入，再把同一个
    /// Arc 交给 Runner——否则 Runner 拿不到同一份注册表，后台命令能启动但没有完成通知。
    pub fn new(backup_dir: &str, data_dir: &str, db: Arc<Db>) -> Self {
        Self::new_with_shell_tasks(backup_dir, data_dir, db, Arc::new(crate::shell_tasks::ShellTasks::new()))
    }

    /// 用调用方提供的后台任务注册表装配（与 Runner 共享同一份）。
    pub fn new_with_shell_tasks(
        backup_dir: &str,
        data_dir: &str,
        db: Arc<Db>,
        shell_tasks: Arc<crate::shell_tasks::ShellTasks>,
    ) -> Self {
        Self {
            tools: Mutex::new(HashMap::new()),
            env: Arc::new(ToolEnv { backup_dir: backup_dir.to_string(), data_dir: data_dir.to_string(), db: Arc::clone(&db), shell_tasks, proxy_config: db_proxy_config(&db) }),
        }
    }

    /// 本注册表使用的后台任务注册表句柄（壳层用它把同一个 Arc 交给 Runner）。
    pub fn shell_tasks(&self) -> Arc<crate::shell_tasks::ShellTasks> {
        Arc::clone(&self.env.shell_tasks)
    }

    /// 注册工具；MCP 动态注册同名替换。
    pub fn register(&self, tool: Box<dyn Tool>) {
        // Box → Arc（From 转换）：保持调用方签名不变，注册表内部以 Arc 持有
        self.tools.lock().unwrap().insert(tool.spec().name, Arc::from(tool));
    }

    /// 注销工具（MCP 服务器断开时移除其工具）。
    pub fn unregister(&self, name: &str) {
        self.tools.lock().unwrap().remove(name);
    }

    pub fn spec_for(&self, name: &str) -> Option<ToolSpec> {
        self.tools.lock().unwrap().get(name).map(|t| t.spec())
    }

    /// 工具是否具备写语义（计划模式硬门卡的判定依据之一）。
    ///
    /// 不看风险等级：`fs_write` / `fs_edit` / `apply_patch` 都是 R1（单文件改动本身风险不高），
    /// 按 R2 判定会恰好漏掉计划模式下最该拦的文件写入。
    /// 未知工具（已被注销、或尚未注册的 MCP 工具）按「可能写」保守兜底——
    /// 计划态下宁可拦一次让模型走 exit_plan，也不放行一个来路不明的工具。
    /// 调用方（Runner）只在分级档启用该判定：自由档的计划态写操作照常执行（D8 2026-11-10）。
    pub fn is_mutating(&self, name: &str) -> bool {
        match self.spec_for(name) {
            Some(spec) => spec.mutates,
            None => true,
        }
    }

    /// 工具是否是「以 `path` 参数为写入目标」的文件写入（自动计划门的计数口径，D32）。
    ///
    /// 只认 `mutates && path_arg == Some("path")`：当前为 fs_write / fs_edit / apply_patch /
    /// doc_edit / xlsx_set_cell 五个。**明确排除** `shell_exec`（path_arg 是 cwd，命令串里
    /// 改了几个文件策略层看不进去，与 D10 同口径）、`git_commit`（无 path_arg）与全部
    /// MCP 工具。未知工具返回 false——与 [`Self::is_mutating`] 的「可能写」保守兜底相反：
    /// 这里只是计数口径，误判会制造假计数、过早打断模型的工作流。
    pub fn is_file_write(&self, name: &str) -> bool {
        match self.spec_for(name) {
            Some(spec) => spec.mutates && spec.path_arg.as_deref() == Some("path"),
            None => false,
        }
    }

    /// 文件写入计数的归一键（自动计划门 D32）：反斜杠转 `/`、反复剥开头 `./`、去首尾空白。
    /// 只做**写法归一**（`a.txt` 与 `./a.txt` 落到同一个键）；不做符号链接解析与大小写
    /// 折叠——真实解析已由 assess 完成，别名路径被算成两个文件只会更早要求计划，不会漏拦。
    /// target 为空串（工具未解析出目标）时回落原始 `path` 参数；两者都空返回空串
    /// （调用方对空键跳过计数）。
    pub fn file_key(target: &str, fallback: &str) -> String {
        let source = if target.trim().is_empty() { fallback } else { target };
        let mut key = source.trim().replace('\\', "/");
        while let Some(stripped) = key.strip_prefix("./") {
            key = stripped.to_string();
        }
        key
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
        self.assess_in_plan(roots, name, args, false)
    }

    /// 带计划态上下文的只读评估。`plan_active` 会随 `ToolCtx` 下发给工具，
    /// 让少数工具（todo_write）能做计划态专属校验；其余工具忽略它。
    pub fn assess_in_plan(&self, roots: &[String], name: &str, args: &Value, plan_active: bool) -> Assessment {
        self.assess_scoped(roots, name, args, plan_active, "")
    }

    /// 带会话归属的评估：后台命令任务按会话记账（`bash_output` / `kill_shell` 只认本会话），
    /// 因此 Runner 侧必须传真实会话 id。其余工具忽略它。
    pub fn assess_scoped(
        &self,
        roots: &[String],
        name: &str,
        args: &Value,
        plan_active: bool,
        thread_id: &str,
    ) -> Assessment {
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
            // 系统临时目录是隐含可访问根（D28）：绝对路径可落在临时目录内，风险按工具基础
            // 等级走（读 R0 / 写 R1），不再判越界。命中时 `resolved.system_temp` 置位。
            match resolve_existing_with_temp(roots, &rel_raw) {
                Ok(resolved) => {
                    target = resolved.abs.clone();
                    // 展示路径相对主要根；临时目录内的目标不在工作区下，`relative` 会原样返回绝对路径
                    display = relative(roots.first().map(String::as_str).unwrap_or_default(), &resolved.abs);
                    target_root = resolved.root.clone();
                    if resolved.system_temp {
                        // 临时目录不是任何仓库的一部分：Git 工具在这里必然失败，
                        // 评估期直接给可读原因，别先弹审批再报 git 错误。
                        if name.starts_with("git_") {
                            err = Some(format!(
                                "{display} 在系统临时目录内，不属于任何 Git 仓库；Git 操作仅支持主要目录。"
                            ));
                        }
                    } else if name.starts_with("git_") && resolved.root_index != 0 {
                        // git 系工具只支持主要目录（保持「主要目录仓库」语义）
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
            target: target.clone(),
            display,
            workspace: roots.first().cloned().unwrap_or_default(),
            roots: roots.to_vec(),
            target_root,
            err,
            plan_active,
            thread_id: thread_id.to_string(),
            env: Arc::clone(&self.env),
        };
        // 工具可覆盖审批理由与授权目标（MCP 等动态工具需要）：覆盖值优先，
        // 因为它们的通用文案说明不了「经哪个外部服务器执行」，且没有路径/命令
        // 参数可供授权缓存建键（Grants 按 tool|target 缓存，target 为空即无法缓存）。
        if let Some(override_reason) = tool.assess_reason() {
            reason = override_reason;
        }
        if let Some(override_target) = tool.grant_target(&ctx) {
            // 授权目标覆盖时，审批卡展示的目标也跟着换（卡片上两者是同一行信息）
            target = override_target;
        }
        drop(tools_guard);

        Assessment {
            risk,
            reason: if reason.is_empty() { reason_for_risk(spec.risk).to_string() } else { reason },
            target,
            prepared: Some(ctx),
            exists: true,
        }
    }

    /// 统一执行出口：err 置位直接返回错误；panic 隔离；输出按预算裁决。
    ///
    /// 锁纪律：锁内只解析出工具句柄（Arc 克隆），`tool.execute` 必须在锁外——
    /// shell_exec 会同步等待子进程数十秒到分钟级，若持锁执行（曾踩坑），其余并发
    /// run 的 execute 与 assess 全部阻塞在 tools 锁上，表现为「所有任务卡住」。
    pub fn execute(&self, ctx: &ToolCtx) -> ToolResult {
        let tool = {
            let tools_guard = self.tools.lock().unwrap();
            tools_guard.get(&ctx.name).cloned()
        };
        let Some(tool) = tool else {
            return ToolResult::err_code(format!("未知工具：{}", ctx.name), "unknown_tool");
        };
        // 评估期发现路径越界/参数非法：不执行，落回统一错误
        if let Some(err) = &ctx.err {
            return ToolResult::err_code(err.clone(), "tool_error");
        }
        let ceiling = tool.output_ceiling();
        // 工具 panic 不允许拖垮调用线程（Agent 循环），转为普通错误结果
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.execute(ctx)));
        match result {
            Ok(mut result) => {
                apply_output_budget(&mut result, ceiling);
                result
            }
            Err(_) => ToolResult::err_code("工具执行异常", "tool_error"),
        }
    }
}

/// 执行出口的输出预算裁决（唯一一处）。
///
/// 两条路径：
/// - **已申报截断**（工具自己按功能上限裁过）：不二次截断，只在超过 `ceiling` 时兜底
///   补切并追加一行说明。申报的丢弃量与文案原样保留——产生方才知道怎么继续取。
/// - **未申报**：按 `ceiling` 截断并追加统一的「原始长度」提示。
///
/// 历史上两条路径都被无条件 32 KiB 截断，导致内层标记被切掉、`fs_read` 静默丢行、
/// `skill` 的 100 KB 正文只剩三分之一（D29）。
fn apply_output_budget(result: &mut ToolResult, ceiling: usize) {
    if let Some(info) = &result.truncation {
        // 申报不能变成绕过上限的后门：申报后仍超 ceiling 就再按 ceiling 切一次
        if result.output.len() > ceiling {
            let head = keep_head(&result.output, ceiling);
            result.output =
                format!("{}\n…（输出超出上限，已再截断；此前申报丢弃 {} 字节）", head.text, info.dropped_bytes);
        }
        return;
    }
    if result.output.len() <= ceiling {
        return;
    }
    let head = keep_head(&result.output, ceiling);
    let original = result.output.len();
    result.output = format!("{}\n…（输出已截断，原始长度 {original} 字节）", head.text);
    result.truncation = Some(TruncationInfo { dropped_bytes: head.dropped_bytes, note: String::new() });
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
    use std::path::PathBuf;
    use std::sync::Arc;

    /// 自动计划门的计数口径（D32）：五个 `path` 写入工具命中；`shell_exec`（path_arg 是
    /// cwd）/ `git_commit`（无 path_arg）/ 只读与未知工具不命中——未知工具与 is_mutating
    /// 的「可能写」兜底方向相反（误判会制造假计数）。
    #[test]
    fn file_write_detection_scoped_to_path_targeted_mutators() {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cw-registry-d32-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(craft_store::db::Db::open(&dir.join("t.db")).unwrap());
        let registry = Registry::new("", dir.to_string_lossy().as_ref(), Arc::clone(&db));
        for tool in crate::builtin_tools() {
            registry.register(tool);
        }
        for name in ["fs_write", "fs_edit", "apply_patch", "doc_edit", "xlsx_set_cell"] {
            assert!(registry.is_file_write(name), "{name} 应计为文件写入");
        }
        for name in ["shell_exec", "git_commit", "fs_read", "todo_write", "enter_plan", "mcp__x__y", "不存在的工具"] {
            assert!(!registry.is_file_write(name), "{name} 不应计为文件写入");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// file_key 只做写法归一：`\` → `/`、剥 `./`、trim；空 target 回落原始 path 参数。
    #[test]
    fn file_key_normalizes_spelling_only() {
        assert_eq!(Registry::file_key("a.txt", ""), "a.txt");
        assert_eq!(Registry::file_key("./a.txt", ""), "a.txt", "`./` 前缀剥除后与裸名同键");
        assert_eq!(Registry::file_key("././a.txt", ""), "a.txt", "反复剥除");
        assert_eq!(Registry::file_key("src\\lib\\a.rs", ""), "src/lib/a.rs", "反斜杠归一");
        assert_eq!(Registry::file_key("  a.txt  ", ""), "a.txt");
        assert_eq!(Registry::file_key("", "fallback/a.txt"), "fallback/a.txt", "空 target 回落");
        assert_eq!(Registry::file_key("  ", "b.txt"), "b.txt");
        assert_eq!(Registry::file_key("", ""), "", "双空返回空串，调用方跳过计数");
    }

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
    fn keep_head_reports_dropped_bytes_without_exceeding_limit() {
        let text = "a".repeat(100);
        let kept = keep_head(&text, 40);
        assert_eq!(kept.text.len(), 40);
        // 丢弃量由截断方算准：60，而不是让上层拿截断后的长度反推
        assert_eq!(kept.dropped_bytes, 60);
        // 未超上限时是恒等变换，丢弃量为 0（上层据此不追加任何提示）
        let untouched = keep_head("short", 40);
        assert_eq!(untouched.text, "short");
        assert_eq!(untouched.dropped_bytes, 0);
    }

    #[test]
    fn keep_head_backs_off_to_utf8_boundary() {
        // 每个汉字 3 字节：limit=4 落在第二个字中间，必须退到 3（第一个字完整）
        let text = "中文内容";
        let kept = keep_head(text, 4);
        assert_eq!(kept.text, "中");
        assert_eq!(kept.dropped_bytes, text.len() - 3);
        // 退化情形：切点落在首字节之前时不 panic
        assert_eq!(keep_head(text, 0).text, "");
    }

    /// 核心回归守卫（D29）：工具已申报截断时，执行出口不得二次截断正文，
    /// 也不得替换掉产生方写的文案。
    #[test]
    fn declared_truncation_is_not_truncated_again() {
        let mut result = ToolResult::ok("x".repeat(1000));
        result.truncation = Some(TruncationInfo { dropped_bytes: 500, note: "工具自己的说明".into() });
        apply_output_budget(&mut result, 2000);
        // 正文原样保留（含工具自己追加的标记位置），没有第二句「输出已截断」
        assert_eq!(result.output.len(), 1000);
        assert!(!result.output.contains("输出已截断"));
        assert_eq!(result.truncation.as_ref().unwrap().dropped_bytes, 500);
    }

    /// 申报不能变成绕过上限的后门：申报之后仍超 ceiling 依然要兜底切。
    #[test]
    fn declared_truncation_above_ceiling_is_still_capped() {
        let mut result = ToolResult::ok("y".repeat(1000));
        result.truncation = Some(TruncationInfo { dropped_bytes: 10, note: "申报".into() });
        apply_output_budget(&mut result, 400);
        assert!(result.output.len() < 1000, "超过 ceiling 必须兜底再截");
        assert!(result.output.contains("输出超出上限"));
    }

    /// 未申报时按 ceiling 截断，并把真实丢弃量写回申报字段。
    #[test]
    fn undeclared_output_is_capped_and_reports_original_length() {
        let mut result = ToolResult::ok("z".repeat(1000));
        apply_output_budget(&mut result, 400);
        assert!(result.output.contains("原始长度 1000 字节"));
        assert_eq!(result.truncation.as_ref().unwrap().dropped_bytes, 600);
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

    /// 一个只用来调 `ToolCtx` 纯方法的 ctx（attach_diff 不碰路径与 env）。
    /// db 走唯一临时目录：固定路径会让同一进程内的第二次 open 重放迁移，报「duplicate column」。
    fn diff_ctx() -> (ToolCtx, PathBuf) {
        let dir = crate::test_util::unique_temp_dir("attach-diff");
        std::fs::create_dir_all(&dir).unwrap();
        let db = std::sync::Arc::new(craft_store::db::Db::open(&dir.join("t.db")).unwrap());
        let ctx = ToolCtx {
            name: "fs_edit".into(),
            args: Value::Null,
            target: String::new(),
            display: String::new(),
            workspace: String::new(),
            roots: Vec::new(),
            target_root: String::new(),
            err: None,
            plan_active: false,
            thread_id: String::new(),
            env: std::sync::Arc::new(ToolEnv { backup_dir: String::new(), data_dir: String::new(), db, shell_tasks: std::sync::Arc::new(crate::shell_tasks::ShellTasks::new()), proxy_config: direct_proxy_config() }),
        };
        (ctx, dir)
    }

    #[test]
    fn attach_diff_skips_identical_text() {
        let (ctx, dir) = diff_ctx();
        let mut meta = serde_json::json!({ "path": "a" });
        ctx.attach_diff(&mut meta, "same\ntext", "same\ntext");
        assert!(meta.get("diff").is_none(), "无变化不应写 diff 字段: {meta}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_diff_degrades_to_stats_when_over_budget() {
        let (ctx, dir) = diff_ctx();
        let old_text: String = (0..6000).map(|i| format!("old line {i} with some padding text\n")).collect();
        let new_text: String = (0..6000).map(|i| format!("new line {i} with some padding text\n")).collect();
        let mut meta = serde_json::json!({});
        ctx.attach_diff(&mut meta, &old_text, &new_text);
        let diff = &meta["diff"];
        assert_eq!(diff["truncated"], true, "超预算应降级为统计形态");
        assert_eq!(diff["hunks"].as_array().unwrap().len(), 0, "降级后不留行内容");
        assert_eq!(diff["additions"], 6000, "统计仍保留: {diff}");
        assert_eq!(diff["deletions"], 6000, "统计仍保留: {diff}");
        let size = serde_json::to_string(diff).unwrap().len();
        assert!(size < MAX_DIFF_META_BYTES, "降级后必须回到预算内，实际 {size} 字节");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 写语义标注是计划模式硬门卡的唯一依据（分级档）：标错的直接后果是「计划态下偷偷改了文件」。
    /// 这里把内置工具的标注钉死——fs_write / fs_edit / apply_patch 都是 R1，
    /// 若按风险等级判定会被漏拦，所以必须显式声明 mutates。
    #[test]
    fn builtin_tool_mutates_flags_match_actual_write_semantics() {
        let registry = Registry::new("", "", std::sync::Arc::new(
            craft_store::db::Db::open(&std::env::temp_dir().join("cw-mut-flags.db")).unwrap(),
        ));
        for tool in crate::builtin_tools() {
            registry.register(tool);
        }
        // 会改动工作区 / 文件系统 / 外部状态的工具
        for name in [
            "fs_write", "fs_edit", "apply_patch", "shell_exec", "git_commit", "doc_edit", "xlsx_set_cell",
        ] {
            assert!(registry.is_mutating(name), "{name} 应标记为写工具（mutates=true）");
        }
        // 关键证据：文件写入类工具的风险等级低于 R2——若按「风险 ≥ R2」判定门卡，
        // 计划模式下最该拦的 fs_write / fs_edit / apply_patch 会被放行。
        for name in ["fs_write", "fs_edit", "apply_patch"] {
            let spec = registry.spec_for(name).expect("内置工具应已注册");
            assert!(
                spec.risk.level() < PolicyRisk::R2.level(),
                "{name} 的风险等级本应低于 R2（正是「按等级判定」会失效的原因）"
            );
        }
        // 只读工具与协议 / 展示工具
        for name in [
            "fs_read", "fs_list", "fs_glob", "fs_grep", "git_status", "git_diff", "git_log", "doc_read", "todo_write",
            "enter_plan", "exit_plan",
        ] {
            assert!(!registry.is_mutating(name), "{name} 不应标记为写工具");
        }
        // 未知工具按「可能写」保守兜底
        assert!(registry.is_mutating("mcp__unknown__tool"), "未知工具应保守视为写工具");
    }

    /// 系统临时目录是隐含可访问根（D28）：评估期不得判越界，且风险等级按工具基础等级走。
    ///
    /// 这条测试钉的是「临时产物可用」这个用户可见结果：写临时脚本再执行、读回临时文件，
    /// 不该被 `Registry::assess` 用「路径超出工作区」拦下（R3 + err 置位 = 直接不执行）。
    #[test]
    fn system_temp_paths_are_assessable_from_any_workspace() {
        let dir = crate::test_util::unique_temp_dir("temp-root-ws");
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Registry::new("", "", std::sync::Arc::new(
            craft_store::db::Db::open(&dir.join("t.db")).unwrap(),
        ));
        for tool in crate::builtin_tools() {
            registry.register(tool);
        }
        let roots = vec![dir.to_string_lossy().to_string()];

        let temp_file = std::env::temp_dir().join(format!("cw-reg-temp-{}-0.txt", std::process::id()));
        std::fs::write(&temp_file, "hello").unwrap();
        let temp_path = temp_file.to_string_lossy().to_string();

        // 读临时文件：R0 且无评估错误
        let read = registry.assess(&roots, "fs_read", &serde_json::json!({ "path": temp_path }));
        assert!(read.prepared.as_ref().unwrap().err.is_none(), "临时文件不该判越界：{:?}", read.prepared.as_ref().unwrap().err);
        assert_eq!(read.risk, PolicyRisk::R0);
        assert_eq!(read.target, craft_policy::resolve::strip_verbatim(&std::fs::canonicalize(&temp_file).unwrap()).to_string_lossy().to_string());

        // 写临时文件：R1（写工作区的等级，改由审批档位决定是否弹卡）
        let new_temp = std::env::temp_dir().join(format!("cw-reg-temp-{}-1.py", std::process::id()));
        let write = registry.assess(
            &roots,
            "fs_write",
            &serde_json::json!({ "path": new_temp.to_string_lossy().to_string(), "content": "x" }),
        );
        assert!(write.prepared.as_ref().unwrap().err.is_none(), "临时目录新建文件不该判越界");
        assert_eq!(write.risk, PolicyRisk::R1);

        // shell_exec 的 cwd 同样可用（临时目录里建目录再执行）
        let temp_cwd = std::env::temp_dir().join(format!("cw-reg-cwd-{}-0", std::process::id()));
        std::fs::create_dir_all(&temp_cwd).unwrap();
        let shell = registry.assess(
            &roots,
            "shell_exec",
            &serde_json::json!({ "command": "ls", "cwd": temp_cwd.to_string_lossy().to_string() }),
        );
        assert!(shell.prepared.as_ref().unwrap().err.is_none(), "临时目录 cwd 不该判越界");

        std::fs::remove_file(&temp_file).ok();
        std::fs::remove_dir_all(&temp_cwd).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Git 工具落到系统临时目录：评估期直接给出可读原因，不先弹审批再必然失败。
    #[test]
    fn git_tools_reject_system_temp_paths_at_assess_time() {
        let dir = crate::test_util::unique_temp_dir("temp-git-ws");
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Registry::new("", "", std::sync::Arc::new(
            craft_store::db::Db::open(&dir.join("t.db")).unwrap(),
        ));
        for tool in crate::builtin_tools() {
            registry.register(tool);
        }
        let roots = vec![dir.to_string_lossy().to_string()];
        let temp_file = std::env::temp_dir().join(format!("cw-reg-git-{}-0.txt", std::process::id()));
        std::fs::write(&temp_file, "x").unwrap();

        let assessed = registry.assess(
            &roots,
            "git_diff",
            &serde_json::json!({ "path": temp_file.to_string_lossy().to_string() }),
        );
        let err = assessed.prepared.as_ref().unwrap().err.clone().expect("git 工具应快速失败");
        assert!(err.contains("系统临时目录") && err.contains("Git 操作仅支持主要目录"), "实际：{err}");

        std::fs::remove_file(&temp_file).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 工作区与临时目录之外的绝对路径仍然越界（R3 + 不执行）：放宽只针对临时根。
    #[test]
    fn paths_outside_workspace_and_temp_still_rejected() {
        let dir = crate::test_util::unique_temp_dir("outside-ws");
        std::fs::create_dir_all(&dir).unwrap();
        let registry = Registry::new("", "", std::sync::Arc::new(
            craft_store::db::Db::open(&dir.join("t.db")).unwrap(),
        ));
        for tool in crate::builtin_tools() {
            registry.register(tool);
        }
        let roots = vec![dir.to_string_lossy().to_string()];
        let assessed = registry.assess(&roots, "fs_read", &serde_json::json!({ "path": "/etc/hosts" }));
        assert_eq!(assessed.risk, PolicyRisk::R3);
        assert!(assessed.prepared.as_ref().unwrap().err.is_some(), "/etc/hosts 必须仍判越界");
        std::fs::remove_dir_all(&dir).ok();
    }
}
