//! CraftWork 应用壳（Tauri 2）：启动装配与 IPC 接线。
//!
//! 分层纪律：本 crate 只做入口、窗口与 IPC 接线；全部领域逻辑在 crates/craft-*（不依赖 Tauri）。
//! 渲染层所有系统操作经统一入口 `craftwork_invoke` 按方法名分发到 services，与 channels.ts
//! 通道表三处同步接线（本文件 dispatch 缺臂由 tests/dispatch_test.rs 对拍防漏）；会话事件由
//! Bus 攒批后经 emit `craftwork:events` 广播，applyBatch 是渲染层唯一事件入口。

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
use craft_core::contract::{Batch, Settings};
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

/// 启动窗口装配：按持久化主题偏好设底色，最大化后显示（conf 的 visible:false）。
/// 底色与渲染层 styles.css 的 --zai-background 两套值一致（theme.ts 的
/// syncWindowBackground 首帧后接管，值相同无跳变）；system 偏好按系统当前外观解析。
/// 顺序固定为设底色 → maximize → show：底色先就位，最大化露出的填充区域与内容同色；
/// maximize 在 show 前，窗口以最大化形态首现，避免先小后大的跳动。
/// 任一步失败保持 conf 兜底底色，仅跳过 show 由默认可见性兜底，不阻塞启动。
fn setup_main_window(window: &tauri::WebviewWindow, db: &Db) {
    let pref = db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Settings>(v).ok())
        .map(|s| s.theme)
        .unwrap_or_else(|| "system".into());
    let resolved = match pref.as_str() {
        "dark" => tauri::Theme::Dark,
        "light" => tauri::Theme::Light,
        _ => window.theme().unwrap_or(tauri::Theme::Light),
    };
    let color = match resolved {
        tauri::Theme::Dark => tauri::window::Color(0x16, 0x16, 0x16, 0xff),
        _ => tauri::window::Color(0xf8, 0xf8, 0xf8, 0xff),
    };
    let _ = window.set_background_color(Some(color));
    let _ = window.maximize();
    let _ = window.show();
}

