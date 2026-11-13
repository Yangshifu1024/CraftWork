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
    /// 计划态切换（纯广播，不入库）：enter_plan 工具执行时广播，前端据此同步 composer 勾选态。
    /// 运行期瞬态——重启后由用户重新勾选，不持久化。
    pub const PLAN_ENTERED: &str = "plan.entered";
    pub const TODO_UPDATED: &str = "todo.updated";
    pub const USER_MESSAGE: &str = "user.message";
    /// 引导（steer）已投递：消费点或收尾丢弃时发，前端据此摘除「待送达」标记；
    /// payload 带 nextMessageId 时表示后续输出切换到新 assistant 消息。
    pub const STEER_DELIVERED: &str = "steer.delivered";
    pub const NOTICE: &str = "notice";
    pub const CONTEXT_COMPRESSED: &str = "context.compressed";
    pub const ERROR: &str = "error";
    /// 错误已解决：新 run 开始时对上一轮失败（error/interrupted）回填，落库供快照
    /// 摘除旧错误部件（重启/切会话不再还原错误卡片），广播供前端实时摘卡片。
    pub const ERROR_RESOLVED: &str = "error.resolved";
    pub const THREAD_UPDATED: &str = "thread.updated";
    pub const USAGE_UPDATED: &str = "usage.updated";
    pub const SETTINGS_UPDATED: &str = "settings.updated";
    pub const WORKSPACE_UPDATED: &str = "workspace.updated";
    /// 子代理状态变化（决策 D15）：派发（running）与终态各广播一次，落库供快照还原卡片。
    /// 载荷见 shared/events.ts 的 SubagentPart。
    pub const SUBAGENT_UPDATED: &str = "subagent.updated";
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
    /// 最近一轮首 token 延迟（毫秒）；0 = 无记录（旧行/未产生用量）。
    pub context_ttft_ms: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// 父会话 id（空串 = 主会话）。决策 D15：非空表示这是一次子代理的子会话，
    /// 视图层据此把它排除在侧栏之外，并允许主会话打开只读的「子代理对话」视图。
    #[serde(default)]
    pub parent_thread_id: String,
    /// 子代理可寻址短名（空串 = 非子代理会话）
    #[serde(default)]
    pub agent_name: String,
    /// 子代理类型（general / explore；空串 = 非子代理会话）
    #[serde(default)]
    pub agent_type: String,
    /// 已完成但用户尚未查看（last_done_at > last_read_at；判定在项目投射时完成）。
    /// 只对主会话有意义：子会话不进侧栏，本字段恒为 false。
    #[serde(default)]
    pub has_unread: bool,
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

// ---------- 子代理配置（决策 D15 扩展） ----------

/// 子代理全局配置（settings 键 `subagent.config`）。
///
/// 为什么不放进 [`Settings`]：Settings 是全量回传的扁平结构（渲染层 saveSettings
/// 把整个对象发回来），加数组字段会让每次改主题都重写子代理定义；子代理配置的
/// 体量与生命周期都不同，独立键可按需单独读写（与 `model.custom_providers`、
/// `skills.disabled` 同一先例）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentSettings {
    /// 总开关：关闭后 `agent_*` 从模型可见工具面消失、调用直接报错
    pub enabled: bool,
    /// 单父会话并发上限（1~8，默认 4）
    pub max_concurrent: i64,
    /// 内置两型的可调默认值（键为 general / explore；缺失时按内置默认）
    pub builtin_profiles: Vec<BuiltinAgentProfile>,
}

impl Default for SubAgentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent: 4,
            builtin_profiles: Vec::new(),
        }
    }
}

/// 内置子代理类型的可调默认值。
///
/// 只允许改模型与迭代上限：工具面与角色提示词是内置类型的安全属性（explore 的
/// 只读约束），不通过设置暴露。要改这两项请用自定义子代理。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinAgentProfile {
    /// general | explore
    pub agent_type: String,
    /// 默认模型引用 `providerID/modelID`；空串 = 继承父会话模型
    #[serde(default)]
    pub model: String,
    /// 默认迭代上限；0 = 使用封顶值（见 [`AGENT_MAX_ITERATIONS_CAP`]），否则钳制 1~封顶值
    #[serde(default)]
    pub max_iterations: i64,
}

/// 自定义子代理的工具面档位。
///
/// 三档预设而非自由勾选：explore 的只读约束是安全属性，让用户随手加 `shell_exec`
/// 会破坏它；`Only` 档留给确需精调的用户，前端按风险等级分组呈现。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CustomAgentTools {
    /// 只读（等同 explore 的只读白名单）
    #[default]
    ReadOnly,
    /// 全部（等同 general 全集减协议工具）
    All,
    /// 指定名单（白名单交集，注册表里不存在的名字忽略）
    Only { names: Vec<String> },
}

