//! 服务层：统一 invoke 分发（与 Electron 版 ipc.ts + services/* 等价）。
//! 只做参数校验、编排与错误转换；领域逻辑全部在 crates/craft-*。
//! 各服务函数由 lib.rs 的 dispatch 按方法名接线（签名对应 channels.ts 声明）；
//! Err 为可读中文字符串，经 invoke 拒绝直达渲染层 toast。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use craft_agent::approvals::Approvals;
use craft_agent::runner::Runner;
use craft_core::contract::{AppInfo, Settings};
use craft_core::editor;
use craft_core::platform::{is_binary, sha256_hex, write_file_atomic};
use craft_core::secret::SecretStore;
use craft_model::catalog::{catalog_model_to_persisted, catalog_provider_env, Catalog};
use craft_model::factory::ModelFactory;
use craft_model::types::{validate_headers, HeaderPair, PersistedProvider};
use craft_policy::engine::Engine;
use craft_policy::resolve::resolve_existing;
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_store::records::{get_setting, set_setting, usage_summary};
use craft_store::threads;
use craft_store::workspaces;
use craft_tools::registry::Registry;

/// 应用名与版本（getInfo 返回值）。版本直接取 Cargo 工作区版本（`Cargo.toml` 的
/// `workspace.package.version`）：此前在此硬编码常量，与 `src-tauri/tauri.conf.json`
/// 双源维护并已发生漂移（代码侧 0.1.0 / 打包侧 0.1.1，关于页会显示错版本）。
/// 发布产物版本仍以 tauri.conf.json 为事实源，发版时两处同步。
pub const APP_NAME: &str = "CraftWork";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// 构建期注入的 Git 短哈希与编译时间（见 src-tauri/build.rs）；未经构建脚本注入时为 unknown。
pub const GIT_HASH: &str = match option_env!("CRAFTWORK_GIT_HASH") {
    Some(value) => value,
    None => "unknown",
};
pub const BUILD_TIME: &str = match option_env!("CRAFTWORK_BUILD_TIME") {
    Some(value) => value,
    None => "unknown",
};
/// 用户提供商在 settings 中的存储键
const CUSTOM_PROVIDERS_KEY: &str = "model.custom_providers";

/// 全部共享服务（tauri State 管理）。
#[allow(dead_code)] // engine/registry 供审批档位热切换与 MCP 动态注册使用
pub struct AppState {
    pub db: Arc<Db>,
    pub secrets: Arc<SecretStore>,
    pub engine: Arc<Mutex<Engine>>,
    pub catalog: Arc<Catalog>,
    pub models: Arc<ModelFactory>,
    pub registry: Arc<Registry>,
    pub runner: Arc<Runner>,
    pub approvals: Arc<Approvals>,
    /// 事件写入与广播的唯一出口：落库事件必须经它广播到渲染层
    /// （直接调 record_event 只落库，界面收不到事件——用户消息曾因此不显示）。
    pub recorder: Recorder,
    pub data_dir: String,
    pub attachment_dir: String,
    /// 运行日志等级缓存（设置保存侧更新，会话日志写入侧只读；与 Runner 的 SessionLog 共享）
    pub log_level: Arc<std::sync::atomic::AtomicU8>,
    /// 共享会话日志：Runner / Bus / 托盘状态线程写同一份 app.log（托盘失败与状态查询失败
    /// 需要一个不依赖 Runner 的日志出口）。
    pub session_log: Arc<craft_agent::session_log::SessionLog>,
    /// 后台命令任务注册表（与 Registry / Runner 共享同一份；通道层直接查它做只读展示）
    pub shell_tasks: Arc<craft_tools::shell_tasks::ShellTasks>,
    /// MCP 服务器池（与 Registry / Runner 共享同一份；服务层做配置持久化与状态聚合，
    /// McpTool 经它调用工具）
    pub mcp_pool: Arc<craft_mcp::McpPool>,
}

/// 参数按位置取值（渲染层桥按 TS 服务签名传位置数组）。
pub struct Args<'a> {
    values: &'a [Value],
}

impl<'a> Args<'a> {
    pub fn new(values: &'a [Value]) -> Self {
        Self { values }
    }

    pub fn str_at(&self, i: usize) -> String {
        self.values.get(i).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    pub fn value_at(&self, i: usize) -> Value {
        self.values.get(i).cloned().unwrap_or(Value::Null)
    }

    pub fn bool_at(&self, i: usize) -> bool {
        self.values.get(i).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn i64_at(&self, i: usize) -> i64 {
        self.values.get(i).and_then(Value::as_i64).unwrap_or(0)
    }
}

/// 服务返回值：Ok 为 JSON 载荷，Err 为可读中文错误（渲染层 toast 直接展示）。
pub type SvcResult = Result<Value, String>;

fn err<T>(msg: impl Into<String>) -> Result<T, String> {
    Err(msg.into())
}

// ---------- 应用 ----------

pub fn get_info(state: &AppState) -> SvcResult {
    let info = AppInfo {
        name: APP_NAME.into(),
        version: APP_VERSION.into(),
        platform: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        data_dir: state.data_dir.clone(),
        // 显示名解析随 D13 迁移落到 craft-git（git 全局 user.name 经 gix 读取，
        // 回退链 USER → CraftWork 不变；craft-core 不依赖 craft-git，避免成环）。
        user_name: craft_git::user_display_name(),
        git_hash: GIT_HASH.into(),
        build_time: BUILD_TIME.into(),
    };
    serde_json::to_value(info).map_err(|e| e.to_string())
}

// ---------- 设置 ----------

use craft_core::contract::PersonalizationInfo;

/// 个性化全局指令上限：100KB（字节）。超出拒绝保存，前端同步展示计数。
const PERSONALIZATION_MAX_BYTES: usize = 100 * 1024;

/// 个性化全局指令文件路径（<数据目录>/AGENTS.md）。
fn personalization_path(data_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(data_dir).join("AGENTS.md")
}

/// 读取个性化全局指令；文件缺失/不可读时返回 exists=false 与空内容（不视为错误）。
pub fn personalization_get(state: &AppState) -> SvcResult {
    let path = personalization_path(&state.data_dir);
    let (exists, content) = match std::fs::read(&path) {
        Ok(bytes) => (true, String::from_utf8_lossy(&bytes).into_owned()),
        Err(_) => (false, String::new()),
    };
    let info = PersonalizationInfo {
        exists,
        size: content.len(),
        content,
    };
    serde_json::to_value(info).map_err(|e| e.to_string())
}

/// 保存个性化全局指令：原子覆盖写入；空内容删除文件（清空 = 无全局指令）。
/// 上限 100KB 硬校验；目录随数据目录启动时已创建，create_dir_all 兜底首启竞态。
pub fn personalization_save(state: &AppState, content: &str) -> SvcResult {
    let bytes_len = content.len();
    if bytes_len > PERSONALIZATION_MAX_BYTES {
        return Err(format!(
            "个性化指令超出上限：{bytes_len} 字节（上限 100KB），请精简后保存"
        ));
    }
    let path = personalization_path(&state.data_dir);
    if content.trim().is_empty() {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删除个性化指令文件失败：{e}"))?;
        }
    } else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败：{e}"))?;
        }
        craft_core::platform::write_file_atomic_str(&path, content, 0o644)
            .map_err(|e| format!("保存个性化指令失败：{e}"))?;
    }
    personalization_get(state)
}

/// 默认设置：无持久化设置或反序列化失败时的整体兜底（不做字段级部分合并）。
pub fn default_settings() -> Settings {
    Settings {
        theme: "system".into(),
        mode: "code".into(),
        approval_level: "graded".into(),
        default_model: String::new(),
        reasoning_level: String::new(),
        fallback_model: String::new(),
        active_workspace: String::new(),
        language: "zh-CN".into(),
        font_size: 15,
        code_theme: "dark".into(),
        auto_update: true,
        notifications: true,
        update_channel: "stable".into(),
        telemetry: false,
        max_iterations: 0,
        logs_level: "info".into(),
        // 空串 = 不使用（跑最小环境）。全新数据目录的实际默认值由 settings_get 填成
        // 登录 shell，这里保持空串以便测试与「解析失败」兜底不依赖进程环境。
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
    }
}

// ---------- 计划 ----------

/// 输入预测缓存读取（`crates/craft-agent/src/prediction.rs` 模块头）：run 完成后端侧预计算的
/// 下一句建议，瞬态内存值（不入库），无则 null。
pub fn input_prediction_get(state: &AppState, thread_id: &str) -> SvcResult {
    Ok(match state.runner.prediction(thread_id) {
        Some(text) => json!(text),
        None => json!(null),
    })
}

/// 持久化设置（无 app 设置行时为 `None`）。区分「从未保存过」与「保存了空串」是
/// 默认登录 shell 的前提：前者可以给默认值，后者是用户显式选的「不使用」。
fn saved_settings(db: &Db) -> Option<Settings> {
    db.with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
}

/// 全新数据目录的默认登录 shell：`$SHELL` 指向、且出现在候选清单里的那一项。
///
/// 走 `detect_shells` 而不是直接读 `$SHELL`：候选路径的写法与设置页下拉一致（含
/// realpath 去重与 `（登录 Shell）` 标记），避免默认值在 UI 里匹配不到选项而显示空白。
pub fn default_login_shell() -> String {
    craft_core::shellenv::detect_shells()
        .into_iter()
        .find(|s| s.current)
        .map(|s| s.path)
        .unwrap_or_default()
}

/// 当前应生效的 shell 路径：有持久化设置时尊重存值（空串 = 不使用）；全新数据目录
/// （从未保存过设置）默认登录 shell。启动恢复与设置读取共用这一份口径。
pub fn effective_shell_path(db: &Db) -> String {
    match saved_settings(db) {
        Some(saved) => saved.shell_path,
        None => default_login_shell(),
    }
}

pub fn settings_get(state: &AppState) -> SvcResult {
    let mut merged = match saved_settings(&state.db) {
        Some(saved) => saved,
        // 全新数据目录默认登录 shell（D18 修订）：用户不必先做一次选择才能看到自己的
        // 工具链。保存过任何设置的用户（含显式选择「不使用」）保持原值。
        None => {
            let mut defaults = default_settings();
            defaults.shell_path = default_login_shell();
            defaults
        }
    };
    // 历史档位（strict/readonly，已移除）与未知值回落分级档，保证前端拿到的档位恒合法
    merged.approval_level = craft_policy::parse_level(&merged.approval_level).as_str().into();
    // 历史库可能存了空日志等级（select 无匹配显示空白）：回落标准档
    if merged.logs_level.is_empty() {
        merged.logs_level = "info".into();
    }
    serde_json::to_value(merged).map_err(|e| e.to_string())
}

/// 保存设置：白名单字段逐项校验合并（越界值回落默认），返回合并后的完整设置。
pub fn settings_save(state: &AppState, next: &Value) -> SvcResult {
    // 白名单字段校验合并（services/settings.ts 校验规则直译）
    let mut merged: Settings = serde_json::from_value(settings_get(state)?).unwrap_or_else(|_| default_settings());
    if let Some(theme) = next.get("theme").and_then(Value::as_str).filter(|t| ["system", "dark", "light"].contains(t)) {
        merged.theme = theme.into();
    }
    if let Some(level) = next.get("reasoningLevel").and_then(Value::as_str) {
        merged.reasoning_level = level.into();
    }
    if let Some(mode) = next.get("mode").and_then(Value::as_str).filter(|m| ["code", "work"].contains(m)) {
        merged.mode = mode.into();
    }
    if let Some(level) = next.get("approvalLevel").and_then(Value::as_str) {
        merged.approval_level = craft_policy::parse_level(level).as_str().into();
    }
    if let Some(size) = next.get("fontSize").and_then(Value::as_i64) {
        merged.font_size = if (11..=18).contains(&size) { size } else { 15 };
    }
    if let Some(v) = next.get("language").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        merged.language = v.into();
    }
    if let Some(v) = next.get("codeTheme").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        merged.code_theme = v.into();
    }
    if let Some(v) = next.get("updateChannel").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        merged.update_channel = v.into();
    }
    for key in [
        "autoUpdate",
        "notifications",
        "telemetry",
        "smoothFollow",
        "inputPrediction",
        "quitOnClose",
        "skillsEnabled",
        "networkGithubAcceleratorEnabled",
    ] {
        if let Some(v) = next.get(key).and_then(Value::as_bool) {
            match key {
                "autoUpdate" => merged.auto_update = v,
                "notifications" => merged.notifications = v,
                "smoothFollow" => merged.smooth_follow = v,
                "inputPrediction" => merged.input_prediction = v,
                "quitOnClose" => merged.quit_on_close = v,
                "skillsEnabled" => merged.skills_enabled = v,
                "networkGithubAcceleratorEnabled" => merged.network_github_accelerator_enabled = v,
                _ => merged.telemetry = v,
            }
        }
    }
    if let Some(iterations) = next.get("maxIterations").and_then(Value::as_i64) {
        // 0 = 不限制（默认）；越界回落默认值 0，而不是回落一个隐形上限
        merged.max_iterations = if (0..=1000).contains(&iterations) { iterations } else { 0 };
    }
    if let Some(level) = next.get("logsLevel").and_then(Value::as_str) {
        merged.logs_level = level.into();
    }
    // Shell 环境（D18）：必须是实际存在的绝对路径，且出现在检测结果里才接受；
    // 空串 = 不使用。非法值保持现值而不是静默改成空（避免用户存了个错路径就被清零）。
    if let Some(v) = next.get("shellPath").and_then(Value::as_str) {
        let v = v.trim();
        if v.is_empty() {
            merged.shell_path = String::new();
        } else if std::path::Path::new(v).is_absolute() && std::path::Path::new(v).is_file() {
            merged.shell_path = v.to_string();
        }
    }
    // 网络（决策 D14）：模式白名单过滤；代理地址保存时
    // 归一校验（无协议头补 http://、仅 http/https/socks5/socks5h），非法值保持现值；
    // 空代理地址允许存（切回其他模式时清空）；加速前缀归一补尾斜杠。
    // 白名单键与契约 serde camelCase 键一致（渲染层 saveSettings 全量回传 settingsGet 的 JSON）。
    if let Some(v) = next
        .get("networkProxyMode")
        .and_then(Value::as_str)
        .filter(|v| ["system", "none", "custom"].contains(v))
    {
        merged.network_proxy_mode = v.into();
    }
    if let Some(v) = next.get("networkProxyUrl").and_then(Value::as_str) {
        if v.trim().is_empty() {
            merged.network_proxy_url = String::new();
        } else if let Ok(normalized) = craft_core::network::normalize_proxy_url(v) {
            merged.network_proxy_url = normalized;
        }
        // 非法代理地址：保持现值（解析侧 custom 模式同样会拒绝）
    }
    if let Some(v) = next.get("networkGithubAcceleratorPrefix").and_then(Value::as_str) {
        merged.network_github_accelerator_prefix =
            craft_core::network::normalize_accelerator_prefix(v);
    }
    // 子代理（决策 D15）：开关即时生效（关闭后下一次 run 的模型请求里就没有 agent_* 工具）；
    // 并发上限钳制到 1~8，越界回落默认 4。
    if let Some(v) = next.get("subagentsEnabled").and_then(Value::as_bool) {
        merged.subagents_enabled = v;
    }
    if let Some(v) = next.get("subagentMaxConcurrent").and_then(Value::as_i64) {
        merged.subagent_max_concurrent = v.clamp(1, 8);
    }
    if let Some(v) = next.get("defaultModel").and_then(Value::as_str) {
        merged.default_model = v.into();
    }
    if let Some(v) = next.get("fallbackModel").and_then(Value::as_str) {
        merged.fallback_model = v.into();
    }
    if let Some(v) = next.get("activeWorkspace").and_then(Value::as_str) {
        merged.active_workspace = v.into();
    }
    state
        .db
        .with(|conn| set_setting(conn, "app", &serde_json::to_value(&merged).unwrap_or(Value::Null)))
        .map_err(|e| e.to_string())?;
    // 档位热切换：引擎仅在启动时按设置初始化，保存必须同步生效（否则切换档位要重启才起作用）
    state.engine.lock().unwrap().set_level(craft_policy::parse_level(&merged.approval_level));
    // 运行日志等级热切换：写每行日志前读缓存，保存即生效（Runner 的会话日志即时跟随）
    state
        .log_level
        .store(craft_agent::session_log::parse_level(&merged.logs_level), std::sync::atomic::Ordering::Relaxed);
    // Shell 环境热切换（D18）：改选后立即重新捕获，下一条命令即生效。
    // 捕获失败不阻断保存（存进去的值仍合法），但要清缓存并回报——否则用户会以为切换成功了。
    // 失败同时进 app.log 与 settingsGet 之外的通道：这里返回给 save 的调用方拿不到，
    // 故由 shell_env_refresh（设置页选中后立刻调）把原因展示出来。
    apply_shell_env(&merged.shell_path, Some(&state.session_log));
    settings_get(state)
}

/// 应用（或清空）当前生效的 shell 环境。返回捕获失败原因（成功或未启用时为 None）。
///
/// 失败时清空缓存：宁可回落到最小环境让命令「找不到工具」，也不要静默沿用上一个
/// shell 的环境——那会让「我明明切了 shell」变成无法排查的错觉。
/// 失败原因同时写 app.log：之前只有 `eprintln!`，而 GUI 应用（双击启动）的 stderr
/// 无处可看，等于丢弃——「选了 shell 但工具还是找不到」就失去了唯一的线索。
pub fn apply_shell_env(shell_path: &str, log: Option<&craft_agent::session_log::SessionLog>) -> Option<String> {
    if shell_path.is_empty() {
        craft_core::shellenv::clear();
        return None;
    }
    let home = craft_core::platform::home_dir_string();
    match craft_core::shellenv::configure(shell_path, &home) {
        Ok(()) => None,
        Err(err) => {
            craft_core::shellenv::clear();
            if let Some(log) = log {
                log.write_app(
                    craft_agent::session_log::LEVEL_ERROR,
                    "shell_env_capture_failed",
                    json!({ "shellPath": shell_path, "error": err }),
                );
            } else {
                eprintln!("craftwork: shell 环境捕获失败，已回落最小环境：{err}");
            }
            Some(err)
        }
    }
}

/// 可选 shell 清单（设置页下拉）：路径 / 家族 / 是否当前登录 shell / 它会读哪些配置。
///
/// 配置文件按真实存在过滤——设置页要展示「选了它会发生什么」，列一堆不存在的路径
/// 等于误导。
pub fn shells_list() -> SvcResult {
    let home = craft_core::platform::home_dir_string();
    let shells: Vec<Value> = craft_core::shellenv::detect_shells()
        .into_iter()
        .map(|s| {
            let configs = craft_core::shellenv::config_files(s.kind, &home);
            json!({
                "path": s.path,
                "kind": s.kind.as_str(),
                "current": s.current,
                "configs": configs,
            })
        })
        .collect();
    Ok(json!(shells))
}

/// 重新捕获当前选中 shell 的环境（设置页「重新读取」；缓存过期后也靠它手动触发）。
///
/// 返回读到的变量数与环境概况，**不回传变量值**——值与密钥同类，不进渲染层。
/// 捕获失败时 `error` 带原因：这正是「选了下拉却什么都没发生」的唯一可见线索。
pub fn shell_env_refresh(state: &AppState) -> SvcResult {
    let settings: Value = settings_get(state)?;
    let shell_path = settings.get("shellPath").and_then(Value::as_str).unwrap_or_default();
    if shell_path.is_empty() {
        craft_core::shellenv::clear();
        return Ok(json!({ "shellPath": "", "configs": [], "varCount": 0, "error": null }));
    }
    // 失败原因直接来自这次捕获，不再二次跑一遍 shell（启动配置文件有副作用，不该执行两遍）
    if let Some(err) = apply_shell_env(shell_path, Some(&state.session_log)) {
        return Ok(json!({
            "shellPath": shell_path,
            "configs": [],
            "varCount": 0,
            "error": err,
        }));
    }
    match craft_core::shellenv::current_meta() {
        Some(meta) => Ok(json!({
            "shellPath": meta.shell_path,
            "configs": meta.configs,
            "varCount": meta.vars.len(),
            "error": null,
        })),
        None => Ok(json!({
            "shellPath": shell_path,
            "configs": [],
            "varCount": 0,
            "error": "环境已过期且未能重新读取，请点「重新读取」",
        })),
    }
}

// ---------- 会话 ----------

pub fn thread_list(state: &AppState, mode: &str, include_archived: bool) -> SvcResult {
    let rows = threads::list_threads(&state.db.lock_conn(), mode, include_archived).map_err(|e| e.to_string())?;
    let mut views = Vec::with_capacity(rows.len());
    for row in &rows {
        views.push(thread_view_to_json(state, row)?);
    }
    Ok(json!(views))
}

