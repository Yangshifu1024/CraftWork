//! 服务层：统一 invoke 分发（与 Electron 版 ipc.ts + services/* 等价）。
//! 只做参数校验、编排与错误转换；领域逻辑全部在 crates/craft-*。
//! 各服务函数由 lib.rs 的 dispatch 按方法名接线（签名对应 channels.ts 声明）；
//! Err 为可读中文字符串，经 invoke 拒绝直达渲染层 toast。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use craft_agent::approvals::Approvals;
use craft_agent::runner::Runner;
use craft_core::contract::{AppInfo, Settings};
use craft_core::editor;
use craft_core::platform::{is_binary, sha256_hex, write_file_atomic};
use craft_core::secret::SecretStore;
use craft_model::catalog::{catalog_model_to_persisted, catalog_provider_env, Catalog};
use craft_model::factory::ModelFactory;
use craft_policy::engine::Engine;
use craft_policy::resolve::resolve_existing;
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_store::records::{add_audit, get_setting, list_audit, list_backups, set_setting, usage_summary};
use craft_store::threads;
use craft_store::workspaces;
use craft_tools::registry::Registry;

/// 应用名与版本（getInfo 返回值）。注意版本与 `src-tauri/tauri.conf.json` 双源维护：
/// 发布产物版本以 tauri.conf.json 为事实源，发版时两处需同步修改。
pub const APP_NAME: &str = "CraftWork";
pub const APP_VERSION: &str = "0.1.0";
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
    pub backup_dir: String,
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
    };
    serde_json::to_value(info).map_err(|e| e.to_string())
}

// ---------- 设置 ----------

/// 默认设置：无持久化设置或反序列化失败时的整体兜底（不做字段级部分合并）。
pub fn default_settings() -> Settings {
    Settings {
        theme: "system".into(),
        mode: "code".into(),
        panel_width: 320,
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
        max_iterations: 300,
    }
}

// ---------- 计划 ----------

/// 会话的最新计划（无则返回 null）。前端据此决定「批准并执行」提示条：
/// file_path 非空且未批准才显示（中断/失败的计划无文件，不出现）。
pub fn plan_get(state: &AppState, thread_id: &str) -> SvcResult {
    let plan = state.db.with(|conn| craft_store::plans::latest_plan(conn, thread_id)).map_err(|e| e.to_string())?;
    Ok(match plan {
        Some(p) => serde_json::to_value(p).map_err(|e| e.to_string())?,
        None => json!(null),
    })
}