/// 用户自定义子代理（settings 键 `subagent.profiles` 的元素）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomAgentProfile {
    /// 服务端生成的 `agent_<16hex>`
    #[serde(default)]
    pub id: String,
    /// 模型可见的 `subagent_type` 值（`^[a-z][a-z0-9_-]{0,31}$`，全局唯一，
    /// 不得与内置 general / explore 冲突）
    pub name: String,
    /// 写进 agent_spawn 的 enum 说明，告诉模型何时用它（≤200 字符）
    #[serde(default)]
    pub description: String,
    /// 追加到子代理提示词的角色段（≤4000 字符）
    #[serde(default)]
    pub role_prompt: String,
    /// 默认模型引用；空串 = 继承父会话模型
    #[serde(default)]
    pub model: String,
    /// 默认迭代上限；0 = 使用封顶值（见 [`AGENT_MAX_ITERATIONS_CAP`]），否则钳制 1~封顶值
    #[serde(default)]
    pub max_iterations: i64,
    #[serde(default)]
    pub tools: CustomAgentTools,
    /// 停用后不出现在 enum 里，但保留定义
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// 自定义子代理的可用性上限（服务层校验与前端即时反馈共用同一份常量）。
pub const CUSTOM_AGENT_NAME_MAX_CHARS: usize = 32;
pub const CUSTOM_AGENT_DESCRIPTION_MAX_CHARS: usize = 200;
pub const CUSTOM_AGENT_ROLE_PROMPT_MAX_CHARS: usize = 4000;
/// 自定义子代理的迭代上限封顶（与内置封顶一致）。封顶是失控兜底，不是预期用量：
/// 正常任务由软上限收尾提醒引导在触及封顶前交回结论（见 craft-agent::subagent）。
pub const AGENT_MAX_ITERATIONS_CAP: i64 = 500;
/// 名称保留字：内置两型不允许被自定义占用。
pub const RESERVED_AGENT_NAMES: [&str; 2] = ["general", "explore"];

/// 子代理配置的存储键（settings 表）。
pub const SUBAGENT_CONFIG_KEY: &str = "subagent.config";
/// 自定义子代理定义的存储键（settings 表）。
pub const SUBAGENT_PROFILES_KEY: &str = "subagent.profiles";

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

/// 新增布尔设置的 serde 兜底：老库 JSON 无该字段时按 true 补齐（否则整体反序列化
/// 失败会回落默认设置，丢失用户已有配置）。
fn default_true() -> bool {
    true
}

/// 出站代理模式默认 system（环境变量 > OS 系统代理 > 直连，D14）。
fn default_proxy_mode() -> String {
    "system".to_string()
}

/// GitHub 加速前缀默认 gh-proxy.com（第三方前缀式加速服务，国内网络建议开启）。
fn default_accelerator_prefix() -> String {
    "https://gh-proxy.com/".to_string()
}

