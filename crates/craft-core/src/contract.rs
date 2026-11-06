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
    pub const PLAN_CREATED: &str = "plan.created";
    pub const TODO_UPDATED: &str = "todo.updated";
    pub const USER_MESSAGE: &str = "user.message";
    /// 引导（steer）已投递：消费点或收尾丢弃时发，前端据此摘除「待送达」标记；
    /// payload 带 nextMessageId 时表示后续输出切换到新 assistant 消息。
    pub const STEER_DELIVERED: &str = "steer.delivered";
    pub const NOTICE: &str = "notice";
    pub const CONTEXT_COMPRESSED: &str = "context.compressed";
    pub const ERROR: &str = "error";
    pub const THREAD_UPDATED: &str = "thread.updated";
    pub const USAGE_UPDATED: &str = "usage.updated";
    pub const SETTINGS_UPDATED: &str = "settings.updated";
    pub const WORKSPACE_UPDATED: &str = "workspace.updated";
}

/// 事件信封：seq 为会话内单调递增（总线序，用于前端去重排序）。
/// 字段名必须与 src/shared/events.ts 的 Envelope 契约一致（camelCase）：
/// 一旦漂移，渲染层读不到 threadId，所有事件回落到活动会话应用，出现乱序与串会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub v: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub events: Vec<Envelope>,
}

// ---------- 会话与工作区 ----------

/// 会话对外视图（列表与详情共用；用量推导见 craft-store::view）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub id: String,
    /// code | work
    pub mode: String,
    pub title: String,
    pub model_id: String,
    pub model_name: String,
    /// 思考档位（空串=未选择）
    #[serde(default)]
    pub reasoning_level: String,
    pub workspace_id: String,
    /// 绑定的 git worktree 目录（空串 = 未绑定）
    #[serde(default)]
    pub worktree_path: String,
    /// 绑定的 worktree 分支名（空串 = 未绑定）
    #[serde(default)]
    pub worktree_branch: String,
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

/// 工作区下的一个源文件夹（首条约定为主目录）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFolder {
    pub id: String,
    pub path: String,
    pub position: i64,
}

/// 工作区对外视图（含按打开序排列的文件夹列表）。
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

/// 计划数据（计划模式产物）：关联会话与生成它的 run，落盘文件与批准状态。
/// file_path 为空表示计划尚未生成（run 中断/失败/未完成）——前端提示条以此为准。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInfo {
    pub id: String,
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub file_path: String,
    pub approved: bool,
    /// 用户点过「忽略」：提示条不再出现（等待用户直接输入继续）。
    #[serde(default)]
    pub ignored: bool,
    pub updated_at: i64,
}

/// 消息内单个部件的快照形态；payload 为该部件 kind 的原始 JSON。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartSnapshot {
    pub id: String,
    pub kind: PartKind,
    pub seq: i64,
    pub payload: Value,
}

/// 快照重建出的一条消息：部件有序，seq 为最后部件的事件序（翻页与排序游标）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSnapshot {
    pub id: String,
    /// user | assistant
    pub role: String,
    pub parts: Vec<PartSnapshot>,
    /// complete | streaming | incomplete | error
    pub status: String,
    /// 最后部件 seq（翻页游标）
    pub seq: i64,
    /// 消息起始时刻（毫秒；快照取消息名下最早事件，事件流取信封 ts；0 = 未知）
    #[serde(default)]
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_duration_ms: Option<i64>,
    /// 回复完成时刻（毫秒；= 所属 run 的 finished_at，事件流取终态信封 ts）。
    /// 界面消息时间戳显示它而非 created_at（用户关心「什么时候答完」）；运行中缺省不显示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
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

/// 审批卡片上的一条事实项（label/value 成对展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalFact {
    pub label: String,
    pub value: String,
}

/// 会话快照：渲染层装载会话的完整载荷（消息列表 + 待决定审批 + 翻页游标）。
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

