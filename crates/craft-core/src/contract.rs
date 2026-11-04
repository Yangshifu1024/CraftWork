//! 跨进程共享契约类型（src/shared/types.ts、events.ts 的 Rust 对应）。
//! 命名与 JSON 形状必须与 TS 侧逐字段一致；对拍测试防漂移（见 tests/）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------- 事件 ----------

/// 事件类型常量（shared/events.ts 的 EventType）。
pub mod event_type {
    pub const RUN_STARTED: &str = "run.started";
    pub const RUN_FINISHED: &str = "run.finished";
    pub const TEXT_DELTA: &str = "text.delta";
    pub const TEXT_COMPLETED: &str = "text.completed";
    pub const REASONING_DELTA: &str = "reasoning.delta";
    pub const REASONING_COMPLETED: &str = "reasoning.completed";
    pub const TOOL_COMPLETED: &str = "tool.completed";
    pub const APPROVAL_REQUESTED: &str = "approval.requested";
    pub const APPROVAL_RESOLVED: &str = "approval.resolved";
    pub const TODO_UPDATED: &str = "todo.updated";
    pub const USER_MESSAGE: &str = "user.message";
    pub const NOTICE: &str = "notice";
    pub const CONTEXT_COMPRESSED: &str = "context.compressed";
    pub const ERROR: &str = "error";
    pub const THREAD_UPDATED: &str = "thread.updated";
    pub const USAGE_UPDATED: &str = "usage.updated";
    pub const SETTINGS_UPDATED: &str = "settings.updated";
    pub const WORKSPACE_UPDATED: &str = "workspace.updated";
}

/// 事件信封：seq 为会话内单调递增（总线序，用于前端去重排序）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub seq: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub ts: i64,
    pub data: Value,
}

/// 一次批量推送（craftwork:events 通道的载荷）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    pub v: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub events: Vec<Envelope>,
}

// ---------- 会话与工作区 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub id: String,
    /// code | work
    pub mode: String,
    pub title: String,
    pub model_id: String,
    pub model_name: String,
    pub workspace_id: String,
    /// idle | running | awaiting | error
    pub status: String,
    pub archived: bool,
    pub tokens: i64,
    pub context_tokens: i64,
    pub context_cache_tokens: i64,
    pub context_speed: f64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFolder {
    pub id: String,
    pub path: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    pub id: String,
    pub name: String,
    pub path: String,
    /// trusted | restricted
    pub trust: String,
    /// project | conversation
    pub purpose: String,
    pub last_opened_at: i64,
    pub folders: Vec<WorkspaceFolder>,
}

// ---------- 快照 ----------

/// 部件 kind（text/reasoning/tool/approval/todo/error/notice/draft/callout）。
pub type PartKind = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartSnapshot {
    pub id: String,
    pub kind: PartKind,
    pub seq: i64,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSnapshot {
    pub id: String,
    /// user | assistant
    pub role: String,
    pub parts: Vec<PartSnapshot>,
    /// complete | streaming | incomplete | error
    pub status: String,
    /// 最后部件 seq（翻页游标）
    pub seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_duration_ms: Option<i64>,
}

/// 待审批卡片（快照恢复用，载荷与事件中的审批部件一致）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalCard {
    #[serde(rename = "partId")]
    pub part_id: String,
    #[serde(rename = "approvalId")]
    pub approval_id: String,
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    pub tool: String,
    pub risk: String,
    pub title: String,
    pub target: String,
    pub detail: String,
    pub facts: Vec<ApprovalFact>,
    pub args: Value,
    #[serde(rename = "allowPermanent")]
    pub allow_permanent: bool,
    /// waiting | approved | denied | canceled
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalFact {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub thread: ThreadView,
    pub messages: Vec<MessageSnapshot>,
    pub pending_approvals: Vec<ApprovalCard>,
    pub last_seq: i64,
    pub oldest_seq: i64,
    pub has_more: bool,
}

// ---------- 设置 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// system | dark | light
    pub theme: String,
    /// code | work
    pub mode: String,
    pub panel_width: i64,
    /// graded | strict | readonly | free
    pub approval_level: String,
    pub default_model: String,
    pub fallback_model: String,
    pub active_workspace: String,
    pub language: String,
    pub font_size: i64,
    pub code_theme: String,
    pub auto_update: bool,
    pub notifications: bool,
    pub update_channel: String,
    pub telemetry: bool,
    pub max_iterations: i64,
}

// ---------- 审计 / 用量 / 备份 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: i64,
    pub ts: i64,
    pub thread_id: String,
    pub run_id: String,
    pub tool: String,
    pub risk: String,
    pub decision: String,
    pub scope: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageByModel {
    pub model_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageByDay {
    pub day: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_micros: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today: TokenTotals,
    pub week: TokenTotals,
    pub month: TokenTotals,
    pub by_model: Vec<UsageByModel>,
    pub by_day: Vec<UsageByDay>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub path: String,
    pub backup_path: String,
    pub hash: String,
    pub size: i64,
    pub created_at: i64,
}

// ---------- 差异 ----------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffLine {
    /// ctx | add | del
    pub kind: String,
    pub old: i64,
    pub r#new: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub additions: i64,
    pub deletions: i64,
    pub hunks: Vec<DiffHunk>,
    pub truncated: bool,
}

// ---------- 应用信息 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub data_dir: String,
}
