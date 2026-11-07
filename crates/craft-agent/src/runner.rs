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
use craft_model::types::{Client, DoneVia, Message, StreamEnd, ToolCall, Usage};
use craft_policy::engine::{reason_for, Engine};
use craft_store::db::Db;
use craft_store::events::EventRecord;
use craft_store::recorder::Recorder;
use craft_store::records::{add_audit, add_usage, get_setting};
use craft_store::runs::{create_run, finish_run, last_run_status};
use craft_store::threads::{add_thread_tokens, get_thread};
use craft_store::usage_stats;
use craft_store::workspaces::get_workspace;
use craft_tools::registry::{Registry, ToolResult};
use craft_tools::todo::normalize_items;

use crate::approvals::{ApprovalFact, ApprovalRequest, Approvals};
use craft_tools::plan::{EnterPlan, ExitPlan};

/// run 级计划状态在 run_tool 间传递的最终值：退出计划态时为 false。
/// 批准路径由 exit_plan 特判分支翻转；调用方据此更新 Run 循环的 plan_active。
pub type PlanExit = bool;
use crate::attachments::load_attachments;
use crate::context::run_compression_with;
use crate::envcontext::collect_env_context;
use crate::history::build_history;
use crate::prompt::build_prompt;
use crate::session_log::{tail_snapshot, SessionLog, LEVEL_DEBUG, LEVEL_ERROR, LEVEL_INFO};
use crate::title::auto_title;

/// 单次模型请求（含流式接收全程）的整体超时。
pub const MODEL_REQUEST_TIMEOUT_MS: u64 = 3 * 60 * 1000;

/// 单会话运行中引导（steer）消息的排队上限，超过即拒绝入队。
pub const MAX_STEER_QUEUE: usize = 10;

/// 单次运行的模型调用轮数上限（每轮一次模型请求）；达到上限统一走 fail 路径结束。
pub const DEFAULT_MAX_ITERATIONS: i64 = 300;

/// 计划软闸门：非计划态 run 内写类（R2+）工具调用达到该次数且从未调用过 enter_plan，
/// 下一迭代在系统提示词注入提醒（不硬拦截，把跑偏的模型拉回计划协议）。
pub const PLAN_NUDGE_THRESHOLD: usize = 4;

/// 运行环境（值对象）：运行开始时从会话绑定的工作区一次性解析，沿调用图下发，
/// 不做全局查询；无工作区绑定的会话为默认空值。
#[derive(Debug, Clone, Default)]
pub struct RunEnv {
    /// 主要目录（会话工作目录）；空串表示未绑定工作区
    pub workspace_path: String,
    /// 可访问的源文件夹：首项为主要目录，其余为附加目录
    pub roots: Vec<String>,
    /// 工作区是否受限（未信任）：高风险直接拒绝、已有授权不生效、不提供永久授权
    pub restricted: bool,
    /// 工作区用途：project | conversation
    pub purpose: String,
}

/// 运行中引导（steer）：已落事件表、排队等待并入下一次模型请求的用户消息。
#[derive(Debug, Clone)]
pub struct SteerItem {
    /// 引导消息 id：投递时按它去重（接力 run 重建的历史可能已包含该消息）
    pub message_id: String,
    /// 用户输入文本
    pub text: String,
    /// 附件路径（投递时经 load_attachments 重新解析加载）
    pub attachments: Vec<String>,
    /// 该引导消息的事件 seq：队列剩余项进入接力 run 时作为其 trigger_seq。
    pub trigger_seq: i64,
}

/// Runner 的依赖装配：存储、策略引擎、审批、模型工厂与工具注册表。
pub struct RunnerDeps {
    pub db: Arc<Db>,
    /// 事件写入与广播的唯一出口
    pub records: Recorder,
    pub registry: Arc<Registry>,
    /// 审批档位策略引擎：档位 × 风险 → allow / ask / deny
    pub engine: Arc<Mutex<Engine>>,
    pub approvals: Arc<Approvals>,
    pub models: Arc<ModelFactory>,
    /// 附件目录：对话附件的存取根（附件路径解析先于工作区）
    pub attachment_dir: String,
    /// 数据目录：读取全局 AGENTS.md 等环境上下文用
    pub data_dir: String,
    /// 会话日志（观测证据：流结束方式 / finish_reason / 落库对照 / 任务退出）
    pub log: Arc<SessionLog>,
    /// 应用版本（run_start 行自描述，跨版本排查）
    pub app_version: String,
}

/// 已建立的模型会话：客户端实例 + 模型元数据（能力/上下文窗口/计价）。
struct Session {
    client: Box<dyn Client>,
    info: craft_model::types::ModelInfo,
}

/// 一次模型请求的用量上报（record_usage 的入参包：口径输入集中一份，避免参数列失控）。
struct UsageReport<'a> {
    usage: &'a Usage,
    /// 请求全程时长（发起调用 → 收流结束）
    duration_ms: i64,
    /// 首 token 延迟（发起调用 → 首个内容片段）；0 = 未产出内容
    ttft_ms: i64,
    /// 本轮输出字符数（正文 + 思考），前端实时速率自校准用
    output_chars: usize,
    /// 思考是否流式可见（流里出现过思考片段）：隐藏思考特判的输入
    reasoning_streamed: bool,
    /// 自动命名等辅助调用：不带上下文水位字段，避免小请求覆盖「当前上下文占用」
    sidecar: bool,
    /// 取消/失败轮：正常落库与累计，但速率 unrated（duration 不完整）
    partial: bool,
}