/// 忽略计划：ignored 置 1（提示条不再出现，等待用户直接输入继续）。
pub fn plan_ignore(state: &AppState, plan_id: &str) -> SvcResult {
    state.db.with(|conn| craft_store::plans::ignore_plan(conn, plan_id)).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 批准计划：approved 置 1（提示条随之消失），确认消息由前端继续发送。
pub fn plan_approve(state: &AppState, plan_id: &str) -> SvcResult {
    state.db.with(|conn| craft_store::plans::approve_plan(conn, plan_id)).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

/// 打开计划文件（系统默认 markdown 应用）；路径白名单校验在 system::open_plan_file。
pub fn plan_open(state: &AppState, path: &str) -> SvcResult {
    let db_dir = Path::new(&state.data_dir);
    crate::system::open_plan_file(db_dir, path)
}

pub fn settings_get(state: &AppState) -> SvcResult {
    let saved: Option<Settings> = state
        .db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok());
    let mut merged = saved.unwrap_or_else(default_settings);
    // 历史档位（strict/readonly，已移除）与未知值回落分级档，保证前端拿到的档位恒合法
    merged.approval_level = craft_policy::parse_level(&merged.approval_level).as_str().into();
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
    if let Some(width) = next.get("panelWidth").and_then(Value::as_i64) {
        merged.panel_width = width.clamp(320, 760);
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
    for key in ["autoUpdate", "notifications", "telemetry"] {
        if let Some(v) = next.get(key).and_then(Value::as_bool) {
            match key {
                "autoUpdate" => merged.auto_update = v,
                "notifications" => merged.notifications = v,
                _ => merged.telemetry = v,
            }
        }
    }
    if let Some(iterations) = next.get("maxIterations").and_then(Value::as_i64) {
        merged.max_iterations = if (0..=1000).contains(&iterations) { iterations } else { 300 };
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
    settings_get(state)
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
    threads::delete_thread(&state.db, id).map_err(|e| e.to_string())?;
    cleanup_thread_worktree(state, &thread);
    Ok(json!(null))
}

/// 会话快照：读事件按 message_id 归组重建消息流（时间线装载的唯一入口）。
/// limit=0 不限条数；before_seq>0 只保留 seq 小于游标的事件（向上翻页）。
pub fn thread_snapshot(state: &AppState, id: &str, limit: i64, before_seq: i64) -> SvcResult {
    let snap = craft_store::events::snapshot(&state.db.lock_conn(), id, limit.max(0) as usize, before_seq)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(snap).map_err(|e| e.to_string())
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

fn workspace_roots(state: &AppState, workspace_id: &str) -> Result<Vec<String>, String> {
    let ws = workspaces::get_workspace(&state.db.lock_conn(), workspace_id).map_err(|e| e.to_string())?;
    Ok(ws.folders.iter().map(|f| f.path.clone()).collect())
}

pub fn workspace_add(state: &AppState, path: &str, trust: &str) -> SvcResult {
    if path.is_empty() || !Path::new(path).is_dir() {
        return err("目录不存在");
    }
    let name = Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    workspaces::upsert_workspace(&state.db.lock_conn(), &name, path, trust, "project").map_err(|e| e.to_string())?;
    workspace_list(state)
}

pub fn workspace_add_project(state: &AppState, name: &str, folders: &[Value], trust: &str) -> SvcResult {
    let raw: Vec<&str> = folders.iter().filter_map(Value::as_str).collect();
    if raw.is_empty() {
        return err("请至少选择一个源文件夹");
    }
    // 规范化：绝对路径 + 符号链接解析 + 必须是目录 + 请求内去重
    let mut normalized: Vec<String> = Vec::with_capacity(raw.len());
    for folder in raw {
        let abs = std::fs::canonicalize(folder).map_err(|_| "目录不存在".to_string())?;
        if !abs.is_dir() {
            return err("源文件夹必须是目录");
        }
        let abs = abs.to_string_lossy().to_string();
        if normalized.iter().any(|p| p == &abs) {
            return err(format!("源文件夹重复：{abs}"));
        }
        normalized.push(abs);
    }
    let existing = {
        let conn = state.db.lock_conn();
        workspaces::list_workspaces(&conn).map_err(|e| e.to_string())?
    };
    // 内置目录守卫：数据目录与内置临时会话目录不可作为项目目录（避免两套授权范围交叉）。
    // 存储路径可能与规范化形式不同（如 macOS /var → /private/var），两侧都规范化后比较。
    let data_dir_canon = std::fs::canonicalize(&state.data_dir).ok().map(|p| p.to_string_lossy().to_string());
    let conv_paths: Vec<String> = existing
        .iter()
        .filter(|w| w.workspace.purpose == "conversation")
        .filter_map(|w| std::fs::canonicalize(&w.workspace.path).ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    for abs in &normalized {
        if Some(abs) == data_dir_canon.as_ref() {
            return err("该目录是应用数据目录，不能作为项目目录");
        }
        if conv_paths.iter().any(|p| p == abs) {
            return err("该目录是内置的临时会话目录，无需添加为项目");
        }
    }
    // 任一目录已是其他工作区的源文件夹时拒绝（同样规范化后比较）
    let used: Vec<(String, String)> = existing
        .iter()
        .flat_map(|w| w.folders.iter().map(move |f| (w.workspace.name.clone(), f.path.clone())))
        .filter_map(|(name, p)| std::fs::canonicalize(&p).ok().map(|c| (name, c.to_string_lossy().to_string())))
        .collect();
    if let Some((ws_name, _)) = used.iter().find(|(_, p)| normalized.iter().any(|n| n == p)) {
        return err(format!("目录已被工作区「{ws_name}」使用"));
    }
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
pub fn workspace_list_dir(state: &AppState, workspace_id: &str, _folder_id: &str, rel: &str) -> SvcResult {
    let roots = workspace_roots(state, workspace_id)?;
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
    let roots = workspace_roots(state, workspace_id)?;
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
    let _ = folder_id;
    let roots = workspace_roots(state, workspace_id)?;
    // 新建文件（父目录缺失）场景：词法校验后按主要根拼接，写前建目录
    let resolved = match resolve_existing(&roots, rel) {
        Ok(resolved) => resolved,
        Err(_) => {
            let main = roots.first().cloned().ok_or("没有可用工作区目录")?;
            let (abs, rel_out) = craft_policy::resolve::lexical_at(&main, rel).map_err(|_| "路径超出工作区".to_string())?;
            craft_policy::resolve::Resolved { root_index: 0, root: main, abs, rel: rel_out }
        }
    };
    // expectedHash 阻止覆盖外部修改
    if let Ok(existing) = std::fs::read(&resolved.abs) {
        let current_hash = sha256_hex(&existing);
        if !expected_hash.is_empty() && current_hash != expected_hash {
            return err("文件已被外部修改，请先重新读取再保存");
        }
        let env = craft_tools::ToolEnv { backup_dir: state.backup_dir.clone(), db: Arc::clone(&state.db) };
        env.backup(&resolved.abs, &existing)?;
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
    let _ = folder_id;
    let roots = workspace_roots(state, workspace_id)?;
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
pub fn workspace_git_statuses(state: &AppState) -> SvcResult {
    let list = workspaces::list_workspaces(&state.db.lock_conn()).map_err(|e| e.to_string())?;
    let mut statuses = serde_json::Map::new();
    for ws in list {
        if ws.workspace.purpose != "project" {
            continue;
        }
        let result = craft_core::platform::run(craft_core::platform::RunSpec {
            file: "git".into(),
            args: vec!["status".into(), "--short".into(), "--branch".into()],
            dir: Some(ws.workspace.path.clone()),
            timeout_ms: Some(5000),
            output_limit: None,
        });
        statuses.insert(ws.workspace.id, json!(if result.exit_code == 0 { result.output.trim() } else { "" }));
    }
    Ok(Value::Object(statuses))
}

// ---------- worktree ----------

/// git 命令出口：经 platform::run（环境白名单清洗 + 独立进程组 + 超时杀树），
/// 与 envcontext 的 git_run、workspace_git_statuses 同源；dir 为执行目录。
fn git_run(dir: &str, args: &[&str], timeout_ms: u64) -> craft_core::platform::RunResult {
    craft_core::platform::run(craft_core::platform::RunSpec {
        file: "git".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        dir: Some(dir.to_string()),
        timeout_ms: Some(timeout_ms),
        output_limit: None,
    })
}

/// git 命令失败的可读信息（stdout/stderr 或 spawn 错误）。
fn git_error_detail(result: &craft_core::platform::RunResult) -> String {
    let detail = result.output.trim();
    if !detail.is_empty() {
        return detail.to_string();
    }
    if result.timed_out {
        return "git 命令超时".into();
    }
    result.error.clone().unwrap_or_else(|| format!("退出码 {}", result.exit_code))
}

/// 会话绑定的 worktree 目录：数据目录下统一管理（随 CRAFTWORK_HOME 隔离，不污染项目目录）。
fn thread_worktree_dir(data_dir: &str, thread_id: &str) -> PathBuf {
    PathBuf::from(data_dir).join("worktrees").join(thread_id)
}

/// worktree 分支名：craftwork/<会话 id 去前缀后的前 8 位>；跨重建稳定，便于终端辨认。
fn worktree_branch_for(thread_id: &str) -> String {
    let short: String = thread_id.strip_prefix("thr_").unwrap_or(thread_id).chars().take(8).collect();
    format!("craftwork/{short}")
}

/// 确保会话已绑定可用的 git worktree（幂等）：
/// · 未绑定 → 校验项目工作区 + 主目录为 git 仓库，创建 `<数据目录>/worktrees/<会话id>`（分支 craftwork/<短码>）并落库；
/// · 已绑定但目录丢失 → `git worktree prune` 后重建，优先复用原分支（历史提交不丢）；
/// · 已绑定且目录存在 → 直接返回，不重复创建。
/// 失败返回可读中文错误，发送链路据此中止（不落消息）。
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
    let probe = git_run(&repo, &["rev-parse", "--is-inside-work-tree"], 10_000);
    if probe.exit_code != 0 {
        return err("当前项目不是 git 仓库，无法使用 worktree");
    }
    let path = thread_worktree_dir(&state.data_dir, thread_id);
    let path_str = path.to_string_lossy().to_string();
    let branch = worktree_branch_for(thread_id);

    // 幂等：已绑定且目录在，直接返回既有绑定
    if !thread.worktree_path.is_empty() && path.is_dir() {
        return Ok((thread.worktree_path.clone(), thread.worktree_branch.clone()));
    }

    // 目录丢失（手动删除等）：清理主仓库残留的 worktree 元数据后重建
    let _ = git_run(&repo, &["worktree", "prune"], 15_000);
    std::fs::create_dir_all(&path).map_err(|e| format!("创建工作树目录失败：{e}"))?;
    // 优先新建分支；分支已存在（重建场景）则复用，保留工作线上的历史提交
    let mut added = git_run(&repo, &["worktree", "add", "-b", &branch, &path_str], 30_000);
    if added.exit_code != 0 {
        added = git_run(&repo, &["worktree", "add", &path_str, &branch], 30_000);
    }
    if added.exit_code != 0 {
        return err(format!("创建 git 工作树失败：{}", git_error_detail(&added)));
    }
    threads::set_thread_worktree(&state.db.lock_conn(), thread_id, &path_str, &branch).map_err(|e| e.to_string())?;
    let _ = add_audit(
        &state.db.lock_conn(),
        craft_store::records::AuditRecord {
            ts: None,
            thread_id: Some(thread_id),
            run_id: None,
            tool: "worktree",
            risk: "R0",
            decision: "allowed",
            scope: None,
            detail: Some(&format!("创建 git 工作树：{path_str}（分支 {branch}）")),
        },
    );
    Ok((path_str, branch))
}

/// 删除会话时清理其绑定的 worktree（best-effort）：强制移除工作树并删除分支，
/// 失败只落审计不阻塞删除（残留目录可手动清理；未合并改动的告知由前端确认框承担）。
fn cleanup_thread_worktree(state: &AppState, thread: &threads::ThreadRow) {
    if thread.worktree_path.is_empty() {
        return;
    }
    // 主仓库目录：worktree 的 git 元数据挂在来源工作区主目录；工作区已删则退化直接删目录
    let repo = state.db.with(|conn| {
        Ok(workspaces::get_workspace(conn, &thread.workspace_id)?.workspace.path)
    });
    let removed = repo
        .as_ref()
        .map(|repo| git_run(repo, &["worktree", "remove", "--force", &thread.worktree_path], 30_000).exit_code == 0)
        .unwrap_or(false);
    if !removed {
        let _ = std::fs::remove_dir_all(&thread.worktree_path);
    }
    if let Ok(repo) = &repo {
        let _ = git_run(repo, &["branch", "-D", &thread.worktree_branch], 10_000);
    }
    let _ = add_audit(
        &state.db.lock_conn(),
        craft_store::records::AuditRecord {
            ts: None,
            thread_id: Some(&thread.id),
            run_id: None,
            tool: "worktree",
            risk: "R0",
            decision: "allowed",
            scope: None,
            detail: Some(&format!("删除会话，清理 git 工作树：{}", thread.worktree_path)),
        },
    );
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
    let title = craft_agent::title::derive_title(text, &[]);
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
    let (_, seq) = append_user_message(state, &id, text, attachments, plan_mode)?;
    let run_id = state.runner.run(&id, Some(seq))?;
    Ok(json!({ "threadId": id, "runId": run_id }))
}

fn append_user_message(
    state: &AppState,
    thread_id: &str,
    text: &str,
    attachments: &[Value],
    plan_mode: bool,
) -> Result<(String, i64), String> {
    append_user_message_via(state, thread_id, text, attachments, None, plan_mode)
}

/// 落库并广播用户消息；via 非空时写入载荷（"steer" = 运行中引导，前端显示「待送达」）。
/// plan_mode 随载荷落库：run 启动时按 trigger_seq 还原（计划模式提示词与审批硬约束的依据）。
/// 返回 (message_id, seq)：seq 随 run 启动透传为 trigger_seq，快照占位定序的权威信号。
fn append_user_message_via(
    state: &AppState,
    thread_id: &str,
    text: &str,
    attachments: &[Value],
    via: Option<&str>,
    plan_mode: bool,
) -> Result<(String, i64), String> {
    let message_id = craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE);
    // 经 Recorder 落库并广播 user.message：直接调 record_event 只写库不发事件，
    // 渲染层时间线将看不到这条用户消息（首轮对话靠首发快照掩盖了该缺陷）。
    let mut payload = json!({ "text": text, "attachments": attachments, "planMode": plan_mode });
    if let Some(via) = via {
        payload["via"] = json!(via);
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
    let (_, seq) = append_user_message(state, thread_id, text, attachments, plan_mode)?;
    let run_id = state.runner.run(thread_id, Some(seq))?;
    Ok(json!(run_id))
}

pub fn chat_steer(state: &AppState, thread_id: &str, text: &str, attachments: &[Value], plan_mode: bool) -> SvcResult {
    // 预检引导容量再落库：只有确定并入队列的消息才带 via=steer（「待送达」标记）
    let steer = state.runner.can_steer(thread_id)?;
    let (message_id, seq) = if steer {
        append_user_message_via(state, thread_id, text, attachments, Some("steer"), plan_mode)?
    } else {
        append_user_message(state, thread_id, text, attachments, plan_mode)?
    };
    let item = craft_agent::runner::SteerItem {
        message_id: message_id.clone(),
        text: text.to_string(),
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

/// 窗口关闭/应用退出：统一取消在途运行并落审计（AGENTS 6.2「窗口关闭、应用退出
/// 统一按拒绝或取消落审计」）。同步返回；run 循环的收尾是异步的，进程随即退出时
/// 由下次启动的 interrupt_stale_runs 兜底（中断标记 + 可重试错误事件）。
pub fn cancel_all_runs(state: &AppState) {
    for thread_id in state.runner.running_threads() {
        if state.runner.cancel(&thread_id) {
            let _ = add_audit(
                &state.db.lock_conn(),
                craft_store::records::AuditRecord {
                    ts: None,
                    thread_id: Some(&thread_id),
                    run_id: None,
                    tool: "run",
                    risk: "R0",
                    decision: "canceled",
                    scope: None,
                    detail: Some("窗口关闭/应用退出，运行已取消"),
                },
            );
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

fn provider_view_json(state: &AppState, p: &craft_model::types::PersistedProvider) -> Value {
    let secrets = &*state.secrets;
    let env_refs: Vec<&str> = p.env_vars.iter().map(String::as_str).collect();
    let key_source = secrets.source_for(&p.id, &env_refs);
    let key_masked = secrets.masked_for(&p.id, &env_refs);
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
        "source": p.source, "models": models,
    })
}

pub fn provider_list(state: &AppState) -> SvcResult {
    state.catalog.reload();
    provider_views(state)
}

pub fn provider_save(state: &AppState, input: &Value) -> SvcResult {
    let mut providers: Vec<craft_model::types::PersistedProvider> = state
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
    let entry = json!({
        "id": id,
        "name": input.get("name").and_then(Value::as_str).unwrap_or(&id),
        "kind": input.get("kind").and_then(Value::as_str).unwrap_or("openai"),
        "endpoint": input.get("endpoint").and_then(Value::as_str).unwrap_or(""),
        "keyRequired": input.get("keyRequired").and_then(Value::as_bool).unwrap_or(false),
        "note": "", "source": "manual", "catalogId": "", "catalogVersion": "", "envVars": [],
        "enabled": true,
        "models": input.get("models").cloned().unwrap_or(json!([])),
    });
    let provider: craft_model::types::PersistedProvider =
        serde_json::from_value(entry).map_err(|e| format!("提供商数据无效：{e}"))?;
    if let Some(slot) = providers.iter_mut().find(|p| p.id == id) {
        *slot = provider;
    } else {
        providers.push(provider);
    }
    save_providers(state, &providers)?;
    provider_view_one(state, &id)
}

fn save_providers(state: &AppState, providers: &[craft_model::types::PersistedProvider]) -> Result<(), String> {
    let value = serde_json::to_value(providers).map_err(|e| e.to_string())?;
    state.db.with(|conn| set_setting(conn, CUSTOM_PROVIDERS_KEY, &value)).map_err(|e| e.to_string())?;
    state.catalog.reload();
    Ok(())
}

pub fn provider_delete(state: &AppState, id: &str) -> SvcResult {
    let providers: Vec<craft_model::types::PersistedProvider> = state
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
    let mut providers: Vec<craft_model::types::PersistedProvider> = state
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

pub fn provider_test(state: &AppState, id: &str) -> SvcResult {
    // 连接测试：GET models 端点探测
    let Some(provider) = state.catalog.find_provider(id) else {
        return err("提供商不存在");
    };
    let env_refs: Vec<&str> = provider.env_vars.iter().map(String::as_str).collect();
    let key = state.secrets.get_for(&provider.id, &env_refs);
    let started = std::time::Instant::now();
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
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
    match req.send() {
        Ok(resp) => {
            let latency = started.elapsed().as_millis() as i64;
            let ok = resp.status().is_success();
            let code = if ok { String::new() } else { format!("http_{}", resp.status().as_u16()) };
            let message = if ok { "连接成功".to_string() } else { format!("服务返回 HTTP {}", resp.status().as_u16()) };
            Ok(json!({ "ok": ok, "message": message, "latencyMs": latency, "code": code }))
        }
        Err(e) => Ok(json!({ "ok": false, "message": format!("无法连接：{e}"), "latencyMs": started.elapsed().as_millis() as i64, "code": "network" })),
    }
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
    let mut providers: Vec<craft_model::types::PersistedProvider> = state
        .db
        .with(|conn| get_setting(conn, CUSTOM_PROVIDERS_KEY))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let provider = craft_model::types::PersistedProvider {
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

// ---------- 审计 / 用量 / 备份 / 附件 ----------

pub fn audit_list(state: &AppState, limit: i64) -> SvcResult {
    let entries = list_audit(&state.db.lock_conn(), limit).map_err(|e| e.to_string())?;
    serde_json::to_value(entries).map_err(|e| e.to_string())
}

pub fn audit_usage(state: &AppState) -> SvcResult {
    let summary = usage_summary(&state.db.lock_conn()).map_err(|e| e.to_string())?;
    serde_json::to_value(summary).map_err(|e| e.to_string())
}

pub fn audit_backups(state: &AppState, limit: i64) -> SvcResult {
    let backups = list_backups(&state.db.lock_conn(), limit).map_err(|e| e.to_string())?;
    serde_json::to_value(backups).map_err(|e| e.to_string())
}

/// 粘贴附件落盘：文件名只保留字母数字与 .-_（剥离路径分隔符等危险字符），再加 att_ ID
/// 前缀保证唯一；返回服务端文件名，后续经 readAttachment 取回。
pub fn save_pasted_attachment(state: &AppState, name: &str, base64_data: &str) -> SvcResult {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| format!("附件解码失败：{e}"))?;
    let safe_name: String = name.chars().filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_').collect();
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

fn load_providers(state: &AppState) -> Vec<craft_model::types::PersistedProvider> {
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
    let slot = provider.models.iter_mut().find(|m| m.id == model_id).ok_or("模型不存在")?;
    let mut persisted = model_input_to_persisted(model)?;
    // 档位保留语义：请求未携带 reasoningLevels 字段时保留现值（目录归一值或上次手动声明），
    // 避免编辑显示名等操作把目录导入的档位元数据抹掉
    if model.get("reasoningLevels").is_none() {
        persisted.reasoning_kind = std::mem::take(&mut slot.reasoning_kind);
        persisted.reasoning_levels = std::mem::take(&mut slot.reasoning_levels);
    }
    // ID 不可变：保留原 ID
    let id = slot.id.clone();
    *slot = craft_model::types::PersistedModel { id, ..persisted };
    save_providers(state, &providers)?;
    provider_view_one(state, provider_id)
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
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    if provider.kind == "anthropic" {
        req = req.header("x-api-key", &key).header("anthropic-version", "2023-06-01");
    } else if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = req.send().map_err(|e| format!("拉取失败：{e}"))?;
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

// ---------- 审计恢复 ----------

/// 从备份恢复文件：恢复前先把当前内容再备份一次（可回退），随后原子写回原路径。
pub fn audit_restore(state: &AppState, backup_id: &str) -> SvcResult {
    let backup = craft_store::records::get_backup(&state.db.lock_conn(), backup_id).map_err(|e| e.to_string())?;
    let current = std::fs::read(&backup.path);
    if let Ok(existing) = current {
        // 恢复前把当前内容也做一次备份（可再恢复回来）
        state
            .registry_backup_env()
            .backup(&backup.path, &existing)?;
    }
    let data = std::fs::read(&backup.backup_path).map_err(|e| format!("备份读取失败：{e}"))?;
    write_file_atomic(Path::new(&backup.path), &data, 0o644).map_err(|e| format!("恢复失败：{e}"))?;
    Ok(json!(null))
}

impl AppState {
    /// 构造写前备份用的 ToolEnv（备份目录 + DB），审计恢复与工具执行共用同一备份链路。
    pub fn registry_backup_env(&self) -> craft_tools::ToolEnv {
        craft_tools::ToolEnv { backup_dir: self.backup_dir.clone(), db: Arc::clone(&self.db) }
    }
}

// ---------- MCP（连接留桩，配置可存） ----------

pub fn mcp_list(state: &AppState) -> SvcResult {
    let servers: Vec<Value> = state
        .db
        .with(|conn| get_setting(conn, "mcp.servers"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Vec<Value>>(v).ok())
        .unwrap_or_default();
    let views: Vec<Value> = servers
        .into_iter()
        .map(|mut s| {
            if s.get("connected").is_none() {
                s["connected"] = json!(false);
            }
            if s.get("tools").is_none() {
                s["tools"] = json!([]);
            }
            s
        })
        .collect();
    Ok(json!(views))
}

pub fn mcp_add(state: &AppState, name: &str, command_line: &str) -> SvcResult {
    if name.trim().is_empty() || command_line.trim().is_empty() {
        return err("名称与命令不能为空");
    }
    let mut parts = command_line.split_whitespace().map(String::from);
    let command = parts.next().unwrap_or_default();
    let args: Vec<String> = parts.collect();
    let mut servers: Vec<Value> = state
        .db
        .with(|conn| get_setting(conn, "mcp.servers"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let server = json!({
        "id": craft_core::ids::new_id(craft_core::ids::id_prefix::MCP_SERVER),
        "name": name, "command": command, "args": args, "enabled": true,
    });
    servers.push(server);
    state
        .db
        .with(|conn| set_setting(conn, "mcp.servers", &serde_json::to_value(&servers).unwrap_or(Value::Null)))
        .map_err(|e| e.to_string())?;
    mcp_list(state)
}

pub fn mcp_remove(state: &AppState, server_id: &str) -> SvcResult {
    let servers: Vec<Value> = state
        .db
        .with(|conn| get_setting(conn, "mcp.servers"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let filtered: Vec<Value> = servers.into_iter().filter(|s| s["id"] != json!(server_id)).collect();
    state
        .db
        .with(|conn| set_setting(conn, "mcp.servers", &serde_json::to_value(&filtered).unwrap_or(Value::Null)))
        .map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn mcp_connect(_state: &AppState, _server_id: &str) -> SvcResult {
    // stdio 连接能力留桩（决策 D5）：设置可存，连接迁移后单独立项
    err("MCP 连接能力尚未启用（迁移后单独立项）")
}

pub fn mcp_disconnect(_state: &AppState, _server_id: &str) -> SvcResult {
    err("MCP 连接能力尚未启用（迁移后单独立项）")
}