fn thread_view_to_json(state: &AppState, row: &threads::ThreadRow) -> Result<Value, String> {
    let view = craft_store::view::to_thread_view(&state.db.lock_conn(), row).map_err(|e| e.to_string())?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

pub fn thread_search(state: &AppState, mode: &str, query: &str) -> SvcResult {
    let rows = threads::search_threads(&state.db.lock_conn(), mode, query).map_err(|e| e.to_string())?;
    let mut views = Vec::with_capacity(rows.len());
    for row in &rows {
        views.push(thread_view_to_json(state, row)?);
    }
    Ok(json!(views))
}

pub fn thread_create(state: &AppState, mode: &str, workspace_id: &str, title: &str) -> SvcResult {
    let id = craft_core::ids::new_id(craft_core::ids::id_prefix::THREAD);
    let row = threads::create_thread(
        &state.db.lock_conn(),
        threads::NewThread {
            id: &id,
            mode,
            title: if title.is_empty() { "新对话" } else { title },
            title_auto: title.is_empty(),
            model_id: "",
            reasoning_level: "",
            workspace_id,
            status: "idle",
            archived: false,
        },
    )
    .map_err(|e| e.to_string())?;
    thread_view_to_json(state, &row)
}

pub fn thread_rename(state: &AppState, id: &str, title: &str) -> SvcResult {
    threads::rename_thread(&state.db.lock_conn(), id, title).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 标记会话已读：走 Recorder 以带广播（thread.updated），其它窗口/重建后的列表同步清未读点。
pub fn thread_mark_read(state: &AppState, id: &str) -> SvcResult {
    state.recorder.mark_read(id).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn thread_set_model(state: &AppState, id: &str, model_ref: &str) -> SvcResult {
    // 规范化：可解析则存规范 ref（与 TS「存规范引用」一致）
    let normalized = state
        .catalog
        .normalize(model_ref)
        .map(|n| n.reference)
        .unwrap_or_else(|| model_ref.to_string());
    threads::set_thread_model(&state.db.lock_conn(), id, &normalized).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 会话级思考档位（与模型选择同构：随会话记忆，下一条消息生效）。
/// 档位合法性由渲染层按模型元数据约束；空串=清除（不发 reasoning 参数）。
pub fn thread_set_reasoning(state: &AppState, id: &str, level: &str) -> SvcResult {
    threads::set_thread_reasoning(&state.db.lock_conn(), id, level).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn thread_set_archived(state: &AppState, id: &str, archived: bool) -> SvcResult {
    threads::set_thread_archived(&state.db.lock_conn(), id, archived).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 彻底删除会话：事务内物理移除事件、审批、运行与线程行，不可恢复。
/// 当前对运行中/等待审批的会话无删除守卫（GLOSSARY 已知限制）。
pub fn thread_delete(state: &AppState, id: &str) -> SvcResult {
    // 删除前快照绑定信息（删行后无法追溯）；工作树清理 best-effort，不阻塞删除
    let thread = threads::get_thread(&state.db.lock_conn(), id).map_err(|e| e.to_string())?;
    delete_thread_fully(state, &thread);
    Ok(json!(null))
}

/// 删除会话的全部痕迹：子代理收尾 → 库内行 → 工作树 → 按会话 id 命名的文件。
///
/// 单一入口：`thread_delete` 与 `thread_clear_temp` 都走它。为什么要收敛成一处——
/// 同一动作存在两条实现路径时，「删了会话但日志还在」这种不一致必然漂移，而存储页
/// 恰恰要把这些残留报成「孤儿」；删干净后它们根本不会出现在孤儿清单里。
///
/// 文件清理（日志 / 后台输出 / 计划正文）是 best-effort：失败写会话日志、不阻塞删除。
/// 计划文件必须在删行**之前**读出来——`plans` 行一旦删掉就无从知道哪些文件属于它。
fn delete_thread_fully(state: &AppState, thread: &threads::ThreadRow) {
    let plan_paths = state
        .db
        .with(|conn| craft_store::storage::thread_plan_paths(conn, &thread.id))
        .unwrap_or_default();
    // 子代理收尾（决策 D15）：先取消在跑的子代理、丢弃待投递报告与登记，再删行——
    // 否则子代理会在父会话消失后继续烧 token，或把报告投给一个不存在的会话。
    state.runner.subagents().cancel_all_under(&thread.id);
    state.runner.subagents().drop_reports(&thread.id);
    state.runner.subagents().forget_under(&thread.id);
    if let Err(err) = threads::delete_thread(&state.db, &thread.id) {
        state.session_log.write_app(
            craft_agent::session_log::LEVEL_ERROR,
            "thread_delete_failed",
            json!({ "threadId": thread.id, "error": err.to_string() }),
        );
        return;
    }
    cleanup_thread_worktree(state, thread);
    let failures = crate::storage::remove_thread_files(&state.data_dir, &thread.id, &plan_paths);
    if !failures.is_empty() {
        state.session_log.write_app(
            craft_agent::session_log::LEVEL_ERROR,
            "thread_delete_leftover_files",
            json!({ "threadId": thread.id, "failures": failures }),
        );
    }
}

/// 一键清理临时会话：删除内置 conversation 工作区下指定模式、未归档的全部线程
/// （与侧栏「临时会话」分组可见列表一致）。运行中/等待审批的会话跳过，避免与
/// 进行中的 run 互相踩踏；返回 { removed, skipped } 供前端提示。临时会话禁止绑定
/// worktree（ensure_thread_worktree 守卫），无需级联清理工作树。
///
/// 删除动作走 [`delete_thread_fully`]（与单条删除同一入口）：会话的文件残留（日志、
/// 后台输出、计划正文）一并清掉，否则存储页会把它们报成孤儿。
pub fn thread_clear_temp(state: &AppState, mode: &str) -> SvcResult {
    let ws_id = state
        .db
        .with(|conn| Ok(workspaces::default_conversation_workspace_id(conn)))
        .map_err(|e| e.to_string())?;
    let (mut removed, mut skipped) = (0u32, 0u32);
    if !ws_id.is_empty() {
        let targets: Vec<threads::ThreadRow> = {
            let conn = state.db.lock_conn();
            threads::list_threads_by_workspace(&conn, &ws_id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|t| t.mode == mode && !t.archived)
                .collect()
        };
        for thread in &targets {
            if thread.status == "running" || thread.status == "awaiting" {
                skipped += 1;
                continue;
            }
            delete_thread_fully(state, thread);
            removed += 1;
        }
    }
    Ok(json!({ "removed": removed, "skipped": skipped }))
}

/// 会话快照：读事件按 message_id 归组重建消息流（时间线装载的唯一入口）。
/// limit=0 不限条数；before_seq>0 只保留 seq 小于游标的事件（向上翻页）。
/// 两段式：锁内只拉原始行，JSON 解析与消息归组在锁外——切长会话不阻塞其他会话的写路径。
pub fn thread_snapshot(state: &AppState, id: &str, limit: i64, before_seq: i64) -> SvcResult {
    let inputs = {
        let conn = state.db.lock_conn();
        craft_store::events::snapshot_inputs(&conn, id).map_err(|e| e.to_string())?
    };
    let mut snap = craft_store::events::rebuild_snapshot(inputs, limit.max(0) as usize, before_seq);
    apply_tool_titles(&state.registry, &mut snap);
    serde_json::to_value(snap).map_err(|e| e.to_string())
}

/// 工具部件的显示名归一：把 `title` 覆写为注册表里该工具的当前人类可读标题。
///
/// 为什么在读取侧做而不是只在写入侧做：`title` 随事件落库，早期会话（标题补齐之前）
/// 与当时回退成工具名的记录都存着 `fs_read` 这类内部名，只改写入侧救不回旧历史。
/// 注册表里查不到的（MCP 动态工具、已移除的工具）保持原载荷，不做推测性替换。
fn apply_tool_titles(registry: &Registry, snap: &mut craft_core::contract::Snapshot) {
    for message in &mut snap.messages {
        for part in &mut message.parts {
            if part.kind != "tool" {
                continue;
            }
            let Some(name) = part.payload.get("name").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let Some(title) = registry.spec_for(&name).map(|s| s.title).filter(|t| !t.is_empty()) else {
                continue;
            };
            if let Some(obj) = part.payload.as_object_mut() {
                obj.insert("title".into(), Value::String(title));
            }
        }
    }
}

// ---------- 工作区 ----------

/// 内置临时会话目录：purpose=conversation 不存在时创建（启动 ensure 语义）。
pub fn ensure_conversation_workspace(state: &AppState) -> SvcResult {
    let existing = state.db.with(|conn| Ok(workspaces::default_conversation_workspace_id(conn))).map_err(|e| e.to_string())?;
    if !existing.is_empty() {
        return Ok(json!(existing));
    }
    let dir = PathBuf::from(&state.data_dir).join("workspace").join("default");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ws = workspaces::upsert_workspace(
        &state.db.lock_conn(),
        "临时会话",
        &dir.to_string_lossy(),
        "trusted",
        "conversation",
    )
    .map_err(|e| e.to_string())?;
    Ok(json!(ws.id))
}

pub fn workspace_list(state: &AppState) -> SvcResult {
    let list = workspaces::list_workspaces(&state.db.lock_conn()).map_err(|e| e.to_string())?;
    let views: Vec<Value> = list.iter().map(workspace_view_json).collect();
    Ok(json!(views))
}

/// 工作区视图（与 workspace_list 同构的 json 形状，camelCase 对齐 WorkspaceView）。
fn workspace_view_json(ws: &workspaces::WorkspaceWithFolders) -> Value {
    json!({
        "id": ws.workspace.id, "name": ws.workspace.name, "path": ws.workspace.path,
        "trust": ws.workspace.trust, "purpose": ws.workspace.purpose,
        "lastOpenedAt": ws.workspace.last_opened_at,
        "folders": ws.folders.iter().map(|f| json!({"id": f.id, "path": f.path, "position": f.position})).collect::<Vec<_>>(),
    })
}

/// 文件面板的根集合：`folder_id` 非空时只取该源文件夹（多根项目按文件夹分树浏览）；
/// 空串 = 全部根（多根解析，首个存在者胜出——兼容旧通道调用方）。
/// 找不到对应文件夹时报错（前端下拉里的 id 与库内记录不一致的场景）。
fn workspace_folder_roots(state: &AppState, workspace_id: &str, folder_id: &str) -> Result<Vec<String>, String> {
    let ws = workspaces::get_workspace(&state.db.lock_conn(), workspace_id).map_err(|e| e.to_string())?;
    if folder_id.is_empty() {
        return Ok(ws.folders.iter().map(|f| f.path.clone()).collect());
    }
    let matched: Vec<String> = ws
        .folders
        .iter()
        .filter(|f| f.id == folder_id)
        .map(|f| f.path.clone())
        .collect();
    if matched.is_empty() {
        return Err("源文件夹不存在（可能已被移除），请重新选择".to_string());
    }
    Ok(matched)
}

pub fn workspace_add(state: &AppState, path: &str, trust: &str) -> SvcResult {
    if path.is_empty() || !Path::new(path).is_dir() {
        return err("目录不存在");
    }
    let name = Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    workspaces::upsert_workspace(&state.db.lock_conn(), &name, path, trust, "project").map_err(|e| e.to_string())?;
    workspace_list(state)
}

/// 源文件夹校验：新建项目与编辑项目**共用同一套规则**，避免两条入口的标准漂移。
///
/// 规则（顺序即报错优先级）：绝对路径 + 符号链接解析 → 必须是目录 → 请求内去重 →
/// 内置目录守卫（应用数据目录、内置临时会话目录）→ 跨工作区占用。
///
/// `exclude_workspace_id` 用于编辑场景：跳过该工作区自身已占用的目录，
/// 否则「原地保存不改变任何一项」会被「目录已被工作区使用」误伤。
/// canonicalize 后剥 Windows verbatim 前缀：`std::fs::canonicalize` 在 Windows 返回
/// `\\?\D:\...`，直接入库会以 `\\` 开头，被 craft-policy 的 UNC 词法守卫全灭
/// （所有路径调用报「路径超出工作区、附加目录与系统临时目录」）。存储与比较统一用非 verbatim 形式；
/// 守卫类比较（数据目录 / 会话目录 / 跨工作区占用）两侧都必须过同一助手，
/// 否则形式不一致会让守卫静默失效。
fn canon_folder(path: &Path) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| craft_policy::resolve::strip_verbatim(&p).to_string_lossy().to_string())
}

fn normalize_workspace_folders(
    state: &AppState,
    raw: &[&str],
    exclude_workspace_id: Option<&str>,
) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Err("请至少选择一个源文件夹".to_string());
    }
    // 规范化：绝对路径 + 符号链接解析 + 必须是目录 + 请求内去重（verbatim 前缀剥离后入库）
    let mut normalized: Vec<String> = Vec::with_capacity(raw.len());
    for folder in raw {
        let abs = std::fs::canonicalize(folder).map_err(|_| "目录不存在".to_string())?;
        let abs = craft_policy::resolve::strip_verbatim(&abs);
        if !abs.is_dir() {
            return Err("源文件夹必须是目录".to_string());
        }
        let abs = abs.to_string_lossy().to_string();
        if normalized.iter().any(|p| p == &abs) {
            return Err(format!("源文件夹重复：{abs}"));
        }
        normalized.push(abs);
    }
    let existing = {
        let conn = state.db.lock_conn();
        workspaces::list_workspaces(&conn).map_err(|e| e.to_string())?
    };
    // 内置目录守卫：数据目录与内置临时会话目录不可作为项目目录（避免两套授权范围交叉）。
    // 存储路径可能与规范化形式不同（如 macOS /var → /private/var），两侧都规范化后比较。
    let data_dir_canon = canon_folder(Path::new(&state.data_dir));
    let conv_paths: Vec<String> = existing
        .iter()
        .filter(|w| w.workspace.purpose == "conversation")
        .filter_map(|w| canon_folder(Path::new(&w.workspace.path)))
        .collect();
    for abs in &normalized {
        if Some(abs) == data_dir_canon.as_ref() {
            return Err("该目录是应用数据目录，不能作为项目目录".to_string());
        }
        if conv_paths.iter().any(|p| p == abs) {
            return Err("该目录是内置的临时会话目录，无需添加为项目".to_string());
        }
    }
    // 任一目录已是其他工作区的源文件夹时拒绝（同样规范化后比较；编辑场景排除自身）
    let used: Vec<(String, String)> = existing
        .iter()
        .filter(|w| Some(w.workspace.id.as_str()) != exclude_workspace_id)
        .flat_map(|w| w.folders.iter().map(move |f| (w.workspace.name.clone(), f.path.clone())))
        .filter_map(|(name, p)| canon_folder(Path::new(&p)).map(|c| (name, c)))
        .collect();
    if let Some((ws_name, _)) = used.iter().find(|(_, p)| normalized.iter().any(|n| n == p)) {
        return Err(format!("目录已被工作区「{ws_name}」使用"));
    }
    Ok(normalized)
}

pub fn workspace_add_project(state: &AppState, name: &str, folders: &[Value], trust: &str) -> SvcResult {
    let raw: Vec<&str> = folders.iter().filter_map(Value::as_str).collect();
    let normalized = normalize_workspace_folders(state, &raw, None)?;
    let existing = {
        let conn = state.db.lock_conn();
        workspaces::list_workspaces(&conn).map_err(|e| e.to_string())?
    };
    let trust = if trust == "trusted" { "trusted" } else { "restricted" };
    // 名称推导：空名称用主要目录的目录名；重名自动追加序号
    let mut name = name.trim().to_string();
    if name.is_empty() {
        name = Path::new(&normalized[0])
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| normalized[0].clone());
    }
    let name = unique_workspace_name(name, &existing);
    let ws = {
        let conn = state.db.lock_conn();
        workspaces::upsert_workspace(&conn, &name, &normalized[0], trust, "project").map_err(|e| e.to_string())?
    };
    workspaces::replace_workspace_folders(&state.db, &ws.id, &normalized).map_err(|e| e.to_string())?;
    // 重新读取装配文件夹列表，返回单对象与前端 Promise<WorkspaceView> 声明一致
    let saved = {
        let conn = state.db.lock_conn();
        workspaces::get_workspace(&conn, &ws.id).map_err(|e| e.to_string())?
    };
    Ok(workspace_view_json(&saved))
}

/// 项目重名自动加序号：「name (2)」「name (3)」……（对齐旧版 AddProject 语义）。
fn unique_workspace_name(name: String, existing: &[workspaces::WorkspaceWithFolders]) -> String {
    let taken: Vec<&str> = existing.iter().map(|w| w.workspace.name.as_str()).collect();
    if !taken.contains(&name.as_str()) {
        return name;
    }
    let mut index = 2;
    loop {
        let candidate = format!("{name} ({index})");
        if !taken.contains(&candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

/// 编辑项目源文件夹：增删附加目录、或更换主要目录（首项即主要目录）。
///
/// 规则要点：
/// - 校验复用 [`normalize_workspace_folders`]，与新建项目同一套（规范化 / 去重 / 内置目录 /
///   跨工作区占用），并 `exclude_workspace_id = Some(id)` 跳过自身占用；
/// - 内置临时会话目录（purpose=conversation）不支持编辑；
/// - **更换主要目录的守卫**：已绑定 worktree 的会话，其工作树由旧主要目录的仓库创建，
///   换主目录会让来源仓库失配，因此存在此类会话时拒绝，用户需先处理这些会话；
/// - 落库走 `replace_workspace_folders`（事务内整体替换 + 同步 workspaces.path）；
/// - 生效时机：每轮 run 重新读取工作区，改完对下一轮对话生效，不打断在跑的 run。
///
/// **受影响面**（改这条链路前先确认这几处仍然自洽）：一次「主要目录」变更会同时改变
/// ① RunEnv 的 roots 顺序（`craft-agent` 每轮重建环境时取首项为主目录）；
/// ② 系统提示词里的附加目录清单（`craft-agent/src/prompt.rs`）；
/// ③ Git 相关工具与侧栏状态——只有主目录参与，附加目录不视为仓库；
/// ④ 技能扫描的工作区级根（`craft-tools/src/skills.rs`）；
/// ⑤ 新建会话时 worktree 的来源仓库。上一行那条守卫正是 ⑤ 的保护。
pub fn workspace_replace_folders(state: &AppState, id: &str, folders: &[Value]) -> SvcResult {
    let current = {
        let conn = state.db.lock_conn();
        workspaces::get_workspace(&conn, id).map_err(|e| e.to_string())?
    };
    if current.workspace.purpose == "conversation" {
        return err("内置临时会话目录不支持编辑源文件夹");
    }
    let raw: Vec<&str> = folders.iter().filter_map(Value::as_str).collect();
    let normalized = normalize_workspace_folders(state, &raw, Some(id))?;
    // 主要目录变更守卫：worktree 的来源仓库固定在旧主要目录，换主目录后工作树与项目失配。
    // 只动附加目录不受影响（工作树来源仍是同一个主要目录）。
    let new_main = &normalized[0];
    if new_main != &current.workspace.path {
        let bound = {
            let conn = state.db.lock_conn();
            threads::list_threads_by_workspace(&conn, id).map_err(|e| e.to_string())?
        };
        let bound_count = bound.iter().filter(|t| !t.worktree_path.is_empty()).count();
        if bound_count > 0 {
            return err(format!(
                "该项目有 {bound_count} 个会话绑定了 git 工作树，工作树的来源仓库是当前主要目录；\
                 更换主要目录会让这些工作树与项目失配。请先删除这些会话或改用普通会话，再更换主要目录。"
            ));
        }
    }
    workspaces::replace_workspace_folders(&state.db, id, &normalized).map_err(|e| e.to_string())?;
    // 广播更新：渲染层 applyEvent 收到后 refreshWorkspaces（无 removed 字段即纯刷新）
    state.recorder.emit(
        craft_core::contract::event_type::WORKSPACE_UPDATED,
        None,
        None,
        json!({ "workspaceId": id }),
    );
    let saved = {
        let conn = state.db.lock_conn();
        workspaces::get_workspace(&conn, id).map_err(|e| e.to_string())?
    };
    Ok(workspace_view_json(&saved))
}

pub fn workspace_set_trust(state: &AppState, id: &str, trust: &str) -> SvcResult {
    workspaces::update_workspace_trust(&state.db.lock_conn(), id, trust).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn workspace_rename(state: &AppState, id: &str, name: &str) -> SvcResult {
    workspaces::update_workspace_name(&state.db.lock_conn(), id, name).map_err(|e| e.to_string())?;
    workspace_list(state)
}

pub fn workspace_remove(state: &AppState, id: &str) -> SvcResult {
    // 级联删除其下会话（AGENTS.md：删除级联清理会话）；删除前逐会话清理绑定的 worktree
    let doomed = {
        let conn = state.db.lock_conn();
        threads::list_threads_by_workspace(&conn, id).map_err(|e| e.to_string())?
    };
    for thread in &doomed {
        cleanup_thread_worktree(state, thread);
    }
    let count = threads::delete_threads_by_workspace(&state.db, id).map_err(|e| e.to_string())?;
    workspaces::delete_workspace(&state.db, id).map_err(|e| e.to_string())?;
    // 广播删除事件：渲染层据此清理该工作区的草稿槽（localStorage），避免残留
    state.recorder.emit(
        craft_core::contract::event_type::WORKSPACE_UPDATED,
        None,
        None,
        json!({ "workspaceId": id, "removed": true }),
    );
    Ok(json!({ "removedThreads": count }))
}

/// 列工作区目录：相对路径经路径安全解析（越界报错）；目录优先、名称升序，最多 500 项。
pub fn workspace_list_dir(state: &AppState, workspace_id: &str, folder_id: &str, rel: &str) -> SvcResult {
    let roots = workspace_folder_roots(state, workspace_id, folder_id)?;
    let resolved = resolve_existing(&roots, rel).map_err(|_| "路径超出工作区".to_string())?;
    let mut nodes: Vec<Value> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&resolved.abs) else {
        return Ok(json!(nodes));
    };
    let mut collected: Vec<(bool, String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let abs = entry.path();
        let is_dir = entry.file_type().ok().map(|t| t.is_dir()).unwrap_or(false);
        collected.push((is_dir, name, abs.to_string_lossy().to_string()));
    }
    collected.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (is_dir, name, abs) in collected.into_iter().take(500) {
        let meta = std::fs::metadata(&abs).ok();
        let size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
        let modified = meta.as_ref().and_then(|m| m.modified().ok()).map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64).unwrap_or(0);
        nodes.push(json!({
            "name": name,
            "path": if resolved.rel == "." { name.clone() } else { format!("{}/{}", resolved.rel, name) },
            "type": if is_dir { "dir" } else { "file" },
            "size": size,
            "modified": modified,
        }));
    }
    Ok(json!(nodes))
}

/// 读工作区文件：超 8MB 拒绝；二进制内容不返回文本（binary 标记）；超过 MAX_READ_BYTES
/// 截断（truncated 标记）；hash 为全文 SHA-256，保存时作 expectedHash 乐观锁比对。
pub fn workspace_read_file(state: &AppState, workspace_id: &str, folder_id: &str, rel: &str) -> SvcResult {
    let roots = workspace_folder_roots(state, workspace_id, folder_id)?;
    let resolved = resolve_existing(&roots, rel).map_err(|_| "路径超出工作区".to_string())?;
    let meta = std::fs::metadata(&resolved.abs).map_err(|e| format!("读取失败：{e}"))?;
    if meta.len() > 8 * 1024 * 1024 {
        return err(format!("文件过大（{}），超过 8MB 读取上限", craft_tools::human_size(meta.len() as i64)));
    }
    let raw = std::fs::read(&resolved.abs).map_err(|e| format!("读取失败：{e}"))?;
    let binary = is_binary(&raw);
    let truncated = raw.len() > craft_tools::MAX_READ_BYTES;
    let content = if binary {
        String::new()
    } else {
        let capped = if truncated { &raw[..craft_tools::MAX_READ_BYTES] } else { &raw };
        String::from_utf8_lossy(capped).to_string()
    };
    let language = Path::new(&resolved.abs)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    Ok(json!({
        "workspaceId": workspace_id,
        "folderId": folder_id,
        "path": resolved.rel,
        "content": content,
        "hash": sha256_hex(&raw),
        "size": meta.len() as i64,
        "modified": meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0),
        "binary": binary,
        "truncated": truncated,
        "language": language,
    }))
}

pub fn workspace_write_file(state: &AppState, workspace_id: &str, folder_id: &str, rel: &str, content: &str, expected_hash: &str) -> SvcResult {
    let roots = workspace_folder_roots(state, workspace_id, folder_id)?;
    // 新建文件（父目录缺失）场景：词法校验后按主要根拼接，写前建目录
    let resolved = match resolve_existing(&roots, rel) {
        Ok(resolved) => resolved,
        Err(_) => {
            let main = roots.first().cloned().ok_or("没有可用工作区目录")?;
            let (abs, rel_out) = craft_policy::resolve::lexical_at(&main, rel).map_err(|_| "路径超出工作区".to_string())?;
            craft_policy::resolve::Resolved { root_index: 0, root: main, abs, rel: rel_out, system_temp: false }
        }
    };
    // expectedHash 阻止覆盖外部修改
    if let Ok(existing) = std::fs::read(&resolved.abs) {
        let current_hash = sha256_hex(&existing);
        if !expected_hash.is_empty() && current_hash != expected_hash {
            return err("文件已被外部修改，请先重新读取再保存");
        }
    }
    if let Some(parent) = Path::new(&resolved.abs).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    write_file_atomic(Path::new(&resolved.abs), content.as_bytes(), 0o644).map_err(|e| format!("写入失败：{e}"))?;
    Ok(json!(null))
}

/// 文档预览（docx/xlsx/pdf 文本层）：格式不支持返回 supported=false；解析失败仍返回
/// supported=true、错误信息进 note（界面展示「为何打不开」而非空白）。
pub fn document_preview(state: &AppState, workspace_id: &str, folder_id: &str, rel: &str) -> SvcResult {
    let roots = workspace_folder_roots(state, workspace_id, folder_id)?;
    let resolved = resolve_existing(&roots, rel).map_err(|_| "路径超出工作区".to_string())?;
    if !craft_doc::doc_supported(&resolved.abs) {
        return Ok(json!({
            "kind": "", "path": resolved.rel, "folderId": "", "name": "",
            "size": 0, "modified": 0, "supported": false,
            "note": "不支持的文档格式", "text": "",
        }));
    }
    let meta = std::fs::metadata(&resolved.abs).map_err(|e| format!("读取失败：{e}"))?;
    let name = Path::new(&resolved.abs).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    match craft_doc::read_file(&resolved.abs) {
        Ok(doc) => Ok(json!({
            "kind": doc.kind, "path": resolved.rel, "folderId": folder_id, "name": name,
            "size": meta.len() as i64,
            "modified": meta.modified().ok().map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64).unwrap_or(0),
            "supported": true, "note": doc.note, "text": doc.text,
        })),
        Err(err) => Ok(json!({
            "kind": craft_doc::doc_kind(&resolved.abs), "path": resolved.rel, "folderId": folder_id, "name": name,
            "size": meta.len() as i64,
            "modified": meta.modified().ok().map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64).unwrap_or(0),
            "supported": true, "note": err, "text": "",
        })),
    }
}

