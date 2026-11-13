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
use craft_policy::engine::{reason_for, Engine, Level};
use craft_store::db::Db;
use craft_store::events::EventRecord;
use craft_store::recorder::Recorder;
use craft_store::records::{add_usage, get_setting};
use craft_store::runs::{create_run, finish_run, last_run_status};
use craft_store::threads::{add_thread_tokens, get_thread};
use craft_store::usage_stats;
use craft_store::workspaces::get_workspace;
use craft_tools::registry::{Registry, ToolResult};
use craft_tools::todo::normalize_items;

use crate::approvals::{ApprovalFact, ApprovalRequest, Approvals};
use craft_tools::agent::{AgentClose, AgentList, AgentSpawn, AgentWait};
use craft_tools::plan::{EnterPlan, ExitPlan};
use craft_tools::wait::Wait;

/// 运行种类（决策 D15）：主会话运行与子代理运行共用同一条 `execute_run` 循环，
/// 差异全部收在这里——提示词、工具面、审批落点、报告投递、计划协议与 sidecar 屏蔽。
/// 新增分支时必须问一句「子代理路径也要这样吗」，默认答案是否。
#[derive(Debug, Clone)]
pub enum RunKind {
    /// 主会话运行（用户发起的对话）
    Main,
    /// 子代理运行：spec 携带父会话归属、工具面与回报目标
    Sub(Box<SubAgentSpec>),
}

impl RunKind {
    /// 是否主会话运行（子代理不参与计划协议、自动命名、输入预测与副本回报）。
    pub fn is_main(&self) -> bool {
        matches!(self, RunKind::Main)
    }

    /// 子代理运行时的 spec（主会话为 None）。
    pub fn sub(&self) -> Option<&SubAgentSpec> {
        match self {
            RunKind::Sub(spec) => Some(spec.as_ref()),
            RunKind::Main => None,
        }
    }
}

/// run 级计划状态在 run_tool 间传递的最终值：退出计划态时为 false。
/// 批准路径由 exit_plan 特判分支翻转；调用方据此更新 Run 循环的 plan_active。
pub type PlanExit = bool;
use crate::attachments::{load_attachments, save_tool_media};
use crate::context::run_compression_with;
use crate::envcontext::collect_env_context;
use crate::history::build_history;
use crate::prompt::build_prompt;
use crate::session_log::{tail_snapshot, SessionLog, LEVEL_DEBUG, LEVEL_ERROR, LEVEL_INFO};
use crate::subagent::{AgentStatus, AgentType, SubAgentConfig, SubAgentSnapshot, SubAgentSpec};
use crate::title::auto_title;

/// 子代理协议工具集合（主会话可见、子代理不可见；总开关关闭时从主会话工具面隐藏）。
const SUBAGENT_PROTOCOL_TOOLS: &[&str] = craft_tools::agent::SUBAGENT_TOOLS;

/// 单次模型请求（含流式接收全程）的整体超时。
pub const MODEL_REQUEST_TIMEOUT_MS: u64 = 3 * 60 * 1000;

/// 单会话运行中引导（steer）消息的排队上限，超过即拒绝入队。
pub const MAX_STEER_QUEUE: usize = 10;

/// 引导未被应答时的纠偏文案（用户角色消息，模型可见）。
/// 注入点是下一轮迭代开头，与引导本身同位置；每个 steer 批次最多注入一次
/// （见 D19）。检测条件只认「正文非空」，不评价内容质量。
pub const STEER_UNANSWERED_NUDGE: &str = "（系统提示）你刚才收到了用户在前一轮追加的指令，但没有给出任何回应就直接继续干活。\
     请先用一句话回答这条指令改变了接下来的什么，再继续执行——用户需要确认你读懂了。\
     若这条指令不影响原计划，说明原计划不变即可。";

/// 单次运行的模型调用轮数上限默认值（每轮一次模型请求）。
/// 0 = 不限制：Runner 的循环判定是 `limit <= 0 || iteration < limit`，
/// 因此默认不再有隐形轮数上限；用户可在设置里显式设一个正值（0~1000）。
pub const DEFAULT_MAX_ITERATIONS: i64 = 0;

/// 计划自动进入阈值（D32）：免计划可改动的**不同文件**数上限——对第 3 个不同文件的
/// 写入在执行前拦下并自动进入计划模式。计数按去重后的文件目标（同一文件改多次只算
/// 一个，见 `Registry::is_file_write` / `file_key`）；只有成功落盘的写入才计入
/// （被拒 / 被拦 / 失败没有真的改动文件）。豁免与口径见 D32 决策条目。
pub const PLAN_AUTO_ENTER_FILE_COUNT: usize = 2;

/// 计划软闸门（D32）：非计划态 run 内已改动该数量的**不同文件**且从未调用过
/// enter_plan 时，下一迭代提醒「再改一个文件会被强制进入计划模式」——预告硬规则。
/// 阈值与 [`PLAN_AUTO_ENTER_FILE_COUNT`] 相等：改完第 2 个文件就预告，第 3 个触发拦截。
pub const PLAN_NUDGE_THRESHOLD: usize = 2;

/// 子代理 nudge 阈值（issue 1 P4）：主会话本 run 累计该数量的**大范围检索类**调用
/// 而从未派发子代理时，注入一次派发考虑提醒（哨兵保证只提醒一次）。口径是「本 run
/// 累计」而非严格连续——从未派发的前提下，累计 6 次检索已经是「该考虑并行调研」的
/// 充分信号；哨兵保证不重复打断。检索类清单见 [`SUBAGENT_NUDGE_TOOLS`]。
pub const SUBAGENT_NUDGE_THRESHOLD: usize = 6;