/// 应用设置（settings 表 `app` 键的 JSON 形态）；字段即时生效由壳层保证。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// system | dark | light
    pub theme: String,
    /// code | work
    pub mode: String,
    /// graded | strict | readonly | free
    pub approval_level: String,
    /// 选中的 shell 绝对路径（决策 D18 / D25）；空串 = 不使用，命令由平台默认 shell 执行
    /// 并跑在最小环境里（Unix `/bin/sh -c`、Windows `cmd /C`）。
    /// 非空时命令由该 shell 解释，其启动配置在读取环境时执行一遍（不是每条命令都跑），
    /// 环境经黑名单清洗后交给子进程。
    #[serde(default)]
    pub shell_path: String,
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
    /// 单次运行的模型调用轮数上限；0 = 不限制（默认）。设置页可调 0~1000。
    pub max_iterations: i64,
    /// 运行日志等级：off | error | info | debug（会话日志 <dataDir>/logs 的记录密度）
    #[serde(default)]
    pub logs_level: String,
    /// 流式输出贴底的平滑（弹簧）动画开关；false=即时跟随（直接跳到底）
    #[serde(default)]
    pub smooth_follow: bool,
    /// 输入预测：一轮任务结束后静置 5 秒，在输入框以幽灵文本给出下一句建议（Tab 应用）
    #[serde(default = "default_true")]
    pub input_prediction: bool,
    /// 关闭窗口时退出应用（决策 D20）：默认 false = 隐藏到托盘，run / 后台命令 / MCP 继续
    #[serde(default)]
    pub quit_on_close: bool,
    /// 技能总开关（决策 D11）：关闭后不注入可用技能清单、skill 工具返回错误、斜杠调用不匹配
    #[serde(default = "default_true")]
    pub skills_enabled: bool,
    /// 出站代理模式（决策 D14）：system | none | custom
    #[serde(default = "default_proxy_mode")]
    pub network_proxy_mode: String,
    /// 自定义代理地址（仅 custom 模式生效；保存时归一：无协议头补 http://，可含凭据，明文存本地库）
    #[serde(default)]
    pub network_proxy_url: String,
    /// GitHub 加速总开关：开启后 github.com / raw.githubusercontent.com 的访问经前缀改写加速
    #[serde(default)]
    pub network_github_accelerator_enabled: bool,
    /// 加速前缀（保存时归一补尾斜杠；默认 gh-proxy.com，可换任意同类前缀式服务）
    #[serde(default = "default_accelerator_prefix")]
    pub network_github_accelerator_prefix: String,
    /// 【已迁移，勿写】子代理总开关的历史位置。决策 D15 扩展后子代理配置迁到
    /// settings 的独立键 `subagent.config` / `subagent.profiles`；这两个字段只用于
    /// 首次读取时的兼容回落（旧库升级后不让用户已关掉的子代理重新开启）。
    /// 新代码一律读写 `craft-core::contract::SUBAGENT_CONFIG_KEY`，下个版本删除。
    /// 序列化时保留（不 skip）：渲染层读到的 Settings 形状稳定，避免「字段忽然消失」；
    /// 写回时也不再改动它们（settings_save 不处理这两个键）。
    #[serde(default = "default_true")]
    pub subagents_enabled: bool,
    /// 【已迁移，勿写】子代理并发上限的历史位置（同上，仅兼容回落）。
    #[serde(default = "default_subagent_concurrency")]
    pub subagent_max_concurrent: i64,
}

/// 子代理并发上限默认值（与 craft-agent::subagent::DEFAULT_MAX_CONCURRENT 一致）。
fn default_subagent_concurrency() -> i64 {
    4
}

// ---------- 用量 ----------

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

/// 按会话类型的用量聚合（issue 1 P4）：agent_type 取 threads 行的值——
/// main = 主会话（含悬空 usage 行的兜底归一），general / explore / custom = 子代理类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageByAgentType {
    pub agent_type: String,
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

/// 用量汇总视图（今天/近7天/近30天/按模型/按天/按会话类型，窗口口径见 usage_summary）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub today: TokenTotals,
    pub week: TokenTotals,
    pub month: TokenTotals,
    pub by_model: Vec<UsageByModel>,
    pub by_day: Vec<UsageByDay>,
    /// 按会话类型聚合（issue 1 P4）：判断哪类派发在赚钱。30 天窗口，与 by_model 同口径。
    pub by_agent_type: Vec<UsageByAgentType>,
}

// ---------- 存储占用（设置 → 数据与统计 → 存储，决策 D27） ----------
//
// 口径分两半，界面上不合并成单数：
// - **数据库内**：SQLite 里的东西（含索引页），删了不释放文件除非 VACUUM；
// - **数据目录文件**：库之外的文件（附件、日志、后台输出、工作树、计划）。
// 「各会话占用」只是**归属估算**：events.payload 字节 + 按会话 id 命名的文件体积，
// 不含索引页与行头开销，故不称「独占字节」。

/// 数据目录里的一类文件（体积 + 条数 + 能否清理）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCategoryView {
    /// 稳定标识（清理通道按它派发）：attachments / logs / shellTasks /
    /// worktrees / plans / skillsMarketplace / workspaceDefault / other
    pub kind: String,
    /// 界面展示名（「附件」「写前备份」…）
    pub label: String,
    /// 一句话说明：这一坨是什么、删了会怎样
    pub description: String,
    /// 磁盘占用字节（`truncated` 为真时是下限）
    pub bytes: u64,
    pub files: u64,
    /// 条目数达上限提前截止：界面显示「≥」而不是假精确值
    pub truncated: bool,
    /// 可执行的清理动作（空 = 只读展示）；值即 `storageCleanup(kind)` 的入参
    pub cleanups: Vec<StorageCleanupAction>,
}

/// 一个可执行的清理动作。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupAction {
    /// 动作 id：logsOrphans / logsAll / shellTasksOrphans /
    /// shellTasksAll / attachmentsAll / worktreesOrphans / plansOrphans
    pub kind: String,
    /// 按钮文案（「清理孤儿」「全部清理」…）
    pub label: String,
    /// 是否影响面大、需要二次确认对话框（其余走就近气泡）
    pub destructive: bool,
    /// 确认文案：写明后果与不可逆性
    pub confirm: String,
}