pub fn workspace_compute_diff(_state: &AppState, old_text: &str, new_text: &str) -> SvcResult {
    let result = editor::diff(old_text, new_text);
    serde_json::to_value(result).map_err(|e| e.to_string())
}

/// 各项目工作区的 git status 摘要：仅 purpose=project；单工作区 5s 超时，失败返回空串。
///
/// 采集走 craft-git（D13 迁移：进程内 gix，输出与 `git status --short --branch`
/// 同构）；5s 超时经 craft-git 的协作式中断等价物保留，失败（非 Git 仓库 / 超时 /
/// 读取异常）与迁移前一致返回空串。
pub fn workspace_git_statuses(state: &AppState) -> SvcResult {
    let list = workspaces::list_workspaces(&state.db.lock_conn()).map_err(|e| e.to_string())?;
    let mut statuses = serde_json::Map::new();
    for ws in list {
        if ws.workspace.purpose != "project" {
            continue;
        }
        let text = workspace_git_status_text(&ws.workspace.path);
        statuses.insert(ws.workspace.id, json!(text.as_deref().unwrap_or("")));
    }
    Ok(Value::Object(statuses))
}

/// 单工作区的 git status 文本（`--short --branch` 同构，已 trim）；None = 采集失败。
///
/// 5s 上限为迁移前的等时时长；调用方（workspace_git_statuses 循环）在阻塞线程里
/// 逐个采集，超时只放弃当前工作区。
fn workspace_git_status_text(path: &str) -> Option<String> {
    let path_owned = path.to_string();
    craft_git::with_timeout(std::time::Duration::from_secs(5), "工作区状态采集", move |flag| {
        let repo = craft_git::Repository::discover(&path_owned)?;
        let text = craft_git::status::status_text(&repo, true, flag)?;
        Ok(text.trim().to_string())
    })
    .ok()
}

// ---------- worktree（D9：进程内 craft-git 编排，见 D13） ----------

/// 会话绑定的 worktree 目录：数据目录下统一管理（随 CRAFTWORK_HOME 隔离，不污染项目目录）。
fn thread_worktree_dir(data_dir: &str, thread_id: &str) -> PathBuf {
    PathBuf::from(data_dir).join("worktrees").join(thread_id)
}

/// worktree 分支名：craftwork/<会话 id 去前缀后的前 8 位>；跨重建稳定，便于终端辨认。
fn worktree_branch_for(thread_id: &str) -> String {
    let short: String = thread_id.strip_prefix("thr_").unwrap_or(thread_id).chars().take(8).collect();
    format!("craftwork/{short}")
}

/// 确保会话已绑定可用的 git worktree（幂等；服务层直跑不经审批——发生在 run 之前，
/// 失败回滚由调用方保持「失败零落库」）：
/// · 未绑定 → 校验项目工作区 + 主目录为 git 仓库，创建 `<数据目录>/worktrees/<会话id>`（分支 craftwork/<短码>）并落库；
/// · 已绑定但目录丢失 → prune 清残留登记后重建，优先复用原分支（历史提交不丢）；
/// · 已绑定且目录存在 → 直接返回，不重复创建。
/// 失败返回可读中文错误，发送链路据此中止（不落消息）。
/// 全部 git 操作经 craft-git（gix 进程内，D13 迁移后无系统 git 进程调用）。
pub fn ensure_thread_worktree(state: &AppState, thread_id: &str) -> Result<(String, String), String> {
    let thread = threads::get_thread(&state.db.lock_conn(), thread_id).map_err(|e| e.to_string())?;
    if thread.mode == "work" {
        return err("Work 模式没有 Git 能力，不支持 worktree；请切换到 Code 模式后重试");
    }
    if thread.workspace_id.is_empty() {
        return err("会话未绑定工作区，无法使用 worktree");
    }
    let ws = workspaces::get_workspace(&state.db.lock_conn(), &thread.workspace_id).map_err(|e| e.to_string())?;
    if ws.workspace.purpose != "project" {
        return err("内置临时目录不是 git 仓库，无法使用 worktree；请先将项目添加为工作区");
    }
    let repo = ws.workspace.path.clone();
    // 主目录必须是 git 仓库（等价迁移前的 `rev-parse --is-inside-work-tree` 探测）；
    // open 失败即「不是 git 仓库」，错误文案保持原样
    if craft_git::Repository::open(&repo).is_err() {
        return err("当前项目不是 git 仓库，无法使用 worktree");
    }
    let path = thread_worktree_dir(&state.data_dir, thread_id);
    let path_str = path.to_string_lossy().to_string();
    let branch = worktree_branch_for(thread_id);

    // 幂等：已绑定且目录在，直接返回既有绑定
    if !thread.worktree_path.is_empty() && path.is_dir() {
        return Ok((thread.worktree_path.clone(), thread.worktree_branch.clone()));
    }

    // 目录丢失（手动删除等）：清理主仓库残留的 worktree 登记后重建
    let _ = craft_git::worktree::prune(&repo);
    std::fs::create_dir_all(&path).map_err(|e| format!("创建工作树目录失败：{e}"))?;
    // 分支不存在则从 HEAD 创建（`-b` 语义）、已存在则复用（重建场景历史提交不丢）；
    // 同分支已在其他工作树检出 / bare / unborn HEAD 由 craft-git 拒绝并给可读错误
    craft_git::worktree::add_worktree(&repo, &path, &branch).map_err(|e| e.to_string())?;
    threads::set_thread_worktree(&state.db.lock_conn(), thread_id, &path_str, &branch).map_err(|e| e.to_string())?;
    Ok((path_str, branch))
}

/// 删除会话时清理其绑定的 worktree（best-effort）：强制移除工作树并删除分支，
/// 失败只落审计不阻塞删除（残留目录可手动清理；未合并改动的告知由前端确认框承担）。
/// 全部 git 操作经 craft-git（D13 迁移后无系统 git 进程调用）。
fn cleanup_thread_worktree(state: &AppState, thread: &threads::ThreadRow) {
    if thread.worktree_path.is_empty() {
        return;
    }
    // 主仓库目录：worktree 的 git 元数据挂在来源工作区主目录；工作区已删则退化直接删目录
    let repo = state.db.with(|conn| {
        Ok(workspaces::get_workspace(conn, &thread.workspace_id)?.workspace.path)
    });
    let mut removed = false;
    if let Ok(repo) = &repo {
        // 强制移除（等价迁移前 `worktree remove --force`，丢弃未提交改动）
        removed = craft_git::worktree::remove_worktree(repo, &thread.worktree_path, true).is_ok();
        // 分支强删（不检查合并状态，等价 `branch -D`；幂等，不存在静默成功）
        let _ = craft_git::worktree::delete_branch(repo, &thread.worktree_branch);
    }
    if !removed {
        let _ = std::fs::remove_dir_all(&thread.worktree_path);
    }
}