/// 视为「大范围检索」的工具清单（nudge 的计数口径）：宽面检索 / 通读 / 历史排查 /
/// 在线查阅——它们正是 explore 子代理的专长面。单点读取（fs_read）与写入不计。
pub const SUBAGENT_NUDGE_TOOLS: &[&str] = &["fs_grep", "fs_glob", "fs_list", "git_log", "doc_read", "web_fetch"];

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
    /// 已注册的 MCP 工具数（提示词 MCP 说明段的注入依据；0 = 无 MCP 工具）。
    ///
    /// 做成闭包而不是常量：连接 / 断开服务器、保存设置都发生在 run 之外，常量只会在
    /// 启动时求值一次，之后永不更新（MCP 说明段因此永远不注入）。闭包每次构造提示词时
    /// 求值，与 `subagent_config`、`proxy_config` 同构。
    pub mcp_tool_count: Box<dyn Fn() -> usize + Send + Sync>,
    /// 出站代理解析器（D14）：每次建立模型会话时求值——client 每 run
    /// 新建，设置保存即时生效。craft-agent 不读 SQLite settings 表，由壳层注入读取闭包。
    pub proxy_config: Box<dyn Fn() -> craft_core::network::ProxyConfig + Send + Sync>,
    /// 子代理配置解析器（决策 D15）：每次派发时求值，改设置立即生效。
    /// 未注册时按默认（开启、并发 4）；关闭后 agent_* 工具从工具面消失并直接报错。
    pub subagent_config: Box<dyn Fn() -> crate::subagent::SubAgentConfig + Send + Sync>,
    /// Shell 环境重捕器（决策 D18）：run 启动时缓存已过期才调用，由壳层读设置并执行
    /// `apply_shell_env`（失败写 `app.log`，不阻断 run）。craft-agent 不读 settings 表，
    /// 与 `proxy_config` 同一分工。
    ///
    /// 用 `Arc` 而不是 `Box`：要移进 `spawn_blocking`——捕获可能跑满 5 秒超时。
    pub refresh_shell_env: Arc<dyn Fn() + Send + Sync>,
    /// 后台命令任务注册表（与 Registry 的 ToolEnv 共用同一个 Arc）。
    /// Runner 用它做三件事：取走终态通知并注入会话、空闲时唤醒、应用退出整组清理。
    /// None 表示未接线：后台命令仍能启动，但没有自动通知（测试装配可省）。
    pub shell_tasks: Option<Arc<craft_tools::shell_tasks::ShellTasks>>,
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
    /// 输入预测缓存（`crates/craft-agent/src/prediction.rs` 模块头）：会话 → 下一句建议文本。
    /// 瞬态 UI 辅助只存内存（不入事件表），新完成覆盖旧值。
    predictions: Arc<Mutex<HashMap<String, String>>>,
    /// 子代理运行注册表（决策 D15）：派发登记、状态查询、取消级联与父会话报告队列
    subagents: Arc<crate::subagent::SubAgentRegistry>,
    /// 子代理配置解析器在 deps.subagent_config；这里只缓存注册表与派生状态。
    /// 已取消为 closed 的子代理（模型主动 agent_close）：在 finally 收尾时据此区分
    /// 「被模型停止」与「被父级连带中断」两种终态。
    closed_agents: Mutex<HashSet<String>>,
    /// 在途 wait 的会话集合（D21）：run_wait_tool 开始时登记、结束时移除；
    /// chatCancelWait 据此判断「当前没有等待中的任务」。
    wait_active: Mutex<HashSet<String>>,
    /// wait 跳过请求（会话集合）：通道置位，run_wait_tool 轮询消费。
    /// 用标志 + 100ms 轮询而非 channel：与注册表轮询（agent_wait）同一范式，
    /// 规避 Notify 的 permit 竞态与生命周期管理。
    wait_skips: Mutex<HashSet<String>>,
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
            subagents: Arc::new(crate::subagent::SubAgentRegistry::new()),
            closed_agents: Mutex::new(HashSet::new()),
            wait_active: Mutex::new(HashSet::new()),
            wait_skips: Mutex::new(HashSet::new()),
        }
    }

    /// 子代理注册表句柄（壳层在删除会话时清理登记与待投递报告）。
    pub fn subagents(&self) -> &Arc<crate::subagent::SubAgentRegistry> {
        &self.subagents
    }

    /// 用户跳过在途 wait（chatCancelWait 通道，D21）。返回是否存在在途 wait；
    /// 不在途时不置位，通道层据此回「当前没有等待中的任务」。
    pub fn cancel_wait(&self, thread_id: &str) -> bool {
        let in_flight = self.wait_active.lock().unwrap().contains(thread_id);
        if in_flight {
            self.wait_skips.lock().unwrap().insert(thread_id.to_string());
        }
        in_flight
    }

    /// 是否有在途 wait（服务与测试的断言锚点）。
    pub fn wait_in_flight(&self, thread_id: &str) -> bool {
        self.wait_active.lock().unwrap().contains(thread_id)
    }

    /// 装配后台任务的唤醒回调：命令一结束就通知会话去消费通知（`crates/craft-agent/src/shell_tasks.rs` 模块头）。
    ///
    /// 为什么需要回调而不是在 run 收尾处轮询：后台命令的结束时刻与会话 run 的收尾时刻
    /// 无关——命令常常在会话早已空闲之后才结束。只在收尾处检查队列，等于要求用户下次
    /// 发言才看得到结果，那正是本功能要消灭的体验。
    ///
    /// 必须在 `Arc::new(Runner::new(..))` 之后调用（回调要持有 Runner 自己）。
    pub fn wire_shell_task_wake(self: &Arc<Self>) {
        let Some(shell_tasks) = &self.deps.shell_tasks else { return };
        let this = Arc::clone(self);
        shell_tasks.set_wake_hook(Arc::new(move |thread_id: &str| {
            this.wake_shell_tasks_if_idle(thread_id);
        }));
    }

    /// 本次运行可见的工具：子代理按 spec 白名单裁剪；主会话在子代理总开关关闭时
    /// 隐藏 agent_*（关闭后不可再派发）。模式过滤（Work）在调用方已完成。
    fn tool_visible(&self, name: &str, kind: &RunKind, subagent_enabled: bool) -> bool {
        match kind.sub() {
            Some(spec) => spec.allowed_tools.iter().any(|allowed| allowed == name),
            None => {
                if SUBAGENT_PROTOCOL_TOOLS.contains(&name) {
                    return subagent_enabled;
                }
                true
            }
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

    /// 输入预测 sidecar（`crates/craft-agent/src/prediction.rs` 模块头）：run 正常完成后后台预计算
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
        self.start_run(thread_id, trigger_seq, RunKind::Main, None)
    }

    /// 运行启动的统一实现：主会话与子代理共用（写 run 行 → 待决错误回填 → 登记取消表 →
    /// 按运行种类起执行任务）。`token_override` 让子代理的取消令牌由注册表持有，
    /// 父会话取消级联时才能直接触发它。
    fn start_run(
        self: &Arc<Self>,
        thread_id: &str,
        trigger_seq: Option<i64>,
        kind: RunKind,
        token_override: Option<CancellationToken>,
    ) -> Result<String, String> {
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
        let token = token_override.unwrap_or_default();
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
                let kind = kind.clone();
                let task = handle.spawn(async move {
                    this.execute(token, kind, &thread_id, &run_id, &message_id, &mode, &model_id, trigger_seq)
                        .await;
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
                            this.execute(token, kind, &thread_id, &run_id, &message_id, &mode, &model_id, trigger_seq)
                                .await;
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
    /// 级联：同时取消该会话派生的全部在跑子代理（决策 D15）——子代理不应在主会话
    /// 停止后继续消耗 token，用户按下停止就是对整棵执行树的意图。
    pub fn cancel(&self, thread_id: &str) -> bool {
        self.subagents.cancel_all_under(thread_id);
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
        kind: RunKind,
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
                .execute_run(&token, &kind, thread_id, &run_id, &message_id, mode, model_id, titled, trigger_seq)
                .await;
            if !ok {
                break;
            }
            titled = true;
            // 子代理不消费引导：它的输入只有派发时的 prompt，接力语义对子代理没有意义
            if kind.sub().is_some() {
                break;
            }
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
        // 子代理收尾：把子 run 的终态（含最终文本与统计）写回注册表、父会话卡片与报告队列。
        // 放在这里而不是各收尾函数里的理由——六个终态分支（done/canceled/error/上限/取消/
        // 工作区丢失）都要走这一段，集中一处才不会漏。
        if kind.sub().is_some() {
            self.finalize_subagent(thread_id);
        }
        // 后台命令任务的收尾唤醒：本轮过程中可能有任务刚完成，通知还躺在队列里。
        // 主会话本轮已结束、队列非空时立刻起唤醒 run，让模型读到通知并向用户汇报——
        // 否则要等到用户下次发言，等于「等了半天没人说话」（`crates/craft-agent/src/shell_tasks.rs` 模块头）。
        if kind.is_main() {
            self.wake_shell_tasks_if_idle(thread_id);
            // 子代理报告的收尾唤醒（与上一条同一窗口）：子代理可能在父 run 的
            // 最后一轮迭代期间完成——finish_subagent 里的收尾唤醒会被 cancels
            // 守卫挡掉（设计意图是「报告在下一轮注入点被消费」），但最后一轮
            // 之后没有下一轮了，报告就成了孤儿（CI 实测踩中）。run 收尾时取消
            // 令牌已回收，此处检查报告队列补起唤醒 run。
            self.wake_parent_if_idle(thread_id);
        }
    }

    /// 子 run 结束后收尾：从 runs 表读终态、从事件快照取最终回答文本，交给 `finish_subagent`。
    fn finalize_subagent(self: &Arc<Self>, agent_id: &str) {
        let status = match last_run_status(&self.deps.db.lock_conn(), agent_id).ok().flatten().as_deref() {
            Some("done") => AgentStatus::Completed,
            Some("canceled") | Some("interrupted") => AgentStatus::Interrupted,
            Some("error") => AgentStatus::Failed,
            // 无 run 行（启动前失败）按失败收尾，避免卡片永远停在 running
            _ => AgentStatus::Failed,
        };
        let result = self.subagent_final_text(agent_id);
        self.finish_subagent(agent_id, status, &result);
    }

    /// 子会话的最终回答文本：取快照里最后一条 assistant 消息的所有 text 部件；
    /// 失败时回退到 error 部件的提示信息（让父代理知道为什么没结论）。
    fn subagent_final_text(&self, agent_id: &str) -> String {
        let snapshot = self
            .deps
            .db
            .with(|conn| craft_store::events::snapshot(conn, agent_id, 0, 0))
            .ok();
        let Some(snapshot) = snapshot else {
            return String::new();
        };
        let mut text = String::new();
        let mut error = String::new();
        for message in &snapshot.messages {
            if message.role != "assistant" {
                continue;
            }
            for part in &message.parts {
                match part.kind.as_str() {
                    "text" => {
                        let t = part.payload.get("text").and_then(Value::as_str).unwrap_or_default();
                        if !t.trim().is_empty() {
                            text = t.to_string();
                        }
                    }
                    "error" => {
                        error = part.payload.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
                    }
                    _ => {}
                }
            }
        }
        if text.trim().is_empty() {
            error
        } else {
            text
        }
    }

    /// 单轮运行：返回 true 表示正常完成（可接力），false 表示已写过结束事件。
    #[allow(clippy::too_many_arguments)]
    async fn execute_run(
        self: Arc<Self>,
        token: &CancellationToken,
        kind: &RunKind,
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
        // 守卫纪律：match scrutinee 里的 lock_conn 临时守卫会存活到整条 match 结束，
        // Err 臂内 fail 再取 db 锁就是自死锁（实测挂死测试进程）。先把结果取到 let，
        // 让守卫随语句结束释放，再 match。
        let thread_result = get_thread(&self.deps.db.lock_conn(), thread_id);
        let thread = match thread_result {
            Ok(thread) => thread,
            Err(err) => {
                self.fail(thread_id, run_id, &message_id, "history_failed", &err.to_string(), true, started_at);
                return false;
            }
        };
        // 计划模式标记随触发消息落库，此处按 trigger 还原（regenerate 跟随上一轮、steer 接力取首条引导）。
        // 子代理不参与计划协议（kind 非主会话时恒为 false，也不建 plans 行）。
        let plan_mode = kind.is_main()
            && craft_store::events::plan_mode_for_trigger(&self.deps.db.lock_conn(), thread_id, trigger_seq).unwrap_or(false);
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
        // 计划门状态（D32）：本 run 已成功写入的不同文件键（计数口径见 is_file_write /
        // file_key）+ 是否调过 enter_plan + 会话/本 run 的获批计划豁免。
        // plan_approved_at_start 启动读一次（跨 run「批准并执行」续跑的豁免）；
        // plan_approved_this_run 在本 run 内观察到 plan_active true→false（exit_plan 批准）时置位。
        let mut planned_files: std::collections::HashSet<String> = std::collections::HashSet::new();
        let plan_approved_at_start = Self::read_approved_plan_text(&self.deps.db, thread_id).is_some();
        let mut plan_approved_this_run = false;
        let mut plan_entered = plan_mode;
        // 子代理 nudge 状态（issue 1 P4）：本 run 检索类调用计数 + 是否已派发 + 哨兵
        let mut retrieval_calls = 0usize;
        let mut subagent_spawned_this_run = false;
        let mut subagent_nudged = false;
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
            // 守卫纪律（同 get_thread 处注释）：scrutinee 里的临时守卫活到 match 结束，
            // Err 臂内 fail 再取 db 锁会自死锁。先取结果再 match。
            let ws_result = get_workspace(&self.deps.db.lock_conn(), &thread.workspace_id);
            match ws_result {
                Ok(ws) => {
                    // 提示词、git、附件统一用非 verbatim 路径：历史数据可能存有 canonicalize
                    // 产物 `\\?\D:\...`，在展示与下游消费前剥掉（存储层不迁移，读取时归一）
                    let clean = |s: &String| {
                        craft_policy::resolve::strip_verbatim(std::path::Path::new(s))
                            .to_string_lossy()
                            .to_string()
                    };
                    env = RunEnv {
                        workspace_path: clean(&ws.workspace.path),
                        roots: ws.folders.iter().map(|f| clean(&f.path)).collect(),
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
        let log_for_history = Arc::clone(&self.deps.log);
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
            let history = build_history(&db, &snap, &workspace_path, &attachment_dir, &caps, &log_for_history).messages;
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
        // Shell 环境（D18）：缓存过了 5 分钟就先重捕。否则长会话会在某个时刻静默失去
        // 环境——提示词退回 `$SHELL` 短名、命令回落到最小环境，而用户什么都没改。
        // 只在过期时触发，正常情况零开销；捕获在阻塞线程池里跑（最坏 5 秒超时），
        // 失败由壳层写 app.log 并清缓存，run 继续。
        if !craft_core::shellenv::is_fresh() {
            let refresh = Arc::clone(&self.deps.refresh_shell_env);
            let _ = tokio::task::spawn_blocking(move || refresh()).await;
        }
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
        // 子代理配置一次求值（工具面、清单与派发共用同一份快照）：改设置下一轮生效。
        let subagent_config = (*self.deps.subagent_config)();
        let subagent_enabled = subagent_config.enabled;
        // 自定义子代理清单（只对主会话注入；总开关关闭时不提示派发能力）
        let custom_agents_section = if kind.is_main() && subagent_enabled {
            subagent_config.listing_section()
        } else {
            None
        };
        // MCP 说明段的开关依据：注册表里有多少个 mcp__ 工具（RunnerDeps.mcp_tool_count）。
        // 每次构造提示词即时求值：连接/断开一个服务器，下一轮对话即生效。
        let mcp_tool_count = || if kind.is_main() { (self.deps.mcp_tool_count)() } else { 0 };
        // 子代理运行态（主会话每轮求值）：并发窗口注入 dynamic 块，模型不盲撞上限
        // （D15 扩展的并发上限在派发侧拒绝；提示词先告知窗口，见 D31 后续的派发指引）。
        let subagent_runtime = || {
            if kind.is_main() && subagent_enabled {
                Some(crate::prompt::SubagentRuntimeInfo {
                    running: self.subagents.running_count(thread_id),
                    max_concurrent: subagent_config.max_concurrent,
                })
            } else {
                None
            }
        };
        // 先求值存变量：构造与「上次构造值」记录用同一份，避免两次求值间子代理状态漂移。
        let mut subagent_runtime_now = subagent_runtime();
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
            // 子代理：换成子代理版提示词（角色档案 + 回报契约），且不注入指令文件正文
            // （explore 不取、general 取——由 spec.inject_instructions 决定）
            subagent: kind.sub().map(|spec| crate::prompt::SubagentPrompt {
                agent_name: &spec.agent_name,
                agent_type: spec.agent_type,
                role_prompt: &spec.role_prompt,
                description: &spec.description,
            }),
            inject_instructions: kind.sub().map(|spec| spec.inject_instructions).unwrap_or(true),
            // 自定义子代理清单只对主会话注入（子代理不能再派发）
            custom_agents: custom_agents_section.clone(),
            // MCP 工具说明只对主会话注入：子代理工具面不含 mcp__ 工具（决策见 spec 第 3 节）
            mcp_server_count: mcp_tool_count(),
            // 派发条款的注入门槛与 tool_visible 的 agent_* 过滤同源（总开关）：
            // 关闭时工具面与提示词一致地「看不见」子代理
            subagent_available: subagent_enabled,
            subagent_runtime: subagent_runtime_now,
            // 首轮不会触发 nudge（检索计数为 0）
            subagent_nudge: false,
        });
        // 上次构造提示词时的计划态、软闸门提醒标记、子代理运行态与 nudge（循环内重建的判断依据）
        let mut prompt_plan_mode = plan_active;
        let mut prompt_plan_nudge = false;
        let mut prompt_subagent_nudge = false;
        let mut prompt_subagent_runtime = subagent_runtime_now;

        // agent_spawn 的 agent_type enum 随自定义定义动态变化：注册表里注册的是默认 spec，
        // 这里按本次 run 的可用类型替换（新增/停用定义下一轮对话即生效）。
        let custom_type_names = crate::subagent::available_type_enum(&subagent_config.custom_profiles)
            .into_iter()
            .filter(|name| !craft_core::contract::RESERVED_AGENT_NAMES.contains(&name.as_str()))
            .collect::<Vec<String>>();
        let custom_type_descriptions: String = subagent_config
            .custom_profiles
            .iter()
            .filter(|p| p.enabled)
            .map(|p| {
                if p.description.trim().is_empty() {
                    format!("- {}", p.name)
                } else {
                    format!("- {}：{}", p.name, p.description)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let specs: Vec<craft_model::types::ToolSpec> = self
            .deps
            .registry
            .specs_for_mode(mode)
            .into_iter()
            .filter(|s| self.tool_visible(&s.name, kind, subagent_enabled))
            .map(|s| {
                // 主会话的 agent_spawn：换成带自定义类型的 spec（参数名与风险声明不变）
                if kind.is_main() && s.name == craft_tools::agent::AgentSpawn::NAME && !custom_type_names.is_empty() {
                    let dynamic = craft_tools::agent::AgentSpawn::spec_with_types(&custom_type_names, &custom_type_descriptions);
                    return craft_model::types::ToolSpec { name: dynamic.name, description: dynamic.description, schema: dynamic.schema };
                }
                craft_model::types::ToolSpec { name: s.name, description: s.description, schema: s.schema }
            })
            .collect();
        let parent_limit = self
            .max_iterations
            .lock()
            .unwrap()
            .as_ref()
            .map(|f| f())
            .unwrap_or(DEFAULT_MAX_ITERATIONS);
        // 子代理迭代上限独立封顶（它是「一件事」，不该跑满主会话的轮数预算）；
        // 档案给了正值时优先用它（用户在自定义定义或内置默认里显式配了上限）。
        let limit = match kind.sub() {
            Some(spec) => crate::subagent::iterations_for_profile(parent_limit, spec.max_iterations),
            None => parent_limit,
        };
        // 生效轮数上限在子运行启动时写入运行态快照（D15 的面板补注）：
        // 中间态进度事件与状态列表的「轮 N/M」都以它为准。spawn 时写入的是当时的预估值，
        // 这里以 run 的归一结果为准再对齐一次（派发到子 run 启动之间设置可能已变）。
        // 引导应答的跨迭代哨兵（D19）：
        // - `steer_unanswered`：本轮并入了引导却没有产出任何正文 → 下一轮开头注入一次纠偏；
        // - `answer_due`：本轮末尾并入了计划反馈 → 下一轮是应答轮。
        // 两者只判「正文是否非空」，用完即复位，保证同一批用户输入最多纠偏一次。
        let mut steer_unanswered = false;
        let mut answer_due = false;
        let mut wrapup_sent = false;
        if let Some(spec) = kind.sub() {
            self.subagents.update(&spec.thread_id, |snapshot| snapshot.max_iterations = limit);
        }

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

            // 引导应答纠偏（D19）：上一轮并入了用户引导却一句话
            // 没说就继续干活时，在改动历史之前补一条提醒。位置与引导本身同处（迭代开头），
            // 「assistant 工具调用 → tool 结果」的配对因此不被打断；哨兵用完即复位，
            // 保证同一批引导最多纠偏一次——之后再只调工具就让它继续，不与模型僵持。
            if steer_unanswered {
                steer_unanswered = false;
                let mut msg = Message::new("user", STEER_UNANSWERED_NUDGE);
                msg.id = Some(new_id(id_prefix::MESSAGE));
                history.push(msg);
                self.deps.log.write_thread(
                    thread_id,
                    LEVEL_INFO,
                    "steer_nudge_injected",
                    json!({ "runId": run_id, "iter": iteration }),
                );
            }
            // 本轮是否必须给出应答。两种来源的时序不同，故分两个变量：
            // - 引导在**本轮开头**并入，本轮正文就是应答 → 用 must_answer 当场记录；
            // - 计划反馈在**本轮末尾**并入（整批工具结果就位后），下一轮才是应答轮
            //   → 用 answer_due 跨迭代带到下一轮。
            let mut must_answer = answer_due;
            answer_due = false;
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
                // 本轮并入了用户引导：本轮就是应答轮，正文为空即视为未应答
                // （去重批次同样计入——接力 run 的引导可能已随历史重建进入，投递点仍是这一轮）。
                must_answer = true;
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

            // 子代理报告投递点（决策 D15）：主会话运行中每轮迭代开始处，把已完成的子代理
            // 回报并入历史。与 steer 同位置的理由一致——只在这里改历史，才能保证「assistant
            // 工具调用 → tool 结果」的配对不被插入打断。子代理自己不会有下级子代理，跳过。
            if kind.is_main() {
                let reports = self.subagents.take_reports(thread_id);
                for report in reports {
                    let mut msg = Message::new("user", crate::subagent::report_message_text(&report));
                    msg.id = Some(new_id(id_prefix::MESSAGE));
                    history.push(msg);
                    self.deps.log.write_thread(
                        thread_id,
                        LEVEL_INFO,
                        "subagent_report_injected",
                        json!({ "runId": run_id, "iter": iteration, "agent": report.agent_name }),
                    );
                }
            }

            // 软上限收尾提醒（D15 的面板补注）：子代理运行到「剩余轮数
            // ≤ 收尾预留」时触发一次——向子会话写一条可见 notice（用户正开着详情 tab 就能
            // 看到「开始收尾」），并向子 run 历史注入一条用户角色的收尾指令，要求它停止扩张
            // 范围、用剩余轮数整理结论。注入点与引导、报告同位置（每轮迭代开始处），保证
            // 「助手工具调用 → 工具结果」的配对不被打断；哨兵保证整个 run 只提醒一次。
            if let Some(spec) = kind.sub()
                && limit > 0
                && !wrapup_sent
                && iteration as i64 >= limit - crate::subagent::SUBAGENT_WRAPUP_RESERVED_ROUNDS
            {
                wrapup_sent = true;
                let remaining = limit - iteration as i64;
                self.deps.records.append_or_warn(&EventRecord {
                    thread_id: thread_id.to_string(),
                    run_id: Some(run_id.to_string()),
                    message_id: message_id.clone(),
                    kind: "notice".into(),
                    payload: json!({
                        "partId": new_id(id_prefix::PART),
                        "text": crate::subagent::wrapup_notice_text(remaining),
                    }),
                });
                history.push(Message::new("user", crate::subagent::wrapup_instruction_text(remaining)));
                self.deps.log.write_thread(
                    thread_id,
                    LEVEL_INFO,
                    "subagent_wrapup_notice",
                    json!({ "runId": run_id, "iter": iteration, "agent": spec.agent_name, "remaining": remaining }),
                );
            }

            // 后台命令任务通知投递点（`crates/craft-agent/src/shell_tasks.rs` 模块头）：与子代理报告同位置、
            // 同纪律——每轮迭代开始处把已完成的终态通知并入历史。放在这里而不是别处，是为了
            // 保证「assistant 工具调用 → tool 结果」的配对不被中途插入打断。
            if kind.is_main()
                && let Some(shell_tasks) = &self.deps.shell_tasks
            {
                for note in shell_tasks.take_notifications(thread_id) {
                    let mut msg = Message::new("user", note.render());
                    msg.id = Some(new_id(id_prefix::MESSAGE));
                    history.push(msg);
                    self.deps.log.write_thread(
                        thread_id,
                        LEVEL_INFO,
                        "shell_task_notification_injected",
                        json!({
                            "runId": run_id,
                            "iter": iteration,
                            "taskId": note.task_id,
                            "state": note.state.as_str(),
                            "exitCode": note.exit_code,
                        }),
                    );
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
                let message_id_owned = message_id.clone();
                let model_id_owned = session.info.id.clone();
                let context_k = session.info.context_k;
                let client = session.client.clone_box();
                let records = self.deps.records.clone();
                let compressed = tokio::task::spawn_blocking(move || {
                    // 已批准计划的全文：有则读盘（压缩后拼保留窗头部，保计划文件连续性）
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
                            message_id: &message_id_owned,
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

            // 自动命名（首轮并行，防并发重复）。子代理不做自动命名：它的标题在创建
            // 子会话时就由派发摘要给出，且有专门的命名不值得一次额外模型调用。
            if kind.is_main()
                && iteration == 0
                && !titled
                && thread.title_auto
                && !self.title_pending.lock().unwrap().contains(thread_id)
            {
                self.title_pending.lock().unwrap().insert(thread_id.to_string());
                let first_user = history.iter().find(|m| m.role == "user");
                let first_text = first_user.map(|m| m.content.clone()).unwrap_or_default();
                let first_attachments = first_user.map(|m| m.attachments.clone()).unwrap_or_default();
                let db = Arc::clone(&self.deps.db);
                let records = self.deps.records.clone();
                let this = Arc::clone(&self);
                let thread_id_owned = thread_id.to_string();
                let model_id_owned = session.info.id.clone();
                let client = session.client.clone_box();
                tokio::task::spawn_blocking(move || {
                    let wrote = auto_title(
                        &db,
                        crate::title::AutoTitleInput {
                            client: client.as_ref(),
                            model_id: &model_id_owned,
                            thread_id: &thread_id_owned,
                            first_text: &first_text,
                            first_attachments: &first_attachments,
                        },
                    );
                    // 命名只改 DB 的 title 列，不发事件的话界面要等 run 结束（下一次
                    // set_thread_status 广播）才刷新——长 run 期间侧栏一直停留在首条
                    // 消息截断的兜底标题，看起来就像「没有自动命名」。写成功即广播，
                    // 渲染层的 thread.updated 分支会重拉会话列表。
                    if wrote {
                        records.emit(
                            event_type::THREAD_UPDATED,
                            Some(&thread_id_owned),
                            None,
                            json!({ "threadId": thread_id_owned.as_str() }),
                        );
                    }
                    // 防重标记必须等命名真正结束才释放：此前在 spawn 后立刻移除，
                    // 任务还没开始跑标记就没了，同一会话仍可能被并发起两次名。
                    this.title_pending.lock().unwrap().remove(&thread_id_owned);
                });
            }

            let call_start = now_millis();
            // 计划软闸门：非计划态 + 写类调用达到阈值 + 本 run 从未调过 enter_plan + 无已批准计划
            // （豁免跨 run「批准并执行」的实施续跑）→ 提醒拉回协议。只提醒不拦截。
            // 子代理不参与计划协议，闸门对它们恒不触发。
            // 计划门规则开关（D32）：主会话 + 非工作模式 + 分级档 + 会话与本 run 都没有
            // 获批计划。五条豁免任一不成立就维持普通行为（自由档 = 放行一切，D8）。
            let plan_rule_on = kind.is_main()
                && mode != "work"
                && self.deps.engine.lock().unwrap().level() != Level::Free
                && !plan_approved_at_start
                && !plan_approved_this_run;
            // 计划软闸门（D32）：已改动 2 个不同文件且从未调过 enter_plan → 提醒
            // 「再改一个文件会被强制进入计划模式」。按文件数而非调用次数计（同一文件
            // 反复修改不算），恰好达到阈值时提醒一次。
            let plan_nudge = plan_rule_on
                && !plan_active
                && !plan_entered
                && planned_files.len() == PLAN_NUDGE_THRESHOLD;
            // 子代理 nudge（issue 1 P4）：主会话 + 总开关开启 + 非工作模式下，本 run
            // 检索类调用达标却从未派发 → 注入一次派发考虑提醒。哨兵置位保证整个 run
            // 只提醒一次（派发发生或提醒已给过都不再触发）。
            let subagent_nudge = subagent_enabled
                && kind.is_main()
                && mode != "work"
                && !subagent_spawned_this_run
                && !subagent_nudged
                && retrieval_calls >= SUBAGENT_NUDGE_THRESHOLD;
            if subagent_nudge {
                subagent_nudged = true;
            }
            // 子代理运行态每迭代求值：派发与子代理终态都会改变并发窗口（D31 的派发指引
            // 配套——模型据此知道还能派几个），变化时随下一轮请求重建 dynamic 块
            subagent_runtime_now = subagent_runtime();
            // 计划态中途翻转后重建提示词（dynamic 块随 plan_active / plan_nudge /
            // subagent_runtime / subagent_nudge 变化；用最后构造时的标记判断是否需要
            // 重建，避免 PromptResult 比较开销）
            let need_rebuild = prompt_plan_mode != plan_active
                || prompt_plan_nudge != plan_nudge
                || prompt_subagent_runtime != subagent_runtime_now
                || prompt_subagent_nudge != subagent_nudge;
            if need_rebuild {
                prompt_plan_mode = plan_active;
                prompt_plan_nudge = plan_nudge;
                prompt_subagent_nudge = subagent_nudge;
                prompt_subagent_runtime = subagent_runtime_now;
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
                    subagent: kind.sub().map(|spec| crate::prompt::SubagentPrompt {
                        agent_name: &spec.agent_name,
                        agent_type: spec.agent_type,
                        role_prompt: &spec.role_prompt,
                        description: &spec.description,
                    }),
                    inject_instructions: kind.sub().map(|spec| spec.inject_instructions).unwrap_or(true),
                    custom_agents: custom_agents_section.clone(),
                    mcp_server_count: mcp_tool_count(),
                    subagent_available: subagent_enabled,
                    subagent_runtime: subagent_runtime_now,
                    subagent_nudge,
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
            // 工具调用配对整理（`crates/craft-model/src/integrity.rs` 模块头）：交给提供商的消息序列
            // 在唯一出口保证不变量——assistant 工具调用的结果必须紧邻且齐全。历史重建
            // 已整理过一遍，此处兜底运行期历史注入与任何顺序疏忽（失败转移、压缩后的
            // 历史也走这里）。合规序列零改动、不写日志（幂等，正常请求无噪音）；
            // 整理结果回写内存历史，后续轮次与注入基于干净序列继续。
            let integrity = craft_model::integrity::repair_tool_call_integrity(std::mem::take(&mut history));
            if !integrity.fixes.is_empty() {
                self.deps.log.write_thread(
                    thread_id,
                    LEVEL_INFO,
                    "history_integrity_repaired",
                    json!({
                        "runId": run_id,
                        "iter": iteration,
                        "fixed": integrity.fixes.len(),
                        "details": integrity
                            .fixes
                            .iter()
                            .map(|f| json!({ "index": f.index, "kind": f.kind.as_str() }))
                            .collect::<Vec<_>>(),
                    }),
                );
            }
            history = integrity.messages;
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
                    // 工具图片（MCP）只在模型支持视觉时随请求发送
                    supports_vision: session.info.caps.vision,
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

            // 没有工具调用 = 最终回答。最后一轮的模型调用同样计入已用轮数（轮 = 已完成的
            // 模型调用次数），终态卡片的「轮 N/M」才与真实消耗一致。
            if tool_calls.is_empty() {
                if let Some(spec) = kind.sub() {
                    self.bump_subagent_progress(run_id, spec, iteration, 0);
                }
                self.complete(thread_id, run_id, started_at);
                // 输入预测只服务主会话（子代理没有输入框，预测无人消费）
                if kind.is_main() {
                    self.spawn_input_prediction(thread_id, &session, &history, &text);
                }
                return true;
            }

            // 引导应答判定所需的「本轮是否说过话」：必须在 text 被移进 assistant 消息
            // （下一行）之前取，之后 text 已失效。只认非空，不按长度、不评价质量。
            let answered = !text.trim().is_empty();
            let mut assistant = Message::new("assistant", text);
            assistant.tool_calls = tool_calls.clone();
            history.push(assistant);
            // 计划审阅反馈暂存位：反馈必须在**整批工具结果**全部入历史之后再入历史
            // （`crates/craft-model/src/integrity.rs` 模块头的不变量）。此前反馈在工具循环内、当前
            // 调用的结果之前 push，形成 assistant(调用) → user → tool 顺序，OpenAI 系
            // 直接 400 拒绝整条历史；多调用批次里即使只有 exit_plan 一个审阅、其余是
            // 普通工具，也必须等整批结果就位再插。
            let mut pending_plan_feedback: Option<(String, String)> = None;
            for call in tool_calls {
                // 子代理协议调用走专门的分支（派发/等待/列表/停止），不经过常规审批执行链：
                // 它们是编排动作而非工具副作用，各自内部会按需发起自己的审批。
                let is_subagent_call = kind.is_main() && SUBAGENT_PROTOCOL_TOOLS.contains(&call.name.as_str());
                // 计划门计数键（D32）在派发前统一求值（子代理 / wait 恒 None：is_file_write
                // 对它们为 false）：本 run 第 3 个不同文件的写入在执行前拦下并自动进入计划态
                let planned_file_key: Option<String> = if !is_subagent_call
                    && call.name != Wait::NAME
                    && plan_rule_on
                    && !plan_active
                    && self.deps.registry.is_file_write(&call.name)
                {
                    let assessment = self.deps.registry.assess(&env.roots, &call.name, &call.args);
                    let fallback = call.args.get("path").and_then(Value::as_str).unwrap_or_default();
                    Some(craft_tools::registry::Registry::file_key(&assessment.target, fallback))
                } else {
                    None
                };
                let auto_plan_block: Option<String> = planned_file_key.as_ref().and_then(|key| {
                    if key.is_empty() || planned_files.contains(key) || planned_files.len() < PLAN_AUTO_ENTER_FILE_COUNT {
                        return None;
                    }
                    // 拦截说明：已改文件清单 + 被拦文件 + exit_plan 出路，模型据此
                    // 组织计划（已完成部分 + 剩余方案）而不是盲目重试
                    let listed = planned_files.iter().map(String::as_str).collect::<Vec<_>>().join("、");
                    Some(format!(
                        "本次任务已先后修改 {n} 个不同文件（{listed}），现在要再写「{key}」：\
                         多文件改动需要先提交计划让用户复核。已自动进入计划模式，这次写入没有执行。\
                         请调用 exit_plan 提交完整计划（说明已完成的部分与剩余改动方案），\
                         用户批准后即可继续实施。",
                        n = planned_files.len(),
                    ))
                });
                if let Some(text) = &auto_plan_block {
                    self.deps.log.write_thread(
                        thread_id,
                        crate::session_log::LEVEL_INFO,
                        "plan_auto_entered",
                        json!({ "runId": run_id, "iter": iteration, "tool": call.name, "note": text }),
                    );
                }
                let (result, aborted, plan_ended, plan_active_after, plan_feedback_msg, tool_media) = if is_subagent_call {
                    let result = self
                        .run_subagent_tool(token, thread_id, run_id, &message_id, &call, &env, mode)
                        .await;
                    (result, token.is_cancelled(), false, plan_active, None, Vec::new())
                } else if call.name == Wait::NAME {
                    // wait 走 Runner 异步分支（D21）：等待必须对「停止 / 跳过 / 新事件」即时响应，
                    // 而普通工具在 spawn_blocking 里跑完才检查取消令牌。R0 无副作用不经审批，
                    // 计划态不翻转、无媒体——结果落库与审计在 run_wait_tool 内完成。
                    let (result, aborted) = self.run_wait_tool(token, kind, thread_id, run_id, &message_id, &call).await;
                    (result, aborted, false, plan_active, None, Vec::new())
                } else {
                    self.run_tool(
                        token,
                        kind,
                        thread_id,
                        run_id,
                        &message_id,
                        &call,
                        &env,
                        mode,
                        plan_active,
                        plan_id.as_deref(),
                        auto_plan_block.as_deref(),
                    )
                    .await
                };
                // 计划态随协议工具翻转（enter_plan 进入 / exit_plan 批准退出 / D32 自动进入），影响后续 iter
                // true→false 只发生在 exit_plan 批准：标记本 run 已获批，自动门不再触发
                if plan_active && !plan_active_after {
                    plan_approved_this_run = true;
                }
                plan_active = plan_active_after;
                // 自动进入的 run 可能没有计划行（非 planMode 启动）：补建，保证「批准产物 =
                // approved=1 且正文落盘」与手动路径一致（persist_plan_text 侧也有兜底建行）
                if plan_active && plan_id.is_none() && kind.is_main()
                    && let Ok(plan) = craft_store::plans::create_plan(&self.deps.db.lock_conn(), thread_id, run_id)
                {
                    plan_id = Some(plan.id);
                }
                // 计划门计数（D32）：协议进入置标记；只有真正落盘的文件写入才并入
                // planned_files（被拒 / 被拦 / 失败没有真的改动文件，一次误拒不应永久
                // 推高计数），取消的调用同样不计。
                if call.name == EnterPlan::NAME {
                    plan_entered = true;
                }
                if let Some(key) = planned_file_key
                    && !aborted
                    && !result.is_error
                    && !key.is_empty()
                {
                    planned_files.insert(key);
                }
                // 子代理 nudge 计数（issue 1 P4）：检索类调用累计；本 run 派发过即解除
                // 提醒资格（模型已经会用子代理，不需要再教）
                if !aborted && SUBAGENT_NUDGE_TOOLS.contains(&call.name.as_str()) {
                    retrieval_calls += 1;
                }
                if call.name == craft_tools::agent::AgentSpawn::NAME {
                    subagent_spawned_this_run = true;
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
                // 拒绝带反馈：先暂存（持久化已在 run_tool 完成），等整批工具结果
                // 入历史后再注入——模型下一 iter 在计划态下看到反馈并修订计划
                if let Some(feedback) = plan_feedback_msg {
                    pending_plan_feedback = Some(feedback);
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
                // 工具图片（MCP）：挂回 tool 消息，模型下一轮就能看到
                // （Anthropic 放进 tool_result 内容块、OpenAI 延后为 user 消息，见 craft-model）。
                if !tool_media.is_empty() {
                    tool_msg.attachments =
                        load_attachments(&env.workspace_path, &self.deps.attachment_dir, &tool_media).attachments;
                }
                history.push(tool_msg);
            }
            // 整批工具结果已就位：反馈作为真实用户消息注入本 run 历史
            if let Some((feedback_message_id, feedback_text)) = pending_plan_feedback {
                let mut msg = Message::new("user", feedback_text);
                msg.id = Some(feedback_message_id);
                history.push(msg);
                // 计划反馈同属「用户刚说了话」，下一轮要先应答（D19
                // 第 1 节第 4 条）。本轮的正文出现在反馈之前，不代表对它作了回应，
                // 所以置 answer_due 交给下一轮判定。
                answer_due = true;
            }

            // 引导应答判定（D19）：本轮本该应答却一句话没说 →
            // 下一轮开头纠偏一次。只认「正文非空」，不按长度、不评价内容质量——按长度会
            // 误伤「收到，继续」这类合法短应答。子代理没有引导，不参与判定。
            if must_answer && !answered && kind.is_main() {
                steer_unanswered = true;
            }

            // 运行中进度写点（D15 的面板补注）：子代理每完成一次工具
            // 调用，就向父会话追加一条同一部件 id 的中间态子代理事件——卡片与列表的工具
            // 次数、已用轮数实时递增，仍是一张卡（快照重建与渲染层对同一 partId 都是合并
            // 语义）。终态收尾时仍从数据库统计真值覆盖计数，这里的累加值不会被污染。
            if let Some(spec) = kind.sub() {
                self.bump_subagent_progress(run_id, spec, iteration, 1);
            }
        }

        if kind.sub().is_some() {
            // 子代理触顶（D15 的面板补注）：失败信息写明「已用满 N 轮，
            // 上限 M」，准确归因。触顶前最后一段正文作为部分结论带回的既有路径保留
            //（finalize_subagent 的 subagent_final_text：有文本带文本，无文本回退这条失败信息）。
            self.fail(
                thread_id,
                run_id,
                &message_id,
                "max_iterations",
                &format!("已用满 {limit} 轮，上限 {limit} 轮，已停止本次运行"),
                false,
                started_at,
            );
        } else {
            self.fail(
                thread_id,
                run_id,
                &message_id,
                "max_iterations",
                &format!("达到模型调用上限（{limit} 轮），已停止本次运行"),
                false,
                started_at,
            );
        }
        false
    }

    // ---------- 会话建立与失败转移 ----------

    /// 建立单模型会话；失败时带出可读原因（模型缺失/已停用/密钥缺失等），供转移文案使用。
    fn open_session(&self, model_ref: &str) -> (Option<Session>, String) {
        match self.deps.models.resolve(model_ref) {
            None => (None, "模型不存在".into()),
            Some(info) => {
                // 每次建立会话即时求值代理配置（设置保存即时生效），经 ModelFactory 应用到协议客户端
                let proxy = (self.deps.proxy_config)();
                match self.deps.models.client(model_ref, Some(&proxy)) {
                    Ok(client) => (Some(Session { client, info }), String::new()),
                    Err(err) => (None, err.message),
                }
            }
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

    // ---------- 子代理（决策 D15） ----------

    /// 子代理协议工具的统一入口：agent_spawn / agent_wait / agent_list / agent_close。
    /// 与 `run_tool` 的分工：这里不经过路径解析与风险审批（四个工具本身是编排动作），
    /// 但每一次派发都要落 tool_call 部件与审计，保持「工具调用一律可审计」不变。
    #[allow(clippy::too_many_arguments)]
    async fn run_subagent_tool(
        self: &Arc<Self>,
        token: &CancellationToken,
        thread_id: &str,
        run_id: &str,
        message_id: &str,
        call: &ToolCall,
        env: &RunEnv,
        mode: &str,
    ) -> ToolResult {
        let part_id = new_id(id_prefix::PART);
        let spec = self.deps.registry.spec_for(&call.name);
        let summary = match (&spec, &spec.as_ref().and_then(|s| s.summary_arg.clone())) {
            (Some(_), Some(arg)) => call.args.get(arg).and_then(Value::as_str).unwrap_or_default().to_string(),
            (Some(s), None) => s.summary_text.clone().unwrap_or_else(|| call.name.clone()),
            _ => call.name.clone(),
        };
        let title = spec.as_ref().map(|s| s.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| call.name.clone());
        let risk = spec.as_ref().map(|s| s.risk).unwrap_or(craft_policy::PolicyRisk::R0);
        self.deps.records.emit(
            event_type::TOOL_COMPLETED,
            Some(thread_id),
            Some(run_id),
            json!({
                "partId": part_id,
                "toolCallId": call.id,
                "name": call.name,
                "args": call.args,
                "risk": risk.as_str(),
                "status": "running",
                "summary": summary,
                "title": title,
            }),
        );

        let config = (*self.deps.subagent_config)();
        let exec_start = now_millis();
        let result = if !config.enabled {
            ToolResult::err_code("子代理功能已在设置中关闭：请在 设置 → 子代理 中开启后再派发", "subagent_disabled")
        } else {
            match call.name.as_str() {
                AgentSpawn::NAME => self.spawn_subagent(token, thread_id, run_id, message_id, call, env, mode, config).await,
                AgentWait::NAME => self.wait_subagents(token, thread_id, run_id, call).await,
                AgentList::NAME => ToolResult::ok(crate::subagent::format_status_list(&self.subagents.list(thread_id))),
                AgentClose::NAME => self.close_subagent(thread_id, call),
                other => ToolResult::err_code(format!("未知的子代理工具：{other}"), "unknown_tool"),
            }
        };
        let mut result = result;
        {
            let mut meta = result.meta.take().unwrap_or(json!({}));
            meta["durationMs"] = json!(now_millis() - exec_start);
            result.meta = Some(meta);
        }
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
                "risk": risk.as_str(),
                "status": if result.is_error { "error" } else { "done" },
                "output": result.output,
                "isError": result.is_error,
                "code": result.code,
                "durationMs": duration_ms,
                "summary": summary,
                "title": title,
            }),
        });
        result
    }

    /// 派发子代理：校验 → 建子会话 → 登记注册表 → 写 running 卡片 → 启动子 run。
    /// 前台：等子 run 终态并返回结论；后台：立即返回启动回执。
    #[allow(clippy::too_many_arguments)]
    async fn spawn_subagent(
        self: &Arc<Self>,
        token: &CancellationToken,
        thread_id: &str,
        run_id: &str,
        message_id: &str,
        call: &ToolCall,
        env: &RunEnv,
        mode: &str,
        config: SubAgentConfig,
    ) -> ToolResult {
        // 1) 并发窗口：在跑的子代理（含后台）占名额
        let running = self.subagents.running_count(thread_id);
        if running >= config.max_concurrent {
            return ToolResult::err_code(
                format!(
                    "子代理并发已达上限（{}）：请先用 agent_wait 等待已有结果，或用 agent_close 停止一个再派发",
                    config.max_concurrent
                ),
                "subagent_limit",
            );
        }
        // 2) 参数校验（工具薄壳已验证过一遍；这里按编排需要重新取，防御模型漏参）
        let prompt = call.args.get("prompt").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if prompt.is_empty() {
            return ToolResult::err_code("prompt 不能为空：请把要交给子代理的完整任务写进 prompt", "invalid_args");
        }
        let description = call.args.get("description").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if description.is_empty() {
            return ToolResult::err_code("description 不能为空", "invalid_args");
        }
        // 3) 档案解析（决策 D15 扩展）：内置两型或用户自定义定义 → 统一形状。
        // 解析失败给可读错误并列出可用名字（含自定义），让模型能自纠。
        let requested_type = call.args.get("agent_type").and_then(Value::as_str).unwrap_or_default();
        let profile = match config.resolve(requested_type) {
            Ok(profile) => profile,
            Err(err) => return ToolResult::err_code(err, "invalid_args"),
        };
        let agent_type = profile.agent_type;
        // 缺省即后台（D31）：串行工具循环下前台缺省会让「同批多派」排队等终态，并行能力
        // 形同虚设；显式 false 才走前台等待。前台路径保留给「必须立刻拿到结论才能继续」的场景。
        let run_in_background = call.args.get("run_in_background").and_then(Value::as_bool).unwrap_or(true);
        // 4) 模型：工具参数 > 档案默认 > 父会话继承，三级回落
        let parent_thread = match get_thread(&self.deps.db.lock_conn(), thread_id) {
            Ok(row) => row,
            Err(err) => return ToolResult::err_code(format!("读取父会话失败：{err}"), "tool_error"),
        };
        let explicit_model = call.args.get("model").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let profile_model = if profile.model.trim().is_empty() { None } else { Some(profile.model.as_str()) };
        let model_ref = match explicit_model.or(profile_model) {
            Some(requested) => {
                if self.deps.models.resolve(requested).is_none() {
                    return ToolResult::err_code(
                        format!("指定的子代理模型不存在或已停用：{requested}"),
                        "provider_unavailable",
                    );
                }
                requested.to_string()
            }
            None => parent_thread.model_id.clone(),
        };
        // 5) 工具面：按档案裁剪（内置 explore 只读；自定义按定义三档；都没有 agent_* 与计划协议工具）
        let agent_profile = crate::subagent::AgentProfile {
            agent_type,
            allowed_tools: profile.allowed_tools.clone(),
            role_prompt: profile.role_prompt.clone(),
        };
        let allowed_tools = crate::subagent::resolve_tool_pool(mode, &self.deps.registry, &agent_profile);
        if allowed_tools.is_empty() {
            return ToolResult::err_code("当前模式下子代理没有可用工具，请检查工作区与模式设置", "tool_error");
        }
        // 6) 子会话：标题取派发摘要（用户只看一眼就知道这次派了什么）；工作区继承父会话。
        // 自定义定义的生效档案快照进 agent_profile：定义可被用户随时改删，历史子会话
        // 与卡片仍要能解释「当时按什么跑的」。
        let child_id = new_id(id_prefix::THREAD);
        let requested_name = call.args.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let normalized = craft_tools::agent::normalize_agent_name(&requested_name);
        // 短名缺省时用类型名（自定义定义用它的 name，内置用 general/explore），
        // 让 "谁在跑" 在卡片上一眼可读；注册表还会按冲突加序号。
        let preset_name = if normalized.is_empty() { profile.type_name.clone() } else { normalized };
        // 标题取派发摘要前 60 字符：够在卡片上辨识「谁在跑什么」，又不把卡片撑成一段正文。
        let title: String = description.chars().take(60).collect();
        let profile_snapshot = if agent_type == AgentType::Custom {
            profile.snapshot_json()
        } else {
            String::new()
        };
        if let Err(err) = craft_store::threads::create_subagent_thread(
            &self.deps.db.lock_conn(),
            craft_store::threads::NewSubagentThread {
                id: &child_id,
                mode: &parent_thread.mode,
                title: &title,
                model_id: &model_ref,
                reasoning_level: &parent_thread.reasoning_level,
                workspace_id: &parent_thread.workspace_id,
                parent_thread_id: thread_id,
                agent_name: &preset_name,
                agent_type: agent_type.as_str(),
                spawn_call_id: &call.id,
                agent_profile: &profile_snapshot,
            },
        ) {
            return ToolResult::err_code(format!("创建子代理会话失败：{err}"), "tool_error");
        }
        // 6) 注册：同一父会话内同名自动加序号，最终名字回写子会话（agent_name 供模型引用）
        let part_id = new_id(id_prefix::PART);
        let started_at = now_millis();
        // 生效轮数上限的预估值（running 卡片当场就能带出「轮 0/N」）：档案正值优先，
        // 否则按父会话上限与封顶取小。子 run 启动时以同一公式对齐一次（见 run 循环开头）。
        let parent_limit = self
            .max_iterations
            .lock()
            .unwrap()
            .as_ref()
            .map(|f| f())
            .unwrap_or(DEFAULT_MAX_ITERATIONS);
        let effective_limit = crate::subagent::iterations_for_profile(parent_limit, profile.max_iterations);
        let snapshot = SubAgentSnapshot {
            thread_id: child_id.clone(),
            parent_thread_id: thread_id.to_string(),
            parent_message_id: message_id.to_string(),
            part_id: part_id.clone(),
            agent_name: preset_name,
            agent_type,
            description: description.clone(),
            model_ref: model_ref.clone(),
            run_in_background,
            status: AgentStatus::Running,
            started_at,
            finished_at: None,
            result: String::new(),
            tool_uses: 0,
            tokens: 0,
            iterations: 0,
            max_iterations: effective_limit,
            notify: run_in_background,
        };
        let cancel = CancellationToken::new();
        let effective_name = self.subagents.register(snapshot, cancel.clone());
        let registered = self.subagents.snapshot(&child_id).expect("刚登记的子代理必然存在");
        let _ = craft_store::threads::set_thread_agent_name(&self.deps.db.lock_conn(), &child_id, &effective_name);
        // 7) 写 running 卡片（落在发起这次调用的 assistant 消息里，用户当场就能看到）
        self.append_subagent_part(thread_id, message_id, Some(run_id), &registered);
        // 8) 子会话首条用户消息：派发正文（它自己的历史只有这一条 + 自己的产出）
        let first_message_id = new_id(id_prefix::MESSAGE);
        let _ = self.deps.records.append(&EventRecord {
            thread_id: child_id.clone(),
            message_id: first_message_id.clone(),
            run_id: None,
            kind: "user_message".into(),
            payload: json!({ "text": prompt, "attachments": [], "planMode": false }),
        });
        let spec = SubAgentSpec {
            thread_id: child_id.clone(),
            parent_thread_id: thread_id.to_string(),
            parent_run_id: run_id.to_string(),
            parent_message_id: message_id.to_string(),
            spawn_call_id: call.id.clone(),
            part_id: part_id.clone(),
            agent_name: effective_name.clone(),
            agent_type,
            description: description.clone(),
            model_ref: model_ref.clone(),
            allowed_tools: allowed_tools.clone(),
            run_in_background,
            inject_instructions: profile.inject_instructions,
            max_iterations: profile.max_iterations,
            role_prompt: profile.role_prompt.clone(),
            display_name: profile.display_name.clone(),
            profile_snapshot: profile_snapshot.clone(),
        };
        let trigger_seq = self
            .deps
            .db
            .with(|conn| {
                let mut stmt = conn
                    .prepare("SELECT seq FROM events WHERE thread_id = ?1 AND kind = 'user_message' ORDER BY seq DESC LIMIT 1")
                    .map_err(craft_store::db::other_err)?;
                Ok(stmt.query_row([child_id.as_str()], |row| row.get::<_, i64>(0)).ok())
            })
            .ok()
            .flatten();
        if let Err(err) = self.start_run(&child_id, trigger_seq, RunKind::Sub(Box::new(spec)), Some(cancel)) {
            self.finish_subagent(&child_id, AgentStatus::Failed, &format!("启动子代理运行失败：{err}"));
            return ToolResult::err_code(format!("启动子代理运行失败：{err}"), "tool_error");
        }
        self.deps.log.write_thread(
            thread_id,
            LEVEL_INFO,
            "subagent_spawned",
            json!({
                "runId": run_id,
                "agentId": child_id,
                "name": effective_name,
                "agentType": agent_type.as_str(),
                "background": run_in_background,
                "tools": allowed_tools.len(),
                "restricted": env.restricted,
            }),
        );
        if run_in_background {
            // 后台：立即返回启动回执；结论在终态时经报告队列回到主会话
            ToolResult::ok_with_meta(
                crate::subagent::format_background_ack(&registered),
                json!({ "agentId": child_id, "name": effective_name, "background": true }),
            )
        } else {
            // 前台：等子 run 终态（父级取消同样打断等待，由 run 循环按 canceled 收尾）
            let final_snapshot = self.await_subagent(token, &child_id).await;
            match final_snapshot {
                Some(snapshot) => ToolResult::ok_with_meta(
                    crate::subagent::format_foreground_result(&snapshot),
                    json!({
                        "agentId": child_id,
                        "name": snapshot.agent_name,
                        "status": snapshot.status.as_str(),
                        "toolUses": snapshot.tool_uses,
                        "tokens": snapshot.tokens,
                    }),
                ),
                None => ToolResult::err_code("等待子代理结果时被取消", "tool_error"),
            }
        }
    }

    /// 等待单个子代理进入终态：轮询注册表（子 run 收尾时写终态），父级取消即返回 None。
    /// 轮询而非 channel 的理由：终态由子 run 在完成路径上写入，轮询只需 100ms 粒度，
    /// 且天然支持「注册表被重启清理」与「子代理被 agent_close 提前终止」两种旁路。
    async fn await_subagent(&self, token: &CancellationToken, agent_id: &str) -> Option<SubAgentSnapshot> {
        loop {
            if token.is_cancelled() {
                return None;
            }
            if let Some(snapshot) = self.subagents.snapshot(agent_id)
                && snapshot.status.is_terminal()
            {
                return Some(snapshot);
            }
            tokio::select! {
                _ = token.cancelled() => return None,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        }
    }

    /// agent_wait：等任一目标进入终态（缺省等全部在跑子代理），超时返回当前状态。
    ///
    /// 缺省目标的口径：只取仍在跑的条目。登记列表含终态条目（finish 只 update 不 remove），
    /// 不过滤会让「任一终态即返回」被历史终态立即满足——没等到任何东西却回成功。
    async fn wait_subagents(&self, token: &CancellationToken, thread_id: &str, _run_id: &str, call: &ToolCall) -> ToolResult {
        let requested: Vec<String> = call
            .args
            .get("names")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let mut targets: Vec<SubAgentSnapshot> = if requested.is_empty() {
            // 省略 names：只等仍在跑的（口径见函数注释）
            self.subagents
                .list(thread_id)
                .into_iter()
                .filter(|t| !t.status.is_terminal())
                .collect()
        } else {
            // 显式 names：保留含终态查找——查询已终结子代理的状态仍有用
            requested
                .iter()
                .filter_map(|name| self.subagents.find_by_name(thread_id, name))
                .collect()
        };
        if targets.is_empty() {
            // 空目标是误用：agent_wait 只等待 agent_spawn 派出的子代理，不等待后台任务
            // （后台任务完成会自动通知）。这里必须返回可读错误让模型纠偏——返回成功会让
            // 模型把后台任务误当子代理连续重试，界面还显示「完成」。
            return if requested.is_empty() {
                // 省略 names 且过滤后为空：本会话没有在跑的子代理（从未派发，或全部已终态）
                ToolResult::err_code(
                    "没有在跑的子代理可等待：agent_wait 只等待 agent_spawn 派出且尚未结束的子代理；\
                     后台任务完成时会自动通知，不需要等待（需要输出时用 bash_output 读一次）。",
                    "subagent_not_found",
                )
            } else {
                ToolResult::err_code(
                    format!(
                        "找不到子代理：{}（用 agent_list 查看；agent_wait 只等待 agent_spawn 派出的子代理，不等待后台任务）",
                        requested.join("、")
                    ),
                    "subagent_not_found",
                )
            };
        }
        let timeout = craft_tools::agent::clamp_wait_timeout(call.args.get("timeout_ms").and_then(Value::as_i64));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout as u64);
        let mut timed_out = true;
        loop {
            // 任一目标终态即返回（另一个仍在跑的继续在后台推进，不浪费已投入的工作）
            if targets.iter().any(|t| {
                self.subagents
                    .snapshot(&t.thread_id)
                    .map(|s| s.status.is_terminal())
                    .unwrap_or(true)
            }) {
                timed_out = false;
                break;
            }
            if token.is_cancelled() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        }
        // 用最新快照重组结果列表
        targets = targets
            .into_iter()
            .map(|t| self.subagents.snapshot(&t.thread_id).unwrap_or(t))
            .collect();
        let mut text = crate::subagent::format_status_list(&targets);
        if timed_out {
            text.push_str("\n（等待超时：结果尚未产生。请继续做别的工作，不要反复短等。）");
        } else if targets.iter().all(|t| t.status.is_terminal()) {
            // 显式 names 查询已全部终态的子代理（或等待收尾时恰好全部跑完）：立即返回，
            // 注明无需等待——让模型知道这不是超时，也不必再等
            text.push_str("\n（全部目标已终结，无需等待。）");
        }
        ToolResult::ok_with_meta(text, json!({ "timedOut": timed_out, "timeoutMs": timeout }))
    }

    /// wait 工具的 Runner 分支（决策 D21）：可被事件与用户打断的定时等待。
    ///
    /// 为什么不走 run_tool：普通工具在 spawn_blocking 里执行，取消令牌要等它跑完才生效，
    /// 事件队列也无法唤醒在途 await；等待必须对「停止 / 跳过 / 新事件」即时响应。
    /// 三条出局路径：睡满正常返回；后台任务完成 / 子代理回报 / 运行中引导任一到达提前
    /// 返回（只报告被什么唤醒，事件正文仍走既有唯一注入点——下一轮迭代开头）；用户在
    /// 界面跳过则记 canceled 且 run 继续，模型据此立即核实等待目的。会话停止也记
    /// canceled 留痕（对 wait 例外于「取消即丢弃」的通用语义：纯计时无副作用）。
    async fn run_wait_tool(
        self: &Arc<Self>,
        token: &CancellationToken,
        kind: &RunKind,
        thread_id: &str,
        run_id: &str,
        message_id: &str,
        call: &ToolCall,
    ) -> (ToolResult, bool) {
        let part_id = new_id(id_prefix::PART);
        let spec = self.deps.registry.spec_for(Wait::NAME);
        let risk = spec.as_ref().map(|s| s.risk).unwrap_or(craft_policy::PolicyRisk::R0);
        let title = spec
            .as_ref()
            .map(|s| s.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| Wait::NAME.to_string());
        // 摘要优先取 purpose：等待卡的界面显示与被跳过后的继续锚点都靠它
        let summary = spec
            .as_ref()
            .and_then(|s| s.summary_arg.clone())
            .and_then(|arg| call.args.get(arg))
            .and_then(Value::as_str)
            .map(craft_tools::wait::clamp_purpose)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| spec.as_ref().and_then(|s| s.summary_text.clone()).unwrap_or_else(|| Wait::NAME.to_string()));
        self.deps.records.emit(
            event_type::TOOL_COMPLETED,
            Some(thread_id),
            Some(run_id),
            json!({
                "partId": part_id,
                "toolCallId": call.id,
                "name": Wait::NAME,
                "args": call.args,
                "risk": risk.as_str(),
                "status": "running",
                "summary": summary,
                "title": title,
            }),
        );

        let duration_ms = craft_tools::wait::clamp_wait_duration(call.args.get("duration_ms").and_then(Value::as_i64));
        let purpose = craft_tools::wait::clamp_purpose(call.args.get("purpose").and_then(Value::as_str).unwrap_or_default());
        self.wait_active.lock().unwrap().insert(thread_id.to_string());
        // 清掉陈旧跳过请求：一次跳过只作用于它发出时的那一次等待
        self.wait_skips.lock().unwrap().remove(thread_id);
        let started = now_millis();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(duration_ms as u64);
        // 唤醒源。事件唤醒只对主会话生效：通知 / 报告 / steer 的正文注入点都在主会话的
        // 迭代开头（runner 的注入点全部 kind.is_main() 门控），提前唤醒却等不来正文是假信号。
        enum WakeSource {
            Timeout,
            Steer,
            TaskNotification,
            SubagentReport,
            UserSkip,
            RunCanceled,
        }
        let wake = loop {
            if kind.is_main() && self.has_steers(thread_id) {
                break WakeSource::Steer;
            }
            if kind.is_main() && self.deps.shell_tasks.as_ref().is_some_and(|s| s.has_notifications(thread_id)) {
                break WakeSource::TaskNotification;
            }
            if kind.is_main() && self.subagents.has_reports(thread_id) {
                break WakeSource::SubagentReport;
            }
            if self.wait_skips.lock().unwrap().remove(thread_id) {
                break WakeSource::UserSkip;
            }
            if token.is_cancelled() {
                break WakeSource::RunCanceled;
            }
            if tokio::time::Instant::now() >= deadline {
                break WakeSource::Timeout;
            }
            tokio::select! {
                _ = token.cancelled() => break WakeSource::RunCanceled,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        };
        self.wait_active.lock().unwrap().remove(thread_id);
        let elapsed = (now_millis() - started).max(0);
        let (status, output, woke_by) = match wake {
            WakeSource::Timeout => (
                "done",
                "已等待到时，期间无新事件。请继续你的下一步；不要把本工具当轮询节奏器反复调用。".to_string(),
                "timeout",
            ),
            WakeSource::Steer => (
                "done",
                "等待被中断：用户追加了输入（正文将在下一步并入对话）。请先处理用户输入。".to_string(),
                "steer",
            ),
            WakeSource::TaskNotification => (
                "done",
                "等待被中断：后台命令任务出现终态（通知正文将在下一步并入对话）。请继续处理相关任务。".to_string(),
                "task_notification",
            ),
            WakeSource::SubagentReport => (
                "done",
                "等待被中断：子代理已回报（报告正文将在下一步并入对话）。".to_string(),
                "subagent_report",
            ),
            WakeSource::UserSkip => (
                "canceled",
                if purpose.is_empty() {
                    "等待被用户跳过。请立即继续你等待后的下一步（去核实要等的状态），不要重新等待。".to_string()
                } else {
                    format!("等待被用户跳过（原目的：{purpose}）。请立即继续核实该状态，不要重新等待。")
                },
                "user_cancel",
            ),
            WakeSource::RunCanceled => ("canceled", "等待随会话停止而中断。".to_string(), "run_canceled"),
        };
        self.deps.records.append_or_warn(&EventRecord {
            thread_id: thread_id.to_string(),
            run_id: Some(run_id.to_string()),
            message_id: message_id.to_string(),
            kind: "tool_call".into(),
            payload: json!({
                "partId": part_id,
                "toolCallId": call.id,
                "name": Wait::NAME,
                "args": call.args,
                "risk": risk.as_str(),
                "status": status,
                "output": output,
                "isError": false,
                "durationMs": elapsed,
                "summary": summary,
                "title": title,
                "wokeBy": woke_by,
            }),
        });
        self.deps.log.write_thread(
            thread_id,
            LEVEL_DEBUG,
            "wait_done",
            json!({
                "runId": run_id,
                "requestedMs": duration_ms,
                "elapsedMs": elapsed,
                "wokeBy": woke_by,
                "purpose": purpose,
            }),
        );
        let aborted = matches!(wake, WakeSource::RunCanceled);
        (
            ToolResult {
                output,
                is_error: false,
                code: String::new(),
                meta: Some(json!({ "wokeBy": woke_by, "requestedMs": duration_ms })),
                media: Vec::new(),
                truncation: None,
            },
            aborted,
        )
    }

    /// agent_close：停止一个在跑的子代理（终态记为 closed，与父级连带中断区分）。
    fn close_subagent(&self, thread_id: &str, call: &ToolCall) -> ToolResult {
        let name = call.args.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
        if name.is_empty() {
            return ToolResult::err_code("name 不能为空：请用 agent_list 查看子代理名称", "invalid_args");
        }
        let Some(snapshot) = self.subagents.find_by_name(thread_id, &name) else {
            return ToolResult::err_code(format!("找不到子代理：{name}（用 agent_list 查看现有子代理）"), "subagent_not_found");
        };
        if snapshot.status.is_terminal() {
            return ToolResult::ok(format!(
                "子代理 {} 已处于终态（{}），无需停止。",
                snapshot.agent_name,
                snapshot.status.as_str()
            ));
        }
        // 标记「模型主动停止」：子 run 的取消收尾据此写 closed 而不是 interrupted
        self.closed_agents.lock().unwrap().insert(snapshot.thread_id.clone());
        if let Some(cancel) = self.subagents.cancel_token(&snapshot.thread_id) {
            cancel.cancel();
        }
        ToolResult::ok(format!("已请求停止子代理 {}，它的结论不会再回报。", snapshot.agent_name))
    }

    /// 子代理终态收尾：更新注册表 → 回填子会话 token → 写终态卡片 → 后台派发时投递报告。
    /// `status` 由调用方按失败/取消/正常完成给出；被 agent_close 停止的由这里改写为 closed。
    fn finish_subagent(self: &Arc<Self>, agent_id: &str, status: AgentStatus, result: &str) {
        let closed = self.closed_agents.lock().unwrap().remove(agent_id);
        let status = if closed && status == AgentStatus::Interrupted {
            AgentStatus::Closed
        } else {
            status
        };
        let stats = self.subagent_stats(agent_id);
        let mut finished: Option<SubAgentSnapshot> = None;
        self.subagents.update(agent_id, |snapshot| {
            snapshot.status = status;
            snapshot.finished_at = Some(now_millis());
            snapshot.result = result.to_string();
            snapshot.tool_uses = stats.0;
            snapshot.tokens = stats.1;
            // 前台派发的结论经工具结果直接回到主循环，不需要再注入一次报告
            snapshot.notify = snapshot.run_in_background && status.is_terminal();
            finished = Some(snapshot.clone());
        });
        let Some(snapshot) = finished else {
            // 注册表里没有（重启后的残留 / 已被父会话清理）：无可收尾
            return;
        };
        self.append_subagent_part(&snapshot.parent_thread_id, &snapshot.parent_message_id.clone(), None, &snapshot);
        self.deps.log.write_thread(
            &snapshot.parent_thread_id,
            LEVEL_INFO,
            "subagent_finished",
            json!({
                "agentId": snapshot.thread_id,
                "name": snapshot.agent_name,
                "status": status.as_str(),
                "toolUses": snapshot.tool_uses,
                "tokens": snapshot.tokens,
                "ms": snapshot.finished_at.unwrap_or(0) - snapshot.started_at,
            }),
        );
        if snapshot.notify {
            self.subagents.push_report(crate::subagent::PendingReport {
                thread_id: snapshot.parent_thread_id.clone(),
                agent_name: snapshot.agent_name.clone(),
                text: crate::subagent::format_report(&snapshot),
            });
            self.wake_parent_if_idle(&snapshot.parent_thread_id);
        }
    }

    /// 子会话内的工具调用次数与累计 token（收尾统计）。
    fn subagent_stats(&self, agent_id: &str) -> (i64, i64) {
        self.deps
            .db
            .with(|conn| {
                let tool_uses: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM events WHERE thread_id = ?1 AND kind = 'tool_call'",
                        [agent_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                Ok(tool_uses)
            })
            .map(|tool_uses| {
                let tokens = craft_store::threads::get_thread(&self.deps.db.lock_conn(), agent_id)
                    .map(|row| row.tokens)
                    .unwrap_or(0);
                (tool_uses, tokens)
            })
            .unwrap_or((0, 0))
    }

    /// 写子代理卡片事件（running / 终态各一次；快照按 partId 合并成一张卡）。
    fn append_subagent_part(&self, parent_thread_id: &str, message_id: &str, run_id: Option<&str>, snapshot: &SubAgentSnapshot) {
        self.deps.records.append_or_warn(&EventRecord {
            thread_id: parent_thread_id.to_string(),
            run_id: run_id.map(str::to_string),
            message_id: message_id.to_string(),
            kind: "subagent".into(),
            payload: crate::subagent::part_payload(snapshot),
        });
    }

    /// 运行中进度推进（D15 的面板补注）：更新注册表里的已用轮数与
    /// 工具次数，并向父会话追加一条同一部件 id 的中间态子代理事件（卡片与列表实时递增，
    /// 仍是一张卡）。子 run 每完成一次工具调用调用一次（tool_delta = 1）；无工具调用的
    /// 最终回答轮在 complete 前调用（tool_delta = 0），保证轮数与真实模型调用次数一致。
    /// 注册表无记录（重启后的残留等）时静默跳过。
    fn bump_subagent_progress(&self, run_id: &str, spec: &SubAgentSpec, iteration: u64, tool_delta: i64) {
        let updated = self.subagents.update(&spec.thread_id, |snapshot| {
            snapshot.iterations = iteration as i64 + 1;
            snapshot.tool_uses += tool_delta;
        });
        if updated
            && let Some(snapshot) = self.subagents.snapshot(&spec.thread_id)
        {
            self.append_subagent_part(&snapshot.parent_thread_id, &snapshot.parent_message_id, Some(run_id), &snapshot);
        }
    }

    /// 父会话空闲时自动唤醒：有在途运行则报告留在队列里（下一次迭代注入）；
    /// 空闲则起新 run，让主代理读到结论并向用户汇报。
    fn wake_parent_if_idle(self: &Arc<Self>, parent_thread_id: &str) {
        if self.cancels.lock().unwrap().contains_key(parent_thread_id) {
            return;
        }
        if !self.subagents.has_reports(parent_thread_id) {
            return;
        }
        // 记录一条 user 消息作为唤醒锚点：让这次 run 有 trigger_seq（快照定序用），
        // 也让用户在主会话时间线上看到「子代理回来了」的明确起点。
        let names: Vec<String> = self
            .subagents
            .list(parent_thread_id)
            .into_iter()
            .filter(|s| s.notify && s.status.is_terminal())
            .map(|s| s.agent_name)
            .collect();
        let text = if names.is_empty() {
            "（子代理已完成，结论随后附上）".to_string()
        } else {
            format!("（子代理 {} 已完成，结论随后附上）", names.join("、"))
        };
        let message_id = new_id(id_prefix::MESSAGE);
        let seq = self
            .deps
            .records
            .append(&EventRecord {
                thread_id: parent_thread_id.to_string(),
                message_id: message_id.clone(),
                run_id: None,
                kind: "user_message".into(),
                payload: json!({ "text": text, "attachments": [], "planMode": false, "via": "subagent_report" }),
            })
            .map(|(_, seq)| seq)
            .ok();
        // 报告本身在 run 的投递点注入（与运行中到达的报告走同一条路径）。
        // 起 run 用与 start_run 相同的双路径：tokio 上下文里直接 spawn，否则起线程驱动
        // 独立运行时（与 Runner::start_run 的取舍一致——测试与同步上下文没有 reactor）。
        let this = Arc::clone(self);
        let parent = parent_thread_id.to_string();
        let run = move || {
            // 唤醒失败是静默丢通知的根源之一：留日志，不静默吞掉
            if let Err(err) = this.start_run(&parent, seq, RunKind::Main, None) {
                this.deps.log.write_thread(
                    &parent,
                    LEVEL_ERROR,
                    "subagent_wake_start_failed",
                    json!({ "error": err }),
                );
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move { run() });
            }
            Err(_) => {
                std::thread::spawn(run);
            }
        }
    }

    /// 后台命令任务完成后唤醒空闲会话（`crates/craft-agent/src/shell_tasks.rs` 模块头）。
    ///
    /// 与子代理的 `wake_parent_if_idle` 同一套路，但少一步：子代理需要先写一条锚点用户
    /// 消息，后台任务的通知本身就会在注入点被并入历史，因此这里只起 run。
    /// 会话仍在跑时不唤醒——通知会在下一次迭代的注入点被消费。
    fn wake_shell_tasks_if_idle(self: &Arc<Self>, thread_id: &str) {
        let Some(shell_tasks) = &self.deps.shell_tasks else { return };
        if self.cancels.lock().unwrap().contains_key(thread_id) {
            return;
        }
        if !shell_tasks.has_notifications(thread_id) {
            return;
        }
        let this = Arc::clone(self);
        let thread = thread_id.to_string();
        let run = move || {
            // trigger_seq 传 None：这是一次由后台任务驱动的唤起，没有对应的用户触发消息，
            // 与子代理唤醒（有锚点消息）在定序上不同——快照按 run 自身的 message_id 定序。
            // 唤醒失败同样留日志（与 subagent_wake_start_failed 同理）
            if let Err(err) = this.start_run(&thread, None, RunKind::Main, None) {
                this.deps.log.write_thread(
                    &thread,
                    LEVEL_ERROR,
                    "shell_task_wake_start_failed",
                    json!({ "error": err }),
                );
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move { run() });
            }
            Err(_) => {
                std::thread::spawn(run);
            }
        }
    }

    /// 应用退出 / 窗口关闭：整组杀掉全部仍在跑的后台命令任务。
    ///
    /// 后台任务提交后已脱离 run 与轮次，会话取消不会带走它们；进程一旦成为孤儿就再没有
    /// 人回收。因此退出路径必须有这一手——否则用户关掉应用，构建进程还在后台跑。
    pub fn shutdown_shell_tasks(&self) -> usize {
        let Some(shell_tasks) = &self.deps.shell_tasks else { return 0 };
        let running = shell_tasks.running_all();
        for task in &running {
            craft_core::platform::kill_process_group(task.pid);
            shell_tasks.settle(&task.id, craft_tools::shell_tasks::TaskState::Killed, None);
        }
        running.len()
    }

    // ---------- 工具执行 ----------
    /// 单工具调用全流程：评估 → tool.started → 拦截判定/审批 → 执行 → 落库 → 审计。
    /// plan_active 为本 run 当前是否处于计划态：进入分支会翻转它，返回值第 3 元带回
    /// 最终值；Run 循环据此更新自己的 plan_active（影响后续 iter 的提示词与闸门）。
    /// plan_id 为本 run 的计划行 id（exit_plan 批准/拒绝时精确回填，None=无计划行）。
    /// kind 为运行种类：子代理运行时审批卡代理到父会话、审计带子代理前缀。
    /// auto_plan_block（D32）为 Some 时：本次调用在执行前被自动计划门拦下——置
    /// plan_active = true 并按 enter_plan 的幂等分支广播，文本作为 denied 说明
    /// （被拦写入不落盘、不进审批，只回带出路的错误结果）。
    /// 返回 (结果, 取消, 正常终止轮次, 计划态最终值, 反馈用户消息, 媒体文件名)：
    /// - 取消为 true：Run 循环按 canceled 收尾（审批取消 / 执行中取消）；
    /// - 正常终止轮次为 true：计划被无反馈拒绝，Run 循环按正常完成（done）收尾；
    /// - 反馈用户消息 Some((message_id, 正文)) 时 Run 循环把它 push 进本 run 历史
    ///   （模型下一 iter 看到反馈；持久化已在本函数完成）；
    /// - 媒体文件名：工具图片（MCP）落盘后的文件名列表，Run 循环把图片挂回
    ///   tool 消息（模型可见），事件载荷也已携带（界面缩略图用）。
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    async fn run_tool(
        self: &Arc<Self>,
        token: &CancellationToken,
        kind: &RunKind,
        thread_id: &str,
        run_id: &str,
        message_id: &str,
        call: &ToolCall,
        env: &RunEnv,
        mode: &str,
        mut plan_active: bool,
        plan_id: Option<&str>,
        auto_plan_block: Option<&str>,
    ) -> (ToolResult, bool, bool, bool, Option<(String, String)>, Vec<String>) {
        let part_id = new_id(id_prefix::PART);
        let assessment = self.deps.registry.assess_scoped(
            &if env.roots.is_empty() { vec![".".to_string()] } else { env.roots.clone() },
            &call.name,
            &call.args,
            plan_active,
            thread_id,
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

        // ---------- 计划模式协议分支（enter_plan / exit_plan）----------
        //
        // 工具 execute 是薄壳（参数校验 + 确认文本），状态切换与审批边界在 Runner：
        // 这里修改本函数局部副本 plan_active，最终状态经返回值第 3 元带回 Run 循环
        // （Run 循环据此更新自己的 plan_active，影响下一 iter 的提示词注入与闸门）。
        // exit_plan 三种结果（审阅边界：只认批准的显式决定与带/不带反馈的拒绝）：
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
                // 不在计划态时拒绝（普通错误结果，模型可自行纠正）
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
                        return (ToolResult::err_code("计划审批已取消", "tool_error"), true, false, plan_active, None, Vec::new());
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
                        // 忽略（拒绝无反馈）：计划落盘（未批准），本轮立即终止——
                        // 不给模型继续迭代或原样重提的口子；会话按正常完成收尾回到
                        // idle 等用户下一条输入。不是 fail/canceled：这是用户的明确
                        // 决定，run 正常结束（侧栏不亮错误，也不算用户主动取消）。
                        self.persist_plan_text(thread_id, run_id, plan_id, plan_text, false);
                        let notice = "用户选择了忽略：本轮任务已停止，等待你的下一步反馈或新指示。".to_string();
                        return (ToolResult::err(notice), false, true, plan_active, None, Vec::new());
                    }
                }
            }
        } else if let Some(block_text) = auto_plan_block {
            // 自动进入计划态（D32）：本 run 第 3 个不同文件的写入在执行前被拦下，与
            // enter_plan 走同一广播路径（幂等）；denied 置拦截说明——被拦写入不落盘、
            // 不进审批，只回带出路的错误结果（先拦后审，与下方硬门卡同一位置纪律）。
            // 计划行由 Run 循环在返回后补建（plan_id 归 Run 循环管）。
            if !plan_active {
                plan_active = true;
                self.deps.records.emit(
                    event_type::PLAN_ENTERED,
                    Some(thread_id),
                    Some(run_id),
                    json!({ "partId": part_id, "threadId": thread_id }),
                );
            }
            denied = block_text.to_string();
        }

        // ---------- 计划态硬门卡（仅分级档；对应「写操作执行硬门卡计划模式」）----------
        //
        // 计划模式此前只有提示词约束，模型完全可以「嘴上说在调研，手上改文件」——提示词是
        // 软约束，一旦模型判断偏差就直接落到磁盘。这里把它变成硬拦截：分级档下计划态的写类
        // 工具一律不执行，只回一条带出路的错误结果。
        //
        // 自由档不启用本门卡（2026-11-10 用户裁决：这就是「自由」的含义）：free 在策略引擎里
        // 是全量豁免（`Engine::action` 在受限工作区判定之前就对 free 返回 Allow），本门卡曾是
        // 唯一不认档位的地方。用户经确认门显式接受「放行一切」后，计划态的写操作照常执行，
        // 安全网退化为审计 + 写前备份——与自由档在其他位置的兜底口径一致。
        // 不受影响的边界：exit_plan 的无条件审批走上方独立分支（不看档位、不过本门卡），
        // 自由档下模型想退出计划态照样弹审阅卡（D8 核心边界）。
        //
        // 位置纪律：本门卡必须在下方「档位审批」块之前——fs_write / fs_edit 是 R1，分级档
        // 对它们就是 Ask；若先走档位评估，写操作会先弹审批卡等待用户决定，门卡永远
        // 轮不到执行（实测测试挂死在审批等待上）。先拦后审，被拦的工具不进审批。
        // 自由档不进本门卡，直接落到下方档位判定，引擎返回 Allow（不弹卡、不拦）。
        //
        // 判定用 spec.mutates 而不是风险等级：fs_write / fs_edit / apply_patch 都是 R1，
        // 按「风险 ≥ R2」判定会恰好漏掉最该拦的文件写入。
        //
        // 放行范围：
        // - 协议工具 enter_plan / exit_plan：它们正是「退出计划模式」的申请通道，必须畅通；
        // - 只读工具：调研要用；
        // - todo_write：计划模式下更新待办是正常行为（且其自身有首项校验）；
        // - 子代理协议工具走的是另一个分支，不会到这里；
        // - 自由档：整道门卡不启用（见上）。
        //
        // 与计划软闸门 / 自动进入门的分工（D32）：软闸门管「非计划态已改 2 个文件」的预告
        // 提醒；自动进入门（run_tool 的 auto_plan_block，本函数协议分支链尾）管「非计划态
        // 写第 3 个不同文件」的强制进入——它置 plan_active 后本次调用走 denied 出路，轮不到
        // 本门卡；本门卡管「已在计划态却想写」的拦截。三者不重叠，都保留。
        let plan_gate_on = plan_active && self.deps.engine.lock().unwrap().level() != Level::Free;
        if plan_gate_on
            && plan_overridden.is_none()
            && denied.is_empty()
            && !craft_tools::plan::PLAN_MODE_TOOLS.contains(&call.name.as_str())
            && call.name != "todo_write"
            && self.deps.registry.is_mutating(&call.name)
        {
            denied = format!(
                "当前处于计划模式，不能执行写操作（{}）。先把方案定下来：完成调研后调用 exit_plan \
                 提交完整计划，用户批准后会自动退出计划模式并开始实施；若用户已明确同意直接实施，\
                 也请先调用 exit_plan 把你的计划交给用户确认，不要绕过计划模式动手。",
                call.name
            );
        }

        if plan_overridden.is_none() && denied.is_empty() {
            // denied 已被上方硬门卡置位（分级档下的计划态写操作）时整块跳过：被拦的工具
            // 不再进入档位评估——否则 R1 写操作仍会按 Ask 弹审批等待用户，
            // 「先拦后审」变成「先审后拦」，门卡形同虚设（实测测试挂死在审批等待）。
            // 自由档不进本门卡，所以计划态的写操作在这里照常按档位判定（引擎返回 Allow）。
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
                    if !cached {
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
                        // 子代理的审批卡代理到父会话（决策 D15）：用户只在一个对话里做决定，
                        // 卡片标题标明来源；run_id 保留子 run（审计能追到哪次子代理执行）。
                        let approval_thread = kind
                            .sub()
                            .map(|spec| spec.parent_thread_id.clone())
                            .unwrap_or_else(|| thread_id.to_string());
                        let approval_message = kind
                            .sub()
                            .map(|spec| spec.parent_message_id.clone())
                            .unwrap_or_else(|| message_id.to_string());
                        let display_title = match kind.sub() {
                            Some(spec) => {
                                if title.is_empty() {
                                    format!("子代理 {} 需要你的批准", spec.agent_name)
                                } else {
                                    format!("子代理 {}：{title}", spec.agent_name)
                                }
                            }
                            None => {
                                if title.is_empty() {
                                    "需要你的批准".into()
                                } else {
                                    title.clone()
                                }
                            }
                        };
                        let request = ApprovalRequest {
                            part_id: part_id.clone(),
                            thread_id: approval_thread,
                            run_id: run_id.to_string(),
                            message_id: approval_message,
                            tool_call_id: call.id.clone(),
                            tool: call.name.clone(),
                            risk: assessment.risk.as_str().to_string(),
                            title: display_title,
                            target,
                            detail: if assessment.reason.is_empty() { title.clone() } else { assessment.reason.clone() },
                            facts,
                            args: call.args.clone(),
                            allow_permanent: permanent,
                            grant_target: Some(grant_target),
                        };
                        let outcome = self.deps.approvals.request(&request, token.clone()).await;
                        if outcome.status == "canceled" {
                                return (ToolResult::err_code("审批已取消", "tool_error"), true, false, plan_active, None, Vec::new());
                        }
                        if !outcome.allow {
                            denied = "用户拒绝了该操作".into();
                        }
                    }
                }
                }
            }
        } // if plan_overridden.is_none() 包裹层闭合

        // 执行：协议分支接管时用其预置结果（enter_plan 的薄壳结果 / exit_plan 的批准或反馈桥接），
        // 其余按「拒绝即错误结果、否则进注册表执行」的统一出口。
        let was_denied = !denied.is_empty();
        let mut result = if let Some(overridden) = plan_overridden {
            overridden
        } else if was_denied {
            ToolResult {
                output: denied,
                is_error: true,
                code: "tool_error".into(),
                meta: None,
                media: Vec::new(),
                truncation: None,
            }
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
                return (result, true, false, plan_active, plan_feedback, Vec::new());
            }
            result
        };

        let duration_ms = result.meta.as_ref().and_then(|m| m.get("durationMs")).and_then(Value::as_i64).unwrap_or(0);
        // 工具产生的图片（MCP）：落盘到附件目录 → 挂到 tool 消息 → 事件载荷记文件名。
        // base64 不进事件库（体积会膨胀几十倍），三个消费方都经文件名按需读取：
        // 模型侧由 Attachment 携带 base64，界面侧走 readAttachment 懒加载，
        // 历史重建走 load_attachments 按名恢复。
        let media_names = if result.media.is_empty() {
            Vec::new()
        } else {
            let expected = result.media.len();
            let names = save_tool_media(&self.deps.attachment_dir, &result.media);
            if names.len() != expected {
                self.deps.log.write_thread(
                    thread_id,
                    LEVEL_INFO,
                    "tool_media_save_partial",
                    json!({
                        "tool": call.name,
                        "expected": expected,
                        "saved": names.len(),
                    }),
                );
            }
            // 结果文本必须说明本次带图（模型只读文本部分，图片是另挂的附件）；
            // 落盘失败时按差额逐条占位，不谎报张数。
            result.output = crate::attachments::append_media_notice(&result.output, names.len(), expected);
            names
        };
        // 后台命令任务的关键事实单独提到 payload 顶层（前端工具行与「读取输出」入口要用）：
        // meta 是给内部调用方的结构化载荷，不进事件表；这里只挑渲染层真正需要的几项，
        // 避免把整个 meta（可能含 partialOutput 这类大字段）写进事件库。
        let background_task_id = result.meta.as_ref().and_then(|m| m.get("backgroundTaskId")).and_then(Value::as_str).map(String::from);
        let background_from_timeout = result.meta.as_ref().and_then(|m| m.get("fromTimeout")).and_then(Value::as_bool).unwrap_or(false);
        // 任务状态（bash_output 轮询回报）：渲染层按任务号合并多次查看，并在终态原位切换，
        // 不解析输出正文。只有带任务号的工具结果才可能携带。
        let background_task_state = promote_background_task_state(result.meta.as_ref(), background_task_id.as_deref());
        // 写类工具的行级差异（D21）：同样只挑顶层字段进事件库。
        // 预算已在工具侧（ToolCtx::attach_diff）降到 64KB 内，超限时这里拿到的是统计形态。
        let tool_diff = result.meta.as_ref().and_then(|m| m.get("diff")).cloned();
        let mut payload = json!({
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
            "backgroundTaskId": background_task_id,
            "backgroundFromTimeout": background_from_timeout,
            "attachments": media_names,
        });
        // 缺省不写 null：渲染层以「字段不存在」回退旧行为，且合并时不能覆盖同任务上一条已知状态。
        if let Some(state) = background_task_state {
            payload["backgroundTaskState"] = json!(state);
        }
        if let Some(diff) = tool_diff {
            payload["diff"] = diff;
        }
        self.deps.records.append_or_warn(&EventRecord {
            thread_id: thread_id.to_string(),
            run_id: Some(run_id.to_string()),
            message_id: message_id.to_string(),
            kind: "tool_call".into(),
            payload,
        });

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

        (result, false, false, plan_active, plan_feedback, media_names)
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
    /// plan_id 为 None（本 run 无计划行，如 D32 自动进入路径）时**先建行再写盘**——
    /// 保证「批准产物 = approved=1 且 file_path 非空」在自动进入与手动 enter_plan 两条
    /// 路径上一致；建行失败才放弃落盘（数据目录异常，写盘同样无意义）。
    fn persist_plan_text(&self, thread_id: &str, run_id: &str, plan_id: Option<&str>, text: &str, approved: bool) {
        // 补建计划行（D32）：db.lock_conn 是非重入锁，create_plan 与后续 set_plan_file
        // 拆成两次取锁（锁不得跨语句持有）
        let owned_plan_id: String = match plan_id {
            Some(id) => id.to_string(),
            None => {
                let conn = self.deps.db.lock_conn();
                match craft_store::plans::create_plan(&conn, thread_id, run_id) {
                    Ok(plan) => plan.id,
                    Err(_) => return,
                }
            }
        };
        let dir = match craft_store::plans::plans_dir(&self.deps.db) {
            Ok(dir) => dir,
            Err(_) => return,
        };
        // 文件名：plan_<剥前缀 plan_id>.md
        let plan_id = owned_plan_id.as_str();
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
        // 未读标记：写完成时刻必须先于 set_thread_status——后者广播 thread.updated，
        // 渲染层收到即重拉会话列表，此时 last_done_at 已就位，侧栏才能首次刷新就带出未读点。
        // 子代理不发（它是主会话的一部分，且子会话不进侧栏，投影层同样会挡掉）。
        if self.subagents.snapshot(thread_id).is_none() {
            let _ = self
                .deps
                .records
                .mark_done(thread_id, now_millis());
        }
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
        // 子代理不发桌面通知：它是主会话的一部分，通知只在主会话完成/失败时发一次
        if self.subagents.snapshot(thread_id).is_some() {
            return;
        }
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

/// 从工具 meta 提升后台任务状态（事件载荷用）：只有带任务号的结果才可能携带状态，
/// 避免把无关工具的 `state` 写进事件表。`bash_output` 的 meta 同时带任务号与状态。
fn promote_background_task_state(meta: Option<&Value>, task_id: Option<&str>) -> Option<String> {
    task_id?;
    meta.and_then(|m| m.get("state")).and_then(Value::as_str).map(String::from)
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

    #[test]
    fn background_task_state_promoted_only_with_task_id() {
        let meta = json!({ "backgroundTaskId": "bash_a", "state": "running" });
        // 有任务号：状态进事件载荷（渲染层据此合并等待行并原位切终态）
        assert_eq!(promote_background_task_state(Some(&meta), Some("bash_a")).as_deref(), Some("running"));
        // 没有任务号的普通工具即使 meta 里有 state 也不提升
        assert_eq!(promote_background_task_state(Some(&meta), None), None);
        // meta 缺失时安全返回 None
        assert_eq!(promote_background_task_state(None, Some("bash_a")), None);
    }
}
