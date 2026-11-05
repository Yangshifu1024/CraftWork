//! 审批状态机：请求 → 卡片 → 用户决定 → 执行/跳过。
//! 不允许静默放行：任何需要审批的操作都必须显式等待用户决定；
//! 决定一经写入不可更改（数据库层并发保护）；拒绝时一律 once，防止误授永久权限。
//! TS 的 Promise 等待在 Rust 用 tokio oneshot 表达。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use craft_core::ids::{id_prefix, new_id};
use crate::grants::Grants;
use craft_store::db::Db;
use craft_store::events::EventRecord;
use craft_store::recorder::Recorder;
use craft_store::runs::{create_approval, get_approval, resolve_approval};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalScope {
    Once,
    Session,
    Workspace,
}

impl ApprovalScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalScope::Once => "once",
            ApprovalScope::Session => "session",
            ApprovalScope::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApprovalFact {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub part_id: String,
    pub thread_id: String,
    pub run_id: String,
    pub message_id: String,
    pub tool_call_id: String,
    pub tool: String,
    pub risk: String,
    pub title: String,
    pub target: String,
    pub detail: String,
    pub facts: Vec<ApprovalFact>,
    pub args: Value,
    pub allow_permanent: bool,
    /// shell_exec 授权前缀（grants 键用，与展示 target 可不同）
    pub grant_target: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApprovalOutcome {
    pub allow: bool,
    /// approved | denied | canceled
    pub status: String,
    pub decision: String,
    pub scope: String,
}

impl ApprovalOutcome {
    fn canceled() -> Self {
        Self {
            allow: false,
            status: "canceled".into(),
            decision: "canceled".into(),
            scope: "once".into(),
        }
    }
}

struct PendingEntry {
    resolve: oneshot::Sender<ApprovalOutcome>,
    permanent: bool,
}

pub struct Approvals {
    db: Arc<Db>,
    recorder: Recorder,
    grants: Arc<Grants>,
    pending: Mutex<HashMap<String, PendingEntry>>,
    grant_keys: Mutex<HashMap<String, (String, String)>>,
}

impl Approvals {
    pub fn new(db: Arc<Db>, recorder: Recorder, grants: Arc<Grants>) -> Self {
        Self {
            db,
            recorder,
            grants,
            pending: Mutex::new(HashMap::new()),
            grant_keys: Mutex::new(HashMap::new()),
        }
    }

    /// 已有授权是否命中（会话/工作区授权缓存）。
    pub fn allowed(&self, tool: &str, target: &str) -> bool {
        self.grants.allowed(tool, target)
    }

    /// 发起审批并等待决定：登记等待表 → 写审批行 → 写 approval 事件（卡片载荷）→
    /// awaiting 状态 → 等待。取消（token）按 canceled 收尾并广播 resolved。
    pub async fn request(&self, req: &ApprovalRequest, token: tokio_util::sync::CancellationToken) -> ApprovalOutcome {
        let approval_id = create_approval(
            &self.db.lock_conn(),
            craft_store::runs::NewApproval {
                thread_id: &req.thread_id,
                run_id: &req.run_id,
                tool_call_id: &req.tool_call_id,
                tool: &req.tool,
                risk: &req.risk,
            },
        )
        .map(|row| row.id)
        .unwrap_or_default();

        let (tx, rx) = oneshot::channel::<ApprovalOutcome>();
        self.pending
            .lock()
            .await
            .insert(approval_id.clone(), PendingEntry { resolve: tx, permanent: req.allow_permanent });
        self.grant_keys
            .lock()
            .await
            .insert(approval_id.clone(), (req.tool.clone(), req.grant_target.clone().unwrap_or_else(|| req.target.clone())));

        // 卡片载荷（ApprovalPart 形状，与渲染层约定一致）
        let card = json!({
            "partId": req.part_id,
            "approvalId": approval_id,
            "toolCallId": req.tool_call_id,
            "tool": req.tool,
            "risk": req.risk,
            "title": req.title,
            "target": req.target,
            "detail": req.detail,
            "facts": req.facts.iter().map(|f| json!({"label": f.label, "value": f.value})).collect::<Vec<_>>(),
            "args": req.args,
            "allowPermanent": req.allow_permanent,
            "status": "waiting",
        });
        let append_result = self.recorder.append(&EventRecord {
            thread_id: req.thread_id.clone(),
            message_id: req.message_id.clone(),
            run_id: Some(req.run_id.clone()),
            kind: "approval".into(),
            payload: card,
        });
        if append_result.is_err() {
            self.pending.lock().await.remove(&approval_id);
            self.grant_keys.lock().await.remove(&approval_id);
            // 写入失败：直接返回取消语义（不允许静默放行）
            return ApprovalOutcome::canceled();
        }
        let _ = self.recorder.set_thread_status(&req.thread_id, Some(&req.run_id), "awaiting");

        // 等待用户决定或取消
        let outcome = tokio::select! {
            result = rx => result.unwrap_or_else(|_| ApprovalOutcome::canceled()),
            _ = token.cancelled() => ApprovalOutcome::canceled(),
        };

        self.pending.lock().await.remove(&approval_id);
        self.grant_keys.lock().await.remove(&approval_id);

        if outcome.status == "canceled" {
            let _ = resolve_approval(&self.db.lock_conn(), &approval_id, "canceled", "");
            let _ = self.recorder.set_thread_status(&req.thread_id, Some(&req.run_id), "running");
            // 终态落库 + 广播：只广播不落库时，重启后已取消的审批卡会回显「等待决定」成僵尸卡
            let _ = self.recorder.append(&EventRecord {
                thread_id: req.thread_id.clone(),
                message_id: req.message_id.clone(),
                run_id: Some(req.run_id.clone()),
                kind: "approval_resolved".into(),
                payload: json!({
                    "partId": req.part_id,
                    "approvalId": approval_id,
                    "status": "canceled",
                    "decision": "canceled",
                    "scope": "",
                }),
            });
            return outcome;
        }

        let _ = self.recorder.set_thread_status(&req.thread_id, Some(&req.run_id), "running");
        let _ = self.recorder.append(&EventRecord {
            thread_id: req.thread_id.clone(),
            message_id: req.message_id.clone(),
            run_id: Some(req.run_id.clone()),
            kind: "approval_resolved".into(),
            payload: json!({
                "partId": req.part_id,
                "approvalId": approval_id,
                "status": outcome.status,
                "decision": outcome.decision,
                "scope": outcome.scope,
            }),
        });
        outcome
    }

    /// 用户决定：状态校验 → scope 归一（拒绝一律 once）→ 并发保护落库 →
    /// 授权缓存 → 唤醒等待方（resolved 事件在 request 侧唤醒后广播）。
    pub async fn decide(&self, approval_id: &str, allow: bool, scope: &str) -> Result<(), String> {
        let approval = get_approval(&self.db.lock_conn(), approval_id).map_err(|e| e.to_string())?;
        if approval.decision.as_ref().is_some_and(|d| !d.is_empty()) {
            return Err(format!("审批已处理：{}", approval.decision.clone().unwrap_or_default()));
        }
        let normalized = if !allow || (scope != "session" && scope != "workspace") {
            ApprovalScope::Once
        } else if scope == "workspace" {
            ApprovalScope::Workspace
        } else {
            ApprovalScope::Session
        };
        // 校验在落库前：不提供永久授权的审批只能允许一次
        {
            let pending = self.pending.lock().await;
            let Some(entry) = pending.get(approval_id) else {
                return Err("审批等待已失效（可能已取消）".into());
            };
            if allow && normalized != ApprovalScope::Once && !entry.permanent {
                return Err("该审批不提供永久授权，只能允许一次".into());
            }
        }
        // 先落库（即使广播失败状态也不回退）
        resolve_approval(
            &self.db.lock_conn(),
            approval_id,
            if allow { "allow" } else { "deny" },
            normalized.as_str(),
        )
        .map_err(|e| e.to_string())?;
        // 审批决定写审计（AGENTS 6.2：审批决定必须落 audit_log）
        let _ = craft_store::records::add_audit(
            &self.db.lock_conn(),
            craft_store::records::AuditRecord {
                ts: None,
                thread_id: Some(approval.thread_id.as_str()),
                run_id: Some(approval.run_id.as_str()),
                tool: &approval.tool,
                risk: &approval.risk,
                decision: if allow { "allow" } else { "deny" },
                scope: Some(normalized.as_str()),
                detail: None,
            },
        );
        let entry = self.pending.lock().await.remove(approval_id);
        if let Some(entry) = entry {
            if let (true, Some(key)) = (allow && normalized != ApprovalScope::Once, self.grant_keys.lock().await.get(approval_id)) {
                let (tool, target) = key.clone();
                self.grants.grant(&tool, &target, normalized.as_str());
            }
            let _ = entry
                .resolve
                .send(ApprovalOutcome {
                    allow,
                    status: if allow { "approved".into() } else { "denied".into() },
                    decision: if allow { "allow".into() } else { "deny".into() },
                    scope: normalized.as_str().to_string(),
                });
        }
        let _ = new_id(id_prefix::PART); // 保留 ID 生成引用（未来扩展部件用）
        Ok(())
    }
}