/// 存储分节：首屏概览（数据库 + 数据目录各类 + 运行门禁）。
///
/// 服务端只做编排与错误转中文；统计在 `crate::storage`。门禁原因一并回传，让界面
/// 能在按钮变灰时给出「为什么」——原因文案由服务端计算，避免前后端两套口径。
pub fn storage_overview(state: &AppState) -> SvcResult {
    let running = state.runner.running_threads();
    let background = state.shell_tasks.running_all().len();
    let view = crate::storage::overview(&state.db, &state.data_dir, &running, background)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

/// 存储分节：会话占用（按项目分组）。点开会话卡才调用。
pub fn storage_threads(state: &AppState) -> SvcResult {
    let view = crate::storage::threads(&state.db, &state.data_dir).map_err(|e| e.to_string())?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

/// 存储分节：表级明细（dbstat）。不可用时返回空数组，界面隐藏明细。
pub fn storage_db_tables(state: &AppState) -> SvcResult {
    serde_json::to_value(crate::storage::db_tables(&state.db)).map_err(|e| e.to_string())
}

/// 存储分节：执行清理。
///
/// 门禁在服务端再判一次（`storage::cleanup` 内部）：前端按钮禁用只是体验层，
/// 状态过期的界面不该能绕过「有任务在跑就不许清」。成功与失败都写审计——清理
/// 是不可逆动作，事后必须能从审计追出「什么时候清了什么、释放多少」。
pub fn storage_cleanup(state: &AppState, kind: &str) -> SvcResult {
    let Some(kind) = crate::storage::CleanupKind::parse(kind) else {
        return err("未知的清理项");
    };
    let running = state.runner.running_threads();
    let background = state.shell_tasks.running_all().len();
    let result = match crate::storage::cleanup(&state.db, &state.data_dir, kind, &running, background) {
        Ok(result) => result,
        Err(crate::storage::StorageError::Busy(reason)) => {
            return err(&reason);
        }
        Err(e) => {
            let message = e.to_string();
            return err(&message);
        }
    };
    serde_json::to_value(result).map_err(|e| e.to_string())
}

// ---------- 对话 ----------

/// 原子首发（services/chat.ts 的 chatCreateAndStart 等价）：
/// 模型校验 → 创建会话 →（勾选 worktree 时先创建并绑定）→ 写用户消息事件 → 启动 Run。失败零落库。
#[allow(clippy::too_many_arguments)]
pub fn chat_create_and_start(
    state: &AppState,
    mode: &str,
    workspace_id: &str,
    text: &str,
    attachments: &[Value],
    model_ref: &str,
    reasoning_level: &str,
    plan_mode: bool,
    use_worktree: bool,
) -> SvcResult {
    if state.models.resolve(model_ref).is_none() {
        return err("模型不存在或已被删除，请重新选择模型");
    }
    // 草稿槽空串（内置临时会话）回落到 conversation 工作区：侧栏「临时会话」分组按 workspaceId 匹配，
    // 原样落空串会把会话挤进「其他」分组。
    let ws_id_owned;
    let workspace_id = if workspace_id.is_empty() {
        ws_id_owned = craft_store::workspaces::default_conversation_workspace_id(&state.db.lock_conn());
        ws_id_owned.as_str()
    } else {
        workspace_id
    };
    let title = craft_agent::title::derive_title(text);
    let id = craft_core::ids::new_id(craft_core::ids::id_prefix::THREAD);
    threads::create_thread(
        &state.db.lock_conn(),
        threads::NewThread {
            id: &id,
            mode,
            title: &title,
            title_auto: true,
            model_id: model_ref,
            reasoning_level,
            workspace_id,
            status: "idle",
            archived: false,
        },
    )
    .map_err(|e| e.to_string())?;
    // worktree 创建放在消息落库之前：失败回滚刚建的空会话，保持「失败零落库」语义
    if use_worktree
        && let Err(e) = ensure_thread_worktree(state, &id)
    {
        let _ = threads::delete_thread(&state.db, &id);
        return err(e);
    }
    let (_, seq) = append_user_message(state, &id, text, attachments, plan_mode, detect_skill_invoke(state, &id, text).as_deref())?;
    let run_id = state.runner.run(&id, Some(seq))?;
    Ok(json!({ "threadId": id, "runId": run_id }))
}

fn append_user_message(
    state: &AppState,
    thread_id: &str,
    text: &str,
    attachments: &[Value],
    plan_mode: bool,
    skill_invoke: Option<&str>,
) -> Result<(String, i64), String> {
    append_user_message_via(state, thread_id, text, attachments, None, plan_mode, skill_invoke)
}

/// 落库并广播用户消息；via 非空时写入载荷（"steer" = 运行中引导，前端显示「待送达」）。
/// plan_mode 随载荷落库：run 启动时按 trigger_seq 还原（计划模式提示词与审批硬约束的依据）。
/// skill_invoke 非空表示斜杠技能调用（载荷带 skillInvoke.name，历史重建时改写为引导 prompt）。
/// 返回 (message_id, seq)：seq 随 run 启动透传为 trigger_seq，快照占位定序的权威信号。
fn append_user_message_via(
    state: &AppState,
    thread_id: &str,
    text: &str,
    attachments: &[Value],
    via: Option<&str>,
    plan_mode: bool,
    skill_invoke: Option<&str>,
) -> Result<(String, i64), String> {
    let message_id = craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE);
    // 经 Recorder 落库并广播 user.message：直接调 record_event 只写库不发事件，
    // 渲染层时间线将看不到这条用户消息（首轮对话靠首发快照掩盖了该缺陷）。
    let mut payload = json!({ "text": text, "attachments": attachments, "planMode": plan_mode });
    if let Some(via) = via {
        payload["via"] = json!(via);
    }
    if let Some(name) = skill_invoke {
        payload["skillInvoke"] = json!({ "name": name });
    }
    let (_, seq) = state
        .recorder
        .append(&craft_store::events::EventRecord {
            thread_id: thread_id.to_string(),
            message_id: message_id.clone(),
            run_id: None,
            kind: "user_message".into(),
            payload,
        })
        .map_err(|e| e.to_string())?;
    Ok((message_id, seq))
}

pub fn chat_send(state: &AppState, thread_id: &str, text: &str, attachments: &[Value], plan_mode: bool, use_worktree: bool) -> SvcResult {
    // 已绑定会话总是确保工作树可用（目录丢失自动重建）；未绑定仅在勾选时创建并绑定
    let thread = threads::get_thread(&state.db.lock_conn(), thread_id).map_err(|e| e.to_string())?;
    if use_worktree || !thread.worktree_path.is_empty() {
        ensure_thread_worktree(state, thread_id)?;
    }
    let (_, seq) = append_user_message(state, thread_id, text, attachments, plan_mode, detect_skill_invoke(state, thread_id, text).as_deref())?;
    let run_id = state.runner.run(thread_id, Some(seq))?;
    Ok(json!(run_id))
}

pub fn chat_steer(state: &AppState, thread_id: &str, text: &str, attachments: &[Value], plan_mode: bool) -> SvcResult {
    // 预检引导容量再落库：只有确定并入队列的消息才带 via=steer（「待送达」标记）
    let steer = state.runner.can_steer(thread_id)?;
    let skill_invoke = detect_skill_invoke(state, thread_id, text);
    let (message_id, seq) = if steer {
        append_user_message_via(state, thread_id, text, attachments, Some("steer"), plan_mode, skill_invoke.as_deref())?
    } else {
        append_user_message(state, thread_id, text, attachments, plan_mode, skill_invoke.as_deref())?
    };
    let item = craft_agent::runner::SteerItem {
        message_id: message_id.clone(),
        // 斜杠技能调用：队列里的投递文本用引导 prompt（模型视角）；落库与前端显示保留原文
        text: match &skill_invoke {
            Some(name) => craft_tools::skills::manual_skill_prompt_from_text(name, text),
            None => text.to_string(),
        },
        attachments: attachments.iter().filter_map(Value::as_str).map(String::from).collect(),
        trigger_seq: seq,
    };
    if steer {
        match state.runner.try_steer(thread_id, item) {
            Ok(true) => return Ok(json!({ "queued": true })),
            // 预检后在途运行恰好收尾：按普通发送兜底（消息已落库，直接启动 run）
            Ok(false) => {}
            Err(e) => return err(e),
        }
    }
    let run_id = state.runner.run(thread_id, Some(seq))?;
    Ok(json!({ "queued": false, "runId": run_id }))
}

pub fn chat_regenerate(state: &AppState, thread_id: &str) -> SvcResult {
    // 重跑上一轮，无新触发消息：trigger_seq 走 NULL（快照按旧兜底定序）
    let run_id = state.runner.run(thread_id, None)?;
    Ok(json!(run_id))
}

pub fn chat_cancel(state: &AppState, thread_id: &str) -> SvcResult {
    Ok(json!(state.runner.cancel(thread_id)))
}

/// 读取后台命令任务的输出尾部（界面「读取输出」按钮与只读展示）。
///
/// 与模型侧的 `bash_output` 工具同源（都走 `read_output_tail`），但**不经过工具链**：
/// 这是用户主动看一眼进度，不该产生工具调用事件、审批或审计。任务号按会话校验，
/// 跨会话读不到别人的输出。
pub fn chat_bash_output(state: &AppState, thread_id: &str, task_id: &str, tail_bytes: i64) -> SvcResult {
    let Some(task) = state.shell_tasks.get(thread_id, task_id) else {
        return err(format!("找不到后台任务 {task_id}（可能已清理，或不属于当前会话）"));
    };
    let max_bytes = tail_bytes.clamp(1024, 65536) as usize;
    let (tail, total) = read_shell_output_tail(&task.output_path, max_bytes)?;
    Ok(json!({
        "output": tail,
        "state": task.state.as_str(),
        "exitCode": task.exit_code,
        "totalBytes": total,
    }))
}

/// 本会话仍在运行的后台命令任务（右栏「后台任务」tab 的唯一数据源）。
///
/// 只列 `Running`：已终态的任务不保留展示（`ShellTasks::prune_finished` 会丢弃已通知
/// 终态记录，这里若把它们列出来，列表长度会因为「通知是否已投递」而抖动）。要回看
/// 已结束任务的输出走聊天里的 `bash_output` 工具卡，或按 task_id 调 `chatBashOutput`。
///
/// 按会话取（`running_of`），跨会话查不到——与 `chatBashOutput` / `chatKillShell` 同一
/// 防串号口径。纯读，不产生工具调用事件、审批或审计。
pub fn chat_shell_tasks(state: &AppState, thread_id: &str) -> SvcResult {
    let mut tasks = state.shell_tasks.running_of(thread_id);
    // 按启动时刻排序：列表稳定，徽标计数与行顺序不随 HashMap 迭代顺序抖动
    tasks.sort_by_key(|t| t.started_at);
    let views: Vec<serde_json::Value> = tasks
        .iter()
        .map(|task| {
            json!({
                "id": task.id,
                "command": task.command,
                "state": task.state.as_str(),
                // Killed / 启动失败时为 null，前端按「未取到退出码」展示
                "exitCode": task.exit_code,
                "pid": task.pid,
                "startedAt": task.started_at,
                // 前台超时转后台：文案需要说明来源
                "fromTimeout": task.from_timeout,
            })
        })
        .collect();
    Ok(json!(views))
}

/// 用户主动终止后台命令任务（整组杀）。与 `kill_shell` 工具不同：这是用户直接操作，
/// 不经审批，但要落审计（AGENTS 6.2 的口径：用户决定与执行都要可追溯）。
pub fn chat_kill_shell(state: &AppState, thread_id: &str, task_id: &str) -> SvcResult {
    let Some(task) = state.shell_tasks.get(thread_id, task_id) else {
        return err(format!("找不到后台任务 {task_id}（可能已清理，或不属于当前会话）"));
    };
    if task.state.is_terminal() {
        return Ok(json!({ "killed": false }));
    }
    craft_core::platform::kill_process_group(task.pid);
    state.shell_tasks.settle(&task.id, craft_tools::shell_tasks::TaskState::Killed, None);
    Ok(json!({ "killed": true }))
}

/// 用户跳过在途 wait 等待（D21）。与 `wait` 工具不同：这是用户直接操作，不经审批，
/// 但要落审计（同 `chat_kill_shell` 口径：用户决定与执行都要可追溯）。
/// run 不终止——wait 返回 canceled 后模型立即去核实等待目的。
pub fn chat_cancel_wait(state: &AppState, thread_id: &str) -> SvcResult {
    if !state.runner.cancel_wait(thread_id) {
        return err("当前没有等待中的任务");
    }
    Ok(json!({ "canceled": true }))
}

/// 读输出文件尾部若干字节，切在 UTF-8 字符边界上。
fn read_shell_output_tail(path: &str, max_bytes: usize) -> Result<(String, usize), String> {
    let data = std::fs::read(path).map_err(|e| format!("读取任务输出失败：{e}"))?;
    let total = data.len();
    let mut start = total.saturating_sub(max_bytes);
    while start < total && std::str::from_utf8(&data[start..]).is_err() {
        start += 1;
    }
    Ok((String::from_utf8_lossy(&data[start..]).to_string(), total))
}

// ---------- 子代理（决策 D15） ----------

/// 某主会话派生的子代理状态列表（D15 的面板补注）。
///
/// 主链路是 subagent.updated 事件驱动卡片与右侧面板列表；本通道是**重启后的兜底来源**：
/// 内存注册表为空，前端打开子代理面板时经它恢复历史派发记录（含终态）。合并规则：
/// 事件表恢复为底（全量历史），内存注册表的条目覆盖同 agentId 的条目（运行中的实时值
/// 更准——中间态事件落库可能滞后一拍）。
pub fn subagent_list(state: &AppState, thread_id: &str) -> SvcResult {
    // 1) 事件表恢复（含终态记录；partId 分组取最新、按 startedAt 排序）
    let mut views: Vec<Value> = state
        .db
        .with(|conn| craft_store::events::subagent_latest_parts(conn, thread_id))
        .unwrap_or_default();
    // 2) 注册表覆盖（运行期实时值）。形状与事件载荷（part_payload）一致：
    //    渲染层把两种来源都写进跨会话索引，按同一套 SubagentPart 契约消费。
    let registered: Vec<Value> = state
        .runner
        .subagents()
        .list(thread_id)
        .iter()
        .map(|snapshot| {
            json!({
                "partId": snapshot.part_id,
                "agentId": snapshot.thread_id,
                "name": snapshot.agent_name,
                "agentType": snapshot.agent_type.as_str(),
                "agentTypeLabel": snapshot.agent_type.label(),
                "description": snapshot.description,
                "status": snapshot.status.as_str(),
                "model": snapshot.model_ref,
                "startedAt": snapshot.started_at,
                "finishedAt": snapshot.finished_at,
                "durationMs": snapshot.finished_at.map(|f| (f - snapshot.started_at).max(0)),
                "toolUses": snapshot.tool_uses,
                "tokens": snapshot.tokens,
                "iterations": snapshot.iterations,
                "maxIterations": snapshot.max_iterations,
                "parentThreadId": snapshot.parent_thread_id,
                "result": snapshot.result,
                "notify": snapshot.notify,
            })
        })
        .collect();
    for view in registered {
        let agent_id = view.get("agentId").and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(slot) = views.iter_mut().find(|v| v.get("agentId").and_then(Value::as_str) == Some(agent_id.as_str())) {
            *slot = view;
        } else {
            views.push(view);
        }
    }
    views.sort_by_key(|v| v.get("startedAt").and_then(Value::as_i64).unwrap_or(0));
    Ok(json!(views))
}

// ---------- 子代理配置（决策 D15 扩展：独立键 + 自定义定义） ----------

/// 读取子代理全局配置。
///
/// 兼容迁移（单向）：新键 `subagent.config` 为空时，用旧 `Settings` 里的
/// `subagentsEnabled` / `subagentMaxConcurrent` 初始化一次；之后只读写新键。
/// 旧字段在 `Settings` 里保留一个版本（旧库仍可能带它们），下个版本再清理。
///
/// 返回值附带只读字段 `maxIterationsCap`（D15 的面板补注）：
/// 渲染层轮数上限输入的最大值与「0 = 封顶 N」文案的唯一事实源。
pub fn subagent_settings_get(state: &AppState) -> SvcResult {
    let saved: Option<craft_core::contract::SubAgentSettings> = state
        .db
        .with(|conn| get_setting(conn, craft_core::contract::SUBAGENT_CONFIG_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok());
    let settings = saved.unwrap_or_else(|| {
        // 旧键回落
        let legacy = settings_get(state)
            .ok()
            .and_then(|v| serde_json::from_value::<Settings>(v).ok());
        match legacy {
            Some(legacy) => craft_core::contract::SubAgentSettings {
                enabled: legacy.subagents_enabled,
                max_concurrent: legacy.subagent_max_concurrent.clamp(1, 8),
                builtin_profiles: Vec::new(),
            },
            None => craft_core::contract::SubAgentSettings::default(),
        }
    });
    let mut value = serde_json::to_value(normalize_subagent_settings(settings)).map_err(|e| e.to_string())?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "maxIterationsCap".into(),
            json!(craft_agent::subagent::SUBAGENT_MAX_ITERATIONS_CAP),
        );
    }
    Ok(value)
}

/// 归一：开关保留、并发钳制 1~8、内置档案补全两型且轮数钳制。
fn normalize_subagent_settings(
    mut settings: craft_core::contract::SubAgentSettings,
) -> craft_core::contract::SubAgentSettings {
    settings.max_concurrent = settings.max_concurrent.clamp(1, 8);
    let mut builtin = Vec::new();
    for agent_type in ["general", "explore"] {
        let existing = settings.builtin_profiles.iter().find(|p| p.agent_type == agent_type);
        builtin.push(craft_core::contract::BuiltinAgentProfile {
            agent_type: agent_type.to_string(),
            model: existing.map(|p| p.model.clone()).unwrap_or_default(),
            max_iterations: craft_agent::subagent::clamp_agent_iterations(
                existing.map(|p| p.max_iterations).unwrap_or(0),
            ),
        });
    }
    settings.builtin_profiles = builtin;
    settings
}

/// 保存子代理全局配置（内置两型的模型与迭代上限也在这里）。
pub fn subagent_settings_save(state: &AppState, next: &Value) -> SvcResult {
    let current: craft_core::contract::SubAgentSettings =
        serde_json::from_value(subagent_settings_get(state)?).unwrap_or_default();
    let mut merged = current;
    if let Some(v) = next.get("enabled").and_then(Value::as_bool) {
        merged.enabled = v;
    }
    if let Some(v) = next.get("maxConcurrent").and_then(Value::as_i64) {
        merged.max_concurrent = v.clamp(1, 8);
    }
    if let Some(list) = next.get("builtinProfiles").and_then(Value::as_array) {
        let mut builtin = Vec::new();
        for item in list {
            let Some(agent_type) = item.get("agentType").and_then(Value::as_str) else {
                continue;
            };
            if !craft_core::contract::RESERVED_AGENT_NAMES.contains(&agent_type) {
                continue;
            }
            let model = item.get("model").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            // 模型必须可解析（空串 = 继承父会话，允许）
            if !model.is_empty() && state.models.resolve(&model).is_none() {
                return err(format!("子代理默认模型不存在或已停用：{model}"));
            }
            builtin.push(craft_core::contract::BuiltinAgentProfile {
                agent_type: agent_type.to_string(),
                model,
                max_iterations: craft_agent::subagent::clamp_agent_iterations(
                    item.get("maxIterations").and_then(Value::as_i64).unwrap_or(0),
                ),
            });
        }
        merged.builtin_profiles = builtin;
    }
    let normalized = normalize_subagent_settings(merged);
    state
        .db
        .with(|conn| {
            set_setting(
                conn,
                craft_core::contract::SUBAGENT_CONFIG_KEY,
                &serde_json::to_value(&normalized).unwrap_or(Value::Null),
            )
        })
        .map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 读取自定义子代理定义（含停用的；按创建顺序）。
pub fn subagent_profiles_get(state: &AppState) -> SvcResult {
    let list: Vec<craft_core::contract::CustomAgentProfile> = state
        .db
        .with(|conn| get_setting(conn, craft_core::contract::SUBAGENT_PROFILES_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    Ok(json!(list))
}

/// 写回自定义定义列表（内部用，已校验）。
fn write_profiles(
    state: &AppState,
    list: &[craft_core::contract::CustomAgentProfile],
) -> Result<(), String> {
    state
        .db
        .with(|conn| {
            set_setting(
                conn,
                craft_core::contract::SUBAGENT_PROFILES_KEY,
                &serde_json::to_value(list).unwrap_or(Value::Null),
            )
        })
        .map_err(|e| e.to_string())
}

/// 校验并归一一条自定义定义（新建与更新共用）。
///
/// 校验项与理由：
/// - `name`：模型可见的 `subagent_type` 值，必须过 `normalize_agent_name` 且非空，
///   不得占用内置保留名，同一列表内唯一；
/// - `description` / `role_prompt`：长度上限（前者进 enum 说明，后者进提示词）；
/// - `model`：可解析（空串 = 继承父会话模型）；
/// - `max_iterations`：钳 0~封顶；
/// - `tools.Only`：名单过滤到注册表实际存在的工具名（防幽灵工具）。
fn validate_profile(
    state: &AppState,
    input: &Value,
    existing: &[craft_core::contract::CustomAgentProfile],
    self_id: Option<&str>,
) -> Result<craft_core::contract::CustomAgentProfile, String> {
    use craft_core::contract::{
        CustomAgentProfile, CustomAgentTools, CUSTOM_AGENT_DESCRIPTION_MAX_CHARS, CUSTOM_AGENT_ROLE_PROMPT_MAX_CHARS,
        RESERVED_AGENT_NAMES,
    };
    let raw_name = input.get("name").and_then(Value::as_str).unwrap_or_default();
    let name = craft_tools::agent::normalize_agent_name(raw_name);
    if name.is_empty() {
        return err("名称不能为空：需以小写字母开头，可含数字、下划线与短横线（最长 32 字符）");
    }
    if RESERVED_AGENT_NAMES.contains(&name.as_str()) {
        return err(format!("名称 {name} 是内置子代理类型，请换一个"));
    }
    if existing
        .iter()
        .any(|p| p.name == name && self_id.map(|id| id != p.id).unwrap_or(true))
    {
        return err(format!("名称 {name} 已被其他自定义子代理占用"));
    }
    let description = input.get("description").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if description.chars().count() > CUSTOM_AGENT_DESCRIPTION_MAX_CHARS {
        return err(format!(
            "简介过长：{} 字符，上限 {CUSTOM_AGENT_DESCRIPTION_MAX_CHARS} 字符",
            description.chars().count()
        ));
    }
    let role_prompt = input.get("rolePrompt").and_then(Value::as_str).unwrap_or_default().to_string();
    if role_prompt.chars().count() > CUSTOM_AGENT_ROLE_PROMPT_MAX_CHARS {
        return err(format!(
            "角色提示词过长：{} 字符，上限 {CUSTOM_AGENT_ROLE_PROMPT_MAX_CHARS} 字符",
            role_prompt.chars().count()
        ));
    }
    let model = input.get("model").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if !model.is_empty() && state.models.resolve(&model).is_none() {
        return err(format!("指定的模型不存在或已停用：{model}"));
    }
    let max_iterations = craft_agent::subagent::clamp_agent_iterations(
        input.get("maxIterations").and_then(Value::as_i64).unwrap_or(0),
    );
    // 工具面：三档；Only 名单过滤到注册表实际存在的工具名
    let tools = match input.get("tools") {
        Some(Value::String(kind)) => match kind.as_str() {
            "all" => CustomAgentTools::All,
            _ => CustomAgentTools::ReadOnly,
        },
        Some(value) if value.is_object() => {
            let kind = value.get("kind").and_then(Value::as_str).unwrap_or("readOnly");
            match kind {
                "all" => CustomAgentTools::All,
                "only" => {
                    let names = value
                        .get("names")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .filter(|n| state.registry.spec_for(n).is_some())
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    CustomAgentTools::Only { names }
                }
                _ => CustomAgentTools::ReadOnly,
            }
        }
        _ => CustomAgentTools::ReadOnly,
    };
    Ok(CustomAgentProfile {
        id: self_id.map(str::to_string).unwrap_or_else(|| craft_core::ids::new_id("agent")),
        name,
        description,
        role_prompt,
        model,
        max_iterations,
        tools,
        enabled: input.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// 新建自定义子代理。
pub fn subagent_profile_create(state: &AppState, input: &Value) -> SvcResult {
    let mut list: Vec<craft_core::contract::CustomAgentProfile> =
        serde_json::from_value(subagent_profiles_get(state)?).unwrap_or_default();
    let profile = validate_profile(state, input, &list, None)?;
    list.push(profile.clone());
    write_profiles(state, &list)?;
    Ok(json!(profile))
}

/// 更新自定义子代理（id 不可变；未提供的字段保持原值）。
pub fn subagent_profile_update(state: &AppState, id: &str, input: &Value) -> SvcResult {
    let mut list: Vec<craft_core::contract::CustomAgentProfile> =
        serde_json::from_value(subagent_profiles_get(state)?).unwrap_or_default();
    let Some(index) = list.iter().position(|p| p.id == id) else {
        return err(format!("子代理不存在：{id}"));
    };
    // 合并：未传字段沿用原值（与 providerSave 的部分更新语义一致）
    let current = &list[index];
    let mut merged = json!({
        "name": current.name,
        "description": current.description,
        "rolePrompt": current.role_prompt,
        "model": current.model,
        "maxIterations": current.max_iterations,
        "tools": serde_json::to_value(&current.tools).unwrap_or(Value::Null),
        "enabled": current.enabled,
    });
    if let (Some(target), Some(source)) = (merged.as_object_mut(), input.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    let profile = validate_profile(state, &merged, &list, Some(id))?;
    list[index] = profile.clone();
    write_profiles(state, &list)?;
    Ok(json!(profile))
}

/// 删除自定义子代理，返回删除后的列表。
pub fn subagent_profile_delete(state: &AppState, id: &str) -> SvcResult {
    let mut list: Vec<craft_core::contract::CustomAgentProfile> =
        serde_json::from_value(subagent_profiles_get(state)?).unwrap_or_default();
    let before = list.len();
    list.retain(|p| p.id != id);
    if list.len() == before {
        return err(format!("子代理不存在：{id}"));
    }
    write_profiles(state, &list)?;
    Ok(json!(list))
}

/// 启停自定义子代理，返回刷新后的列表。
pub fn subagent_profile_set_enabled(state: &AppState, id: &str, enabled: bool) -> SvcResult {
    let mut list: Vec<craft_core::contract::CustomAgentProfile> =
        serde_json::from_value(subagent_profiles_get(state)?).unwrap_or_default();
    let Some(profile) = list.iter_mut().find(|p| p.id == id) else {
        return err(format!("子代理不存在：{id}"));
    };
    profile.enabled = enabled;
    write_profiles(state, &list)?;
    Ok(json!(list))
}

/// 窗口关闭/应用退出：统一取消在途运行并落审计（AGENTS 6.2「窗口关闭、应用退出
/// 统一按拒绝或取消落审计」）。同步返回；run 循环的收尾是异步的，进程随即退出时
/// 由下次启动的 interrupt_stale_runs 兜底（中断标记 + 可重试错误事件）。
pub fn cancel_all_runs(state: &AppState) {
    for thread_id in state.runner.running_threads() {
        if state.runner.cancel(&thread_id) {
        }
    }
}

// ---------- 提供商 ----------

fn provider_views(state: &AppState) -> SvcResult {
    let providers = state.catalog.providers();
    let views: Vec<Value> = providers.iter().map(|p| provider_view_json(state, p)).collect();
    Ok(json!(views))
}

/// 单个提供商视图（与 provider_views 同构）：providerSave/AddModel/UpdateModel 返回单对象
/// 与前端声明 Promise<ProviderView> 对齐（此前实现惰性返回整个列表，消费方 view.id 取到 undefined）。
fn provider_view_one(state: &AppState, provider_id: &str) -> SvcResult {
    let provider = state
        .catalog
        .providers()
        .into_iter()
        .find(|p| p.id == provider_id)
        .ok_or("提供商不存在")?;
    Ok(provider_view_json(state, &provider))
}

fn provider_view_json(state: &AppState, p: &PersistedProvider) -> Value {
    let secrets = &*state.secrets;
    let env_refs: Vec<&str> = p.env_vars.iter().map(String::as_str).collect();
    let key_source = secrets.source_for(&p.id, &env_refs);
    let key_masked = secrets.masked_for(&p.id, &env_refs);
    let headers: Vec<Value> = p
        .headers
        .iter()
        .map(|h| json!({ "name": h.name, "value": h.value }))
        .collect();
    let models: Vec<Value> = p
        .models
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "ref": format!("{}/{}", p.id, m.id), "name": m.name,
                "provider": p.id, "providerName": p.name,
                "contextK": m.context_k, "maxTokens": m.max_tokens,
                "caps": m.caps, "cost": m.cost,
                "status": m.status, "canonicalId": m.canonical_id,
                "releaseDate": m.release_date, "source": m.source,
                "reasoningKind": m.reasoning_kind, "reasoningLevels": m.reasoning_levels,
                "enabled": m.enabled, "available": !m.status.is_empty() && m.status != "alpha",
            })
        })
        .collect();
    json!({
        "id": p.id, "name": p.name, "kind": p.kind, "endpoint": p.endpoint,
        "keyRequired": p.key_required, "keySource": key_source, "keyMasked": key_masked,
        "available": true, "enabled": p.enabled, "docUrl": "", "catalogId": p.catalog_id,
        "source": p.source, "headers": headers, "models": models,
    })
}

pub fn provider_list(state: &AppState) -> SvcResult {
    state.catalog.reload();
    provider_views(state)
}

pub fn provider_save(state: &AppState, input: &Value) -> SvcResult {
    let mut providers: Vec<PersistedProvider> = state
        .db
        .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let mut id = input.get("id").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if id.is_empty() {
        // 空串由服务端生成 ID（custom_*）：对齐渲染层手动新增提供商的语义
        id = craft_core::ids::new_id(craft_core::ids::id_prefix::CUSTOM_PROVIDER);
    }
    // 自定义请求头：输入携带 headers 字段时整体替换（服务端复检，trim + 冲突/注入校验）；
    // 未携带（如端点保存路径）时保留现值，避免部分更新抹掉已配置的请求头。
    let headers = match input.get("headers") {
        Some(Value::Array(items)) => {
            let pairs: Vec<HeaderPair> = items
                .iter()
                .map(|v| HeaderPair {
                    name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                    value: v.get("value").and_then(Value::as_str).unwrap_or_default().to_string(),
                })
                .collect();
            let (cleaned, header_err) = validate_headers(&pairs);
            if !header_err.is_empty() {
                return err(header_err);
            }
            cleaned
        }
        _ => providers
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.headers.clone())
            .unwrap_or_default(),
    };
    // 来源溯源字段（source/catalogId/catalogVersion/envVars）不属于输入契约：
    // 更新既有提供商时从现值回填（同 headers 的「未携带保留现值」先例）——
    // 保存连接信息不改变来源，也不丢 env 密钥回退与 logo 归属；
    // 新建（无既有 id）时按手动创建落默认值。
    let existing = providers.iter().find(|p| p.id == id);
    let entry = json!({
        "id": id,
        "name": input.get("name").and_then(Value::as_str).unwrap_or(&id),
        "kind": input.get("kind").and_then(Value::as_str).unwrap_or("openai"),
        "endpoint": input.get("endpoint").and_then(Value::as_str).unwrap_or(""),
        "keyRequired": input.get("keyRequired").and_then(Value::as_bool).unwrap_or(false),
        "note": "",
        "source": existing.map(|p| p.source.as_str()).unwrap_or("manual"),
        "catalogId": existing.map(|p| p.catalog_id.as_str()).unwrap_or(""),
        "catalogVersion": existing.map(|p| p.catalog_version.as_str()).unwrap_or(""),
        "envVars": existing.map(|p| serde_json::to_value(&p.env_vars).unwrap_or(json!([]))).unwrap_or(json!([])),
        "enabled": true,
        "headers": headers,
        "models": input.get("models").cloned().unwrap_or(json!([])),
    });
    let provider: PersistedProvider =
        serde_json::from_value(entry).map_err(|e| format!("提供商数据无效：{e}"))?;
    if let Some(slot) = providers.iter_mut().find(|p| p.id == id) {
        *slot = provider;
    } else {
        providers.push(provider);
    }
    save_providers(state, &providers)?;
    provider_view_one(state, &id)
}

fn save_providers(state: &AppState, providers: &[PersistedProvider]) -> Result<(), String> {
    let value = serde_json::to_value(providers).map_err(|e| e.to_string())?;
    state.db.with(|conn| set_setting(conn, CUSTOM_PROVIDERS_KEY, &value)).map_err(|e| e.to_string())?;
    state.catalog.reload();
    Ok(())
}

pub fn provider_delete(state: &AppState, id: &str) -> SvcResult {
    let providers: Vec<PersistedProvider> = state
        .db
        .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let filtered: Vec<_> = providers.into_iter().filter(|p| p.id != id).collect();
    save_providers(state, &filtered)?;
    provider_list(state)
}

pub fn provider_set_enabled(state: &AppState, id: &str, enabled: bool) -> SvcResult {
    let mut providers: Vec<PersistedProvider> = state
        .db
        .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if let Some(p) = providers.iter_mut().find(|p| p.id == id) {
        p.enabled = enabled;
    }
    save_providers(state, &providers)?;
    provider_list(state)
}

/// 保存提供商密钥：只写 SecretStore（keys.json，0600），不进设置、数据库、日志与渲染层。
pub fn provider_set_key(state: &AppState, id: &str, key: &str) -> SvcResult {
    state.secrets.set(id, key);
    Ok(json!(null))
}

pub fn provider_clear_key(state: &AppState, id: &str) -> SvcResult {
    state.secrets.delete(id);
    Ok(json!(null))
}

/// 从 settings 即时读出出站代理配置（决策 D14）：
/// 每次调用即取值，保存即生效。无持久化设置回落默认 system 模式；custom 模式地址
/// 非法（遗留脏数据）按直连兜底——禁代理比走错代理安全。
fn proxy_config(state: &AppState) -> craft_core::network::ProxyConfig {
    let (mode, url) = state
        .db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Settings>(v).ok())
        .map(|s| (s.network_proxy_mode, s.network_proxy_url))
        .unwrap_or_default();
    craft_core::network::ProxyConfig::from_settings(&mode, &url).unwrap_or(craft_core::network::ProxyConfig {
        mode: craft_core::network::ProxyMode::None,
        effective_url: None,
    })
}

/// 网络错误转可读中文（不向界面抛原始 reqwest 错误串）：代理配错 / 目标不可达 /
/// 超时给出可操作提示；其余错误保留原始描述便于排查。
fn network_error_text(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "连接超时，请检查网络、代理设置与服务地址".to_string()
    } else if err.is_connect() {
        "无法建立连接，请检查网络、代理设置与服务地址".to_string()
    } else {
        format!("{err}")
    }
}