/// 应用设置（settings 表 `app` 键的 JSON 形态）；字段即时生效由壳层保证。
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
    /// 全局默认思考档位（空串=未选择；与会话级 threads.reasoning_level 同语义）
    #[serde(default)]
    pub reasoning_level: String,
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

/// audit_log 一行：一次审批决定或工具执行的审计记录。
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

/// 一段时间窗的 token 与费用合计（cost_micros 为微元，避免浮点累加误差）。
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

/// 用量汇总视图（今天/近7天/近30天/按模型/按天，窗口口径见 usage_summary）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today: TokenTotals,
    pub week: TokenTotals,
    pub month: TokenTotals,
    pub by_model: Vec<UsageByModel>,
    pub by_day: Vec<UsageByDay>,
}

/// backups 一行：一次写前备份的登记（hash 为原文件 SHA-256，外部修改检测用）。
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

/// 一个差异块：unified 风格 header（@@ -a,b +c,d @@）+ 有序行。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// 行级差异结果；truncated 为真时 hunks 只含一条占位块，仅有统计意义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub additions: i64,
    pub deletions: i64,
    pub hunks: Vec<DiffHunk>,
    pub truncated: bool,
}

// ---------- 应用信息 ----------

/// 关于页展示的应用基本信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub data_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 对拍 src/shared/events.ts：信封字段必须序列化为 camelCase（type 除外）。
    /// 漂移后果：渲染层读不到 threadId，所有事件回落到活动会话应用，出现乱序与串会话。
    #[test]
    fn envelope_serializes_camel_case_fields_for_frontend() {
        let env = Envelope {
            seq: 3,
            event_type: event_type::TEXT_DELTA.to_string(),
            thread_id: Some("th_x".into()),
            run_id: Some("run_x".into()),
            ts: 1234,
            data: serde_json::json!({ "partId": "p1", "text": "hi" }),
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["seq"], 3);
        assert_eq!(v["type"], "text.delta");
        assert_eq!(v["threadId"], "th_x");
        assert_eq!(v["runId"], "run_x");
        assert_eq!(v["ts"], 1234);
        assert!(v.get("thread_id").is_none());

        let batch = Batch {
            v: 1,
            thread_id: Some("th_x".into()),
            events: vec![env],
        };
        let bv = serde_json::to_value(&batch).unwrap();
        assert_eq!(bv["threadId"], "th_x");
        assert!(bv.get("thread_id").is_none());
    }

    /// 金样守卫：全部对外契约结构序列化字段必须为 camelCase（不含下划线），与
    /// src/shared/types.ts 对拍。新增字段若漂移为 snake_case 此测试即红
    /// （迁移复审发现 WorkspaceFolder 漏 rename_all，系统性补此守卫——票 09）。
    fn assert_no_snake_keys(prefix: &str, value: &Value) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    assert!(!k.contains('_'), "{prefix} 字段 {k:?} 含下划线（应为 camelCase）");
                    assert_no_snake_keys(&format!("{prefix}.{k}"), v);
                }
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    assert_no_snake_keys(&format!("{prefix}[{i}]"), v);
                }
            }
            _ => {}
        }
    }

    fn sample_thread() -> ThreadView {
        ThreadView {
            id: "t".into(),
            mode: "code".into(),
            title: "t".into(),
            model_id: "m".into(),
            model_name: "m".into(),
            reasoning_level: String::new(),
            workspace_id: "w".into(),
            worktree_path: String::new(),
            worktree_branch: String::new(),
            status: "idle".into(),
            archived: false,
            tokens: 0,
            context_tokens: 0,
            context_cache_tokens: 0,
            context_speed: 0.0,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn contract_structs_serialize_all_camel_case() {
        let folder = WorkspaceFolder { id: "f".into(), path: "/p".into(), position: 0 };
        let part = PartSnapshot { id: "p".into(), kind: "text".into(), seq: 0, payload: Value::Null };
        let message = MessageSnapshot {
            id: "m".into(),
            role: "user".into(),
            parts: vec![part.clone()],
            status: "complete".into(),
            seq: 0,
            created_at: 0,
            work_duration_ms: Some(1),
            finished_at: None,
        };
        let card = ApprovalCard {
            part_id: "p".into(),
            approval_id: "a".into(),
            tool_call_id: "c".into(),
            tool: "t".into(),
            risk: "R1".into(),
            title: "t".into(),
            target: "x".into(),
            detail: "d".into(),
            facts: vec![ApprovalFact { label: "l".into(), value: "v".into() }],
            args: Value::Null,
            allow_permanent: false,
            status: "waiting".into(),
            decision: None,
            scope: None,
        };
        let snapshot = Snapshot {
            thread: sample_thread(),
            messages: vec![message],
            pending_approvals: vec![card],
            last_seq: 0,
            oldest_seq: 0,
            has_more: false,
        };
        let settings = Settings {
            theme: "system".into(),
            mode: "code".into(),
            panel_width: 320,
            approval_level: "graded".into(),
            default_model: String::new(),
            reasoning_level: String::new(),
            fallback_model: String::new(),
            active_workspace: String::new(),
            language: String::new(),
            font_size: 15,
            code_theme: String::new(),
            auto_update: false,
            notifications: false,
            update_channel: String::new(),
            telemetry: false,
            max_iterations: 300,
        };
        let audit = AuditEntry {
            id: 0,
            ts: 0,
            thread_id: String::new(),
            run_id: String::new(),
            tool: "t".into(),
            risk: "R0".into(),
            decision: "auto".into(),
            scope: String::new(),
            detail: String::new(),
        };
        let totals = TokenTotals { input_tokens: 0, output_tokens: 0, cost_micros: 0 };
        let usage = UsageSummary {
            today: totals.clone(),
            week: totals.clone(),
            month: totals.clone(),
            by_model: vec![UsageByModel { model_id: "m".into(), input_tokens: 0, output_tokens: 0, cost_micros: 0 }],
            by_day: vec![UsageByDay { day: "d".into(), input_tokens: 0, output_tokens: 0, cost_micros: 0 }],
        };
        let backup = Backup {
            id: "b".into(),
            path: "/p".into(),
            backup_path: "/b".into(),
            hash: "h".into(),
            size: 0,
            created_at: 0,
        };
        let line = DiffLine { kind: "ctx".into(), old: 0, r#new: 0, text: "t".into() };
        let diff = DiffResult { additions: 0, deletions: 0, hunks: vec![DiffHunk { header: "h".into(), lines: vec![line] }], truncated: false };
        let info = AppInfo { name: "n".into(), version: "v".into(), platform: "p".into(), arch: "a".into(), data_dir: "/d".into() };
        let workspace = WorkspaceView { id: "w".into(), name: "n".into(), path: "/p".into(), trust: "trusted".into(), purpose: "project".into(), last_opened_at: 0, folders: vec![folder] };

        let samples: Vec<(&str, Value)> = vec![
            ("ThreadView", serde_json::to_value(sample_thread()).unwrap()),
            ("WorkspaceView", serde_json::to_value(workspace).unwrap()),
            ("Snapshot", serde_json::to_value(snapshot).unwrap()),
            ("Settings", serde_json::to_value(settings).unwrap()),
            ("AuditEntry", serde_json::to_value(audit).unwrap()),
            ("UsageSummary", serde_json::to_value(usage).unwrap()),
            ("Backup", serde_json::to_value(backup).unwrap()),
            ("DiffResult", serde_json::to_value(diff).unwrap()),
            ("AppInfo", serde_json::to_value(info).unwrap()),
        ];
        // MessageSnapshot 单独构造（含 skip_serializing_if 的 workDurationMs 缺省形态）
        let ms = MessageSnapshot {
            id: "m".into(),
            role: "assistant".into(),
            parts: vec![part],
            status: "streaming".into(),
            seq: 0,
            created_at: 0,
            work_duration_ms: None,
            finished_at: None,
        };
        assert_no_snake_keys("MessageSnapshot", &serde_json::to_value(ms).unwrap());
        for (name, v) in samples {
            assert_no_snake_keys(name, &v);
        }
    }
}
