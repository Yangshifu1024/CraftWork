//! CraftWork 应用壳（Tauri 2）。
//!
//! 分层纪律：本 crate 只做入口、窗口与 IPC 接线；全部领域逻辑在 crates/craft-*（不依赖 Tauri）。
//! invoke 统一入口 `craftwork_invoke` 按方法名分发到 services（与 Electron 版 ipc.ts 通道表等价）。

pub mod services;
pub mod system;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::Manager;

use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_agent::runner::{Runner, RunnerDeps};
use craft_core::bus::Bus;
use craft_core::contract::Batch;
use craft_core::secret::SecretStore;
use craft_model::catalog::Catalog;
use craft_model::factory::{ModelFactory, SecretKeyProvider};
use craft_policy::engine::Engine;
use craft_policy::parse_level;
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_store::records::get_setting;
use craft_tools::registry::Registry;
use services::AppState;

/// 数据目录：CRAFTWORK_HOME（测试/开发隔离）优先，否则 ~/.craftwork。
fn resolve_data_dir() -> PathBuf {
    let override_dir = std::env::var("CRAFTWORK_HOME").ok().filter(|s| !s.trim().is_empty());
    let dir = override_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".craftwork"));
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn build_app_state(app: &tauri::AppHandle) -> Result<AppState, String> {
    let data_dir = resolve_data_dir();
    let data_dir_str = data_dir.to_string_lossy().to_string();
    let db = Arc::new(Db::open(&data_dir.join("craftwork.db")).map_err(|e| e.to_string())?);

    // 事件出口：Bus 批次 → tauri emit craftwork:events
    let handle = app.clone();
    let bus = Arc::new(Bus::new(move |batch: Batch| {
        if let Ok(payload) = serde_json::to_value(&batch) {
            let _ = tauri::Emitter::emit(&handle, "craftwork:events", payload);
        }
    }));

    // 启动收尾：上次退出遗留的 running 运行标记 interrupted + error 事件
    let recorder = Recorder::new(Arc::clone(&db), Arc::clone(&bus));
    let _ = recorder.interrupt_stale_runs();

    let secrets = Arc::new(SecretStore::open(&data_dir));
    let keys_path_warning = secrets.warnings().first().cloned();
    if let Some(warning) = keys_path_warning {
        let handle = app.clone();
        let _ = tauri::Emitter::emit(&handle, "craftwork:toast", serde_json::json!({
            "tone": "warn", "title": "密钥存储警告", "message": warning,
        }));
    }

    // 审批档位恢复（设置为准）
    let saved_level = db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| v.get("approvalLevel").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "graded".into());
    let engine = Arc::new(Mutex::new(Engine::new(parse_level(&saved_level))));

    let grants = Arc::new(Grants::new(Arc::clone(&db)));
    let approvals = Arc::new(Approvals::new(Arc::clone(&db), recorder.clone(), Arc::clone(&grants)));
    let catalog_arc = Arc::new(Catalog::new(Arc::clone(&db)));
    let models = Arc::new(ModelFactory::new(Arc::clone(&catalog_arc), Box::new(SecretKeyProvider { secrets: Arc::clone(&secrets) })));
    let backup_dir = data_dir.join("backups").to_string_lossy().to_string();
    let registry = Arc::new(Registry::new(&backup_dir, Arc::clone(&db)));
    // 内置工具注册（MCP 动态注册后续经同名 register 替换）
    register_builtin_tools(&registry);

    let runner = Arc::new(Runner::new(RunnerDeps {
        db: Arc::clone(&db),
        records: recorder.clone(),
        registry: Arc::clone(&registry),
        engine: Arc::clone(&engine),
        approvals: Arc::clone(&approvals),
        models: Arc::clone(&models),
        attachment_dir: data_dir.join("attachments").to_string_lossy().to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
    }));

    Ok(AppState {
        db,
        secrets,
        engine,
        catalog: catalog_arc,
        models,
        registry,
        runner,
        approvals,
        recorder,
        data_dir: data_dir_str,
        attachment_dir: data_dir.join("attachments").to_string_lossy().to_string(),
        backup_dir,
    })
}

