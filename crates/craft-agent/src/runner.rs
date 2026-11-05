//! Agent 主循环：一次运行的完整状态机——
//! 启动/取消、建立模型流（带整体超时与失败转移）、消费流片段、工具执行、审批门控、
//! 历史重建、上下文压缩、自动命名、用量与事件落库。
//! 同一会话同时只允许一次运行：新消息总是接管（先取消旧运行再覆盖登记）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use craft_core::contract::event_type;
use craft_core::ids::{id_prefix, new_id};
use craft_model::errors::ModelError;
use craft_model::factory::ModelFactory;
use craft_model::types::{Client, Message, ToolCall, Usage};
use craft_policy::engine::{reason_for, Engine};
use craft_store::db::Db;
use craft_store::events::EventRecord;
use craft_store::recorder::Recorder;
use craft_store::records::{add_audit, add_usage};
use craft_store::runs::{create_run, finish_run};
use craft_store::threads::{add_thread_tokens, get_thread};
use craft_store::workspaces::get_workspace;
use craft_tools::registry::{Registry, ToolResult};

use crate::approvals::{ApprovalFact, ApprovalRequest, Approvals};
use crate::attachments::load_attachments;
use crate::context::run_compression_with;
use crate::envcontext::collect_env_context;
use crate::history::build_history;
use crate::prompt::build_prompt;
use crate::title::auto_title;

/// 单次模型请求（含流式接收全程）的整体超时。
pub const MODEL_REQUEST_TIMEOUT_MS: u64 = 3 * 60 * 1000;
pub const MAX_STEER_QUEUE: usize = 10;

/// 会话内不允许超过迭代上限时统一走 fail 路径。
pub const DEFAULT_MAX_ITERATIONS: i64 = 300;

#[derive(Debug, Clone, Default)]
pub struct RunEnv {
    pub workspace_path: String,
    pub roots: Vec<String>,
    pub restricted: bool,
    pub purpose: String,
}

#[derive(Debug, Clone)]
pub struct SteerItem {
    pub message_id: String,
    pub text: String,
    pub attachments: Vec<String>,
    /// 该引导消息的事件 seq：队列剩余项进入接力 run 时作为其 trigger_seq。
    pub trigger_seq: i64,
}

pub struct RunnerDeps {
    pub db: Arc<Db>,
    pub records: Recorder,
    pub registry: Arc<Registry>,
    pub engine: Arc<Mutex<Engine>>,
    pub approvals: Arc<Approvals>,
    pub models: Arc<ModelFactory>,
    pub attachment_dir: String,
    pub data_dir: String,
}

struct Session {
    client: Box<dyn Client>,
    info: craft_model::types::ModelInfo,
}

/// 费用估算（微美元）：token × 美元/百万 token；Anthropic 的输入计数不含缓存 token。
pub fn estimate_cost_micros(usage: &Usage, info: &craft_model::types::ModelInfo, provider_kind: &str) -> i64 {
    let Some(cost) = &info.cost else { return 0 };
    let cache_write = usage.cache_write_tokens.min(usage.cache_tokens);
    let cache_read = usage.cache_tokens - cache_write;
    let mut uncached = usage.input_tokens - usage.cache_tokens;
    if provider_kind == "anthropic" {
        uncached = usage.input_tokens;
    }
    let input_price = cost.input_per_m_tok;
    let read_price = if cost.cache_read_per_m_tok != 0.0 { cost.cache_read_per_m_tok } else { input_price };
    let write_price = if cost.cache_write_per_m_tok != 0.0 { cost.cache_write_per_m_tok } else { input_price };
    let total = uncached as f64 * input_price
        + cache_read as f64 * read_price
        + cache_write as f64 * write_price
        + usage.output_tokens as f64 * cost.output_per_m_tok;
    (total.max(0.0)).round() as i64
}

type NotifyFn = Box<dyn Fn(&str, &str, &str) + Send + Sync>;

pub struct Runner {
    deps: RunnerDeps,
    steers: Mutex<HashMap<String, Vec<SteerItem>>>,
    cancels: Mutex<HashMap<String, CancellationToken>>,
    notifier: Mutex<Option<NotifyFn>>,
    max_iterations: Mutex<Option<Box<dyn Fn() -> i64 + Send + Sync>>>,
    title_pending: Mutex<HashSet<String>>,
}

impl Runner {
    pub fn new(deps: RunnerDeps) -> Self {
        Self {
            deps,
            steers: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            notifier: Mutex::new(None),
            max_iterations: Mutex::new(None),
            title_pending: Mutex::new(HashSet::new()),
        }
    }

    pub fn set_notifier(&self, f: NotifyFn) {
        *self.notifier.lock().unwrap() = Some(f);
    }