/// 数据库占用视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDatabaseView {
    /// 主库 + WAL + 共享内存索引的字节数（`Db::disk_usage_bytes`）
    pub bytes: u64,
    /// 主库文件本身（不含 WAL/SHM），用于展示分解
    pub main_bytes: u64,
    pub wal_bytes: u64,
    /// 库里全部会话的事件载荷字节合计（近似，见本段注释）
    pub event_bytes: u64,
}

/// 表级体积行（dbstat 明细）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageTableRow {
    pub name: String,
    pub bytes: u64,
}

/// 存储分节首屏：数据库 + 数据目录各类 + 运行门禁。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageOverview {
    pub database: StorageDatabaseView,
    pub categories: Vec<StorageCategoryView>,
    /// 数据目录下各类合计（不含数据库）
    pub files_total_bytes: u64,
    /// 数据目录绝对路径（界面展示 + 「在文件管理器中打开」用）
    pub data_dir: String,
    /// 非空 = 有任务在运行，所有清理动作一律禁用；内容为可读原因
    pub busy_reason: String,
}

/// 单个会话的占用（归属估算，口径见本段注释开头）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageThreadRow {
    pub thread_id: String,
    pub title: String,
    pub mode: String,
    pub archived: bool,
    /// 子代理子会话：不单列，折叠在父会话下
    pub is_subagent: bool,
    /// 数据库内：events.payload 字节（近似）
    pub db_bytes: u64,
    /// 数据目录文件：日志 + 后台输出 + 工作树 + 计划文件
    pub file_bytes: u64,
    pub log_bytes: u64,
    pub shell_task_bytes: u64,
    pub worktree_bytes: u64,
    pub plan_bytes: u64,
    pub events: u64,
    pub runs: u64,
    pub approvals: u64,
    pub updated_at: i64,
}

/// 一个项目（工作区）下全部会话的占用汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageProjectGroup {
    pub workspace_id: String,
    pub workspace_name: String,
    /// 内置临时会话目录（purpose = conversation）：界面标注「临时会话」
    pub is_conversation: bool,
    pub db_bytes: u64,
    pub file_bytes: u64,
    pub thread_count: u64,
    /// 按体积降序；服务端截断到上限，其余计入 `omitted_threads`
    pub threads: Vec<StorageThreadRow>,
    pub omitted_threads: u64,
}

/// 会话占用视图（点开「会话」卡才拉）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageThreadsView {
    /// 按体积降序的项目分组
    pub projects: Vec<StorageProjectGroup>,
    /// 未归属到任何项目的会话（工作区已删除等历史残留）
    pub orphan_threads: Vec<StorageThreadRow>,
    /// 本次统计覆盖的会话总数（含子会话）
    pub thread_total: u64,
}

/// 清理动作的执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCleanupResult {
    pub kind: String,
    /// 实际释放的字节数（删除失败的不计）
    pub freed_bytes: u64,
    pub removed_items: u64,
    /// 被守卫跳过（运行中的会话、不可删的共享日志等）
    pub skipped_items: u64,
    /// 逐条失败摘要（可读中文；残留无害，可重试）
    pub failures: Vec<String>,
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
    /// 用户显示名：git 全局 user.name → 系统用户名 → "CraftWork"（侧栏账户位展示用）。
    pub user_name: String,
    /// 构建期注入的 Git 短哈希（工作区有未提交改动时带 `-dirty`）；
    /// 源码 tarball／未装 git 的构建机为 "unknown"。
    pub git_hash: String,
    /// 构建期注入的编译时间（本地时区，形如 2026-11-08 14:03:21）。
    pub build_time: String,
}