pub fn provider_test(state: &AppState, id: &str) -> SvcResult {
    // 连接测试：GET models 端点探测
    let Some(provider) = state.catalog.find_provider(id) else {
        return err("提供商不存在");
    };
    let env_refs: Vec<&str> = provider.env_vars.iter().map(String::as_str).collect();
    let key = state.secrets.get_for(&provider.id, &env_refs);
    let started = std::time::Instant::now();
    let client = proxy_config(state)
        .apply_reqwest_blocking(
            reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)),
        )
        .build()
        .map_err(|e| format!("网络客户端初始化失败：{e}"))?;
    let url = if provider.kind == "anthropic" {
        format!("{}/v1/models", provider.endpoint.trim_end_matches('/'))
    } else {
        format!("{}/models", provider.endpoint.trim_end_matches('/'))
    };
    let mut req = client.get(&url);
    if provider.kind == "anthropic" {
        req = req.header("x-api-key", &key).header("anthropic-version", "2023-06-01");
    } else if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    // 提供商自定义请求头（与模型调用同源，保证测试连接与真实调用行为一致）；
    // {sessionId} 占位符无会话上下文可替换，跳过不发（避免把字面占位符发给网关）
    for h in &provider.headers {
        if craft_model::types::header_has_session_placeholder(&h.value) {
            continue;
        }
        req = req.header(h.name.as_str(), h.value.as_str());
    }
    match req.send() {
        Ok(resp) => {
            let latency = started.elapsed().as_millis() as i64;
            let ok = resp.status().is_success();
            let code = if ok { String::new() } else { format!("http_{}", resp.status().as_u16()) };
            let message = if ok { "连接成功".to_string() } else { format!("服务返回 HTTP {}", resp.status().as_u16()) };
            Ok(json!({ "ok": ok, "message": message, "latencyMs": latency, "code": code }))
        }
        Err(e) => Ok(json!({ "ok": false, "message": format!("无法连接：{}", network_error_text(&e)), "latencyMs": started.elapsed().as_millis() as i64, "code": "network" })),
    }
}

// ---------- 网络页公开端点探测（决策 D14修订） ----------

/// 网络页探测的提供商目录 id 固定清单。拼写以 models.dev 快照实际数据为准（2026-11-07
/// 核对 assets/catalog/catalog.json.gz）：`github-copilot`、`ollama-cloud` 用连字符，不是下划线。
/// 目录缺失的 id 跳过不渲染该行（列表随目录走）。
///
/// 国内常用三家（2026-11-08 追加）统一取国内站端点：`zhipuai` = 智谱 AI
/// （`https://open.bigmodel.cn/api/paas/v4`，目录 id 是 `zhipuai` 而不是 `zhipu`）、
/// `deepseek`（`https://api.deepseek.com/v1`）、`minimax-cn` = MiniMax 国内站
/// （`https://api.minimax.cn/anthropic/v1`；国际站 `minimax` 是另一个域名，不重复列出，
/// 需要时按单行 id 单独探测）。
pub const PROBE_PROVIDER_IDS: [&str; 9] = [
    "anthropic",
    "openai",
    "opencode",
    "github-copilot",
    "ollama-cloud",
    "openrouter",
    "zhipuai",
    "deepseek",
    "minimax-cn",
];

/// 单端点探测结果（status 由 error 是否为 None 推导；序列化为 NetworkProbeView camelCase）。
#[derive(Debug)]
pub struct ProbeOutcome {
    /// 整个请求（连接 + 响应头到达）的耗时
    pub latency_ms: i64,
    /// 收到的 HTTP 状态码；请求失败（未收到响应）时为 None
    pub http_status: Option<u16>,
    /// 失败原因（可读中文）；连通成功时为 None
    pub error: Option<String>,
}

/// 「测试连接」的探测目标清单（唯一事实源）：按 [`PROBE_PROVIDER_IDS`] 的声明序取目录里的
/// 名称与端点。目录里缺失的 id、空端点都跳过——界面据此**固定渲染行**（探测只改行状态），
/// 探测请求也据此过滤，两处口径由本函数保证一致。
fn probe_targets(state: &AppState) -> Vec<(String, String, String)> {
    PROBE_PROVIDER_IDS
        .iter()
        .copied()
        .filter_map(|pid| {
            let p = state.catalog.builtin_provider(pid)?;
            let url = p.endpoint.trim().to_string();
            if url.is_empty() {
                return None;
            }
            Some((pid.to_string(), p.name.clone(), url))
        })
        .collect()
}

/// 网络页「测试连接」的行清单：探测**开始前**就要把九行画出来（固定行、只改单格状态），
/// 因此单独一条只读通道，与 [`network_probe_endpoints`] 共用 [`probe_targets`]。
/// 行不随探测结果增删——列表整块替换曾与多路扫光动画并发，诱发 WebView2 绘制残影。
pub fn network_probe_targets(state: &AppState) -> SvcResult {
    let rows: Vec<Value> = probe_targets(state)
        .into_iter()
        .map(|(id, name, url)| json!({ "id": id, "name": name, "url": url }))
        .collect();
    Ok(json!(rows))
}

/// 探测单个 URL 的可达性（网络页「测试连接」核心语义，与提供商管理页 providerTest
/// 的鉴权级探活语义不同）：GET 端点根路径，不带密钥、不带 /models 后缀——探测的是
/// 网络可达性而非 API 语义。**任意 HTTP 状态码（2xx/4xx/5xx）都算连通成功**；
/// DNS 解析失败 / 连接失败 / 超时才算失败。超时 15 秒，走当前网络配置（复用
/// apply_reqwest_blocking 的接入方式，与 provider_test 同源），UA 由
/// `craft-core::network` 统一注入。
/// pub 供服务层测试（本地 mock 服务器断言判定逻辑）。
pub fn network_probe_url(proxy: &craft_core::network::ProxyConfig, url: &str) -> ProbeOutcome {
    let started = std::time::Instant::now();
    let client = match proxy
        .apply_reqwest_blocking(
            reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)),
        )
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ProbeOutcome { latency_ms: 0, http_status: None, error: Some(format!("网络客户端初始化失败：{e}")) };
        }
    };
    match client.get(url).send() {
        // 收到任意 HTTP 响应即连通（4xx/5xx 也是服务端应答，说明网络与 TLS 均可达）
        Ok(resp) => ProbeOutcome {
            latency_ms: started.elapsed().as_millis() as i64,
            http_status: Some(resp.status().as_u16()),
            error: None,
        },
        Err(e) => ProbeOutcome {
            latency_ms: started.elapsed().as_millis() as i64,
            http_status: None,
            error: Some(probe_error_text(&e)),
        },
    }
}

/// 探测失败原因转简短可读中文（网络页行内展示，不带行动建议后缀）：超时 / 无法解析主机 /
/// 连接失败；其余错误保留原始描述便于排查。DNS 解析失败在错误链上表现为 connect 阶段的
/// resolver 错误，按错误链文本细分为「无法解析主机」，与「连接被拒 / 不可达」区分。
fn probe_error_text(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "连接超时".to_string()
    } else if err.is_connect() {
        let mut source = std::error::Error::source(err);
        let mut dns = false;
        while let Some(e) = source {
            let text = e.to_string().to_lowercase();
            if text.contains("dns")
                || text.contains("failed to lookup")
                || text.contains("nodename")
                || text.contains("name or service")
            {
                dns = true;
                break;
            }
            source = e.source();
        }
        if dns { "无法解析主机".to_string() } else { "连接失败".to_string() }
    } else {
        format!("{err}")
    }
}

/// 网络页「测试连接」：并发探测目录内常用提供商公开端点的可达性。id 为空全测，
/// 非空只测该 id（行内刷新，同一条通道）。返回 NetworkProbeView 数组（目录序）。
pub fn network_probe_endpoints(state: &AppState, id: &str) -> SvcResult {
    let proxy = proxy_config(state);
    let targets: Vec<(String, String, String)> = probe_targets(state)
        .into_iter()
        .filter(|(pid, _, _)| id.is_empty() || pid == id)
        .collect();
    // 并发探测：dispatch 已在阻塞线程池（无 tokio async 上下文），每个端点各起一个
    // 线程跑 blocking client，join 收集——总耗时 ≈ 最慢者。结果按目录序排列。
    let handles: Vec<_> = targets
        .into_iter()
        .map(|(pid, name, url)| {
            let proxy = proxy.clone();
            std::thread::spawn(move || {
                let outcome = network_probe_url(&proxy, &url);
                (pid, name, url, outcome)
            })
        })
        .collect();
    let mut views = Vec::new();
    for handle in handles {
        // network_probe_url 不 panic；线程异常属防御分支，跳过该行不阻塞其余结果
        let (pid, name, url, outcome) = match handle.join() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let status = if outcome.error.is_none() { "ok" } else { "fail" };
        views.push(json!({
            "id": pid,
            "name": name,
            "url": url,
            "status": status,
            "latencyMs": outcome.latency_ms,
            "httpStatus": outcome.http_status,
            "error": outcome.error,
        }));
    }
    Ok(json!(views))
}

// ---------- 目录 ----------

pub fn catalog_meta(state: &AppState) -> SvcResult {
    let (generated_at, version, provider_count, model_count) = state.catalog.builtin_meta();
    Ok(json!({ "generatedAt": generated_at, "version": version, "providerCount": provider_count, "modelCount": model_count }))
}

pub fn catalog_providers(state: &AppState, query: &str) -> SvcResult {
    let list = state.catalog.search_builtin_providers(query);
    let views: Vec<Value> = list
        .iter()
        .map(|p| {
            let added = state.catalog.find_provider(&p.id).is_some();
            json!({
                "id": p.id, "name": p.name, "kind": p.kind, "endpoint": p.endpoint,
                "doc": p.doc.clone().unwrap_or_default(),
                "env": catalog_provider_env(p),
                "modelCount": p.models.len(), "added": added,
            })
        })
        .collect();
    Ok(json!(views))
}

fn raw_cost_to_value(cost: Option<&craft_model::catalog::RawCost>) -> Value {
    cost.map(|c| {
        json!({
            "inputPerMTok": c.input.unwrap_or(0.0),
            "outputPerMTok": c.output.unwrap_or(0.0),
            "cacheReadPerMTok": c.cache_read.unwrap_or(0.0),
            "cacheWritePerMTok": c.cache_write.unwrap_or(0.0),
        })
    })
    .unwrap_or(Value::Null)
}

fn partial_caps_to_value(caps: Option<&craft_model::catalog::PartialCaps>) -> Value {
    // 目录模型 caps 为部分形状：以默认值合并（与 catalogModelToPersisted 一致）
    let full = caps.map(|c| craft_model::catalog::merge_caps_public(Some(c))).unwrap_or(craft_model::types::DEFAULT_CAPS);
    serde_json::to_value(full).unwrap_or(Value::Null)
}

pub fn catalog_models(state: &AppState, provider_id: &str) -> SvcResult {
    let Some(p) = state.catalog.builtin_provider(provider_id) else {
        return err("目录提供商不存在");
    };
    let views: Vec<Value> = p
        .models
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "name": m.name, "contextK": m.context_k,
                "maxTokens": m.max_tokens.unwrap_or(0), "caps": partial_caps_to_value(m.caps.as_ref()),
                "cost": raw_cost_to_value(m.cost.as_ref()), "status": m.status.clone().unwrap_or_default(),
                "releaseDate": m.release_date.clone().unwrap_or_default(),
            })
        })
        .collect();
    Ok(json!(views))
}

/// 从内置模型目录导入：all 或按 modelIds 选中，经目录归一后合并进已存提供商
/// （按模型 ID 去重，保留已有启用状态），不覆盖端点与密钥配置。
pub fn catalog_import(state: &AppState, provider_id: &str, all: bool, model_ids: &[Value]) -> SvcResult {
    let Some(p) = state.catalog.builtin_provider(provider_id) else {
        return err("目录提供商不存在");
    };
    let selected: Vec<String> = model_ids.iter().filter_map(Value::as_str).map(String::from).collect();
    let models: Vec<craft_model::types::PersistedModel> = p
        .models
        .iter()
        .filter(|m| all || selected.contains(&m.id))
        .map(|m| catalog_model_to_persisted(m, "catalog"))
        .collect();
    if models.is_empty() {
        return err("没有选择要导入的模型");
    }
    let mut providers: Vec<PersistedProvider> = state
        .db
        .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let provider = PersistedProvider {
        id: p.id.clone(),
        name: p.name.clone(),
        kind: if p.kind == "anthropic" { "anthropic".into() } else { "openai-compatible".into() },
        endpoint: p.endpoint.clone(),
        key_required: !catalog_provider_env(p).is_empty() || !p.endpoint.is_empty(),
        note: String::new(),
        source: "catalog".into(),
        catalog_id: p.id.clone(),
        catalog_version: String::new(),
        env_vars: catalog_provider_env(p),
        enabled: true,
        // 目录导入不覆盖已存的请求头配置（合并路径下仅新建时生效，取默认空）
        headers: providers.iter().find(|x| x.id == p.id).map(|x| x.headers.clone()).unwrap_or_default(),
        models,
    };
    if let Some(slot) = providers.iter_mut().find(|x| x.id == provider.id) {
        // 目录导入合并模型（按 id 去重，保留已有启用状态）
        for m in provider.models {
            if !slot.models.iter().any(|existing| existing.id == m.id) {
                slot.models.push(m);
            }
        }
    } else {
        providers.push(provider);
    }
    save_providers(state, &providers)?;
    provider_list(state)
}

// ---------- 用量 / 附件 ----------

pub fn usage_summary_view(state: &AppState) -> SvcResult {
    let summary = usage_summary(&state.db.lock_conn()).map_err(|e| e.to_string())?;
    serde_json::to_value(summary).map_err(|e| e.to_string())
}

/// 粘贴附件落盘：文件名只保留字母数字与 .-_（剥离路径分隔符等危险字符），再加 att_ ID
/// 前缀保证唯一；清洗后无可读部分时回落默认名 pasted（避免落盘出纯 att_<hex> 裸 ID，
/// 附件名会被消息气泡与标题等界面直接展示）。返回服务端文件名，后续经 readAttachment 取回。
pub fn save_pasted_attachment(state: &AppState, name: &str, base64_data: &str) -> SvcResult {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| format!("附件解码失败：{e}"))?;
    let safe_name: String = name.chars().filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_').collect();
    let safe_name = if safe_name.is_empty() { "pasted".to_string() } else { safe_name };
    let file_name = format!("{}_{}", craft_core::ids::new_id(craft_core::ids::id_prefix::ATTACHMENT), safe_name);
    let path = Path::new(&state.attachment_dir).join(&file_name);
    std::fs::create_dir_all(&state.attachment_dir).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("附件写入失败：{e}"))?;
    Ok(json!(file_name))
}

