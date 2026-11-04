//! 服务层：统一 invoke 分发（与 Electron 版 ipc.ts + services/* 等价）。
//! 只做参数校验、编排与错误转换；领域逻辑全部在 crates/craft-*。

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
use craft_store::records::{get_setting, list_audit, list_backups, set_setting, usage_summary};
use craft_store::threads;
use craft_store::workspaces;
use craft_tools::registry::Registry;

pub const APP_NAME: &str = "CraftWork";
pub const APP_VERSION: &str = "0.1.0";
const CUSTOM_PROVIDERS_KEY: &str = "model.custom_providers";

/// 全部共享服务（tauri State 管理）。
#[allow(dead_code)] // engine/registry 供 P8 审批档位热切换与 MCP 动态注册使用
pub struct AppState {
    pub db: Arc<Db>,
    pub secrets: Arc<SecretStore>,
    pub engine: Arc<Mutex<Engine>>,
    pub catalog: Arc<Catalog>,
    pub models: Arc<ModelFactory>,
    pub registry: Arc<Registry>,
    pub runner: Arc<Runner>,
    pub approvals: Arc<Approvals>,
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

pub fn default_settings() -> Settings {
    Settings {
        theme: "system".into(),
        mode: "code".into(),
        panel_width: 320,
        approval_level: "graded".into(),
        default_model: String::new(),
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

pub fn settings_get(state: &AppState) -> SvcResult {
    let saved: Option<Settings> = state
        .db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok());
    let merged = saved.unwrap_or_else(default_settings);
    serde_json::to_value(merged).map_err(|e| e.to_string())
}

pub fn settings_save(state: &AppState, next: &Value) -> SvcResult {
    // 白名单字段校验合并（services/settings.ts 校验规则直译）
    let mut merged: Settings = serde_json::from_value(settings_get(state)?).unwrap_or_else(|_| default_settings());
    if let Some(theme) = next.get("theme").and_then(Value::as_str).filter(|t| ["system", "dark", "light"].contains(t)) {
        merged.theme = theme.into();
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

pub fn thread_set_archived(state: &AppState, id: &str, archived: bool) -> SvcResult {
    threads::set_thread_archived(&state.db.lock_conn(), id, archived).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

pub fn thread_delete(state: &AppState, id: &str) -> SvcResult {
    threads::delete_thread(&state.db, id).map_err(|e| e.to_string())?;
    Ok(json!(null))
}

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
    let views: Vec<Value> = list
        .into_iter()
        .map(|ws| {
            json!({
                "id": ws.workspace.id, "name": ws.workspace.name, "path": ws.workspace.path,
                "trust": ws.workspace.trust, "purpose": ws.workspace.purpose,
                "lastOpenedAt": ws.workspace.last_opened_at,
                "folders": ws.folders.iter().map(|f| json!({"id": f.id, "path": f.path, "position": f.position})).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!(views))
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
    let paths: Vec<String> = folders.iter().filter_map(Value::as_str).map(String::from).collect();
    if name.is_empty() || paths.is_empty() {
        return err("名称与目录不能为空");
    }
    let main = paths[0].clone();
    let ws = workspaces::upsert_workspace(&state.db.lock_conn(), name, &main, trust, "project").map_err(|e| e.to_string())?;
    workspaces::replace_workspace_folders(&state.db, &ws.id, &paths).map_err(|e| e.to_string())?;
    workspace_list(state)
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
    // 级联删除其下会话（AGENTS.md：删除级联清理会话）
    let count = threads::delete_threads_by_workspace(&state.db, id).map_err(|e| e.to_string())?;
    workspaces::delete_workspace(&state.db, id).map_err(|e| e.to_string())?;
    Ok(json!({ "removedThreads": count }))
}

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

// ---------- 对话 ----------

/// 原子首发（services/chat.ts 的 chatCreateAndStart 等价）：
/// 模型校验 → 创建会话 → 写用户消息事件 → 启动 Run。失败零落库。
pub fn chat_create_and_start(state: &AppState, mode: &str, workspace_id: &str, text: &str, attachments: &[Value], model_ref: &str) -> SvcResult {
    if state.models.resolve(model_ref).is_none() {
        return err("模型不存在或已被删除，请重新选择模型");
    }
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
            workspace_id,
            status: "idle",
            archived: false,
        },
    )
    .map_err(|e| e.to_string())?;
    append_user_message(state, &id, text, attachments)?;
    let run_id = state.runner.run(&id)?;
    Ok(json!({ "threadId": id, "runId": run_id }))
}

fn append_user_message(state: &AppState, thread_id: &str, text: &str, attachments: &[Value]) -> Result<String, String> {
    let message_id = craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE);
    let (_, seq) = craft_store::events::record_event(
        &state.db,
        &craft_store::events::EventRecord {
            thread_id: thread_id.to_string(),
            message_id: message_id.clone(),
            run_id: None,
            kind: "user_message".into(),
            payload: json!({ "text": text, "attachments": attachments }),
        },
    )
    .map_err(|e| e.to_string())?;
    let _ = seq;
    Ok(message_id)
}

pub fn chat_send(state: &AppState, thread_id: &str, text: &str, attachments: &[Value]) -> SvcResult {
    append_user_message(state, thread_id, text, attachments)?;
    let run_id = state.runner.run(thread_id)?;
    Ok(json!(run_id))
}

pub fn chat_steer(state: &AppState, thread_id: &str, text: &str, attachments: &[Value]) -> SvcResult {
    let message_id = craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE);
    // 引导消息先落事件表（运行收尾丢弃队列时历史仍可见，与 TS 语义一致）
    append_user_message(state, thread_id, text, attachments)?;
    let item = craft_agent::runner::SteerItem {
        message_id,
        text: text.to_string(),
        attachments: attachments.iter().filter_map(Value::as_str).map(String::from).collect(),
    };
    match state.runner.try_steer(thread_id, item) {
        Ok(true) => Ok(json!({ "queued": true })),
        Ok(false) => {
            // 无在途运行：回退普通发送（消息已落库，直接启动 run）
            let run_id = state.runner.run(thread_id)?;
            Ok(json!({ "queued": false, "runId": run_id }))
        }
        Err(e) => err(e),
    }
}

pub fn chat_regenerate(state: &AppState, thread_id: &str) -> SvcResult {
    let run_id = state.runner.run(thread_id)?;
    Ok(json!(run_id))
}

pub fn chat_cancel(state: &AppState, thread_id: &str) -> SvcResult {
    Ok(json!(state.runner.cancel(thread_id)))
}

// ---------- 提供商 ----------

fn provider_views(state: &AppState) -> SvcResult {
    let providers = state.catalog.providers();
    let mut views: Vec<Value> = Vec::new();
    for p in providers {
        let secrets = &*state.secrets;
        let env_refs: Vec<&str> = p.env_vars.iter().map(String::as_str).collect();
        let key_source = secrets.source_for(&p.id, &env_refs);
        let key_masked = secrets.masked_for(&p.id, &env_refs);
        let _ = secrets;
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
                    "enabled": m.enabled, "available": !m.status.is_empty() && m.status != "alpha",
                })
            })
            .collect();
        views.push(json!({
            "id": p.id, "name": p.name, "kind": p.kind, "endpoint": p.endpoint,
            "keyRequired": p.key_required, "keySource": key_source, "keyMasked": key_masked,
            "available": true, "enabled": p.enabled, "docUrl": "", "catalogId": p.catalog_id,
            "source": p.source, "models": models,
        }));
    }
    Ok(json!(views))
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
    let id = input.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    if id.is_empty() {
        return err("提供商 ID 不能为空");
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
    provider_list(state)
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

pub fn provider_set_key(state: &AppState, id: &str, key: &str) -> SvcResult {
    state.secrets.set(id, key);
    provider_list(state)
}

pub fn provider_clear_key(state: &AppState, id: &str) -> SvcResult {
    state.secrets.delete(id);
    provider_list(state)
}

pub fn provider_test(state: &AppState, id: &str) -> SvcResult {
    // 连接测试：GET models 端点探测（完整语义 P8 校准）
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