/// 个性化全局指令（<数据目录>/AGENTS.md）：设置「个性化」分节的读取/保存载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalizationInfo {
    /// 文件是否存在（首次使用前为 false，界面展示空编辑器）。
    pub exists: bool,
    /// 文件内容（不存在时为空串）。
    pub content: String,
    /// 字节数（计数与 100KB 上限展示用）。
    pub size: usize,
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
            context_ttft_ms: 0,
            created_at: 0,
            updated_at: 0,
            parent_thread_id: String::new(),
            agent_name: String::new(),
            agent_type: String::new(),
            has_unread: false,
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
            max_iterations: 0,
            logs_level: "info".into(),
            // 默认不使用：保持「最小环境」的既有行为，由用户显式选择后才读其 shell 配置
            shell_path: String::new(),
            smooth_follow: false,
            input_prediction: true,
            quit_on_close: false,
            skills_enabled: true,
            network_proxy_mode: "system".into(),
            network_proxy_url: String::new(),
            network_github_accelerator_enabled: false,
            network_github_accelerator_prefix: "https://gh-proxy.com/".into(),
            subagents_enabled: true,
            subagent_max_concurrent: 4,
        };
        let totals = TokenTotals { input_tokens: 0, output_tokens: 0, cost_micros: 0 };
        let usage = UsageSummary {
            today: totals.clone(),
            week: totals.clone(),
            month: totals.clone(),
            by_model: vec![UsageByModel { model_id: "m".into(), input_tokens: 0, output_tokens: 0, cost_micros: 0 }],
            by_day: vec![UsageByDay { day: "d".into(), input_tokens: 0, output_tokens: 0, cost_micros: 0 }],
            by_agent_type: vec![UsageByAgentType { agent_type: "main".into(), input_tokens: 0, output_tokens: 0, cost_micros: 0 }],
        };
        let line = DiffLine { kind: "ctx".into(), old: 0, r#new: 0, text: "t".into() };
        let diff = DiffResult { additions: 0, deletions: 0, hunks: vec![DiffHunk { header: "h".into(), lines: vec![line] }], truncated: false };
        let info = AppInfo { name: "n".into(), version: "v".into(), platform: "p".into(), arch: "a".into(), data_dir: "/d".into(), user_name: "u".into(), git_hash: "g".into(), build_time: "b".into() };
        let workspace = WorkspaceView { id: "w".into(), name: "n".into(), path: "/p".into(), trust: "trusted".into(), purpose: "project".into(), last_opened_at: 0, folders: vec![folder] };

        // 存储分节（D27）：四层结构逐个过金样，任何一层漂成 snake_case 都即红
        let storage_category = StorageCategoryView {
            kind: "logs".into(),
            label: "会话日志".into(),
            description: "d".into(),
            bytes: 1,
            files: 1,
            truncated: false,
            cleanups: vec![StorageCleanupAction {
                kind: "logsOrphans".into(),
                label: "清理孤儿".into(),
                destructive: false,
                confirm: "c".into(),
            }],
        };
        let storage_overview_view = StorageOverview {
            database: StorageDatabaseView { bytes: 1, main_bytes: 1, wal_bytes: 0, event_bytes: 1 },
            categories: vec![storage_category],
            files_total_bytes: 1,
            data_dir: "/d".into(),
            busy_reason: String::new(),
        };
        let storage_thread = StorageThreadRow {
            thread_id: "th_x".into(),
            title: "t".into(),
            mode: "code".into(),
            archived: false,
            is_subagent: false,
            db_bytes: 1,
            file_bytes: 1,
            log_bytes: 1,
            shell_task_bytes: 0,
            worktree_bytes: 0,
            plan_bytes: 0,
            events: 1,
            runs: 1,
            approvals: 0,
            updated_at: 0,
        };
        let storage_threads_view = StorageThreadsView {
            projects: vec![StorageProjectGroup {
                workspace_id: "w".into(),
                workspace_name: "n".into(),
                is_conversation: false,
                db_bytes: 1,
                file_bytes: 1,
                thread_count: 1,
                threads: vec![storage_thread.clone()],
                omitted_threads: 0,
            }],
            orphan_threads: vec![storage_thread],
            thread_total: 1,
        };
        let storage_cleanup_result = StorageCleanupResult {
            kind: "logsOrphans".into(),
            freed_bytes: 1,
            removed_items: 1,
            skipped_items: 0,
            failures: Vec::new(),
        };

        let samples: Vec<(&str, Value)> = vec![
            ("ThreadView", serde_json::to_value(sample_thread()).unwrap()),
            ("WorkspaceView", serde_json::to_value(workspace).unwrap()),
            ("Snapshot", serde_json::to_value(snapshot).unwrap()),
            ("Settings", serde_json::to_value(settings).unwrap()),
            ("UsageSummary", serde_json::to_value(usage).unwrap()),
            ("DiffResult", serde_json::to_value(diff).unwrap()),
            ("AppInfo", serde_json::to_value(info).unwrap()),
            ("StorageOverview", serde_json::to_value(storage_overview_view).unwrap()),
            ("StorageThreadsView", serde_json::to_value(storage_threads_view).unwrap()),
            ("StorageTableRow", serde_json::to_value(StorageTableRow { name: "events".into(), bytes: 1 }).unwrap()),
            ("StorageCleanupResult", serde_json::to_value(storage_cleanup_result).unwrap()),
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