/// 读附件：返回 base64 内容与按扩展名推断的 mime（未识别扩展名回落 octet-stream）。
pub fn read_attachment(state: &AppState, name: &str) -> SvcResult {
    let path = Path::new(&state.attachment_dir).join(name);
    let bytes = std::fs::read(&path).map_err(|e| format!("附件读取失败：{e}"))?;
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    };
    use base64::Engine;
    Ok(json!({
        "base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
        "mime": mime,
    }))
}

// ---------- 提供商模型级操作与拉取 ----------

fn load_providers(state: &AppState) -> Vec<PersistedProvider> {
    state
        .db
        .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn model_input_to_persisted(v: &Value) -> Result<craft_model::types::PersistedModel, String> {
    // caps 经 PartialCaps 归一化：调用方可传任意子集（如模型弹窗不含 stream 勾选），
    // 缺失字段回落 DEFAULT_CAPS，避免反序列化因缺字段而拒绝
    let partial_caps: Option<craft_model::catalog::PartialCaps> = v
        .get("caps")
        .and_then(|c| serde_json::from_value(c.clone()).ok());
    let caps = craft_model::catalog::merge_caps_public(partial_caps.as_ref());
    serde_json::from_value(json!({
        "id": v.get("id").cloned().unwrap_or(Value::Null),
        "name": v.get("name").cloned().unwrap_or(Value::Null),
        "contextK": v.get("contextK").cloned().unwrap_or(json!(0)),
        "maxTokens": v.get("maxTokens").cloned().unwrap_or(json!(0)),
        "caps": serde_json::to_value(caps).unwrap_or(Value::Null),
        "cost": v.get("cost").cloned().unwrap_or(Value::Null),
        "status": v.get("status").cloned().unwrap_or(json!("")),
        "canonicalId": "", "releaseDate": "",
        "source": "manual", "enabled": true,
        // 手动声明的思考档位（ModelInput 可选；编辑未携带时由 provider_update_model 保留现值）
        "reasoningKind": v.get("reasoningKind").cloned().unwrap_or(json!("")),
        "reasoningLevels": v.get("reasoningLevels").cloned().unwrap_or(json!([])),
    }))
    .map_err(|e| format!("模型数据无效：{e}"))
}

pub fn provider_rename(state: &AppState, id: &str, name: &str) -> SvcResult {
    let mut providers = load_providers(state);
    match providers.iter_mut().find(|p| p.id == id) {
        Some(p) => p.name = name.into(),
        None => return err("提供商不存在"),
    }
    save_providers(state, &providers)?;
    provider_list(state)
}

pub fn provider_set_model_enabled(state: &AppState, provider_id: &str, model_id: &str, enabled: bool) -> SvcResult {
    let mut providers = load_providers(state);
    match providers.iter_mut().find(|p| p.id == provider_id) {
        Some(p) => match p.models.iter_mut().find(|m| m.id == model_id) {
            Some(m) => m.enabled = enabled,
            None => return err("模型不存在"),
        },
        None => return err("提供商不存在"),
    }
    save_providers(state, &providers)?;
    provider_list(state)
}

pub fn provider_add_model(state: &AppState, provider_id: &str, model: &Value) -> SvcResult {
    let mut providers = load_providers(state);
    let persisted = model_input_to_persisted(model)?;
    let provider = providers.iter_mut().find(|p| p.id == provider_id).ok_or("提供商不存在")?;
    if provider.models.iter().any(|m| m.id == persisted.id) {
        return err(format!("模型 ID 已存在：{}", persisted.id));
    }
    provider.models.push(persisted);
    save_providers(state, &providers)?;
    provider_view_one(state, provider_id)
}

pub fn provider_update_model(state: &AppState, provider_id: &str, model_id: &str, model: &Value) -> SvcResult {
    let mut providers = load_providers(state);
    let provider = providers.iter_mut().find(|p| p.id == provider_id).ok_or("提供商不存在")?;
    let idx = provider.models.iter().position(|m| m.id == model_id).ok_or("模型不存在")?;
    let mut persisted = model_input_to_persisted(model)?;
    let new_id = persisted.id.clone();
    let (old_id, old_source) = {
        let slot = &provider.models[idx];
        // 档位保留语义：请求未携带 reasoningLevels 字段时保留现值（目录归一值或上次手动声明），
        // 避免编辑显示名等操作把目录导入的档位元数据抹掉
        if model.get("reasoningLevels").is_none() {
            persisted.reasoning_kind = slot.reasoning_kind.clone();
            persisted.reasoning_levels = slot.reasoning_levels.clone();
        }
        (slot.id.clone(), slot.source.clone())
    };
    // ID 修改：目录来源不可变（目录事实源，改 ID 等于换了模型）；手动/拉取来源允许改名，
    // 但不得与本提供商下其他模型冲突。历史会话的模型引用不批量改写——
    // 悬空引用在发送时有可读错误（模型不存在或已被删除），由用户重新选择。
    if old_source == "catalog" && new_id != old_id {
        return err("目录来源的模型不可修改 ID");
    }
    if new_id != old_id {
        if new_id.trim().is_empty() {
            return err("模型 ID 不能为空");
        }
        if provider.models.iter().any(|m| m.id == new_id) {
            return err(format!("模型 ID 已存在：{new_id}"));
        }
    }
    provider.models[idx] = craft_model::types::PersistedModel { id: new_id.clone(), ..persisted };
    save_providers(state, &providers)?;
    if new_id != old_id {
        // 改 ID 后同步设置里的模型引用（default/fallback），避免默认模型悄悄悬空
        rewire_model_refs(state, provider_id, &old_id, &new_id)?;
    }
    provider_view_one(state, provider_id)
}

/// 模型改 ID 后同步设置里的模型引用（default/fallback），避免默认模型悄悄悬空。
fn rewire_model_refs(state: &AppState, provider_id: &str, old_id: &str, new_id: &str) -> Result<(), String> {
    let old_ref = format!("{provider_id}/{old_id}");
    let new_ref = format!("{provider_id}/{new_id}");
    let mut merged: Settings =
        serde_json::from_value(settings_get(state)?).unwrap_or_else(|_| default_settings());
    let mut changed = false;
    for slot in [&mut merged.default_model, &mut merged.fallback_model] {
        if *slot == old_ref {
            *slot = new_ref.clone();
            changed = true;
        }
    }
    if changed {
        state
            .db
            .with(|conn| set_setting(conn, "app", &serde_json::to_value(&merged).unwrap_or(Value::Null)))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn provider_remove_model(state: &AppState, provider_id: &str, model_id: &str) -> SvcResult {
    let mut providers = load_providers(state);
    let provider = providers.iter_mut().find(|p| p.id == provider_id).ok_or("提供商不存在")?;
    let before = provider.models.len();
    provider.models.retain(|m| m.id != model_id);
    if provider.models.len() == before {
        return err("模型不存在");
    }
    save_providers(state, &providers)?;
    provider_list(state)
}

/// 拉取模型（spec §7）：按协议 GET 端点模型列表并按 ID 去重合并（source=fetched）。
pub fn provider_fetch_models(state: &AppState, provider_id: &str) -> SvcResult {
    let mut providers = load_providers(state);
    let Some(provider) = providers.iter_mut().find(|p| p.id == provider_id) else {
        return err("提供商不存在");
    };
    let env_refs: Vec<&str> = provider.env_vars.iter().map(String::as_str).collect();
    let key = state.secrets.get_for(&provider.id, &env_refs);
    let endpoint = provider.endpoint.trim_end_matches('/').to_string();
    if endpoint.is_empty() {
        return err("提供商未配置服务地址");
    }
    let url = if provider.kind == "anthropic" {
        format!("{endpoint}/v1/models")
    } else {
        format!("{endpoint}/models")
    };
    let client = proxy_config(state)
        .apply_reqwest_blocking(
            reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20)),
        )
        .build()
        .map_err(|e| format!("网络客户端初始化失败：{e}"))?;
    let mut req = client.get(&url);
    if provider.kind == "anthropic" {
        req = req.header("x-api-key", &key).header("anthropic-version", "2023-06-01");
    } else if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    // {sessionId} 占位符无会话上下文可替换，跳过不发（避免把字面占位符发给网关）
    for h in &provider.headers {
        if craft_model::types::header_has_session_placeholder(&h.value) {
            continue;
        }
        req = req.header(h.name.as_str(), h.value.as_str());
    }
    let resp = req.send().map_err(|e| format!("拉取失败：{}", network_error_text(&e)))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("拉取失败：HTTP {}", status.as_u16()));
    }
    let body: Value = resp.json().map_err(|e| format!("响应解析失败：{e}"))?;
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut fetched = 0usize;
    let mut added: Vec<String> = Vec::new();
    for item in &items {
        let Some(id) = item.get("id").and_then(Value::as_str) else { continue };
        fetched += 1;
        if provider.models.iter().any(|m| m.id == id) {
            continue;
        }
        provider.models.push(craft_model::types::PersistedModel {
            id: id.to_string(),
            name: id.to_string(),
            context_k: 0,
            max_tokens: 0,
            caps: craft_model::types::DEFAULT_CAPS,
            cost: None,
            status: String::new(),
            canonical_id: String::new(),
            release_date: String::new(),
            source: "fetched".into(),
            enabled: true,
            reasoning_kind: String::new(),
            reasoning_levels: Vec::new(),
        });
        added.push(id.to_string());
    }
    save_providers(state, &providers)?;
    Ok(json!({ "fetched": fetched, "added": added, "models": provider_models_json(state, provider_id)? }))
}

fn provider_models_json(state: &AppState, provider_id: &str) -> Result<Value, String> {
    let list = provider_list(state)?;
    let Some(providers) = list.as_array() else { return Ok(json!([])) };
    for p in providers {
        if p["id"] == json!(provider_id) {
            return Ok(p["models"].clone());
        }
    }
    Ok(json!([]))
}

// ---------- 套餐额度查询（决策 D34，crates/craft-model/src/quota.rs 模块头为协议口径） ----------
//
// 行为契约：
// - 识别：provider endpoint 主机白名单（智谱 bigmodel.cn / z.ai；OpenCode Go 须含 /go 段），
//   白名单外（第三方中转）一律不查——额度请求发往固定官方 URL，key 只发给 key 所属服务；
// - 密钥：state.secrets.get_for（配置 key 优先，env 变量回落）；无 key 返回 null 不发请求；
// - 缓存：进程内 30s TTL（QUOTA_CACHE_TTL），多窗口挂载 + 60s 轮询不放大上游请求量；
//   仅缓存 Ok 结果，错误（含 null 语义的「无套餐」）不缓存，下轮轮询自然重试；
// - null = 白名单外 / 无密钥 / 无套餐（OpenCode Go 403 与智谱「CodingPlan 不存在」信封
//   都归入此类），渲染层据此隐藏 chip；其余失败抛可读中文错误（瞬时网络问题）。

/// 单家额度结果缓存 TTL。
const QUOTA_CACHE_TTL: Duration = Duration::from_secs(30);

/// 进程内额度缓存：providerId → (取数时间, Ok 结果)。模块级而非 AppState 字段：
/// 纯展示缓存，重启即失效，不值得动装配代码；不变量由「只在持锁时读写」保证。
static QUOTA_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Value)>>> = OnceLock::new();

fn quota_cache() -> &'static Mutex<HashMap<String, (Instant, Value)>> {
    QUOTA_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 缓存读：命中且未过期时返回 Some；过期条目顺手清掉。
fn quota_cache_get(provider_id: &str) -> Option<Value> {
    let mut cache = quota_cache().lock().ok()?;
    if let Some((at, value)) = cache.get(provider_id) {
        if at.elapsed() < QUOTA_CACHE_TTL {
            return Some(value.clone());
        }
        cache.remove(provider_id);
    }
    None
}

/// 缓存写（仅 Ok 结果）。
fn quota_cache_put(provider_id: &str, value: &Value) {
    if let Ok(mut cache) = quota_cache().lock() {
        cache.insert(provider_id.to_string(), (Instant::now(), value.clone()));
    }
}

pub fn provider_quota(state: &AppState, provider_id: &str) -> SvcResult {
    if let Some(hit) = quota_cache_get(provider_id) {
        return Ok(hit);
    }
    let providers = load_providers(state);
    let Some(provider) = providers.iter().find(|p| p.id == provider_id) else {
        return Ok(json!(null));
    };
    let Some(flavor) = craft_model::quota::detect_flavor(&provider.endpoint) else {
        return Ok(json!(null));
    };
    let env_refs: Vec<&str> = provider.env_vars.iter().map(String::as_str).collect();
    let key = state.secrets.get_for(&provider.id, &env_refs);
    if key.is_empty() {
        return Ok(json!(null));
    }
    let windows = fetch_quota_windows(state, flavor, &key)?;
    let view = json!({
        "providerId": provider.id,
        "providerName": provider.name,
        "windows": windows,
        "balanceText": Value::Null,
        "fetchedAt": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    quota_cache_put(provider_id, &view);
    Ok(view)
}

/// 按提供商族发 GET 并解析窗口：错误分类见 provider_quota 头注释（403/无套餐信封
/// 归 null；解析失败与业务错误归可读 Err）。
fn fetch_quota_windows(
    state: &AppState,
    flavor: craft_model::quota::Flavor,
    key: &str,
) -> Result<Vec<craft_model::quota::QuotaWindow>, String> {
    let client = proxy_config(state)
        .apply_reqwest_blocking(
            reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)),
        )
        .build()
        .map_err(|e| format!("网络客户端初始化失败：{e}"))?;
    let resp = client
        .get(flavor.usage_url())
        .header("Authorization", flavor.auth_header(key))
        .header("Accept", "application/json")
        .send()
        .map_err(|e| format!("额度查询失败：{}", network_error_text(&e)))?;
    let status = resp.status();
    let body = resp.text().map_err(|e| format!("额度查询失败：{e}"))?;
    // 无套餐类：OpenCode Go 403 = key 有效（Zen/Go 共用 workspace key）但无 Go 订阅；
    // 智谱无套餐以 success=false 信封表达，交给解析层识别。
    if flavor == craft_model::quota::Flavor::OpenCodeGo && status.as_u16() == 403 {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        return Err(format!("额度查询失败：HTTP {}", status.as_u16()));
    }
    let parsed: Value =
        serde_json::from_str(&body).map_err(|_| "额度响应不是合法 JSON".to_string())?;
    let windows = match flavor {
        craft_model::quota::Flavor::OpenCodeGo => craft_model::quota::parse_opencode_go(&parsed)?,
        craft_model::quota::Flavor::Zhipu | craft_model::quota::Flavor::Zai => {
            // Ok(None) = 无套餐信封：与白名单外/无密钥同语义（null → chip 隐藏）
            match craft_model::quota::parse_zhipu_limits(&parsed)? {
                Some(windows) => windows,
                None => return Ok(Vec::new()),
            }
        }
    };
    Ok(windows)
}

/// 密钥提示：密钥来源为环境变量的提供商（环境缺失后不可用的提醒）。
pub fn provider_warnings(state: &AppState) -> SvcResult {
    let providers = load_providers(state);
    let mut warnings: Vec<String> = Vec::new();
    for p in providers {
        if !p.key_required {
            continue;
        }
        let env_refs: Vec<&str> = p.env_vars.iter().map(String::as_str).collect();
        if state.secrets.source_for(&p.id, &env_refs) == "env" {
            warnings.push(format!(
                "{} 的密钥来自环境变量：重启后若环境变量缺失，该提供商将不可用。",
                p.name
            ));
        }
    }
    Ok(json!(warnings))
}

// ---------- MCP（rmcp 客户端接线，D16） ----------

/// MCP 配置的持久化键（settings 表）。值为 `Vec<ServerConfig>` 的 JSON。
const MCP_SERVERS_KEY: &str = "mcp.servers";

/// 读配置列表（服务层与启动装配共用）。
pub fn mcp_configs(state: &AppState) -> Vec<craft_mcp::ServerConfig> {
    state
        .db
        .with(|conn| get_setting(conn, MCP_SERVERS_KEY))
        .ok()
        .flatten()
        .map(|v| craft_mcp::config::parse_servers(&v))
        .unwrap_or_default()
}

/// 写配置列表（先同步到池内缓存）。
fn mcp_store_configs(state: &AppState, configs: &[craft_mcp::ServerConfig]) -> Result<(), String> {
    let value: Vec<Value> = configs.iter().map(craft_mcp::config::server_to_json).collect();
    state
        .db
        .with(|conn| set_setting(conn, MCP_SERVERS_KEY, &Value::Array(value)))
        .map_err(|e| e.to_string())?;
    state.mcp_pool.set_configs(configs.to_vec());
    Ok(())
}

/// 配置 + 运行态合并视图（设置页列表）。
///
/// 敏感值不回传：env / headers 只给条数，凭据值只存在本地库（与网络代理同处理，
/// 见 D16），进了前端就等于进了截图与剪贴板。
pub fn mcp_list(state: &AppState) -> SvcResult {
    let configs = mcp_configs(state);
    // 池内缓存与库同步（应用启动后第一次 list 时把库内容喂给池）
    state.mcp_pool.set_configs(configs.clone());
    let runtimes = state.mcp_pool.all_runtimes();
    let views: Vec<Value> = configs
        .into_iter()
        .map(|config| {
            let runtime = runtimes.get(&config.id).cloned().unwrap_or_default();
            json!({
                "id": config.id,
                "name": config.name,
                "transport": config.transport.as_str(),
                "command": config.command,
                "args": config.args,
                "cwd": config.cwd,
                "url": config.url,
                "enabled": config.enabled,
                "timeoutMs": config.timeout_ms,
                "envCount": config.env.len(),
                "headerCount": config.headers.len(),
                "status": runtime.status.as_str(),
                "toolCount": runtime.tool_count,
                "error": runtime.error,
                "serverName": runtime.server_name,
                "serverVersion": runtime.server_version,
            })
        })
        .collect();
    Ok(json!(views))
}

/// 保存（新增或编辑）一个 MCP 服务器。
///
/// `server` 是渲染层传来的 JSON：编辑时必须带 `id`，新增时由服务端生成。
/// 保存后立即应用（连接或断开），返回合并视图列表。
pub fn mcp_upsert(state: &AppState, server: &Value) -> SvcResult {
    let mut configs = mcp_configs(state);
    let name = server.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    let transport = craft_mcp::Transport::parse(server.get("transport").and_then(Value::as_str).unwrap_or(""))
        .unwrap_or_else(|| {
            // 缺省按目标字段推断（与配置读取侧同一规则）
            if !server.get("command").and_then(Value::as_str).unwrap_or_default().trim().is_empty() {
                craft_mcp::Transport::Stdio
            } else {
                craft_mcp::Transport::Http
            }
        });
    let pairs = |key: &str| -> Option<Vec<(String, String)>> {
        server.get(key).and_then(Value::as_object).map(|map| {
            map.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect()
        })
    };
    let existing_id = server.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let id = if existing_id.is_empty() {
        craft_core::ids::new_id(craft_core::ids::id_prefix::MCP_SERVER)
    } else {
        existing_id
    };
    // 编辑时的旧配置：凭据不回传前端（规格第 9 节），前端也就无法把它原样送回来。
    // 因此「字段缺失 = 保留旧值」（显式传 {} 才清空），否则改一句名字就把 env/headers/cwd
    // 全冲掉，停用的服务器还会被静默启用。
    let previous = configs.iter().find(|c| c.id == id).cloned();

    let mut config = craft_mcp::ServerConfig {
        id: id.clone(),
        name,
        transport,
        command: server.get("command").and_then(Value::as_str).unwrap_or_default().to_string(),
        args: server
            .get("args")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        env: pairs("env").or_else(|| previous.as_ref().map(|c| c.env.clone())).unwrap_or_default(),
        cwd: match server.get("cwd").and_then(Value::as_str) {
            Some(cwd) => cwd.to_string(),
            None => previous.as_ref().map(|c| c.cwd.clone()).unwrap_or_default(),
        },
        url: server.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
        headers: pairs("headers").or_else(|| previous.as_ref().map(|c| c.headers.clone())).unwrap_or_default(),
        timeout_ms: server.get("timeoutMs").and_then(Value::as_i64).unwrap_or(craft_mcp::config::DEFAULT_TIMEOUT_MS),
        enabled: server
            .get("enabled")
            .and_then(Value::as_bool)
            .or_else(|| previous.as_ref().map(|c| c.enabled))
            .unwrap_or(true),
    };
    config = config.normalize()?;

    // 名称唯一：与**其他**服务器重名即拒绝（编辑自己允许）
    if configs.iter().any(|c| c.name == config.name && c.id != config.id) {
        return err(format!("名称「{}」已被其他服务器使用", config.name));
    }

    let is_new = !configs.iter().any(|c| c.id == config.id);
    if is_new {
        configs.push(config.clone());
    } else {
        let index = configs.iter().position(|c| c.id == config.id).expect("已在上方确认存在");
        configs[index] = config.clone();
    }
    mcp_store_configs(state, &configs)?;

    // 立即应用：启用则连接并注册工具，停用则断开
    mcp_apply(state, &config);
    mcp_list(state)
}