/// 费用估算（微美元）：token × 美元/百万 token；Anthropic 的输入计数不含缓存 token。
pub fn estimate_cost_micros(usage: &Usage, info: &craft_model::types::ModelInfo, provider_kind: &str) -> i64 {
    let Some(cost) = &info.cost else { return 0 };
    // cache_tokens 为缓存 token 总数（读+写），写部分取与 cache_write_tokens 的较小值防越界
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

/// 桌面通知回调：(thread_id, title, message)；由壳层注册，仅窗口失焦时由壳层自身门控。
type NotifyFn = Box<dyn Fn(&str, &str, &str) + Send + Sync>;

/// Run 循环执行器：登记在途运行与取消令牌、steer 队列与运行参数，
/// 驱动会话串行执行（同会话同时只允许一个 run）。
pub struct Runner {
    deps: RunnerDeps,
    /// 会话 → 待投递 steer 队列（运行中引导）
    steers: Mutex<HashMap<String, Vec<SteerItem>>>,
    /// 会话 → 在途运行取消令牌；键存在即「有在途运行」（单 run 登记表，新运行覆盖旧键）
    cancels: Mutex<HashMap<String, CancellationToken>>,
    /// 桌面通知回调
    notifier: Mutex<Option<NotifyFn>>,
    /// 迭代上限解析器（未注册时用 DEFAULT_MAX_ITERATIONS）
    max_iterations: Mutex<Option<Box<dyn Fn() -> i64 + Send + Sync>>>,
    /// 正在生成自动命名的会话 id（防并发重复起名）
    title_pending: Mutex<HashSet<String>>,
    /// 输入预测缓存（docs/input-prediction/spec.md）：会话 → 下一句建议文本。
    /// 瞬态 UI 辅助只存内存（不入事件表），新完成覆盖旧值。
    predictions: Arc<Mutex<HashMap<String, String>>>,
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
            predictions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 注册桌面通知回调（通知的门控与展示在壳层实现）。
    pub fn set_notifier(&self, f: NotifyFn) {
        *self.notifier.lock().unwrap() = Some(f);
    }

    /// 注册迭代上限解析器（每次运行开始时求值，便于设置即时生效）。
    pub fn set_max_iterations_resolver(&self, f: Box<dyn Fn() -> i64 + Send + Sync>) {
        *self.max_iterations.lock().unwrap() = Some(f);
    }

    /// 会话是否有在途运行（cancels 登记表是否命中）。
    pub fn is_running(&self, thread_id: &str) -> bool {
        self.cancels.lock().unwrap().contains_key(thread_id)
    }

    /// 当前在途运行的会话 id（窗口关闭/应用退出时统一取消并落审计用）。
    pub fn running_threads(&self) -> Vec<String> {
        self.cancels.lock().unwrap().keys().cloned().collect()
    }

    /// 写入输入预测缓存（新完成覆盖旧值）。
    pub fn store_prediction(&self, thread_id: &str, text: String) {
        self.predictions.lock().unwrap().insert(thread_id.to_string(), text);
    }

    /// 读取输入预测缓存（services 层 `inputPredictionGet` 通道）。
    pub fn prediction(&self, thread_id: &str) -> Option<String> {
        self.predictions.lock().unwrap().get(thread_id).cloned()
    }

    /// 输入预测 sidecar（docs/input-prediction/spec.md）：run 正常完成后后台预计算
    /// 下一句建议，写缓存供渲染层 5 秒静默后拉取。best-effort 与自动命名一致：
    /// 开关关闭不调模型；失败静默不阻断；不记账不落事件表。
    fn spawn_input_prediction(&self, thread_id: &str, session: &Session, history: &[Message], final_text: &str) {
        let client = session.client.clone_box();
        let model_id = session.info.id.clone();
        let thread_id = thread_id.to_string();
        // complete 调用点的 history 尚未含本轮最终回答（return 路径不 push），补齐再送预测
        let mut history = history.to_vec();
        let mut assistant = Message::new("assistant", final_text);
        assistant.tool_calls = Vec::new();
        history.push(assistant);
        let db = Arc::clone(&self.deps.db);
        let log = Arc::clone(&self.deps.log);
        let predictions = Arc::clone(&self.predictions);
        let cache_key = thread_id.clone();
        let task = tokio::task::spawn_blocking(move || {
            // 开关门控：关闭则完全不发起模型调用（设置保存即时生效）
            let enabled = db
                .with(|conn| get_setting(conn, "app"))
                .ok()
                .flatten()
                .and_then(|v| v.get("inputPrediction").and_then(Value::as_bool))
                .unwrap_or(true);
            if !enabled {
                return None;
            }
            let started = std::time::Instant::now();
            let prediction = crate::prediction::generate_prediction(client.as_ref(), &model_id, &thread_id, &history);
            // INFO 级：低频 sidecar 事件，默认日志等级下可观测（排查「为何没有预测」的唯一证据源）
            log.write_thread(
                &thread_id,
                LEVEL_INFO,
                "input_prediction",
                json!({
                    "ok": prediction.is_some(),
                    "chars": prediction.as_ref().map(|s| s.chars().count()).unwrap_or(0),
                    "elapsedMs": started.elapsed().as_millis() as u64,
                }),
            );
            prediction
        });
        tokio::task::spawn(async move {
            if let Ok(Some(text)) = task.await {
                predictions.lock().unwrap().insert(cache_key, text);
            }
        });
    }

    /// 启动一次运行：写 run 行 + 异步执行。返回 run_id 或错误。
    /// `trigger_seq` 是触发本次运行的用户消息事件 seq（快照给无部件 run 的占位
    /// 消息定序用）；无对应新消息的调用（如 regenerate 重跑上一轮）传 None。
    pub fn run(self: &Arc<Self>, thread_id: &str, trigger_seq: Option<i64>) -> Result<String, String> {
        let thread = get_thread(&self.deps.db.lock_conn(), thread_id).map_err(|e| e.to_string())?;
        // 上一轮是否以失败收尾（error/interrupted → 消息流挂着未解决的错误卡片）。
        // 查询须在 create_run 之前，否则「最后一个 run」是刚插入的 running；
        // 锁分语句取（lock_conn 不可重入，同表达式嵌套两次取锁会死锁）。
        let last_status = last_run_status(&self.deps.db.lock_conn(), thread_id).map_err(|e| e.to_string())?;
        let run = create_run(&self.deps.db.lock_conn(), thread_id, &thread.model_id, trigger_seq)
            .map_err(|e| e.to_string())?;
        // 会话继续前进即覆盖上一轮失败：回填「错误已解决」，前端实时摘错误卡片、
        // 快照重建（重启/切会话）不再还原。落库失败只降级为卡片暂留，不阻断启动。
        if should_resolve_pending_error(last_status.as_deref()) {
            let _ = self.deps.records.append(&EventRecord {
                thread_id: thread_id.to_string(),
                run_id: Some(run.id.clone()),
                message_id: run.message_id.clone(),
                kind: "error_resolved".into(),
                payload: serde_json::json!({}),
            });
        }
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
        // 壳层命令在 tokio 上下文：直接 spawn；测试/同步上下文：起线程驱动 current-thread 运行时。
        // 两条路径都监控任务退出（run_task_exit 行）：run 任务此前 JoinHandle 被丢弃，
        // panic 时无声消失、无终态事件——日志在此留下「任务死亡」的可见证据。
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let log_thread = thread_id.clone();
                let task = handle.spawn(async move {
                    this.execute(token, &thread_id, &run_id, &message_id, &mode, &model_id, trigger_seq).await;
                });
                let watcher = Arc::clone(self);
                tokio::spawn(async move {
                    let normal = task.await.is_ok();
                    watcher.deps.log.write_thread(
                        &log_thread,
                        LEVEL_INFO,
                        "run_task_exit",
                        json!({ "normal": normal, "note": if normal { "" } else { "run 任务 panic，本次运行无终态事件" } }),
                    );
                });
            }
            Err(_) => {
                let watcher = Arc::clone(self);
                std::thread::spawn(move || {
                    let log_thread = thread_id.clone();
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("craftwork-run 运行时启动失败");
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        rt.block_on(async move {
                            this.execute(token, &thread_id, &run_id, &message_id, &mode, &model_id, trigger_seq).await;
                        })
                    }));
                    watcher.deps.log.write_thread(
                        &log_thread,
                        LEVEL_INFO,
                        "run_task_exit",
                        json!({ "normal": result.is_ok(), "note": if result.is_ok() { "" } else { "run 任务 panic，本次运行无终态事件" } }),
                    );
                });
            }
        }
        Ok(run.id)
    }

    /// 取消会话的在途运行（触发取消令牌）；返回是否确有在途运行被取消。
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

    /// 取出会话的全部待投递 steer（无待投递时返回空 Vec）。
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

    /// 是否还有待投递的 steer（主循环据此决定是否接力新 run）。
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

    /// 主循环：执行单轮运行，正常结束后消费 steer 队列接力新 run，直至无引导或取消/失败。
    /// 收尾统一在此处：丢弃未消费的排队 steer、回收仍归属本次运行的取消令牌。
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        self: Arc<Self>,
        token: CancellationToken,
        thread_id: &str,
        first_run_id: &str,
        first_message_id: &str,
        mode: &str,
        model_id: &str,
        first_trigger_seq: Option<i64>,
    ) {
        let mut run_id = first_run_id.to_string();
        let mut message_id = first_message_id.to_string();
        let mut trigger_seq = first_trigger_seq;
        let mut titled = false;
        loop {
            let ok = self
                .clone()
                .execute_run(&token, thread_id, &run_id, &message_id, mode, model_id, titled, trigger_seq)
                .await;
            if !ok {
                break;
            }
            titled = true;
            if !self.has_steers(thread_id) {
                break;
            }
            // 接力运行：排队引导并入新 run（历史重建时自动带入）；
            // 触发 seq 取队列首（最早入队）引导消息，供快照占位定序与计划模式还原
            let next_trigger_seq = self.first_steer_seq(thread_id);
            let next = match create_run(
                &self.deps.db.lock_conn(),
                thread_id,
                model_id,
                next_trigger_seq,
            ) {
                Ok(next) => next,
                Err(_) => break,
            };
            run_id = next.id;
            message_id = next.message_id;
            trigger_seq = next_trigger_seq;
            let _ = self.deps.records.set_thread_status(thread_id, Some(&run_id), "running");
        }
        // 收尾临界区：丢弃未投递的排队引导（它们已写入事件表，下次运行的历史仍可见）。
        // 丢弃也回执 steer_delivered：内容不会丢，「待送达」语义不再成立（实时流摘标记，
        // 快照重建同样摘除）。
        for item in self.steers.lock().unwrap().remove(thread_id).unwrap_or_default() {
            self.deps.records.append_or_warn(&EventRecord {
                thread_id: thread_id.to_string(),
                run_id: None,
                message_id: item.message_id,
                kind: "steer_delivered".into(),
                payload: json!({}),
            });
        }
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
        first_message_id: &str,
        mode: &str,
        model_id: &str,
        titled: bool,
        trigger_seq: Option<i64>,
    ) -> bool {
        // assistant 消息 id 在 run 内可变：消费引导（steer）时切换新消息，
        // 后续输出不再挂在引导气泡上方的旧消息里（时间线时序与视觉位置一致）
        let mut message_id = first_message_id.to_string();
        let started_at = now_millis();
        // 运行环境是值不是全局态：开始时一次性解析，沿调用图下发
        let thread = match get_thread(&self.deps.db.lock_conn(), thread_id) {
            Ok(thread) => thread,
            Err(err) => {
                self.fail(thread_id, run_id, &message_id, "history_failed", &err.to_string(), true, started_at);
                return false;
            }
        };
        // 计划模式标记随触发消息落库，此处按 trigger 还原（regenerate 跟随上一轮、steer 接力取首条引导）
        let plan_mode =
            craft_store::events::plan_mode_for_trigger(&self.deps.db.lock_conn(), thread_id, trigger_seq).unwrap_or(false);
        // 计划数据：计划 run 启动即建行（未批准、无文件）；exit_plan 决定时 persist_plan_text 落盘回填。
        // plan_id 精确持有本次 run 的计划行（compact 续读与审计档案用，不驱动前端 UI）
        let mut plan_id: Option<String> = None;
        if plan_mode
            && let Ok(plan) = craft_store::plans::create_plan(&self.deps.db.lock_conn(), thread_id, run_id)
        {
            plan_id = Some(plan.id);
        }
        // run 级计划态：随消息落库的 planMode 初始化；运行中由 enter_plan/exit_plan 翻转
        let mut plan_active = plan_mode;
        // 计划软闸门状态：run 内写类（R2+）工具调用计数 + 是否调过 enter_plan
        let mut plan_write_calls = 0usize;
        let mut plan_entered = plan_mode;
        let worktree_path = thread.worktree_path.clone();
        let worktree_branch = thread.worktree_branch.clone();
        self.deps.records.emit(
            event_type::RUN_STARTED,
            Some(thread_id),
            Some(run_id),
            json!({
                "messageId": message_id.as_str(),
                "modelId": model_id,
                "planMode": plan_mode,
                "worktreePath": worktree_path.as_str(),
            }),
        );
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
                        &message_id,
                        "workspace_missing",
                        "会话绑定的工作区不存在或已被移除，请在侧栏重新选择工作区后重试",
                        false,
                        started_at,
                    );
                    return false;
                }
            }
        }
        // worktree 绑定：主要目录切换到独立 git 工作树（附加目录保留），fs/命令/Git 全部在工作树内进行；
        // 目录丢失按失败处理（重发时服务层 ensure 会 prune 重建），不允许静默回退到主目录
        let mut worktree_branch_for_prompt: Option<String> = None;
        if !worktree_path.is_empty() && !env.workspace_path.is_empty() {
            if !std::path::Path::new(&worktree_path).exists() {
                self.fail(
                    thread_id,
                    run_id,
                    &message_id,
                    "worktree_missing",
                    "会话绑定的 git 工作树目录已不存在，请重新发送消息以自动重建",
                    true,
                    started_at,
                );
                return false;
            }
            env.workspace_path = worktree_path.clone();
            if let Some(first) = env.roots.first_mut() {
                *first = worktree_path.clone();
            }
            worktree_branch_for_prompt = Some(worktree_branch).filter(|b| !b.is_empty());
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
                self.fail(thread_id, run_id, &message_id, "history_failed", &err, true, started_at);
                return false;
            }
            Err(err) => {
                self.fail(thread_id, run_id, &message_id, "history_failed", &err.to_string(), true, started_at);
                return false;
            }
        };
        // 环境上下文 + 技能清单 + 提示词（阻塞段）
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
        // 技能清单：每次 run 重扫（受限工作区不扫工作区级），禁用/删除下一 run 生效
        let db_for_skills = Arc::clone(&self.deps.db);
        let data_dir_for_skills = self.deps.data_dir.clone();
        let workspace_for_skills = env.workspace_path.clone();
        let restricted_for_skills = env.restricted;
        let skill_list = tokio::task::spawn_blocking(move || {
            craft_tools::skills::scan_available(&db_for_skills, &data_dir_for_skills, &workspace_for_skills, restricted_for_skills)
        })
        .await
        .unwrap_or_default();
        let mut prompt_active = build_prompt(crate::prompt::PromptInput {
            mode,
            workspace: &env.workspace_path,
            // roots 首项是主要目录（已由 workspace 字段承载），附加目录从第二项起
            additional_folders: &env.roots[env.roots.len().min(1)..],
            workspace_purpose: &env.purpose,
            restricted: env.restricted,
            // 计划段按 run 级 plan_active 动态注入：enter_plan/exit_plan 中途翻转后，
            // 下一 iter 的提示词要跟着变（见循环内按 prompt_plan_mode 判断重建）
            plan_mode: plan_active,
            plan_nudge: false,
            worktree_branch: worktree_branch_for_prompt.as_deref(),
            env: env_context.as_ref(),
            skills: &skill_list,
        });
        // 上次构造提示词时的计划态与软闸门提醒标记（循环内重建的判断依据）
        let mut prompt_plan_mode = plan_active;
        let mut prompt_plan_nudge = false;

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

        // 观测：run 起始（自描述起点：版本 / 模型 / 模式 / 历史规模）
        self.deps.log.write_thread(
            thread_id,
            LEVEL_INFO,
            "run_start",
            json!({
                "runId": run_id,
                "appVersion": self.deps.app_version,
                "model": model_id,
                "mode": mode,
                "planMode": plan_mode,
                "messages": history.len(),
                "historyChars": history.iter().map(|m| m.content.chars().count()).sum::<usize>(),
            }),
        );

        for iteration in 0.. {
            // 迭代上限：limit <= 0 视为不限制（配置异常兜底），达到上限即退出走 fail
            if limit <= 0 || iteration < limit as u64 {
            } else {
                break;
            }
            if token.is_cancelled() {
                self.canceled(thread_id, run_id, started_at);
                return false;
            }

            // 引导投递点：每个迭代开始（按消息 ID 去重，接力历史可能已包含）。
            // 每条取出的引导都回执 steer_delivered（落库 + 广播，前端据此摘「待送达」标记）。
            // 先注入收集回执、再统一落库：nextMessageId 取决于全批结果——
            // 注入批次：实际注入的附带新 assistant 消息 id，后续输出切换过去，
            // 不再挂在引导气泡上方的旧消息里；混合批次中已随历史重建进入的去重项不带。
            // 全去重批次（接力 trigger 单独成批、本批无注入）：回复仍流到接力 run 自己的
            // assistant 消息，回执必须带上它的 id——前端流式输出只往「最后一条 streaming
            // 消息」续写，run.started 建的正主占位若无回执背书，会被 steer.delivered 的
            // 空壳清理误删，整条回复从此无处可挂而静默丢失。
            let delivered = self.take_steers(thread_id);
            if !delivered.is_empty() {
                let mut receipts: Vec<(String, Value)> = Vec::with_capacity(delivered.len());
                let mut injected = false;
                let mut next_message_id = String::new();
                for item in delivered {
                    let already =
                        history.iter().any(|m| m.id.as_deref() == Some(item.message_id.as_str()));
                    let mut payload = json!({});
                    if !already {
                        if !injected {
                            injected = true;
                            next_message_id = new_id(id_prefix::MESSAGE);
                        }
                        let loaded =
                            load_attachments(&env.workspace_path, &self.deps.attachment_dir, &item.attachments);
                        let mut msg = Message::new("user", item.text.clone());
                        msg.attachments = loaded.attachments;
                        msg.id = Some(item.message_id.clone());
                        history.push(msg);
                        payload["nextMessageId"] = json!(next_message_id);
                    }
                    receipts.push((item.message_id, payload));
                }
                if !injected {
                    for (_, payload) in receipts.iter_mut() {
                        payload["nextMessageId"] = json!(message_id);
                    }
                }
                for (receipt_message_id, payload) in receipts {
                    self.deps.records.append_or_warn(&EventRecord {
                        thread_id: thread_id.to_string(),
                        run_id: Some(run_id.to_string()),
                        message_id: receipt_message_id,
                        kind: "steer_delivered".into(),
                        payload,
                    });
                }
                if injected {
                    message_id = next_message_id;
                }
            }

            // 建立模型会话（含失败转移）；压缩每轮检查（实测水位优先，长任务中后段也能触发）
            let Some(session) = self.open_session_with_failover(model_id, thread_id, run_id, &message_id) else {
                return false; // 已写过结束事件
            };
            {
                let db = Arc::clone(&self.deps.db);
                let snap_ref = snap.clone();
                let history_in = history.clone();
                let before_messages = history_in.len();
                let thread_id_owned = thread_id.to_string();
                let run_id_owned = run_id.to_string();
                let model_id_owned = session.info.id.clone();
                let context_k = session.info.context_k;
                let client = session.client.clone_box();
                let records = self.deps.records.clone();
                let compressed = tokio::task::spawn_blocking(move || {
                    // 已批准计划的全文：有则读盘（压缩后拼保留窗头部，ZCode plan-file continuity）
                    let approved_plan = Self::read_approved_plan_text(&db, thread_id_owned.as_str());
                    // 最近一次实测的完整提示词（含缓存）：当前上下文的真值，压缩判断优先用它
                    let measured_prompt_tokens = craft_store::view::latest_usage_by_thread(&db.lock_conn(), thread_id_owned.as_str())
                        .map(|u| u.prompt_tokens)
                        .unwrap_or(0);
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
                            approved_plan,
                            measured_prompt_tokens,
                        },
                        history_in,
                    )
                })
                .await;
                match compressed {
                    Ok(after) => {
                        self.deps.log.write_thread(
                            thread_id,
                            LEVEL_INFO,
                            "compaction",
                            json!({ "runId": run_id, "iter": iteration, "beforeMessages": before_messages, "afterMessages": after.len() }),
                        );
                        history = after;
                    }
                    Err(err) => {
                        // 压缩失败此前被静默吞掉（历史原样保留）；至少留日志可查
                        self.deps.log.write_thread(
                            thread_id,
                            LEVEL_ERROR,
                            "compaction_failed",
                            json!({ "runId": run_id, "error": err.to_string() }),
                        );
                    }
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
            // 计划软闸门：非计划态 + 写类调用达到阈值 + 本 run 从未调过 enter_plan + 无已批准计划
            // （豁免跨 run「批准并执行」的实施续跑）→ 提醒拉回协议。只提醒不拦截。
            let plan_nudge = !plan_active
                && !plan_entered
                && plan_write_calls >= PLAN_NUDGE_THRESHOLD
                && Self::read_approved_plan_text(&self.deps.db, thread_id).is_none();
            // 计划态中途翻转后重建提示词（dynamic 块随 plan_active / plan_nudge 变化；
            // 用最后构造时的标记判断是否需要重建，避免 PromptResult 比较开销）
            let need_rebuild = prompt_plan_mode != plan_active || prompt_plan_nudge != plan_nudge;
            if need_rebuild {
                prompt_plan_mode = plan_active;
                prompt_plan_nudge = plan_nudge;
                prompt_active = build_prompt(crate::prompt::PromptInput {
                    mode,
                    workspace: &env.workspace_path,
                    additional_folders: &env.roots[env.roots.len().min(1)..],
                    workspace_purpose: &env.purpose,
                    restricted: env.restricted,
                    plan_mode: plan_active,
                    plan_nudge,
                    worktree_branch: worktree_branch_for_prompt.as_deref(),
                    env: env_context.as_ref(),
                    skills: &skill_list,
                });
            }
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut text_chars = 0usize;
            let mut reasoning_chars = 0usize;
            let mut text_part_id: Option<String> = None;
            let mut reasoning_part_id: Option<String> = None;
            let mut tool_calls: Vec<ToolCall> = Vec::new();
            let mut usage: Option<Usage> = None;
            // 首 token 延迟（发起调用 → 首个内容片段；usage chunk 不算内容）。
            // 0 = 本轮未产出内容（速率 unrated 的输入之一）
            let mut first_token_ms: i64 = 0;
            let mut stream_error: Option<ModelError> = None;
            let mut stream_end_info: Option<StreamEnd> = None;
            // debug 级聚合统计（每 2s 一行；计数增量为 O(delta)，不做全量重扫）
            let mut agg_chunks = 0usize;
            let mut agg_last_ms = call_start;

            let req_max_tokens = if session.info.max_tokens > 0 { session.info.max_tokens } else { 0 };
            {
                let req = craft_model::types::Request {
                    model: session.info.id.clone(),
                    mode: mode.to_string(),
                    system: prompt_active.blocks.clone(),
                    messages: history.clone(),
                    tools: specs.clone(),
                    max_tokens: req_max_tokens,
                    // 思考档位随会话记忆（threads.reasoning_level）；档位类型由模型元数据决定
                    reasoning_kind: session.info.reasoning_kind.clone(),
                    reasoning_level: thread.reasoning_level.clone(),
                    // 会话 ID 供自定义请求头 {sessionId} 占位符替换（网关路由归并）
                    session_id: thread_id.to_string(),
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
                            // 首 token：首个产出内容（文本/思考/工具调用参数）的到达时刻
                            if first_token_ms == 0 && (chunk.text.is_some() || chunk.reasoning.is_some() || chunk.tool_call.is_some()) {
                                first_token_ms = now_millis() - call_start;
                            }
                            if let Some(r) = &chunk.reasoning {
                                let part_id = reasoning_part_id.get_or_insert_with(|| new_id(id_prefix::PART));
                                self.deps.records.emit(
                                    event_type::REASONING_DELTA,
                                    Some(thread_id),
                                    Some(run_id),
                                    json!({ "partId": part_id, "text": r }),
                                );
                                reasoning_chars += r.chars().count();
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
                                text_chars += t.chars().count();
                                text.push_str(t);
                            }
                            if let Some(call) = chunk.tool_call {
                                tool_calls.push(call);
                            }
                            if let Some(u) = chunk.usage {
                                usage = Some(u);
                            }
                            if let Some(end) = chunk.end {
                                stream_end_info = Some(end);
                            }
                            agg_chunks += 1;
                            let agg_now = now_millis();
                            if agg_now - agg_last_ms >= 2000 {
                                self.deps.log.write_thread(
                                    thread_id,
                                    LEVEL_DEBUG,
                                    "stream_progress",
                                    json!({
                                        "runId": run_id,
                                        "iter": iteration,
                                        "chunks": agg_chunks,
                                        "reasoningChars": reasoning_chars,
                                        "textChars": text_chars,
                                    }),
                                );
                                agg_chunks = 0;
                                agg_last_ms = agg_now;
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

            // 观测：每轮流结束——截断排查的关键行。结束方式 / finish_reason / 传输质量 /
            // 网关 token 数与本地字节数对照 / 内容尾部快照；写点在错误与落库检查之前，
            // 保证取消 / 失败路径（内容不落库）也有完整记录。
            let anomaly = stream_end_info.as_ref().and_then(stream_end_anomaly);
            self.deps.log.write_thread(thread_id, LEVEL_INFO, "stream_end", {
                let end = stream_end_info.as_ref();
                json!({
                    "runId": run_id,
                    "iter": iteration,
                    "model": session.info.id,
                    "maxTokens": req_max_tokens,
                    "messages": history.len(),
                    "doneVia": end.map(|e| done_via_name(&e.done_via)).unwrap_or("none"),
                    "finishReason": end.and_then(|e| e.finish_reason.clone()),
                    "usage": usage.as_ref().map(|u| json!({
                        "input": u.input_tokens, "output": u.output_tokens,
                        "cache": u.cache_tokens, "reasoning": u.reasoning_tokens,
                    })),
                    "sse": end.map(|e| json!({
                        "invalidUtf8Chunks": e.sse.invalid_utf8_chunks,
                        "oversizedLines": e.sse.oversized_lines,
                        "trailingPartialBytes": e.sse.trailing_partial_bytes,
                    })),
                    "reasoningChars": reasoning_chars,
                    "textChars": text_chars,
                    "reasoningTail": tail_snapshot(&reasoning, 200),
                    "textTail": tail_snapshot(&text, 200),
                    "toolCalls": tool_calls.len(),
                    "durationMs": now_millis() - call_start,
                    "anomaly": anomaly,
                })
            });
            if let Some(reason) = anomaly {
                // 异常轮留存提供商侧原始事件尾部（限 4KB；截断瞬间的行为铁证）
                self.deps.log.write_thread(
                    thread_id,
                    LEVEL_INFO,
                    "stream_raw_tail",
                    json!({
                        "runId": run_id,
                        "iter": iteration,
                        "reason": reason,
                        "rawTail": stream_end_info.as_ref().map(|e| e.raw_tail.as_str()).unwrap_or_default(),
                    }),
                );
            }

            // 取消/失败轮的部分用量记账：Anthropic 在 message_start 即已知 input/缓存
            //（usage 变量已持有部分值），不记账会整轮丢失；OpenAI 收尾才给 usage，
            // 取消时 usage 为 None 自然跳过。部分轮速率 unrated（duration 不完整），
            // 但 input/缓存照常落库与累计（真实发生的计费量）。
            if let Some(u) = &usage {
                self.record_usage(
                    thread_id,
                    run_id,
                    &session.info,
                    &UsageReport {
                        usage: u,
                        duration_ms: now_millis() - call_start,
                        ttft_ms: first_token_ms,
                        output_chars: text_chars + reasoning_chars,
                        reasoning_streamed: reasoning_chars > 0,
                        sidecar: false,
                        partial: true,
                    },
                );
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
                    self.fail(thread_id, run_id, &message_id, "model_timeout", &format!("模型响应超时：{}", err.message), err.retryable(), started_at);
                    return false;
                }
                let code = if err.code == "unknown" { "model_stream_error".to_string() } else { err.code.clone() };
                self.fail(thread_id, run_id, &message_id, &code, &err.message, err.retryable(), started_at);
                return false;
            }

            // 正文/推理部件落库（事件表存完整部件而非增量）
            if !reasoning.is_empty() {
                self.deps.records.append_or_warn(&EventRecord {
                    thread_id: thread_id.to_string(),
                    run_id: Some(run_id.to_string()),
                    message_id: message_id.clone(),
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
                    message_id: message_id.clone(),
                    kind: "text".into(),
                    payload: json!({
                        "partId": text_part_id.unwrap_or_else(|| new_id(id_prefix::PART)),
                        "text": text,
                    }),
                });
            }

            // 观测：落库对照（与 stream_end 的字节数对账；append_or_warn 失败会静默降级为纯广播）
            self.deps.log.write_thread(
                thread_id,
                LEVEL_INFO,
                "persist",
                json!({
                    "runId": run_id,
                    "iter": iteration,
                    "messageId": message_id,
                    "reasoningPersisted": !reasoning.is_empty(),
                    "textPersisted": !text.is_empty(),
                }),
            );

            match usage {
                Some(u) => self.record_usage(
                    thread_id,
                    run_id,
                    &session.info,
                    &UsageReport {
                        usage: &u,
                        duration_ms: now_millis() - call_start,
                        ttft_ms: first_token_ms,
                        output_chars: text_chars + reasoning_chars,
                        reasoning_streamed: reasoning_chars > 0,
                        sidecar: false,
                        partial: false,
                    },
                ),
                // 网关未给用量：不落库（全 0 行会污染「最新一行用量」推导的水位/速度），留日志可查
                None => self.deps.log.write_thread(
                    thread_id,
                    LEVEL_INFO,
                    "usage_missing",
                    json!({ "runId": run_id, "iter": iteration }),
                ),
            }

            // 没有工具调用 = 最终回答
            if tool_calls.is_empty() {
                self.complete(thread_id, run_id, started_at);
                self.spawn_input_prediction(thread_id, &session, &history, &text);
                return true;
            }

            let mut assistant = Message::new("assistant", text);
            assistant.tool_calls = tool_calls.clone();
            history.push(assistant);
            for call in tool_calls {
                let (result, aborted, plan_ended, plan_active_after, plan_feedback_msg) = self
                    .run_tool(token, thread_id, run_id, &message_id, &call, &env, mode, plan_active, plan_id.as_deref())
                    .await;
                // 计划态随协议工具翻转（enter_plan 进入 / exit_plan 批准退出），影响后续 iter
                plan_active = plan_active_after;
                // 计划软闸门计数：协议进入置标记；非计划态的写类调用累计（含被拒的尝试，
                // 反映模型意图），执行已取消的调用不再追加
                if call.name == EnterPlan::NAME {
                    plan_entered = true;
                } else if !aborted {
                    let risk = if call.name == "shell_exec" {
                        let command = call.args.get("command").and_then(Value::as_str).unwrap_or_default();
                        craft_policy::PolicyRisk::parse(craft_policy::command::classify_command(command).0.as_str())
                            .unwrap_or(craft_policy::PolicyRisk::R2)
                    } else {
                        self.deps.registry.spec_for(&call.name).map(|s| s.risk).unwrap_or(craft_policy::PolicyRisk::R2)
                    };
                    if risk.level() >= craft_policy::PolicyRisk::R2.level() {
                        plan_write_calls += 1;
                    }
                }
                if aborted {
                    self.canceled(thread_id, run_id, started_at);
                    return false;
                }
                // 计划被无反馈拒绝：终止本轮，按正常完成收尾（会话保持计划态等待新输入；
                // 计划文件已落盘未批准，前端提示条数据就绪）。不是 canceled 也不是 fail。
                if plan_ended {
                    self.complete(thread_id, run_id, started_at);
                    return true;
                }
                // 拒绝带反馈：反馈作为真实用户消息注入本 run 历史（持久化已在 run_tool 完成），
                // 模型下一 iter 在计划态下看到反馈并修订计划
                if let Some((feedback_message_id, feedback_text)) = plan_feedback_msg {
                    let mut msg = Message::new("user", feedback_text);
                    msg.id = Some(feedback_message_id);
                    history.push(msg);
                }
                self.deps.log.write_thread(
                    thread_id,
                    LEVEL_DEBUG,
                    "tool_done",
                    json!({
                        "runId": run_id,
                        "iter": iteration,
                        "tool": call.name,
                        "outputChars": result.output.chars().count(),
                        "isError": result.is_error,
                        "code": result.code,
                    }),
                );
                let mut tool_msg = Message::new("tool", result.output);
                tool_msg.tool_call_id = call.id;
                history.push(tool_msg);
            }
        }

        self.fail(
            thread_id,
            run_id,
            &message_id,
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
    /// plan_active 为本 run 当前是否处于计划态：进入分支会翻转它，返回值第 3 元带回
    /// 最终值；Run 循环据此更新自己的 plan_active（影响后续 iter 的提示词与闸门）。
    /// plan_id 为本 run 的计划行 id（exit_plan 批准/拒绝时精确回填，None=无计划行）。
    /// 返回 (结果, 取消, 正常终止轮次, 计划态最终值, 反馈用户消息)：
    /// - 取消为 true：Run 循环按 canceled 收尾（审批取消 / 执行中取消）；
    /// - 正常终止轮次为 true：计划被无反馈拒绝，Run 循环按正常完成（done）收尾；
    /// - 反馈用户消息 Some((message_id, 正文)) 时 Run 循环把它 push 进本 run 历史
    ///   （模型下一 iter 看到反馈；持久化已在本函数完成）。
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
        mut plan_active: bool,
        plan_id: Option<&str>,
    ) -> (ToolResult, bool, bool, bool, Option<(String, String)>) {
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

        // ---------- 计划模式协议分支（对应 ZCode EnterPlanMode / ExitPlanMode）----------
        //
        // 工具 execute 是薄壳（参数校验 + 确认文本），状态切换与审批边界在 Runner：
        // 这里修改本函数局部副本 plan_active，最终状态经返回值第 3 元带回 Run 循环
        // （Run 循环据此更新自己的 plan_active，影响下一 iter 的提示词注入与闸门）。
        // exit_plan 三种结果（对齐 ZCode plan-mode-approval-boundary 的审阅边界）：
        // - 批准：退出计划态（返回值带 plan_exit=true），计划正文落盘回填（approved=1），
        //   run 继续迭代直接实施；
        // - 拒绝带反馈：保持计划态，反馈升级为真实用户消息（本分支落库，返回值带回
        //   (message_id, 正文) 供 Run 循环注入本 run 历史），工具结果只给桥接说明，
        //   模型修订后重新提交；
        // - 拒绝无反馈（含关闭卡片）：保持计划态，计划正文落盘（未批准，提示条数据
        //   就绪），置 denied + 终止轮次信号，Run 循环按正常完成收尾，等待用户新输入。
        let mut plan_feedback: Option<(String, String)> = None;
        let mut plan_overridden: Option<ToolResult> = None;
        if call.name == EnterPlan::NAME {
            // 进入计划态：幂等（已在计划态不重复广播）。纯广播不入库（plan.entered
            // 是运行期瞬态，重启后由用户重新勾选，不持久化）。
            if !plan_active {
                plan_active = true;
                self.deps.records.emit(
                    event_type::PLAN_ENTERED,
                    Some(thread_id),
                    Some(run_id),
                    json!({ "partId": part_id, "threadId": thread_id }),
                );
            }
        } else if call.name == ExitPlan::NAME {
            if !plan_active {
                // 对应 ZCode mode.plan.exitOnly：不在计划态时拒绝（普通错误结果，模型可自行纠正）
                denied = "当前不在计划模式：exit_plan 只用于提交计划结束计划态。如果计划已获批准，请直接继续实施。".into();
            } else {
                let plan_text = call.args.get("plan").and_then(Value::as_str).unwrap_or_default();
                // 参数校验失败按普通拒绝处理（模型可修正重试），不弹审批卡
                if plan_text.trim().is_empty() {
                    denied = "计划内容不能为空，请把完整计划作为 plan 参数重新提交".into();
                } else if plan_text.chars().count() > craft_tools::plan::PLAN_MAX_CHARS {
                    denied = format!(
                        "计划过长（{} 字符），上限 {} 字符，请精简后重新提交",
                        plan_text.chars().count(),
                        craft_tools::plan::PLAN_MAX_CHARS
                    );
                } else {
                    // 无条件审批（D8 修订）：exit_plan 不看档位——自由档也必须经用户确认
                    let request = ApprovalRequest {
                        part_id: part_id.clone(),
                        thread_id: thread_id.to_string(),
                        run_id: run_id.to_string(),
                        message_id: message_id.to_string(),
                        tool_call_id: call.id.clone(),
                        tool: call.name.clone(),
                        risk: assessment.risk.as_str().to_string(),
                        title: "计划审阅".into(),
                        target: String::new(),
                        detail: "模型提交了实施计划，等待你的决定".into(),
                        facts: vec![
                            ApprovalFact {
                                label: "计划长度".into(),
                                value: format!("{} 字符", plan_text.chars().count()),
                            },
                            ApprovalFact {
                                label: "批准后".into(),
                                value: "立即开始按计划实施（写操作遵循当前审批档位）".into(),
                            },
                        ],
                        args: call.args.clone(),
                        allow_permanent: false,
                        grant_target: None,
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
                                detail: Some("计划审批已取消"),
                            },
                        );
                        return (ToolResult::err_code("计划审批已取消", "tool_error"), true, false, plan_active, None);
                    }
                    if outcome.allow {
                        // 批准：退出计划态 → 计划落盘回填（approved=1）→ run 继续迭代直接实施
                        plan_active = false;
                        self.persist_plan_text(thread_id, run_id, plan_id, plan_text, true);
                        plan_overridden = Some(ToolResult::ok(
                            "用户已批准计划，开始实施。写操作恢复按当前审批档位处理；实施过程中如需偏离计划，先向用户说明。",
                        ));
                    } else if let Some(feedback) = outcome.feedback {
                        // 拒绝带反馈：保持计划态，反馈升级为真实用户消息（历史注入由调用方完成）
                        self.persist_plan_text(thread_id, run_id, plan_id, plan_text, false);
                        let user_message_id = new_id(id_prefix::MESSAGE);
                        // 落库（快照/恢复可见）+ 广播 user.message（用户气泡）；via=plan_feedback
                        // 标记来源，前端渲染为普通用户消息。反馈本体在这里持久化，
                        // resolved 事件只带长度摘要。
                        let append_ok = self
                            .deps
                            .records
                            .append(&EventRecord {
                                thread_id: thread_id.to_string(),
                                message_id: user_message_id.clone(),
                                run_id: None,
                                kind: "user_message".into(),
                                payload: json!({
                                    "text": feedback,
                                    "attachments": [],
                                    "planMode": true,
                                    "via": "plan_feedback",
                                }),
                            })
                            .is_ok();
                        if append_ok {
                            plan_feedback = Some((user_message_id, feedback));
                        }
                        plan_overridden = Some(ToolResult::ok(
                            "用户没有批准这份计划，并将修改意见作为下一条用户消息给出。请结合反馈修订计划，修订完成后重新调用 exit_plan 提交。",
                        ));
                    } else {
                        // 拒绝无反馈：保持计划态，计划落盘（未批准），终止当前轮次等待用户新输入。
                        // 注意这里不是 fail/canceled——会话与 run 都按正常完成收尾。
                        self.persist_plan_text(thread_id, run_id, plan_id, plan_text, false);
                        denied = "用户没有批准这份计划。本轮到此为止，等待用户的下一步指示（批准、修改意见或取消）。".into();
                        return (ToolResult::err(denied), false, true, plan_active, None);
                    }
                }
            }
        }

        if plan_overridden.is_none() {
            if !assessment.exists {
                denied = format!("未知工具：{}", call.name);
            } else if let Some(err) = assessment.prepared.as_ref().and_then(|p| p.err.clone()) {
                denied = err;
            } else if !self.deps.registry.allows(mode, &call.name) {
                denied = "当前模式不支持该工具".into();
            } else {
                let action = self.deps.engine.lock().unwrap().action(env.restricted, assessment.risk);
                // 写操作按用户所选审批档位判定（计划态不再有额外只读拦截：
                // 分级档按风险弹卡、自由档静默放行，与普通轮次完全一致）
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
                    // 命中既有授权（session/workspace 范围）则跳过询问；受限工作区授权一律失效
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
                            return (ToolResult::err_code("审批已取消", "tool_error"), true, false, plan_active, None);
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
        } // if plan_overridden.is_none() 包裹层闭合

        // 执行：协议分支接管时用其预置结果（enter_plan 的薄壳结果 / exit_plan 的批准或反馈桥接），
        // 其余按「拒绝即错误结果、否则进注册表执行」的统一出口。
        let was_denied = !denied.is_empty();
        let result = if let Some(overridden) = plan_overridden {
            overridden
        } else if was_denied {
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
                return (result, true, false, plan_active, plan_feedback);
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

        // 任务清单部件（前端渲染待办卡片）；status 别名先归一化，事件落库即规范值
        if call.name == "todo_write" && !result.is_error {
            self.deps.records.append_or_warn(&EventRecord {
                thread_id: thread_id.to_string(),
                run_id: Some(run_id.to_string()),
                message_id: message_id.to_string(),
                kind: "todo".into(),
                payload: json!({
                    "partId": new_id(id_prefix::PART),
                    "items": call
                        .args
                        .get("items")
                        .map(normalize_items)
                        .unwrap_or(Value::Array(vec![])),
                }),
            });
        }

        (result, false, false, plan_active, plan_feedback)
    }

    // ---------- 结束路径 ----------

    /// 已批准计划的全文（compact 注入用）：取会话最新计划行，
    /// 已批准且有文件时读盘返回；任何一步不满足返回 None（无计划/未批准/文件丢失都不注入）。
    fn read_approved_plan_text(db: &Arc<Db>, thread_id: &str) -> Option<String> {
        let (file_path, approved) = {
            let conn = db.lock_conn();
            let plan = craft_store::plans::latest_plan(&conn, thread_id).ok().flatten()?;
            (plan.file_path, plan.approved)
        };
        if !approved || file_path.is_empty() {
            return None;
        }
        std::fs::read_to_string(&file_path).ok().filter(|t| !t.trim().is_empty())
    }

    /// 计划正文落盘 + 回填（exit_plan 的批准/拒绝路径调用）：批准时置 approved=1。
    /// 文件是 compact 续读的数据源（计划原文按模型入参原样保存）；不再广播 plan.created、
    /// 不再驱动前端任何 UI（审批卡状态由 approval.resolved 驱动）。
    /// plan_id 为 None（历史数据或无计划行）时跳过落库回填。
    fn persist_plan_text(&self, _thread_id: &str, _run_id: &str, plan_id: Option<&str>, text: &str, approved: bool) {
        let dir = match craft_store::plans::plans_dir(&self.deps.db) {
            Ok(dir) => dir,
            Err(_) => return,
        };
        // 文件名：plan_<剥前缀 plan_id>.md；无 plan_id（历史数据）无法回填，跳过落盘
        let Some(plan_id) = plan_id else { return };
        let stem = plan_id.trim_start_matches(id_prefix::PLAN);
        let file = dir.join(format!("plan_{stem}.md"));
        if craft_core::platform::write_file_atomic_str(&file, text, 0o600).is_err() {
            return;
        }
        let conn = self.deps.db.lock_conn();
        let _ = craft_store::plans::set_plan_file(&conn, plan_id, file.to_string_lossy().as_ref());
        if approved {
            let _ = craft_store::plans::approve_plan(&conn, plan_id);
        }
    }

    /// 正常收尾：run 置 done、会话切 idle、广播 run.completed、发桌面通知。
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
        self.deps.log.write_thread(thread_id, LEVEL_INFO, "run_end", json!({ "runId": run_id, "status": "done", "durationMs": duration_ms }));
        self.notify(thread_id, "CraftWork", "本轮回复已完成。");
    }

    /// 取消收尾：run 置 canceled、会话切 idle、广播 run.canceled（不发桌面通知）。
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
        self.deps.log.write_thread(thread_id, LEVEL_INFO, "run_end", json!({ "runId": run_id, "status": "canceled", "durationMs": duration_ms }));
        // 取消不发通知
    }

    /// 失败收尾：错误部件落库 → run 置 error → 会话切 error → 广播 run.failed → 桌面通知。
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
        self.deps.log.write_thread(
            thread_id,
            LEVEL_ERROR,
            "run_end",
            json!({ "runId": run_id, "status": "error", "code": code, "message": tail_snapshot(message, 200), "durationMs": duration_ms }),
        );
        self.notify(thread_id, "CraftWork 运行失败", &message.chars().take(120).collect::<String>());
    }

    fn notify(&self, thread_id: &str, title: &str, message: &str) {
        if let Some(f) = self.notifier.lock().unwrap().as_ref() {
            f(thread_id, title, message);
        }
    }

    // ---------- 用量 ----------

    /// 用量落库（usage 表 + 会话 token 累计）并广播 usage.updated。口径统一走
    /// craft_store::usage_stats：上下文占用 = 归一化完整提示词（含缓存）+ 输出；
    /// 速率分母 = 生成窗口（剔除首包等待），隐藏思考与无效窗口按 unrated 处理。
    fn record_usage(&self, thread_id: &str, run_id: &str, info: &craft_model::types::ModelInfo, report: &UsageReport<'_>) {
        let UsageReport { usage, duration_ms, ttft_ms, output_chars, reasoning_streamed, sidecar, partial } = report;
        let kind = self.deps.models.provider_kind(&info.provider);
        let cost_micros = estimate_cost_micros(usage, info, &kind);
        // 生成窗口：取消/失败轮（duration 不完整）直接 unrated；正常轮由 usage_stats 判定
        let gen_window_ms = if *partial {
            usage_stats::GEN_WINDOW_UNRATED
        } else {
            usage_stats::generation_window_ms(*duration_ms, *ttft_ms, usage.reasoning_tokens, *reasoning_streamed)
                .unwrap_or(usage_stats::GEN_WINDOW_UNRATED)
        };
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
                duration_ms: *duration_ms,
                prompt_tokens: usage.prompt_tokens,
                ttft_ms: *ttft_ms,
                gen_window_ms,
            },
        );
        let _ = add_thread_tokens(&self.deps.db.lock_conn(), thread_id, usage.input_tokens + usage.output_tokens);
        let mut payload = json!({
            "inputTokens": usage.input_tokens,
            "outputTokens": usage.output_tokens,
            "costMicros": cost_micros,
            "promptTokens": usage.prompt_tokens,
            "outputChars": output_chars,
        });
        // sidecar 调用（自动命名）不携带上下文水位字段，避免小请求覆盖「当前上下文占用」；
        // 取消/失败轮带水位（input/缓存真实发生）但不带速率（duration 不完整，unrated）
        if !sidecar {
            payload["contextTokens"] = json!(usage_stats::context_tokens(usage.prompt_tokens, usage.input_tokens, usage.output_tokens));
            payload["cacheTokens"] = json!(usage.cache_tokens);
            payload["durationMs"] = json!(duration_ms);
            payload["ttftMs"] = json!(ttft_ms);
            if !partial
                && let Some(tps) = usage_stats::tokens_per_second(usage.output_tokens, gen_window_ms)
            {
                payload["tokensPerSecond"] = json!(tps);
            }
        }
        self.deps.records.emit(event_type::USAGE_UPDATED, Some(thread_id), Some(run_id), payload);
    }
}