fn register_builtin_tools(registry: &Registry) {
    registry.register(Box::new(craft_tools::fs::FsList));
    registry.register(Box::new(craft_tools::fs::FsRead));
    registry.register(Box::new(craft_tools::fs::FsGlob));
    registry.register(Box::new(craft_tools::fs::FsGrep));
    registry.register(Box::new(craft_tools::fs::FsWrite));
    registry.register(Box::new(craft_tools::fs::FsEdit));
    registry.register(Box::new(craft_tools::fs::ApplyPatch));
    registry.register(Box::new(craft_tools::shell::ShellExec));
    registry.register(Box::new(craft_tools::shell::GitStatus));
    registry.register(Box::new(craft_tools::shell::GitDiff));
    registry.register(Box::new(craft_tools::shell::GitLog));
    registry.register(Box::new(craft_tools::shell::GitCommit));
    registry.register(Box::new(craft_tools::todo::TodoWrite));
    registry.register(Box::new(craft_tools::doc::DocRead));
    registry.register(Box::new(craft_tools::doc::DocEdit));
    registry.register(Box::new(craft_tools::doc::XlsxSetCell));
}

/// 渲染层统一入口：所有 `window.craftwork.<method>(...)` 经此分发。
#[tauri::command]
async fn craftwork_invoke(
    method: String,
    args: Vec<Value>,
    state: tauri::State<'_, Arc<AppState>>,
    app: tauri::AppHandle,
) -> Result<Value, String> {
    let state = Arc::clone(&state);
    // 同步领域操作放阻塞线程池，保持 UI 响应
    tokio::task::spawn_blocking(move || dispatch(&method, &args, &state, &app))
        .await
        .map_err(|e| format!("任务执行失败：{e}"))?
}