    pub fn set_max_iterations_resolver(&self, f: Box<dyn Fn() -> i64 + Send + Sync>) {
        *self.max_iterations.lock().unwrap() = Some(f);
    }

    pub fn is_running(&self, thread_id: &str) -> bool {
        self.cancels.lock().unwrap().contains_key(thread_id)
    }

    /// 当前在途运行的会话 id（窗口关闭/应用退出时统一取消并落审计用）。
    pub fn running_threads(&self) -> Vec<String> {
        self.cancels.lock().unwrap().keys().cloned().collect()
    }

    /// 启动一次运行：写 run 行 + 异步执行。返回 run_id 或错误。
    /// `trigger_seq` 是触发本次运行的用户消息事件 seq（快照给无部件 run 的占位
    /// 消息定序用）；无对应新消息的调用（如 regenerate 重跑上一轮）传 None。
    pub fn run(self: &Arc<Self>, thread_id: &str, trigger_seq: Option<i64>) -> Result<String, String> {
        let thread = get_thread(&self.deps.db.lock_conn(), thread_id).map_err(|e| e.to_string())?;
        let run = create_run(&self.deps.db.lock_conn(), thread_id, &thread.model_id, trigger_seq)
            .map_err(|e| e.to_string())?;
        let _ = self.deps.records.set_thread_status(thread_id, Some(&run.id), "running");
        // 新消息接管：取消旧运行
        if let Some(old) = self.cancels.lock().unwrap().get(thread_id) {
            old.cancel();
        }
        let token = CancellationToken::new();
        self.cancels.lock().unwrap().insert(thread_id.to_string(), token.clone());

        let this = Arc::clone(self);
        let thread_id = thread_id.to_string();
        let run_id = run.id.clone();
        let message_id = run.message_id.clone();
        let mode = thread.mode.clone();
        let model_id = thread.model_id.clone();
        // 壳层命令在 tokio 上下文：直接 spawn；测试/同步上下文：起线程驱动 current-thread 运行时
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    this.execute(token, &thread_id, &run_id, &message_id, &mode, &model_id).await;
                });
            }
            Err(_) => {
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("craftwork-run 运行时启动失败");
                    rt.block_on(async move {
                        this.execute(token, &thread_id, &run_id, &message_id, &mode, &model_id).await;
                    });
                });
            }
        }
        Ok(run.id)
    }

    pub fn cancel(&self, thread_id: &str) -> bool {
        match self.cancels.lock().unwrap().get(thread_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// 运行中引导：无在途运行返回 false（调用方回退普通发送）；队列满返回错误。
    pub fn try_steer(&self, thread_id: &str, item: SteerItem) -> Result<bool, String> {
        if !self.cancels.lock().unwrap().contains_key(thread_id) {
            return Ok(false);
        }
        let mut steers = self.steers.lock().unwrap();
        let queue = steers.entry(thread_id.to_string()).or_default();
        if queue.len() >= MAX_STEER_QUEUE {
            return Err(format!("排队引导已达上限（{MAX_STEER_QUEUE} 条），请等当前步骤完成后再发"));
        }
        queue.push(item);
        Ok(true)
    }

    /// 引导预检：有在途运行且队列未满时返回 true（不入队）。
    /// 落库前调用，让引导消息带上 via=steer 标记（时间线显示「待送达」）。
    pub fn can_steer(&self, thread_id: &str) -> Result<bool, String> {
        if !self.cancels.lock().unwrap().contains_key(thread_id) {
            return Ok(false);
        }
        let steers = self.steers.lock().unwrap();
        if steers.get(thread_id).map(|q| q.len() >= MAX_STEER_QUEUE).unwrap_or(false) {
            return Err(format!("排队引导已达上限（{MAX_STEER_QUEUE} 条），请等当前步骤完成后再发"));
        }
        Ok(true)
    }

    fn take_steers(&self, thread_id: &str) -> Vec<SteerItem> {
        let mut steers = self.steers.lock().unwrap();
        match steers.remove(thread_id) {
            Some(queue) if !queue.is_empty() => queue,
            _ => {
                steers.insert(thread_id.to_string(), Vec::new());
                Vec::new()
            }
        }
    }

    fn has_steers(&self, thread_id: &str) -> bool {
        self.steers
            .lock()
            .unwrap()
            .get(thread_id)
            .map(|q| !q.is_empty())
            .unwrap_or(false)
    }

    /// 队列首（最早入队）引导消息的事件 seq：接力 run 以它为触发消息定序。
    fn first_steer_seq(&self, thread_id: &str) -> Option<i64> {
        self.steers
            .lock()
            .unwrap()
            .get(thread_id)
            .and_then(|q| q.first().map(|item| item.trigger_seq))
    }

    // ---------- 主循环 ----------

    async fn execute(
        self: Arc<Self>,
        token: CancellationToken,
        thread_id: &str,
        first_run_id: &str,
        first_message_id: &str,
        mode: &str,
        model_id: &str,
    ) {
        let mut run_id = first_run_id.to_string();
        let mut message_id = first_message_id.to_string();
        let mut titled = false;
        loop {
            let ok = self
                .clone()
                .execute_run(&token, thread_id, &run_id, &message_id, mode, model_id, titled)
                .await;
            if !ok {
                break;
            }
            titled = true;
            if !self.has_steers(thread_id) {
                break;
            }
            // 接力运行：排队引导并入新 run（历史重建时自动带入）；
            // 触发 seq 取队列首（最早入队）引导消息，供快照占位定序
            let next = match create_run(
                &self.deps.db.lock_conn(),
                thread_id,
                model_id,
                self.first_steer_seq(thread_id),
            ) {
                Ok(next) => next,
                Err(_) => break,
            };
            run_id = next.id;
            message_id = next.message_id;
            let _ = self.deps.records.set_thread_status(thread_id, Some(&run_id), "running");
        }
        // 收尾临界区：丢弃未投递的排队引导（它们已写入事件表，下次运行的历史仍可见）
        self.steers.lock().unwrap().remove(thread_id);
        // 回收本 run 的取消令牌：只移除仍归属本 run 的条目（新 run 已覆盖时不动）。
        // 残留会让 try_steer 误判有在途运行（steer 回退分支不可达）、chat_cancel 误报。
        {
            let mut cancels = self.cancels.lock().unwrap();
            if cancels.get(thread_id).map(|t| *t == token).unwrap_or(false) {
                cancels.remove(thread_id);
            }
        }
    }

    /// 单轮运行：返回 true 表示正常完成（可接力），false 表示已写过结束事件。
    #[allow(clippy::too_many_arguments)]
    async fn execute_run(
        self: Arc<Self>,
        token: &CancellationToken,
        thread_id: &str,
        run_id: &str,
        message_id: &str,
        mode: &str,
        model_id: &str,
        titled: bool,
    ) -> bool {
        let started_at = now_millis();
        self.deps.records.emit(
            event_type::RUN_STARTED,
            Some(thread_id),
            Some(run_id),
            json!({ "messageId": message_id, "modelId": model_id }),
        );

        // 运行环境是值不是全局态：开始时一次性解析，沿调用图下发
        let thread = match get_thread(&self.deps.db.lock_conn(), thread_id) {
            Ok(thread) => thread,
            Err(err) => {
                self.fail(thread_id, run_id, message_id, "history_failed", &err.to_string(), true, started_at);
                return false;
            }
        };
        let mut env = RunEnv::default();
        if !thread.workspace_id.is_empty() {
            match get_workspace(&self.deps.db.lock_conn(), &thread.workspace_id) {
                Ok(ws) => {
                    env = RunEnv {
                        workspace_path: ws.workspace.path.clone(),
                        roots: ws.folders.iter().map(|f| f.path.clone()).collect(),
                        restricted: ws.workspace.trust != "trusted",
                        purpose: ws.workspace.purpose.clone(),
                    };
                }
                Err(_) => {
                    self.fail(
                        thread_id,
                        run_id,
                        message_id,
                        "workspace_missing",
                        "会话绑定的工作区不存在或已被移除，请在侧栏重新选择工作区后重试",
                        false,
                        started_at,
                    );
                    return false;
                }
            }
        }

        // 历史重建（阻塞段放阻塞线程）
        let db = Arc::clone(&self.deps.db);
        let attachment_dir = self.deps.attachment_dir.clone();
        let workspace_path = env.workspace_path.clone();
        let snapshot_thread_id = thread_id.to_string();
        let model_ref_for_caps = model_id.to_string();
        let models_for_caps = Arc::clone(&self.deps.models);
        let history_and_snap = tokio::task::spawn_blocking(move || -> Result<(Vec<Message>, craft_core::contract::Snapshot), String> {
            let snap = craft_store::events::snapshot(&db.lock_conn(), &snapshot_thread_id, 0, 0).map_err(|e| e.to_string())?;
            let caps = models_for_caps
                .resolve(&model_ref_for_caps)
                .map(|info| info.caps.clone())
                .unwrap_or(craft_model::types::Capabilities {
                    stream: true,
                    tools: true,
                    vision: false,
                    reasoning: false,
                    structured: false,
                });
            let history = build_history(&db, &snap, &workspace_path, &attachment_dir, &caps).messages;
            Ok((history, snap))
        })
        .await;
        let (mut history, snap) = match history_and_snap {
            Ok(Ok(pair)) => pair,
            Ok(Err(err)) => {
                self.fail(thread_id, run_id, message_id, "history_failed", &err, true, started_at);
                return false;
            }
            Err(err) => {
                self.fail(thread_id, run_id, message_id, "history_failed", &err.to_string(), true, started_at);
                return false;
            }
        };

        // 环境上下文 + 提示词（阻塞段）
        let data_dir = self.deps.data_dir.clone();
        let workspace_for_env = env.workspace_path.clone();
        let env_context = if !workspace_for_env.is_empty() {
            Some(
                tokio::task::spawn_blocking(move || collect_env_context(&data_dir, &workspace_for_env))
                    .await
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        let prompt_result = build_prompt(crate::prompt::PromptInput {
            mode,
            workspace: &env.workspace_path,
            additional_folders: &env.roots[env.roots.len().min(1)..],
            workspace_purpose: &env.purpose,
            restricted: env.restricted,
            env: env_context.as_ref(),
        });

        let specs: Vec<craft_model::types::ToolSpec> = self
            .deps
            .registry
            .specs_for_mode(mode)
            .into_iter()
            .map(|s| craft_model::types::ToolSpec { name: s.name, description: s.description, schema: s.schema })
            .collect();
        let limit = self
            .max_iterations
            .lock()
            .unwrap()
            .as_ref()
            .map(|f| f())
            .unwrap_or(DEFAULT_MAX_ITERATIONS);
        let mut compression_checked = false;

        for iteration in 0.. {
            if limit <= 0 || iteration < limit as u64 {
                // 允许继续
            } else {
                break;
            }
            if token.is_cancelled() {
                self.canceled(thread_id, run_id, started_at);
                return false;
            }

            // 引导投递点：每个迭代开始（按消息 ID 去重，接力历史可能已包含）
            for item in self.take_steers(thread_id) {
                if history.iter().any(|m| m.id.as_deref() == Some(item.message_id.as_str())) {
                    continue;
                }
                let loaded = load_attachments(&env.workspace_path, &self.deps.attachment_dir, &item.attachments);
                let mut msg = Message::new("user", item.text.clone());
                msg.attachments = loaded.attachments;
                msg.id = Some(item.message_id);
                history.push(msg);
            }

            // 建立模型会话（含失败转移）；压缩只在首轮做一次
            let Some(session) = self.open_session_with_failover(model_id, thread_id, run_id, message_id) else {
                return false; // 已写过结束事件
            };
            if !compression_checked {
                compression_checked = true;
                let db = Arc::clone(&self.deps.db);
                let snap_ref = snap.clone();
                let history_in = history.clone();
                let thread_id_owned = thread_id.to_string();
                let run_id_owned = run_id.to_string();
                let model_id_owned = session.info.id.clone();
                let context_k = session.info.context_k;
                let client = session.client.clone_box();
                let records = self.deps.records.clone();
                let compressed = tokio::task::spawn_blocking(move || {
                    run_compression_with(
                        crate::context::CompressionArgs {
                            db: &db,
                            recorder: &records,
                            snap: &snap_ref,
                            thread_id: &thread_id_owned,
                            run_id: &run_id_owned,
                            client: client.as_ref(),
                            model_id: &model_id_owned,
                            context_k,
                        },
                        history_in,
                    )
                })
                .await;
                if let Ok(compressed) = compressed {
                    history = compressed;
                }
            }

            // 自动命名（首轮并行，防并发重复）
            if iteration == 0 && !titled && thread.title_auto && !self.title_pending.lock().unwrap().contains(thread_id) {
                self.title_pending.lock().unwrap().insert(thread_id.to_string());
                let first_user = history.iter().find(|m| m.role == "user");
                let first_text = first_user.map(|m| m.content.clone()).unwrap_or_default();
                let first_attachments = first_user.map(|m| m.attachments.clone()).unwrap_or_default();
                let db = Arc::clone(&self.deps.db);
                let thread_id_owned = thread_id.to_string();
                let model_id_owned = session.info.id.clone();
                let client = session.client.clone_box();
                tokio::task::spawn_blocking(move || {
                    auto_title(
                        &db,
                        crate::title::AutoTitleInput {
                            client: client.as_ref(),
                            model_id: &model_id_owned,
                            thread_id: &thread_id_owned,
                            first_text: &first_text,
                            first_attachments: &first_attachments,
                        },
                    );
                });
                self.title_pending.lock().unwrap().remove(thread_id);
            }

            let call_start = now_millis();
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut text_part_id: Option<String> = None;
            let mut reasoning_part_id: Option<String> = None;
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut usage: Option<Usage> = None;
            let mut stream_error: Option<ModelError> = None;

            {
                let req = craft_model::types::Request {
                    model: session.info.id.clone(),
                    mode: mode.to_string(),
                    system: prompt_result.blocks.clone(),
                    messages: history.clone(),
                    tools: specs.clone(),
                    max_tokens: if session.info.max_tokens > 0 { session.info.max_tokens } else { 0 },
                    // 思考档位随会话记忆（threads.reasoning_level）；档位类型由模型元数据决定
                    reasoning_kind: session.info.reasoning_kind.clone(),
                    reasoning_level: thread.reasoning_level.clone(),
                };
                let mut stream = session.client.stream(&req);
                loop {
                    let next = tokio::select! {
                        item = stream.next() => item,
                        _ = token.cancelled() => None,
                    };
                    if token.is_cancelled() {
                        break;
                    }
                    match next {
                        Some(Ok(chunk)) => {
                            if let Some(r) = &chunk.reasoning {
                                let part_id = reasoning_part_id.get_or_insert_with(|| new_id(id_prefix::PART));
                                self.deps.records.emit(
                                    event_type::REASONING_DELTA,
                                    Some(thread_id),
                                    Some(run_id),
                                    json!({ "partId": part_id, "text": r }),
                                );
                                reasoning.push_str(r);
                            }
                            if let Some(t) = &chunk.text {
                                let part_id = text_part_id.get_or_insert_with(|| new_id(id_prefix::PART));
                                self.deps.records.emit(
                                    event_type::TEXT_DELTA,
                                    Some(thread_id),
                                    Some(run_id),
                                    json!({ "partId": part_id, "text": t }),
                                );
                                text.push_str(t);
                            }
                            if let Some(call) = chunk.tool_call {
                                tool_calls.push(call);
                            }
                            if let Some(u) = chunk.usage {
                                usage = Some(u);
                            }
                            if chunk.done {
                                break;
                            }
                        }
                        Some(Err(err)) => {
                            stream_error = Some(err);
                            break;
                        }
                        None => break,
                    }
                }
            }

            if token.is_cancelled() {
                self.canceled(thread_id, run_id, started_at);
                return false;
            }
            if let Some(err) = stream_error {
                if err.code == "canceled" {
                    self.canceled(thread_id, run_id, started_at);
                    return false;
                }
                if err.code == "timeout" {
                    self.fail(thread_id, run_id, message_id, "model_timeout", &format!("模型响应超时：{}", err.message), err.retryable(), started_at);
                    return false;
                }
                let code = if err.code == "unknown" { "model_stream_error".to_string() } else { err.code.clone() };
                self.fail(thread_id, run_id, message_id, &code, &err.message, err.retryable(), started_at);
                return false;
            }

            // 正文/推理部件落库（事件表存完整部件而非增量）
            if !reasoning.is_empty() {
                self.deps.records.append_or_warn(&EventRecord {
                    thread_id: thread_id.to_string(),
                    run_id: Some(run_id.to_string()),
                    message_id: message_id.to_string(),
                    kind: "reasoning".into(),
                    payload: json!({
                        "partId": reasoning_part_id.unwrap_or_else(|| new_id(id_prefix::PART)),
                        "text": reasoning,
                    }),
                });
            }
            if !text.is_empty() {
                self.deps.records.append_or_warn(&EventRecord {
                    thread_id: thread_id.to_string(),
                    run_id: Some(run_id.to_string()),
                    message_id: message_id.to_string(),
                    kind: "text".into(),
                    payload: json!({
                        "partId": text_part_id.unwrap_or_else(|| new_id(id_prefix::PART)),
                        "text": text,
                    }),
                });
            }

            if let Some(u) = usage {
                self.record_usage(thread_id, run_id, &session.info, &u, now_millis() - call_start, false);
            }

            // 没有工具调用 = 最终回答
            if tool_calls.is_empty() {
                self.complete(thread_id, run_id, started_at);
                return true;
            }

            let mut assistant = Message::new("assistant", text);
            assistant.tool_calls = tool_calls.clone();
            history.push(assistant);
            for call in tool_calls {
                let (result, aborted) = self
                    .run_tool(token, thread_id, run_id, message_id, &call, &env, mode)
                    .await;
                if aborted {
                    self.canceled(thread_id, run_id, started_at);
                    return false;
                }
                let mut tool_msg = Message::new("tool", result.output);
                tool_msg.tool_call_id = call.id;
                history.push(tool_msg);
            }
        }

        self.fail(
            thread_id,
            run_id,
            message_id,
            "max_iterations",
            &format!("达到模型调用上限（{limit} 轮），已停止本次运行"),
            false,
            started_at,
        );
        false
    }

    // ---------- 会话建立与失败转移 ----------

    /// 建立单模型会话；失败时带出可读原因（模型缺失/已停用/密钥缺失等），供转移文案使用。
    fn open_session(&self, model_ref: &str) -> (Option<Session>, String) {
        match self.deps.models.resolve(model_ref) {
            None => (None, "模型不存在".into()),
            Some(info) => match self.deps.models.client(model_ref) {
                Ok(client) => (Some(Session { client, info }), String::new()),
                Err(err) => (None, err.message),
            },
        }
    }

    /// 建流候选链：[主模型, 备用模型]。
    /// 仅在「尚未进入流」的建立阶段转移（client 构造失败/模型缺失），并写可见 notice。
    /// 一旦流建立成功，后续错误不再转移（避免上下文混乱）。
    fn open_session_with_failover(&self, model_ref: &str, thread_id: &str, run_id: &str, message_id: &str) -> Option<Session> {
        let fallback = self.deps.models.fallback_model(model_ref);
        let candidates: Vec<String> = if !fallback.is_empty() && fallback != model_ref {
            vec![model_ref.to_string(), fallback]
        } else {
            vec![model_ref.to_string()]
        };
        let mut failures: Vec<String> = Vec::new();
        for candidate in &candidates {
            let (session, reason) = self.open_session(candidate);
            if let Some(session) = session {
                if !failures.is_empty() {
                    self.deps.records.append_or_warn(&EventRecord {
                        thread_id: thread_id.to_string(),
                        run_id: Some(run_id.to_string()),
                        message_id: message_id.to_string(),
                        kind: "notice".into(),
                        payload: json!({
                            "partId": new_id(id_prefix::PART),
                            "text": format!("主模型暂时不可用（{}），已切换到 {candidate}。", failures.join("；")),
                        }),
                    });
                }
                return Some(session);
            }
            failures.push(if reason.is_empty() { candidate.clone() } else { reason });
        }
        let error = ModelError::new(
            "provider_unavailable",
            model_ref,
            format!("无法建立模型连接（{}），请检查模型配置", failures.join("；")),
            0,
        );
        self.fail(thread_id, run_id, message_id, &error.code, &error.message, false, now_millis());
        None
    }

    // ---------- 工具执行 ----------

    /// 单工具调用全流程：评估 → tool.started → 拦截判定/审批 → 执行 → 落库 → 审计。
    #[allow(clippy::too_many_arguments)]
    async fn run_tool(
        self: &Arc<Self>,
        token: &CancellationToken,
        thread_id: &str,
        run_id: &str,
        message_id: &str,
        call: &ToolCall,
        env: &RunEnv,
        mode: &str,
    ) -> (ToolResult, bool) {
        let part_id = new_id(id_prefix::PART);
        let assessment = self.deps.registry.assess(
            &if env.roots.is_empty() { vec![".".to_string()] } else { env.roots.clone() },
            &call.name,
            &call.args,
        );
        let spec = self.deps.registry.spec_for(&call.name);
        let summary = match (&spec, &spec.as_ref().and_then(|s| s.summary_arg.clone())) {
            (Some(_s), Some(arg)) => call.args.get(arg).and_then(Value::as_str).unwrap_or_default().to_string(),
            (Some(s), None) => s.summary_text.clone().unwrap_or_else(|| call.name.clone()),
            _ => call.name.clone(),
        };
        let title = spec.as_ref().map(|s| s.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| call.name.clone());

        self.deps.records.emit(
            event_type::TOOL_COMPLETED, // tool.started 是增量事件（不入库）；入库走 tool_call 完成部件
            Some(thread_id),
            Some(run_id),
            json!({
                "partId": part_id,
                "toolCallId": call.id,
                "name": call.name,
                "args": call.args,
                "risk": assessment.risk.as_str(),
                "status": "running",
                "summary": summary,
                "title": title,
            }),
        );

        let mut denied = String::new();
        let mut scope = "auto".to_string();

        if !assessment.exists {
            denied = format!("未知工具：{}", call.name);
        } else if let Some(err) = assessment.prepared.as_ref().and_then(|p| p.err.clone()) {
            denied = err;
        } else if !self.deps.registry.allows(mode, &call.name) {
            denied = "当前模式不支持该工具".into();
        } else {
            let action = self.deps.engine.lock().unwrap().action(env.restricted, assessment.risk);
            match action {
                craft_policy::Action::Deny => denied = "当前审批档位拒绝该操作".into(),
                craft_policy::Action::Allow => {}
                craft_policy::Action::Ask => {
                    let target = assessment.target.clone();
                    let command = call.args.get("command").and_then(Value::as_str).unwrap_or_default().to_string();
                    let grant_target = if call.name == "shell_exec" {
                        let prefix = command_prefix_of(&command);
                        if prefix.is_empty() { target.clone() } else { format!("prefix:{prefix}") }
                    } else {
                        target.clone()
                    };
                    let cached = !env.restricted && self.deps.approvals.allowed(&call.name, &grant_target);
                    if cached {
                        scope = "cache".into();
                    } else {
                        // 受限工作区每次必问且无授权缓存
                        let permanent = self.deps.engine.lock().unwrap().allows_permanent(env.restricted, assessment.risk);
                        let mut facts = vec![
                            ApprovalFact { label: "风险等级".into(), value: assessment.risk.as_str().to_string() },
                            ApprovalFact { label: "操作".into(), value: summary.clone() },
                            ApprovalFact { label: "原因".into(), value: reason_for(assessment.risk).to_string() },
                        ];
                        if !target.is_empty() {
                            facts.push(ApprovalFact { label: "目标".into(), value: target.clone() });
                        }
                        if call.name == "shell_exec" {
                            let prefix = command_prefix_of(&command);
                            if !prefix.is_empty() {
                                facts.push(ApprovalFact { label: "授权粒度".into(), value: format!("允许「{prefix}」开头的命令") });
                            }
                        }
                        self.notify(thread_id, "需要审批", &format!("{title} · {summary}").chars().take(120).collect::<String>());
                        let request = ApprovalRequest {
                            part_id: part_id.clone(),
                            thread_id: thread_id.to_string(),
                            run_id: run_id.to_string(),
                            message_id: message_id.to_string(),
                            tool_call_id: call.id.clone(),
                            tool: call.name.clone(),
                            risk: assessment.risk.as_str().to_string(),
                            title: if title.is_empty() { "需要你的批准".into() } else { title.clone() },
                            target,
                            detail: if assessment.reason.is_empty() { title.clone() } else { assessment.reason.clone() },
                            facts,
                            args: call.args.clone(),
                            allow_permanent: permanent,
                            grant_target: Some(grant_target),
                        };
                        let outcome = self.deps.approvals.request(&request, token.clone()).await;
                        if outcome.status == "canceled" {
                            // 审批取消也落审计（AGENTS 6.2：取消统一按取消落审计）
                            let _ = add_audit(
                                &self.deps.db.lock_conn(),
                                craft_store::records::AuditRecord {
                                    ts: None,
                                    thread_id: Some(thread_id),
                                    run_id: Some(run_id),
                                    tool: call.name.as_str(),
                                    risk: assessment.risk.as_str(),
                                    decision: "canceled",
                                    scope: None,
                                    detail: Some(&format!("审批已取消：{summary}")),
                                },
                            );
                            return (ToolResult::err_code("审批已取消", "tool_error"), true);
                        }
                        if !outcome.allow {
                            denied = "用户拒绝了该操作".into();
                        } else {
                            scope = outcome.scope;
                        }
                    }
                }
            }
        }

        // 执行（阻塞段放阻塞线程，保持取消响应性）
        let was_denied = !denied.is_empty();
        let result = if was_denied {
            ToolResult { output: denied, is_error: true, code: "tool_error".into(), meta: None }
        } else {
            let exec_start = now_millis();
            let registry = Arc::clone(&self.deps.registry);
            let ctx = assessment.prepared.clone().expect("assess 已准备 ctx");
            let executed = tokio::task::spawn_blocking(move || registry.execute(&ctx)).await;
            let mut result = executed.unwrap_or_else(|_| ToolResult::err_code("工具执行任务失败", "tool_error"));
            let mut meta = result.meta.take().unwrap_or(json!({}));
            meta["durationMs"] = json!(now_millis() - exec_start);
            result.meta = Some(meta);
            if token.is_cancelled() {
                return (result, true);
            }
            result
        };

        let duration_ms = result.meta.as_ref().and_then(|m| m.get("durationMs")).and_then(Value::as_i64).unwrap_or(0);
        self.deps.records.append_or_warn(&EventRecord {
            thread_id: thread_id.to_string(),
            run_id: Some(run_id.to_string()),
            message_id: message_id.to_string(),
            kind: "tool_call".into(),
            payload: json!({
                "partId": part_id,
                "toolCallId": call.id,
                "name": call.name,
                "args": call.args,
                "risk": assessment.risk.as_str(),
                "status": if result.is_error { "error" } else { "done" },
                "output": result.output,
                "isError": result.is_error,
                "code": result.code,
                "durationMs": duration_ms,
                "summary": summary,
                "title": title,
            }),
        });

        // 审计：不区分「拦截」与「执行失败」
        let _ = add_audit(
            &self.deps.db.lock_conn(),
            craft_store::records::AuditRecord {
                ts: None,
                thread_id: Some(thread_id),
                run_id: Some(run_id),
                tool: &call.name,
                risk: assessment.risk.as_str(),
                decision: if was_denied {
                    "deny"
                } else if scope != "auto" && scope != "cache" {
                    "allow"
                } else {
                    "auto"
                },
                scope: if scope == "auto" || scope == "cache" { None } else { Some(&scope) },
                detail: Some(&summary),
            },
        );

        // 任务清单部件（前端渲染待办卡片）
        if call.name == "todo_write" && !result.is_error {
            self.deps.records.append_or_warn(&EventRecord {
                thread_id: thread_id.to_string(),
                run_id: Some(run_id.to_string()),
                message_id: message_id.to_string(),
                kind: "todo".into(),
                payload: json!({
                    "partId": new_id(id_prefix::PART),
                    "items": call.args.get("items").cloned().unwrap_or(Value::Array(vec![])),
                }),
            });
        }

        (result, false)
    }

    // ---------- 结束路径 ----------

    fn complete(&self, thread_id: &str, run_id: &str, started_at: i64) {
        let duration_ms = now_millis() - started_at;
        let _ = finish_run(&self.deps.db.lock_conn(), run_id, "done", "", "");
        let _ = self.deps.records.set_thread_status(thread_id, Some(run_id), "idle");
        self.deps.records.emit(
            "run.completed",
            Some(thread_id),
            Some(run_id),
            json!({ "status": "done", "durationMs": duration_ms }),
        );
        self.notify(thread_id, "CraftWork", "本轮回复已完成。");
    }

    fn canceled(&self, thread_id: &str, run_id: &str, started_at: i64) {
        let duration_ms = now_millis() - started_at;
        let _ = finish_run(&self.deps.db.lock_conn(), run_id, "canceled", "canceled", "用户取消");
        let _ = self.deps.records.set_thread_status(thread_id, Some(run_id), "idle");
        self.deps.records.emit(
            "run.canceled",
            Some(thread_id),
            Some(run_id),
            json!({ "status": "canceled", "durationMs": duration_ms }),
        );
        // 取消不发通知
    }

    #[allow(clippy::too_many_arguments)]
    fn fail(&self, thread_id: &str, run_id: &str, message_id: &str, code: &str, message: &str, retryable_flag: bool, started_at: i64) {
        // 错误部件先落库再切状态（等待方以「会话进入 error」为完成信号读快照）
        self.deps.records.append_or_warn(&EventRecord {
            thread_id: thread_id.to_string(),
            run_id: Some(run_id.to_string()),
            message_id: message_id.to_string(),
            kind: "error".into(),
            payload: json!({
                "partId": new_id(id_prefix::PART),
                "code": code,
                "message": message,
                "retryable": retryable_flag,
            }),
        });
        let duration_ms = now_millis() - started_at;
        let _ = finish_run(&self.deps.db.lock_conn(), run_id, "error", code, message);
        let _ = self.deps.records.set_thread_status(thread_id, Some(run_id), "error");
        self.deps.records.emit(
            "run.failed",
            Some(thread_id),
            Some(run_id),
            json!({ "status": "error", "code": code, "message": message, "durationMs": duration_ms }),
        );
        self.notify(thread_id, "CraftWork 运行失败", &message.chars().take(120).collect::<String>());
    }

    fn notify(&self, thread_id: &str, title: &str, message: &str) {
        if let Some(f) = self.notifier.lock().unwrap().as_ref() {
            f(thread_id, title, message);
        }
    }

    // ---------- 用量 ----------

    fn record_usage(&self, thread_id: &str, run_id: &str, info: &craft_model::types::ModelInfo, usage: &Usage, duration_ms: i64, sidecar: bool) {
        let kind = self.deps.models.provider_kind(&info.provider);
        let cost_micros = estimate_cost_micros(usage, info, &kind);
        let _ = add_usage(
            &self.deps.db.lock_conn(),
            craft_store::records::UsageRecord {
                thread_id: Some(thread_id),
                model_id: &format!("{}/{}", info.provider, info.id),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cache_tokens: usage.cache_tokens,
                reasoning_tokens: usage.reasoning_tokens,
                cost_micros,
                duration_ms,
            },
        );
        let _ = add_thread_tokens(&self.deps.db.lock_conn(), thread_id, usage.input_tokens + usage.output_tokens);
        let mut payload = json!({
            "inputTokens": usage.input_tokens,
            "outputTokens": usage.output_tokens,
            "costMicros": cost_micros,
        });
        // sidecar 调用（自动命名）不携带上下文水位字段，避免小请求覆盖「当前上下文占用」
        if !sidecar {
            payload["contextTokens"] = json!(usage.input_tokens + usage.output_tokens);
            payload["cacheTokens"] = json!(usage.cache_tokens);
            payload["durationMs"] = json!(duration_ms);
            payload["tokensPerSecond"] = json!(if duration_ms > 0 { usage.output_tokens as f64 / (duration_ms as f64 / 1000.0) } else { 0.0 });
        }
        self.deps.records.emit(event_type::USAGE_UPDATED, Some(thread_id), Some(run_id), payload);
    }
}

fn command_prefix_of(command: &str) -> String {
    crate::grants::command_prefix(command)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