/// 启用/停用一个服务器（开关切换即连接/断开，工具随之注册/注销）。
pub fn mcp_set_enabled(state: &AppState, server_id: &str, enabled: bool) -> SvcResult {
    let mut configs = mcp_configs(state);
    {
        let Some(config) = configs.iter_mut().find(|c| c.id == server_id) else {
            return err("服务器不存在或已删除");
        };
        config.enabled = enabled;
    }
    mcp_store_configs(state, &configs)?;
    if enabled {
        if let Some(config) = configs.into_iter().find(|c| c.id == server_id) {
            mcp_apply(state, &config);
        }
    } else {
        state.mcp_pool.disconnect(server_id);
        state.mcp_pool.mark_disabled(server_id);
        sync_mcp_tools(state);
    }
    mcp_list(state)
}

/// 删除一个服务器（先断开再删配置）。
pub fn mcp_remove(state: &AppState, server_id: &str) -> SvcResult {
    let mut configs = mcp_configs(state);
    if !configs.iter().any(|c| c.id == server_id) {
        return err("服务器不存在或已删除");
    }
    configs.retain(|c| c.id != server_id);
    mcp_store_configs(state, &configs)?;
    state.mcp_pool.disconnect(server_id);
    sync_mcp_tools(state);
    mcp_list(state)
}

/// 手动重连（列表行点击或测试入口）。
pub fn mcp_connect(state: &AppState, server_id: &str) -> SvcResult {
    let Some(config) = mcp_configs(state).into_iter().find(|c| c.id == server_id) else {
        return err("服务器不存在或已删除");
    };
    let result = state.mcp_pool.connect(&config);
    sync_mcp_tools(state);
    if let Err(message) = result {
        return err(message);
    }
    mcp_list(state)
}

/// 手动断开。
pub fn mcp_disconnect(state: &AppState, server_id: &str) -> SvcResult {
    state.mcp_pool.disconnect(server_id);
    sync_mcp_tools(state);
    mcp_list(state)
}

/// 测试连接：临时连接取工具清单（不注册工具、不改配置），返回服务器信息与工具表。
pub fn mcp_test(state: &AppState, server: &Value) -> SvcResult {
    // 复用 upsert 的归一逻辑，但不落库：构造临时配置（id 用 test 前缀避免撞池内缓存）
    let view = mcp_upsert_inner_test_config(state, server)?;
    let started = std::time::Instant::now();
    let result = state.mcp_pool.connect(&view);
    let elapsed = started.elapsed().as_millis() as i64;
    match result {
        Ok(snapshot) => {
            // 测试完立即断开：测试连接不占用长驻子进程
            state.mcp_pool.disconnect(&view.id);
            Ok(json!({
                "success": true,
                "elapsedMs": elapsed,
                "serverName": snapshot.server_name,
                "serverVersion": snapshot.server_version,
                "protocolVersion": snapshot.protocol_version,
                "tools": snapshot.tools.iter().map(|t| json!({
                    "name": t.tool_name,
                    "description": t.description,
                })).collect::<Vec<_>>(),
            }))
        }
        Err(message) => Ok(json!({
            "success": false,
            "elapsedMs": elapsed,
            "error": message,
        })),
    }
}

/// mcp_test 的辅助：把渲染层 JSON 归一成 ServerConfig（不落库，id 固定为 mcp_test）。
fn mcp_upsert_inner_test_config(state: &AppState, server: &Value) -> Result<craft_mcp::ServerConfig, String> {
    let _ = state;
    let name = server.get("name").and_then(Value::as_str).unwrap_or("test").to_string();
    let transport = craft_mcp::Transport::parse(server.get("transport").and_then(Value::as_str).unwrap_or(""))
        .unwrap_or_else(|| {
            if !server.get("command").and_then(Value::as_str).unwrap_or_default().trim().is_empty() {
                craft_mcp::Transport::Stdio
            } else {
                craft_mcp::Transport::Http
            }
        });
    let pairs = |key: &str| -> Vec<(String, String)> {
        server
            .get(key)
            .and_then(Value::as_object)
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
            .unwrap_or_default()
    };
    craft_mcp::ServerConfig {
        id: "mcp_test".into(),
        name,
        transport,
        command: server.get("command").and_then(Value::as_str).unwrap_or_default().to_string(),
        args: server
            .get("args")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        env: pairs("env"),
        cwd: server.get("cwd").and_then(Value::as_str).unwrap_or_default().to_string(),
        url: server.get("url").and_then(Value::as_str).unwrap_or_default().to_string(),
        headers: pairs("headers"),
        timeout_ms: server.get("timeoutMs").and_then(Value::as_i64).unwrap_or(craft_mcp::config::DEFAULT_TIMEOUT_MS),
        enabled: true,
    }
    .normalize()
}

/// 按配置把一个服务器应用起来：启用 → 连接；失败状态留在池里（设置页显示红灯）。
pub fn mcp_apply(state: &AppState, config: &craft_mcp::ServerConfig) {
    if !config.enabled {
        state.mcp_pool.disconnect(&config.id);
        state.mcp_pool.mark_disabled(&config.id);
    } else {
        let _ = state.mcp_pool.connect(config);
    }
    sync_mcp_tools(state);
}

/// 把池内全部已连接服务器的工具注册进 Registry（全量重建）。
///
/// 由 McpTool 借用池与工具描述符；注册名就是模型可见名（`mcp__…`）。
/// 规划逻辑见 [`plan_mcp_tools`]（纯函数，可独立测试），这里只负责注册与注销。
fn sync_mcp_tools(state: &AppState) {
    sync_mcp_tools_into(&state.registry, &state.mcp_pool);
}

/// 全量重建 MCP 工具：先注销全部 `mcp__` 工具，再按当前连接注册。
///
/// 为什么全量重建而不是增量比对：数量少（几十个量级）、调用频率低（保存设置时），
/// 全量重建比增量比对简单且不会留下状态。
fn sync_mcp_tools_into(registry: &craft_tools::registry::Registry, pool: &Arc<craft_mcp::McpPool>) {
    let stale: Vec<String> =
        registry.specs().into_iter().filter(|s| s.name.starts_with("mcp__")).map(|s| s.name).collect();
    for name in stale {
        registry.unregister(&name);
    }
    for planned in plan_mcp_tools(&connected_by_server(pool)) {
        let tool = craft_tools::mcp::McpTool::new(
            planned.server_id,
            planned.server_name,
            planned.remote,
            planned.timeout_ms,
            planned.descriptor,
            Arc::clone(pool),
        );
        registry.register(Box::new(tool));
    }
}

/// 池内已连接工具按服务器分组（配置 + 工具描述符）。
///
/// 一次取全量再分组，避免「每个工具都重查一遍 connected_tools」的 O(n²)
/// （旧实现在注册循环里为算单服务器工具数反复全量扫描）。
fn connected_by_server(pool: &Arc<craft_mcp::McpPool>) -> Vec<(craft_mcp::ServerConfig, Vec<craft_mcp::ToolDescriptor>)> {
    let mut grouped: std::collections::BTreeMap<String, Vec<craft_mcp::ToolDescriptor>> =
        std::collections::BTreeMap::new();
    for (server_id, descriptor) in pool.connected_tools() {
        grouped.entry(server_id).or_default().push(descriptor);
    }
    grouped
        .into_iter()
        // 配置已删除但连接还挂着的服务器不注册（工具名要以配置名为准）
        .filter_map(|(server_id, descriptors)| pool.config(&server_id).map(|config| (config, descriptors)))
        .collect()
}

/// 一次注册决策：该注册哪些工具、用什么名字。
///
/// `pub` 而非 `pub(crate)`：集成测试（`src-tauri/tests/`）是独立 crate，需要直接验证规划结果
/// （上限截断、撞名 fail closed、名字来源），而完整走一遍「真连接 + 注册」代价过高。
pub struct PlannedMcpTool {
    pub server_id: String,
    /// 服务器配置名（工具名第一段来源，也是审批理由与标题里的名字）
    pub server_name: String,
    pub remote: bool,
    pub timeout_ms: i64,
    pub descriptor: craft_mcp::ToolDescriptor,
    /// 即将注册的模型可见名（判重用，也是 McpTool 构造后的 `model_name()`）
    pub model_name: String,
}

/// 规划全量注册：每服务器最多 [`craft_tools::mcp::MAX_TOOLS_PER_SERVER`] 个工具；
/// 归一后撞名 fail closed（后来者不注册，见 D16）。
///
/// 名字第一段取**配置名**而不是服务器自述名：自述名随服务器版本变化会让模型工具名漂移，
/// 而授权缓存（`Grants` 按 `tool|target` 建键）与历史重建都依赖它稳定。
pub fn plan_mcp_tools(servers: &[(craft_mcp::ServerConfig, Vec<craft_mcp::ToolDescriptor>)]) -> Vec<PlannedMcpTool> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut planned: Vec<PlannedMcpTool> = Vec::new();
    for (config, descriptors) in servers {
        for descriptor in descriptors.iter().take(craft_tools::mcp::MAX_TOOLS_PER_SERVER) {
            let model_name = craft_tools::mcp::model_tool_name(&config.name, &descriptor.tool_name);
            if !seen.insert(model_name.clone()) {
                // 归一后撞名：第二个不注册，而不是静默覆盖
                continue;
            }
            planned.push(PlannedMcpTool {
                server_id: config.id.clone(),
                server_name: config.name.clone(),
                remote: config.transport.is_remote(),
                timeout_ms: config.timeout_ms,
                descriptor: descriptor.clone(),
                model_name,
            });
        }
    }
    planned
}

// ---------- Skills（技能，决策 D11；插件技能接入见 D12） ----------

/// 管理与扫描共用的插件技能根：已启用插件按安装顺序（skill_plugins 表即时读取，
/// 停用/卸载插件下一轮生效）。
fn plugin_skill_roots(state: &AppState) -> Vec<craft_tools::skills::PluginSkillRoot> {
    craft_tools::skills::load_plugin_skill_roots(&state.db)
}

/// 管理视角的技能根内路径校验：realpath 落在技能根之一内（用户级 + 指定工作区级 +
/// 插件缓存根）、且是 SKILL.md 文件。管理操作（启停/删除/读取）的唯一入口校验，
/// 防误操作任意路径。校验通过返回插件归属：`Some(插件名)` = 插件缓存域（缓存只读，
/// 删除类操作按来源拒绝），`None` = 手工技能根。
fn ensure_skill_managed_path(state: &AppState, path: &str, workspace: &str) -> Result<Option<String>, String> {
    if path.trim().is_empty() {
        return Err("技能路径不能为空".into());
    }
    let target = std::path::Path::new(path);
    let target_canon = target.canonicalize().map_err(|_| "技能路径不存在".to_string())?;
    if !target_canon.is_file() || !target_canon.file_name().is_some_and(|n| n == craft_tools::skills::SKILL_FILE_NAME) {
        return Err("技能路径不是有效的 SKILL.md".into());
    }
    for root in craft_tools::skills::skill_roots(&state.data_dir, workspace, false, &plugin_skill_roots(state)) {
        if let Ok(root_canon) = root.dir.canonicalize()
            && target_canon.starts_with(&root_canon)
        {
            return Ok(if root.plugin.is_empty() { None } else { Some(root.plugin) });
        }
    }
    Err("路径不在技能目录内，拒绝操作".into())
}

/// 管理页传 workspaceId：取工作区主目录（folders 首条）；空串 = 只操作用户级技能。
fn managed_workspace_path(state: &AppState, workspace_id: &str) -> String {
    if workspace_id.is_empty() {
        return String::new();
    }
    let conn = state.db.lock_conn();
    workspaces::get_workspace(&conn, workspace_id)
        .ok()
        .and_then(|ws| ws.folders.first().map(|f| f.path.clone()))
        .unwrap_or_default()
}

/// 扫描技能列表（管理页全量视角）：用户级 + 指定工作区级 + 插件缓存根。不做受限过滤
/// ——受限工作区约束的是 Agent 自动加载（run 注入按 env.restricted 过滤），用户管理
/// 视角可见全部。插件技能以限定名（`插件名:技能名`）出现，启停按 SKILL.md 绝对路径
/// 沿用 skills.disabled。
pub fn skills_list(state: &AppState, workspace_id: &str) -> SvcResult {
    let workspace = managed_workspace_path(state, workspace_id);
    let (_enabled, disabled) = craft_tools::skills::load_skills_config(&state.db);
    let roots = craft_tools::skills::skill_roots(&state.data_dir, &workspace, false, &plugin_skill_roots(state));
    let metas = craft_tools::skills::discover_skills(&roots, &disabled);
    serde_json::to_value(&metas).map_err(|e| e.to_string())
}