fn dispatch(method: &str, args: &[Value], state: &AppState, app: &tauri::AppHandle) -> Result<Value, String> {
    use services::*;
    let a = Args::new(args);
    match method {
        // ---------- 应用 ----------
        "getInfo" => get_info(state),
        "openExternal" => system::open_external(&a.str_at(0)),
        "pickFiles" => system::pick_files(app),
        "pickFolder" => system::pick_folder(app),
        "clipboardWrite" => system::clipboard_write(app, &a.str_at(0)),
        "clipboardRead" => system::clipboard_read(app),
        "showInFolder" => system::show_in_folder(&a.str_at(0)),
        "minimiseWindow" => system::minimise_window(app),
        "toggleMaximiseWindow" => system::toggle_maximise_window(app),
        "closeWindow" => system::close_window(app),
        // ---------- 设置 ----------
        "settingsGet" => settings_get(state),
        "settingsSave" => settings_save(state, &a.value_at(0)),
        // ---------- 会话 ----------
        "threadList" => thread_list(state, &a.str_at(0), a.bool_at(1)),
        "threadSearch" => thread_search(state, &a.str_at(0), &a.str_at(1)),
        "threadCreate" => thread_create(state, &a.str_at(0), &a.str_at(1), &a.str_at(2)),
        "threadRename" => thread_rename(state, &a.str_at(0), &a.str_at(1)),
        "threadSetModel" => thread_set_model(state, &a.str_at(0), &a.str_at(1)),
        "threadArchive" => thread_set_archived(state, &a.str_at(0), a.bool_at(1)),
        "threadDelete" => thread_delete(state, &a.str_at(0)),
        "threadSnapshot" => thread_snapshot(state, &a.str_at(0), 0, 0),
        "threadSnapshotPage" => thread_snapshot(state, &a.str_at(0), a.i64_at(1), a.i64_at(2)),
        // ---------- 对话 ----------
        "chatCreateAndStart" => chat_create_and_start(
            state,
            &a.str_at(0),
            &a.str_at(1),
            &a.str_at(2),
            a.value_at(3).as_array().map(Vec::as_slice).unwrap_or(&[]),
            &a.str_at(4),
        ),
        "chatSend" => chat_send(state, &a.str_at(0), &a.str_at(1), a.value_at(2).as_array().map(Vec::as_slice).unwrap_or(&[])),
        "chatSteer" => chat_steer(state, &a.str_at(0), &a.str_at(1), a.value_at(2).as_array().map(Vec::as_slice).unwrap_or(&[])),
        "chatRegenerate" => chat_regenerate(state, &a.str_at(0)),
        "chatCancel" => chat_cancel(state, &a.str_at(0)),
        "chatDecideApproval" => {
            // async 服务：在当前异步上下文内直接驱动
            let approval_id = a.str_at(0);
            let allow = a.bool_at(1);
            let scope = a.str_at(2);
            // dispatch 在 spawn_blocking 里——decide 需要 tokio 上下文：用 Handle::current 不可得，
            // 改为起一个临时 current-thread 运行时驱动（审批等待是 oneshot，开销可忽略）
            let approvals = Arc::clone(&state.approvals);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            rt.block_on(async move { approvals.decide(&approval_id, allow, &scope).await })?;
            Ok(json!(null))
        }
        // ---------- 工作区与文件 ----------
        "workspaceList" => workspace_list(state),
        "workspaceAdd" => workspace_add(state, &a.str_at(0), &a.str_at(1)),
        "workspaceAddProject" => workspace_add_project(state, &a.str_at(0), a.value_at(1).as_array().map(Vec::as_slice).unwrap_or(&[]), &a.str_at(2)),
        "workspaceSetTrust" => workspace_set_trust(state, &a.str_at(0), &a.str_at(1)),
        "workspaceRename" => workspace_rename(state, &a.str_at(0), &a.str_at(1)),
        "workspaceRemove" => workspace_remove(state, &a.str_at(0)),
        "workspaceListDir" => workspace_list_dir(state, &a.str_at(0), &a.str_at(1), &a.str_at(2)),
        "workspaceReadFile" => workspace_read_file(state, &a.str_at(0), &a.str_at(1), &a.str_at(2)),
        "workspaceWriteFile" => workspace_write_file(state, &a.str_at(0), &a.str_at(1), &a.str_at(2), &a.str_at(3), &a.str_at(4)),
        "workspaceComputeDiff" => workspace_compute_diff(state, &a.str_at(0), &a.str_at(1)),
        "workspaceGitStatuses" => workspace_git_statuses(state),
        // ---------- 提供商与模型 ----------
        "providerList" => provider_list(state),
        "providerSave" => provider_save(state, &a.value_at(0)),
        "providerDelete" => provider_delete(state, &a.str_at(0)),
        "providerSetEnabled" => provider_set_enabled(state, &a.str_at(0), a.bool_at(1)),
        "providerSetKey" => provider_set_key(state, &a.str_at(0), &a.str_at(1)),
        "providerClearKey" => provider_clear_key(state, &a.str_at(0)),
        "providerRename" => provider_rename(state, &a.str_at(0), &a.str_at(1)),
        "providerSetModelEnabled" => provider_set_model_enabled(state, &a.str_at(0), &a.str_at(1), a.bool_at(2)),
        "providerAddModel" => provider_add_model(state, &a.str_at(0), &a.value_at(1)),
        "providerUpdateModel" => provider_update_model(state, &a.str_at(0), &a.str_at(1), &a.value_at(2)),
        "providerRemoveModel" => provider_remove_model(state, &a.str_at(0), &a.str_at(1)),
        "providerFetchModels" => provider_fetch_models(state, &a.str_at(0)),
        "providerWarnings" => provider_warnings(state),
        "providerTest" => provider_test(state, &a.str_at(0)),
        "catalogMeta" => catalog_meta(state),
        "catalogProviders" => catalog_providers(state, &a.str_at(0)),
        "catalogModels" => catalog_models(state, &a.str_at(0)),
        "catalogImport" => catalog_import(state, &a.str_at(0), a.bool_at(1), a.value_at(2).as_array().map(Vec::as_slice).unwrap_or(&[])),
        // ---------- 审计 / 用量 / 备份 / 附件 ----------
        "auditList" => audit_list(state, a.i64_at(0)),
        "auditUsage" => audit_usage(state),
        "auditBackups" => audit_backups(state, a.i64_at(0)),
        "auditRestore" => audit_restore(state, &a.str_at(0)),
        "mcpList" => mcp_list(state),
        "mcpAdd" => mcp_add(state, &a.str_at(0), &a.str_at(1)),
        "mcpRemove" => mcp_remove(state, &a.str_at(0)),
        "mcpConnect" => mcp_connect(state, &a.str_at(0)),
        "mcpDisconnect" => mcp_disconnect(state, &a.str_at(0)),
        "savePastedAttachment" => save_pasted_attachment(state, &a.str_at(0), &a.str_at(1)),
        "readAttachment" => read_attachment(state, &a.str_at(0)),
        _ => Err(format!("方法 {method} 尚未接线（迁移进行中，见 docs/tauri-migration/issues/09）")),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let state = build_app_state(&app.handle().clone()).map_err(std::io::Error::other)?;
            // 启动确保内置临时会话目录
            let _ = services::ensure_conversation_workspace(&state);
            app.manage(Arc::new(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![craftwork_invoke])
        .run(tauri::generate_context!())
        .expect("craftwork 启动失败");
}