/// 装配全部共享服务（AppState），顺序即依赖序：DB → 事件出口（Bus 批次 → emit）→
/// Recorder（顺带把上次退出遗留的 running 标记落为 interrupted）→ 密钥 → 策略引擎/
/// 授权/审批 → 模型目录与工厂 → 工具 Registry → Runner；最后挂三个设置驱动钩子
/// （系统通知 / 失败转移 / 往返上限），均为调用时实时读设置、改开关立即生效。
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

    // 系统通知：审批待决/完成/失败三类（Runner 内置钩子），仅窗口失焦时发（聚焦时应用内已有
    // 完整反馈），受设置「系统通知」开关门控（实时读设置，改开关立即生效）。此前从未装配。
    runner.set_notifier(Box::new({
        let app = app.clone();
        move |_thread_id, title, message| {
            if app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false) {
                return;
            }
            let enabled = app
                .try_state::<Arc<AppState>>()
                .map(|s| {
                    s.db
                        .with(|conn| get_setting(conn, "app"))
                        .ok()
                        .flatten()
                        .and_then(|v| v.get("notifications").and_then(Value::as_bool))
                        .unwrap_or(true)
                })
                .unwrap_or(true);
            if !enabled {
                return;
            }
            use tauri_plugin_notification::NotificationExt;
            let _ = app.notification().builder().title(title).body(message).show();
        }
    }));

    // 失败转移解析：按当前设置的 fallbackModel 取备用模型引用（空/无效/停用时 factory 返回空串即不转移）。
    // 此前从未装配，设置里的备用模型存而不用（migration-parity 票 04）。
    models.set_fallback_resolver({
        let db = Arc::clone(&db);
        Box::new(move || {
            db.with(|conn| get_setting(conn, "app"))
                .ok()
                .flatten()
                .and_then(|v| v.get("fallbackModel").and_then(Value::as_str).map(String::from))
                .unwrap_or_default()
        })
    });
    // 往返上限：读设置即时生效（0 视为默认值）。此前从未装配，恒用常量 300。
    runner.set_max_iterations_resolver(Box::new({
        let db = Arc::clone(&db);
        move || {
            db.with(|conn| get_setting(conn, "app"))
                .ok()
                .flatten()
                .and_then(|v| v.get("maxIterations").and_then(Value::as_i64))
                .filter(|n| *n > 0)
                .unwrap_or(craft_agent::runner::DEFAULT_MAX_ITERATIONS)
        }
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

/// 注册全部内置工具（fs / apply_patch / shell / git / todo / doc）。
/// MCP 动态工具留桩（决策 D5）：后续接入时经同名 register 覆盖。
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

/// 渲染层统一入口：所有 `window.craftwork.<method>(...)` 经此分发（method + 位置参数数组）。
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

/// 分发路由：method 与 channels.ts 的 CraftworkApi 方法一一对应，参数按位置取值
/// （与渲染层桥的 TS 服务签名数组一致）。未声明的方法返回「尚未接线」；
/// dispatch_test.rs 保证 channels.ts 声明的方法在此不缺臂。
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
        "threadSetReasoning" => thread_set_reasoning(state, &a.str_at(0), &a.str_at(1)),
        "threadArchive" => thread_set_archived(state, &a.str_at(0), a.bool_at(1)),
        "threadDelete" => thread_delete(state, &a.str_at(0)),
        // 快照与翻页共用实现：limit=0 不限条数（全量）；翻页传 limit + beforeSeq 游标（保留最新 limit 条）
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
            &a.str_at(5),
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
        "documentPreview" => document_preview(state, &a.str_at(0), &a.str_at(1), &a.str_at(2)),
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
        // 前端传单对象 {providerId, all, modelIds}（channels.ts CatalogImportInput），按字段解包
        "catalogImport" => {
            let input = a.value_at(0);
            let provider_id = input.get("providerId").and_then(Value::as_str).unwrap_or_default();
            let all = input.get("all").and_then(Value::as_bool).unwrap_or(false);
            let model_ids = input.get("modelIds").and_then(Value::as_array).cloned().unwrap_or_default();
            catalog_import(state, provider_id, all, &model_ids)
        }
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
        _ => Err(format!("方法 {method} 尚未接线")),
    }
}

/// 应用入口：注册插件 → 装配 AppState 与内置临时会话目录 → 按主题装配主窗口 → 管理
/// 共享状态。窗口关闭（CloseRequested）与应用退出（ExitRequested）两处兜底取消在途
/// 运行并按取消落审计（AGENTS 6.2：不允许静默中断）。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let state = build_app_state(&app.handle().clone()).map_err(std::io::Error::other)?;
            // 启动确保内置临时会话目录
            let _ = services::ensure_conversation_workspace(&state);
            // 按主题偏好设好底色、最大化后再显示窗口（消除启动黑/白闪 + 启动即最大化）
            if let Some(window) = app.get_webview_window("main") {
                setup_main_window(&window, &state.db);
            }
            app.manage(Arc::new(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![craftwork_invoke])
        // 窗口关闭：取消在途运行并按取消落审计（AGENTS 6.2）
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event
                && let Some(state) = window.app_handle().try_state::<Arc<AppState>>()
            {
                services::cancel_all_runs(state.inner());
            }
        })
        .build(tauri::generate_context!())
        .expect("craftwork 启动失败")
        .run(|app_handle, event| {
            // 应用退出（⌘Q 等）：同上兜底（窗口未关直接退出时 CloseRequested 不触发）
            if matches!(event, tauri::RunEvent::ExitRequested { .. })
                && let Some(state) = app_handle.try_state::<Arc<AppState>>()
            {
                services::cancel_all_runs(state.inner());
            }
        });
}