/// `grants::command_prefix` 的本地别名包装（shell_exec 的授权粒度推导）。
fn command_prefix_of(command: &str) -> String {
    crate::grants::command_prefix(command)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 流结束的异常特征（返回异常描述；None=正常）。
/// 两条「回复半截」的提供商侧通路：finish_reason 为输出上限类（length / max_tokens），
/// 或未见显式结束帧（EOF 兜底）——落库前提是流在客户端视角干净结束，这两种形态
/// 都会原样落库半截内容。EOF 兜底在不规范的网关上是常态，但正因如此值得留原始尾部。
fn stream_end_anomaly(end: &StreamEnd) -> Option<String> {
    match end.finish_reason.as_deref() {
        Some("length") | Some("max_tokens") => Some(format!("finish_reason={}", end.finish_reason.clone().unwrap_or_default())),
        _ if end.done_via == DoneVia::Eof => Some("eof_without_done_frame".into()),
        _ => None,
    }
}

/// 结束方式的可读名（日志用）。
fn done_via_name(via: &DoneVia) -> &'static str {
    match via {
        DoneVia::DoneLine => "done_line",
        DoneVia::Eof => "eof",
    }
}

/// 新 run 启动时是否回填「错误已解决」：上一 run 以失败收尾（error=运行期失败、
/// interrupted=应用退出遗留）即消息流挂着未解决的错误卡片，会话继续前进即被覆盖。
/// done/canceled/running 及无历史 run（None）都不回填——canceled 不写错误部件。
fn should_resolve_pending_error(last_status: Option<&str>) -> bool {
    matches!(last_status, Some("error") | Some("interrupted"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end_with(finish_reason: Option<&str>, done_via: DoneVia) -> StreamEnd {
        StreamEnd {
            finish_reason: finish_reason.map(String::from),
            done_via,
            ..Default::default()
        }
    }

    #[test]
    fn stream_end_anomaly_flags_output_limit_and_missing_done_frame() {
        // 输出上限截断（OpenAI length / Anthropic max_tokens）
        assert_eq!(stream_end_anomaly(&end_with(Some("length"), DoneVia::DoneLine)).as_deref(), Some("finish_reason=length"));
        assert_eq!(stream_end_anomaly(&end_with(Some("max_tokens"), DoneVia::DoneLine)).as_deref(), Some("finish_reason=max_tokens"));
        // 未见显式结束帧
        assert_eq!(stream_end_anomaly(&end_with(None, DoneVia::Eof)).as_deref(), Some("eof_without_done_frame"));
        // 正常结束
        assert_eq!(stream_end_anomaly(&end_with(Some("stop"), DoneVia::DoneLine)), None);
        assert_eq!(stream_end_anomaly(&end_with(Some("end_turn"), DoneVia::DoneLine)), None);
        assert_eq!(stream_end_anomaly(&end_with(None, DoneVia::DoneLine)), None);
    }

    #[test]
    fn tail_snapshot_limits_logged_message_length() {
        let long = "x".repeat(500);
        assert_eq!(tail_snapshot(&long, 200).chars().count(), 200);
    }

    fn cost_model(cost: craft_model::types::Cost) -> craft_model::types::ModelInfo {
        serde_json::from_value(json!({
            "id": "m", "name": "m", "provider": "p", "contextK": 128, "maxTokens": 8192,
            "caps": { "stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false },
            "cost": cost, "status": "stable", "canonicalId": "m", "releaseDate": "",
            "source": "catalog", "envVars": [], "keyRequired": false
        }))
        .expect("测试模型元数据反序列化失败")
    }

    #[test]
    fn estimate_cost_micros_openai_deducts_cached_from_input() {
        // OpenAI：input 已含缓存命中 → 非缓存输入 = input - cache
        let info = cost_model(craft_model::types::Cost { input_per_m_tok: 1.0, output_per_m_tok: 2.0, cache_read_per_m_tok: 0.5, cache_write_per_m_tok: 1.25 });
        let usage = Usage { input_tokens: 1000, output_tokens: 500, cache_tokens: 800, cache_write_tokens: 0, reasoning_tokens: 0, prompt_tokens: 1000 };
        // 非缓存 200×1.0 + 缓存读 800×0.5 + 输出 500×2.0 = 200 + 400 + 1000 = 1600 微美元
        assert_eq!(estimate_cost_micros(&usage, &info, "openai"), 1600);
    }

    #[test]
    fn estimate_cost_micros_anthropic_treats_input_as_uncached() {
        // Anthropic：input 不含缓存 → 非缓存输入 = input 全额；缓存读/写分档计价
        let info = cost_model(craft_model::types::Cost { input_per_m_tok: 1.0, output_per_m_tok: 2.0, cache_read_per_m_tok: 0.5, cache_write_per_m_tok: 1.25 });
        let usage = Usage { input_tokens: 200, output_tokens: 100, cache_tokens: 15, cache_write_tokens: 10, reasoning_tokens: 0, prompt_tokens: 215 };
        // 非缓存 200×1.0 + 读 5×0.5 + 写 10×1.25 + 输出 100×2.0 = 200 + 2.5 + 12.5 + 200 = 415
        assert_eq!(estimate_cost_micros(&usage, &info, "anthropic"), 415);
    }

    #[test]
    fn estimate_cost_micros_zero_cache_prices_fall_back_to_input_price() {
        // 目录未给缓存价（0.0）时按普通输入价计，不出 0 账
        let info = cost_model(craft_model::types::Cost { input_per_m_tok: 1.0, output_per_m_tok: 2.0, cache_read_per_m_tok: 0.0, cache_write_per_m_tok: 0.0 });
        let usage = Usage { input_tokens: 1000, output_tokens: 0, cache_tokens: 800, cache_write_tokens: 0, reasoning_tokens: 0, prompt_tokens: 1000 };
        // 非缓存 200×1.0 + 读 800×1.0（回退）= 1000
        assert_eq!(estimate_cost_micros(&usage, &info, "openai"), 1000);
    }

    #[test]
    fn estimate_cost_micros_without_price_is_zero() {
        let info = cost_model(craft_model::types::Cost { input_per_m_tok: 0.0, output_per_m_tok: 0.0, cache_read_per_m_tok: 0.0, cache_write_per_m_tok: 0.0 });
        let mut info = info;
        info.cost = None;
        let usage = Usage::empty();
        assert_eq!(estimate_cost_micros(&usage, &info, "openai"), 0);
    }

    #[test]
    fn pending_error_resolved_only_for_failed_or_interrupted_last_run() {
        // 失败收尾（运行期 error / 退出遗留 interrupted）→ 回填
        assert!(should_resolve_pending_error(Some("error")));
        assert!(should_resolve_pending_error(Some("interrupted")));
        // 正常完成、取消、在途、无历史 run → 不回填（canceled 收尾不写错误部件）
        assert!(!should_resolve_pending_error(Some("done")));
        assert!(!should_resolve_pending_error(Some("canceled")));
        assert!(!should_resolve_pending_error(Some("running")));
        assert!(!should_resolve_pending_error(None));
    }
}