/// 管理页全量扫描：用户级（含插件缓存根）+ 全部工作区级，每条附带归属
/// （workspaceId / workspaceName）。用户级条目两字段为空串。不做受限过滤
/// （与 [`skills_list`] 同一视角约定）：受限工作区的技能在管理页可见，Agent 自动
/// 加载仍由 run 注入侧过滤。
pub fn skills_list_all(state: &AppState) -> SvcResult {
    let (_enabled, disabled) = craft_tools::skills::load_skills_config(&state.db);
    let list = workspaces::list_workspaces(&state.db.lock_conn()).map_err(|e| e.to_string())?;
    let scopes: Vec<(String, String, String)> = list
        .iter()
        .map(|w| {
            let main = w.folders.first().map(|f| f.path.clone()).unwrap_or_else(|| w.workspace.path.clone());
            (w.workspace.id.clone(), w.workspace.name.clone(), main)
        })
        .collect();
    let pairs: Vec<(String, String)> = scopes.iter().map(|(id, _, main)| (id.clone(), main.clone())).collect();
    let found = craft_tools::skills::discover_all_scopes(&state.data_dir, &pairs, &disabled, &plugin_skill_roots(state));
    let name_of = |id: &str| -> String {
        scopes.iter().find(|(wid, _, _)| wid == id).map(|(_, n, _)| n.clone()).unwrap_or_default()
    };
    let views: Vec<Value> = found
        .into_iter()
        .map(|(workspace_id, meta)| {
            let mut value = serde_json::to_value(&meta).map_err(|e| e.to_string())?;
            if let Some(obj) = value.as_object_mut() {
                obj.insert("workspaceId".into(), json!(workspace_id));
                obj.insert("workspaceName".into(), json!(name_of(&workspace_id)));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!(views))
}

/// 启停单个技能：enabled=true 从禁用列表移除，false 加入；返回刷新后的列表。
/// 插件缓存技能同样走此通道（skills.disabled 路径数组天然兼容），下一轮对话生效。
pub fn skills_set_enabled(state: &AppState, path: &str, enabled: bool, workspace_id: &str) -> SvcResult {
    let workspace = managed_workspace_path(state, workspace_id);
    ensure_skill_managed_path(state, path, &workspace)?;
    let (_enabled, mut disabled) = craft_tools::skills::load_skills_config(&state.db);
    if enabled {
        disabled.retain(|p| p != path);
    } else if !disabled.iter().any(|p| p == path) {
        disabled.push(path.to_string());
    }
    craft_tools::skills::save_disabled_paths(&state.db, &disabled)?;
    skills_list(state, workspace_id)
}

/// 删除技能目录（SKILL.md 所在目录），并清理禁用列表残留；返回刷新后的列表。
/// 插件缓存技能拒绝删除（缓存是只读产物，删目录会破坏版本缓存完整性——升级/回滚
/// 会坏），引导走商店卸载或单技能禁用。
pub fn skills_remove(state: &AppState, path: &str, workspace_id: &str) -> SvcResult {
    let workspace = managed_workspace_path(state, workspace_id);
    if let Some(plugin) = ensure_skill_managed_path(state, path, &workspace)? {
        return err(format!(
            "插件「{plugin}」的技能是商店缓存（只读），不能在此删除；如需移除请在技能商店卸载该插件，或仅禁用该技能"
        ));
    }
    let target = std::path::Path::new(path);
    let dir = target.parent().unwrap_or(target);
    std::fs::remove_dir_all(dir).map_err(|e| format!("技能目录删除失败：{e}"))?;
    let (_enabled, mut disabled) = craft_tools::skills::load_skills_config(&state.db);
    disabled.retain(|p| p != path);
    craft_tools::skills::save_disabled_paths(&state.db, &disabled)?;
    skills_list(state, workspace_id)
}

/// 读取技能正文（管理页预览）：剥 frontmatter 与目录变量替换后的展示文本。
pub fn skills_read(state: &AppState, path: &str, workspace_id: &str) -> SvcResult {
    let workspace = managed_workspace_path(state, workspace_id);
    ensure_skill_managed_path(state, path, &workspace)?;
    let dir = std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let content = craft_tools::skills::load_skill_body(path, &dir)?;
    Ok(json!({ "content": content }))
}

/// 斜杠技能调用检测：`/名称 [参数]`，名称与可用技能清单匹配时返回技能名（插件技能
/// 的限定名与裸名别名都接受，见 craft_tools::skills::find_skill）。总开关关闭或扫描
/// 失败一律返回 None（按普通消息发送）——检测失败不阻塞发送。
fn detect_skill_invoke(state: &AppState, thread_id: &str, text: &str) -> Option<String> {
    let (name, _) = craft_tools::skills::parse_slash_invocation(text)?;
    let workspace = thread_workspace_path(state, thread_id);
    // 受限工作区不在此限制：显式斜杠调用是用户主动行为，加载本身 R0 且后续命令仍走审批
    let available = craft_tools::skills::scan_available(&state.db, &state.data_dir, &workspace, false);
    craft_tools::skills::find_skill(&available, &name).map(|_| name)
}

/// 会话当前主要目录：绑定 worktree 时用工作树路径（与 RunEnv 语义一致），否则工作区主目录。
/// 任何一步失败回退空串（斜杠检测退化为只扫用户级技能）。
fn thread_workspace_path(state: &AppState, thread_id: &str) -> String {
    let conn = state.db.lock_conn();
    let Ok(thread) = threads::get_thread(&conn, thread_id) else { return String::new() };
    if !thread.worktree_path.is_empty() {
        return thread.worktree_path;
    }
    workspaces::get_workspace(&conn, &thread.workspace_id)
        .ok()
        .and_then(|ws| ws.folders.first().map(|f| f.path.clone()))
        .unwrap_or_default()
}

// ---------- 技能商店（决策 D12；通道见 spec 第 11 节） ----------
//
// 服务层只做参数校验、编排与错误转换：市场源管理 / 安装 / 升级 / 回滚 / 卸载 / 启停 /
// 更新检测全部落在 craft-marketplace 的 Orchestrator（应用内互斥与审计在其内部完成）。
// 同步领域操作在本层直接执行——dispatch 已统一把同步服务放 spawn_blocking（lib.rs），
// 此处不再自行包一层（与 skills_* 服务同一惯例）。

/// 商店布局根（spec 第 3 节：`<数据目录>/skills-marketplace`，随 CRAFTWORK_HOME 隔离）。
fn marketplace_layout(state: &AppState) -> craft_marketplace::cache::MarketplaceLayout {
    craft_marketplace::cache::MarketplaceLayout::new(Path::new(&state.data_dir).join("skills-marketplace"))
}

/// 商店下载网络配置（spec 第 7 节）：settings 即时读出 → craft-core::network 解析 →
/// NetworkOptions。代理出口与模型调用同源（[`proxy_config`]）；GitHub 加速前缀按设置
/// 开关透传（关闭时空串，传输层按空前缀跳过改写）。数据目录根从 AppState 取。
fn store_network_options(state: &AppState) -> craft_marketplace::orchestrator::NetworkOptions {
    let settings = serde_json::from_value::<Settings>(settings_get(state).unwrap_or(Value::Null)).unwrap_or_else(|_| default_settings());
    let prefix = if settings.network_github_accelerator_enabled {
        craft_core::network::normalize_accelerator_prefix(&settings.network_github_accelerator_prefix)
    } else {
        String::new()
    };
    craft_marketplace::orchestrator::NetworkOptions {
        proxy: Some(proxy_config(state).gix_proxy_options()),
        accelerator_prefix: Some(prefix),
        ..Default::default()
    }
}

/// 商店编排器就地构造宏：布局与网络配置都是 Orchestrator 借用对象（Orchestrator<'a>
/// 借用布局），必须在调用方作用域用 let 绑定持有，不能包进返回函数（临时值即销毁）。
/// 网络配置每次组装即读设置：保存即生效。
macro_rules! store_orchestrator {
    ($state:expr, $name:ident) => {
        let store_layout = marketplace_layout($state);
        let $name = craft_marketplace::orchestrator::Orchestrator::with_network(
            &store_layout,
            &$state.db,
            store_network_options($state),
        );
    };
}

/// 市场源输入归一（spec 第 2 节添加方式）：仅接受 `https://` 仓库 URL 或 `owner/repo`
/// 缩写（展开为 GitHub URL）。本地目录路径与 file:// 报「暂不支持」；其余形态报可读
/// 中文错误。返回展开后的 git 仓库 URL。
fn normalize_market_input(raw: &str) -> Result<String, String> {
    let input = raw.trim();
    if input.is_empty() {
        return err("市场源输入不能为空");
    }
    if let Some(rest) = input.strip_prefix("https://") {
        let host = rest.split('/').next().unwrap_or_default();
        if host.is_empty() {
            return err("仓库地址不完整：https:// 后缺少主机名");
        }
        return Ok(input.to_string());
    }
    if input.starts_with("http://") || input.starts_with("file://") || input.starts_with('/') {
        return err("本地目录与 file:// 源暂不支持，请使用 https:// 仓库地址");
    }
    // owner/repo 缩写：恰好一段 `/`、两端非空、不含空白与 `..`
    if !input.contains("://")
        && !input.contains(char::is_whitespace)
        && !input.contains('\\')
        && input.chars().filter(|c| *c == '/').count() == 1
        && let Some((owner, repo)) = input.split_once('/')
        && !owner.is_empty()
        && !repo.is_empty()
        && owner != ".."
        && repo != ".."
        && !repo.contains("..")
    {
        return Ok(format!("https://github.com/{input}"));
    }
    err("市场源格式不支持：请输入 https:// 仓库地址或 owner/repo 缩写")
}

/// 插件 id（`name@marketplace`）拆解为（插件名, 市场 id）：从最后一个 `@` 拆——市场 id
/// 可能含 `@` 的形态按「右侧为市场」归一（与 store::plugin_id 的构造方向一致）。
fn split_plugin_id(id: &str) -> Result<(String, String), String> {
    match id.trim().rsplit_once('@') {
        Some((name, marketplace_id)) if !name.is_empty() && !marketplace_id.is_empty() => {
            Ok((name.to_string(), marketplace_id.to_string()))
        }
        _ => err(format!("插件标识不合法：{id}（应为 名称@市场id）")),
    }
}

/// 商店插件视图（`storeListPlugins` / 商店各变更通道的返回载荷）。
///
/// 已装 / 未装统一形状：未装插件 `installed=false`，`version` 为市场侧解析版本（可能
/// 空串），无安装侧字段；已装插件带版本历史与来源 pin，更新状态由双轴检测计算。
fn store_plugin_view(
    entry: &craft_marketplace::manifest::MarketEntry,
    marketplace_id: &str,
    plugin: Option<&craft_marketplace::store::PluginRow>,
    update: Option<&craft_marketplace::orchestrator::PluginUpdateStatus>,
) -> Value {
    let installed_version = plugin.map(|p| p.current_version.clone());
    json!({
        "id": plugin.map(|p| p.id.clone()).unwrap_or_else(|| craft_marketplace::store::plugin_id(&entry.name, marketplace_id)),
        "marketplaceId": marketplace_id,
        "name": entry.name,
        "description": entry.description.clone().unwrap_or_default(),
        "category": entry.category.clone().unwrap_or_default(),
        "tags": entry.tags.clone(),
        // 已装 = 当前版本指针；未装 = 市场侧可用版本（清单 version > 条目 version，可能缺）
        "version": installed_version.or_else(|| entry.version.clone()).unwrap_or_default(),
        "installed": plugin.is_some(),
        "enabled": plugin.map(|p| p.enabled),
        // 版本历史（磁盘保留，含当前）
        "versions": plugin.map(|p| p.versions.clone()).unwrap_or_default(),
        // 来源 pin（安装时的 git sha 短码，双轴检测的 sha 轴）
        "sourceSha": plugin.and_then(|p| p.source_sha.clone()).unwrap_or_default(),
        "installedAt": plugin.map(|p| p.installed_at).unwrap_or(0),
        // 更新状态三态（spec 第 4 节）：none / update-available / version-changed；未装恒 none
        "updateStatus": match update.map(|u| &u.status) {
            Some(craft_marketplace::update_check::UpdateStatus::UpdateAvailable { available_version }) => {
                json!({ "state": "update-available", "availableVersion": available_version })
            }
            Some(craft_marketplace::update_check::UpdateStatus::VersionChanged) => json!({ "state": "version-changed" }),
            _ => json!({ "state": "none" }),
        },
        // 组件标注（解析仅供详情页标注，不加载；spec 第 1 节）
        "hasHooks": entry.component_annotations.has_hooks,
        "hasAgents": entry.component_annotations.has_agents,
        "hasMcp": entry.component_annotations.has_mcp,
    })
}

/// 市场视图（`storeListMarkets` 载荷，camelCase 对齐 MarketView）。
fn market_view(market: &craft_marketplace::store::MarketplaceRow) -> Value {
    json!({
        "id": market.id,
        "source": market.source,
        "repoUrl": market.repo_url,
        "lastRefreshedAt": market.last_refreshed_at.unwrap_or(0),
        "lastRefreshFailure": market.last_refresh_failure.clone().unwrap_or_default(),
        "createdAt": market.created_at,
    })
}

/// 推荐市场源视图（spec 第 9 节常驻区块；解析失败降级为空清单，不阻塞商店页）。
fn recommended_market_views() -> Vec<Value> {
    match craft_marketplace::recommended::recommended_marketplaces() {
        Ok(items) => items
            .iter()
            .map(|item| {
                json!({
                    "repo": item.repo,
                    "form": match item.form {
                        craft_marketplace::recommended::RecommendedForm::Marketplace => "marketplace",
                        craft_marketplace::recommended::RecommendedForm::Plugin => "plugin",
                        craft_marketplace::recommended::RecommendedForm::Skills => "skills",
                    },
                    "formLabel": item.form.label(),
                    "displayName": item.display_name,
                    "description": item.description,
                })
            })
            .collect(),
        Err(e) => {
            // 资产损坏降级：推荐源不可用不应阻塞商店页（错误写运行日志即可，不 toast）
            eprintln!("craftwork: 推荐市场源资产解析失败：{e}");
            Vec::new()
        }
    }
}

/// `storeListMarkets`：已添加市场 + 推荐源候选。运行时不内置任何源，推荐仅是候选。
pub fn store_list_markets(state: &AppState) -> SvcResult {
    let markets = state
        .db
        .with(craft_marketplace::store::list_marketplaces)
        .map_err(|e| e.to_string())?;
    let views: Vec<Value> = markets.iter().map(market_view).collect();
    Ok(json!({ "markets": views, "recommended": recommended_market_views() }))
}

/// `storeAddMarket`：校验输入（仅 https:// URL 或 owner/repo）→ 编排（clone + probe +
/// 重名拒绝 + 落行 + 快照落位，见 [`craft_marketplace::orchestrator::Orchestrator::add_market`]
/// ）→ 返回刷新后的市场清单。
pub fn store_add_market(state: &AppState, input: &str) -> SvcResult {
    let repo_url = normalize_market_input(input)?;
    store_orchestrator!(state, orchestrator);
    orchestrator.add_market(input.trim(), &repo_url).map_err(|e| e.to_string())?;
    store_list_markets(state)
}

/// `storeRemoveMarket`：保护语义移除（市场下仍有已安装插件时拒绝并提示先卸载）→
/// 返回刷新后的市场清单。
pub fn store_remove_market(state: &AppState, id: &str) -> SvcResult {
    if id.trim().is_empty() {
        return err("市场源 id 不能为空");
    }
    store_orchestrator!(state, orchestrator);
    orchestrator.remove_market(id).map_err(|e| e.to_string())?;
    store_list_markets(state)
}

/// `storeRefreshMarkets(id?)`：刷新单个市场（id 非空）或全部市场；单市场失败不阻塞
/// 其他市场（失败原因已落 `last_refresh_failure`，前端从市场清单读取）。返回每个
/// 成功刷新市场的结果（含是否经加速回退 fell_back）。
pub fn store_refresh_markets(state: &AppState, id: &str) -> SvcResult {
    store_orchestrator!(state, orchestrator);
    let targets: Vec<String> = if id.trim().is_empty() {
        state
            .db
            .with(craft_marketplace::store::list_marketplaces)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|m| m.id)
            .collect()
    } else {
        vec![id.trim().to_string()]
    };
    let mut outcomes = Vec::with_capacity(targets.len());
    for market_id in targets {
        match orchestrator.refresh_market(&market_id) {
            Ok(outcome) => outcomes.push(json!({
                "marketplaceId": outcome.marketplace_id,
                "headSha": outcome.head_sha,
                "fellBack": outcome.fell_back,
                "manifestKind": outcome.manifest_kind,
                "entryCount": outcome.entry_count,
            })),
            // 单市场失败继续刷其他市场；失败详情由市场清单的 lastRefreshFailure 呈现
            Err(e) => eprintln!("craftwork: 刷新市场 {market_id} 失败：{e}"),
        }
    }
    Ok(json!(outcomes))
}

/// `storeListPlugins`：各市场的插件清单 + 已装插件的更新状态（spec 第 11 节）。
///
/// 市场快照缺失（从未刷新成功）的市场只返回其已装插件（快照侧无可列条目）；更新
/// 检测同样依赖快照，缺失时按 none 不误报。条目与已装行按插件名合并：未装条目
/// `installed=false`，已装条目带版本历史与来源 pin。
pub fn store_list_plugins(state: &AppState) -> SvcResult {
    store_orchestrator!(state, orchestrator);
    let markets = state
        .db
        .with(craft_marketplace::store::list_marketplaces)
        .map_err(|e| e.to_string())?;

    let mut views: Vec<Value> = Vec::new();
    for market in &markets {
        // 已装插件（本市场）：与快照条目按名合并
        let installed = state
            .db
            .with(|conn| craft_marketplace::store::list_plugins_by_marketplace(conn, &market.id))
            .map_err(|e| e.to_string())?;

        // 快照侧条目（离线清单）：快照缺失（从未刷新）时无可列条目
        let snapshot = marketplace_layout(state).marketplace_snapshot_dir(&market.id);
        let manifest = if snapshot.is_dir() {
            let sha = craft_marketplace::orchestrator::read_snapshot_head_sha(&snapshot);
            craft_marketplace::manifest::probe_market(&snapshot, &market.id, sha.as_deref()).ok()
        } else {
            None
        };
        if let Some(manifest) = &manifest {
            let statuses = orchestrator.check_updates(&market.id).unwrap_or_default();
            for entry in &manifest.entries {
                let plugin = installed.iter().find(|p| p.name == entry.name);
                let update = plugin
                    .and_then(|p| statuses.iter().find(|s| s.plugin_id == p.id));
                views.push(store_plugin_view(entry, &market.id, plugin, update));
            }
        }
        // 已装但上游条目消失 / 快照缺失：仍返回行（可管理、可卸载），更新状态恒 none
        for plugin in &installed {
            if manifest
                .as_ref()
                .is_some_and(|m| m.entries.iter().any(|e| e.name == plugin.name))
            {
                continue;
            }
            // 快照缺失时元数据退读已装缓存里的清单副本（安装时拷入；探测失败按缺省）
            let cached = craft_marketplace::manifest::probe_plugin_manifest(Path::new(&plugin.install_path))
                .ok()
                .flatten();
            let annotations = cached_component_annotations(Path::new(&plugin.install_path));
            views.push(json!({
                "id": plugin.id,
                "marketplaceId": plugin.marketplace_id,
                "name": plugin.name,
                "description": cached.and_then(|pm| pm.description).unwrap_or_default(),
                "category": "",
                "tags": [],
                "version": plugin.current_version,
                "installed": true,
                "enabled": plugin.enabled,
                "versions": plugin.versions,
                "sourceSha": plugin.source_sha.clone().unwrap_or_default(),
                "installedAt": plugin.installed_at,
                "updateStatus": json!({ "state": "none" }),
                "hasHooks": annotations.has_hooks,
                "hasAgents": annotations.has_agents,
                "hasMcp": annotations.has_mcp,
            }));
        }
    }
    Ok(json!(views))
}

/// 已装插件缓存内的插件清单副本探测（`installedAt` 视图的组件标注与描述回读；
/// 清单副本在安装时按来源相对路径原位拷入缓存，探测链与安装侧同一函数）。
fn cached_component_annotations(
    install_path: &Path,
) -> craft_marketplace::manifest::ComponentAnnotations {
    match craft_marketplace::manifest::probe_plugin_manifest(install_path) {
        Ok(Some(pm)) => craft_marketplace::manifest::ComponentAnnotations {
            has_hooks: pm.has_hooks,
            has_agents: pm.has_agents,
            has_mcp: pm.has_mcp,
        },
        _ => craft_marketplace::manifest::ComponentAnnotations::default(),
    }
}

/// `storeInstallPlugin`：按插件 id 安装（应用内互斥与审计在编排内），返回刷新后的
/// 插件清单。
pub fn store_install_plugin(state: &AppState, id: &str) -> SvcResult {
    let (name, marketplace_id) = split_plugin_id(id)?;
    store_orchestrator!(state, orchestrator);
    orchestrator
        .install_plugin(&marketplace_id, &name)
        .map_err(|e| e.to_string())?;
    store_list_plugins(state)
}

/// `storeUpdatePlugin`：升级到市场侧最新（版本保留上限 3，启停选择保留），返回刷新
/// 后的插件清单。
pub fn store_update_plugin(state: &AppState, id: &str) -> SvcResult {
    let (name, marketplace_id) = split_plugin_id(id)?;
    store_orchestrator!(state, orchestrator);
    orchestrator
        .update_plugin(&marketplace_id, &name)
        .map_err(|e| e.to_string())?;
    store_list_plugins(state)
}

/// `storeRollbackPlugin`：一键回滚上一版（指针切回 versions 中当前版本之前的最近一个，
/// 可再切回），返回刷新后的插件清单。无历史版本时报可读错误。
pub fn store_rollback_plugin(state: &AppState, id: &str) -> SvcResult {
    let (name, marketplace_id) = split_plugin_id(id)?;
    store_orchestrator!(state, orchestrator);
    let plugin_id = craft_marketplace::store::plugin_id(&name, &marketplace_id);
    let plugin = state
        .db
        .with(|conn| craft_marketplace::store::get_plugin(conn, &plugin_id))
        .map_err(|e| e.to_string())?;
    // 版本数组按安装序（旧 → 新，append 到尾部）：上一版 = 当前指针之前最近的那个
    let target = plugin
        .versions
        .iter()
        .rev()
        .find(|v| **v != plugin.current_version)
        .ok_or_else(|| format!("插件 {} 没有可回滚的历史版本", plugin.id))?
        .clone();
    orchestrator
        .rollback_plugin(&marketplace_id, &name, &target)
        .map_err(|e| e.to_string())?;
    store_list_plugins(state)
}

/// `storeUninstallPlugin`：彻底卸载（含全部历史版本 + 清理禁用记录残留），返回刷新
/// 后的插件清单。
pub fn store_uninstall_plugin(state: &AppState, id: &str) -> SvcResult {
    let (name, marketplace_id) = split_plugin_id(id)?;
    store_orchestrator!(state, orchestrator);
    orchestrator
        .uninstall_plugin(&marketplace_id, &name)
        .map_err(|e| e.to_string())?;
    store_list_plugins(state)
}

/// `storeSetPluginEnabled`：插件级启停（停用后该插件全部技能根退出扫描，下一轮对话
/// 生效），返回刷新后的插件清单。
pub fn store_set_plugin_enabled(state: &AppState, id: &str, enabled: bool) -> SvcResult {
    let (name, marketplace_id) = split_plugin_id(id)?;
    store_orchestrator!(state, orchestrator);
    orchestrator
        .set_plugin_enabled(&marketplace_id, &name, enabled)
        .map_err(|e| e.to_string())?;
    store_list_plugins(state)
}

/// 递归收集插件目录下的技能清单（详情页技能列表）：每层目录含 SKILL.md 即一条
/// （name 取 frontmatter，缺省回退目录名；description 截断 200 字）。扫描深度上限
/// 与 craft-tools::skills 的 MAX_SKILL_SCAN_DEPTH 对齐（8）。
fn list_plugin_skills(root: &Path) -> Vec<Value> {
    const MAX_DEPTH: usize = 8;
    fn frontmatter_entry(content: &str, key: &str) -> String {
        craft_tools::skills::parse_frontmatter(content)
            .entries
            .unwrap_or_default()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Value>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, depth + 1, out);
            } else if entry.file_name().to_string_lossy() == craft_tools::skills::SKILL_FILE_NAME {
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                let fm_name = frontmatter_entry(&content, "name");
                let name = if fm_name.is_empty() {
                    path.parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    fm_name
                };
                let description = frontmatter_entry(&content, "description");
                out.push(json!({
                    "name": name,
                    "path": path.to_string_lossy(),
                    "description": description.chars().take(200).collect::<String>(),
                }));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort_by(|a, b| {
        let key = |v: &Value| v["name"].as_str().unwrap_or_default().to_string();
        key(a).cmp(&key(b))
    });
    out
}

/// `storeDescribePlugin`（spec 第 11 节详情）：已安装读缓存（当前版本目录的清单副本 + 技能清单 +
/// 版本历史）；未安装基于市场快照离线描述（probe 条目 + 来源目录扫描，不 clone 不下载）。
/// 市场快照缺失（从未刷新）时报可读错误提示先刷新。
pub fn store_describe_plugin(state: &AppState, id: &str) -> SvcResult {
    let (name, marketplace_id) = split_plugin_id(id)?;
    let layout = marketplace_layout(state);
    let snapshot = layout.marketplace_snapshot_dir(&marketplace_id);
    if !snapshot.is_dir() {
        return err(format!("市场快照不存在：{marketplace_id}，请先刷新该市场"));
    }
    let snapshot_sha = craft_marketplace::orchestrator::read_snapshot_head_sha(&snapshot);
    let manifest = craft_marketplace::manifest::probe_market(&snapshot, &marketplace_id, snapshot_sha.as_deref())
        .map_err(|e| e.to_string())?;
    let entry = manifest
        .entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("市场 {marketplace_id} 中不存在插件条目：{name}"))?;

    // 已装：读插件行 + 当前版本缓存（清单副本与技能清单都在缓存内）
    let plugin_id = craft_marketplace::store::plugin_id(&name, &marketplace_id);
    let plugin = state
        .db
        .with(|conn| Ok(craft_marketplace::store::get_plugin(conn, &plugin_id).ok()))
        .map_err(|e| e.to_string())?;

    let (skills_root, annotations, description) = match &plugin {
        Some(row) => {
            let current = Path::new(&row.install_path);
            let annotations = cached_component_annotations(current);
            let description = craft_marketplace::manifest::probe_plugin_manifest(current)
                .ok()
                .flatten()
                .and_then(|pm| pm.description)
                .or_else(|| entry.description.clone());
            (row.install_path.clone(), annotations, description)
        }
        // 未装：来源目录在快照内（条目相对路径从快照根解析，含 `./` 归一）
        None => {
            let rel = entry.source.source_ref.trim().trim_start_matches("./");
            let source_dir = if rel.is_empty() || rel == "." { snapshot.clone() } else { snapshot.join(rel) };
            let annotations = entry.component_annotations;
            (source_dir.to_string_lossy().to_string(), annotations, entry.description.clone())
        }
    };
    let skills = list_plugin_skills(&Path::new(&skills_root).join("skills"));

    Ok(json!({
        "id": plugin_id,
        "marketplaceId": marketplace_id,
        "name": name,
        "installed": plugin.is_some(),
        "enabled": plugin.as_ref().map(|p| p.enabled),
        "currentVersion": plugin.as_ref().map(|p| p.current_version.clone()).unwrap_or_default(),
        "versions": plugin.as_ref().map(|p| p.versions.clone()).unwrap_or_default(),
        "sourceSha": plugin.as_ref().and_then(|p| p.source_sha.clone()).unwrap_or_default(),
        "sourceKind": plugin.as_ref().map(|p| p.source_kind.clone()).unwrap_or_default(),
        "sourceRef": plugin.as_ref().map(|p| p.source_ref.clone()).unwrap_or_else(|| entry.source.source_ref.clone()),
        "description": description.unwrap_or_default(),
        "category": entry.category.clone().unwrap_or_default(),
        "tags": entry.tags.clone(),
        "hasHooks": annotations.has_hooks,
        "hasAgents": annotations.has_agents,
        "hasMcp": annotations.has_mcp,
        "skills": skills,
    }))
}
