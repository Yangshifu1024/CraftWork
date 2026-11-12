//! CraftWork 应用壳（Tauri 2）：启动装配与 IPC 接线。
//!
//! 分层纪律：本 crate 只做入口、窗口与 IPC 接线；全部领域逻辑在 crates/craft-*（不依赖 Tauri）。
//! 渲染层所有系统操作经统一入口 `craftwork_invoke` 按方法名分发到 services，与 channels.ts
//! 通道表三处同步接线（本文件 dispatch 缺臂由 tests/dispatch_test.rs 对拍防漏）；会话事件由
//! Bus 攒批后经 emit `craftwork:events` 广播，applyBatch 是渲染层唯一事件入口。

pub mod services;
pub mod storage;
pub mod system;
pub mod tray;

/// macOS 应用菜单（把系统「关于」接到设置页）。其他平台不装配菜单，故整模块按 cfg 收口。
#[cfg(target_os = "macos")]
pub mod app_menu;

use std::path::PathBuf;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::Manager;

use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_agent::runner::{Runner, RunnerDeps};
use craft_agent::session_log::{parse_level as parse_log_level, SessionLog, LEVEL_ERROR, LEVEL_INFO};
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
use craft_store::runs::list_stale_runs;
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

/// 主目录兜底链（HOME → USERPROFILE → HOMEDRIVE+HOMEPATH，见 `craft_core::platform::home_dir`）。
/// 全取不到时落系统临时目录：旧实现硬编码 `/tmp`，Windows 上解析成「当前盘符:\tmp」，
/// 数据目录会悄悄落到 `D:\tmp\.craftwork` 这类位置。
fn dirs_home() -> PathBuf {
    craft_core::platform::home_dir().unwrap_or_else(std::env::temp_dir)
}

/// 启动窗口装配：按持久化主题偏好设底色，然后显示窗口（conf 的 visible:false）。
/// 底色与渲染层 styles.css 的 --craft-background 两套值一致（theme.ts 的
/// syncWindowBackground 首帧后接管，值相同无跳变）；system 偏好按系统当前外观解析。
///
/// **Windows 的「启动即最大化」由 conf 的 `maximized: true` 在创建期应用**，不在 setup
/// 里手动 maximize——对隐藏/刚显示的窗口 maximize，Windows 会把当时的内缩残影
/// （14×14）记成「还原边界」，用户点还原窗口塌缩（实机踩坑两次：手动 maximize→show
/// 与 show→maximize 相邻调用都会触发；创建期应用则正确落边界，D22）。
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
        tauri::Theme::Dark => tauri::window::Color(0x2b, 0x2b, 0x2b, 0xff),
        _ => tauri::window::Color(0xec, 0xec, 0xee, 0xff),
    };
    let _ = window.set_background_color(Some(color));
    #[cfg(not(target_os = "windows"))]
    {
        // macOS：无 conf 平台覆盖，维持手动最大化（原生标题栏无还原边界坑）
        let _ = window.maximize();
    }
    let _ = window.show();
}

/// 装配全部共享服务（AppState），顺序即依赖序：DB → 事件出口（Bus 批次 → emit）→
/// Recorder（顺带把上次退出遗留的 running 标记落为 interrupted）→ 密钥 → 策略引擎/
/// 授权/审批 → 模型目录与工厂 → 工具 Registry → Runner；最后挂三个设置驱动钩子
/// （系统通知 / 失败转移 / 往返上限），均为调用时实时读设置、改开关立即生效。
fn build_app_state(app: &tauri::AppHandle, tray_signal: tray::TraySignal) -> Result<AppState, String> {
    let data_dir = resolve_data_dir();
    let data_dir_str = data_dir.to_string_lossy().to_string();
    let db = Arc::new(Db::open(&data_dir.join("craftwork.db")).map_err(|e| e.to_string())?);

    // 运行日志等级：设置初值 → 内存缓存（设置保存时更新，写入侧只读，改即生效）
    let saved_logs_level = db
        .with(|conn| get_setting(conn, "app"))
        .ok()
        .flatten()
        .and_then(|v| v.get("logsLevel").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "info".into());
    let log_level = Arc::new(AtomicU8::new(parse_log_level(&saved_logs_level)));
    let session_log = Arc::new(SessionLog::new(&data_dir, Arc::clone(&log_level)));

    // Shell 环境（决策 D18）：启动时恢复一次。缓存是进程内的，不恢复的话「上次选了
    // zsh」在重启后要等用户再进一次设置才生效。口径与设置读取一致：有持久化设置时
    // 用存值（空串 = 显式不使用），全新数据目录默认登录 shell。
    let saved_shell = services::effective_shell_path(&db);
    if !saved_shell.is_empty() {
        services::apply_shell_env(&saved_shell, Some(&session_log));
    }

    // 事件出口：Bus 批次 → tauri emit craftwork:events。
    // emit 失败此前静默丢弃（后端在跑、前端收不到、无缺口检测不补拉）——
    // 至少留 error 日志，排查「界面停在半截而数据完整」时与后端时间线对照。
    let handle = app.clone();
    let emit_log = Arc::clone(&session_log);
    let bus = Arc::new(Bus::new(move |batch: Batch| {
        // 托盘状态只在会话状态变化时刷新；流式文本事件不进 DB 查询。
        if batch
            .events
            .iter()
            .any(|event| event.event_type == craft_core::contract::event_type::THREAD_UPDATED)
        {
            tray::signal(&tray_signal);
        }
        let emit_result = serde_json::to_value(&batch).map(|payload| tauri::Emitter::emit(&handle, "craftwork:events", payload));
        if !matches!(&emit_result, Ok(Ok(()))) {
            let reason = match emit_result {
                Ok(Err(err)) => err.to_string(),
                Err(err) => err.to_string(),
                _ => String::new(),
            };
            eprintln!("craftwork: 事件广播失败（batch={} 条）: {reason}", batch.events.len());
            if let Some(thread_id) = &batch.thread_id {
                emit_log.write_thread(
                    thread_id,
                    LEVEL_ERROR,
                    "emit_failed",
                    json!({ "events": batch.events.len(), "error": reason }),
                );
            } else {
                emit_log.write_app(LEVEL_ERROR, "emit_failed", json!({ "events": batch.events.len(), "error": reason }));
            }
        }
    }));

    // 启动收尾：上次退出遗留的 running 运行标记 interrupted + error 事件；
    // 观测：启动恢复行留下上次异常退出的现场（哪些 run 被中断）。
    let recorder = Recorder::new(Arc::clone(&db), Arc::clone(&bus));
    let stale_runs = list_stale_runs(&db);
    let _ = recorder.interrupt_stale_runs();
    if !stale_runs.is_empty() {
        session_log.write_app(
            LEVEL_INFO,
            "startup_recovery",
            json!({
                "interruptedRuns": stale_runs.iter().map(|(run, thread)| json!({ "runId": run, "threadId": thread })).collect::<Vec<_>>(),
            }),
        );
    }

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
    // 后台命令任务注册表：同一个 Arc 交给 Registry（工具写入任务）与 Runner
    // （取走完成通知、空闲唤醒、退出清理），两侧共享才有闭环。
    let shell_tasks = Arc::new(craft_tools::shell_tasks::ShellTasks::new());
    // 上一进程遗留的 running 记录随旧进程一起消失，启动时统一标成已终止，
    // 否则任务行会永远停在「运行中」而通知永远不会来（参照 stale_subagent_events）。
    let reaped = shell_tasks.reap_stale();
    if reaped > 0 {
        session_log.write_app(LEVEL_INFO, "startup_shell_tasks_reaped", json!({ "count": reaped }));
    }
    let registry = Arc::new(Registry::new_with_shell_tasks(
        &backup_dir,
        &data_dir.to_string_lossy(),
        Arc::clone(&db),
        Arc::clone(&shell_tasks),
    ));
    // 内置工具注册（MCP 动态注册后续经同名 register 替换）
    register_builtin_tools(&registry);

    // MCP 服务器池：连接管理、工具发现与调用都收在 craft-mcp；壳层只做配置持久化、
    // 状态聚合与工具注册。同一个 Arc 交给 Registry（McpTool 经它调用）与服务层。
    let mcp_pool = Arc::new(craft_mcp::McpPool::new());
    craft_mcp::install_network_settings(Box::new({
        // MCP 远程传输走与模型客户端同一份代理设置（D14：所有出站请求经同一网络配置）
        let db = Arc::clone(&db);
        move || {
            db.with(|conn| get_setting(conn, "app"))
                .ok()
                .flatten()
                .and_then(|v| serde_json::from_value::<Settings>(v).ok())
                .map(|s| (s.network_proxy_mode, s.network_proxy_url))
                .unwrap_or_default()
        }
    }));
    craft_mcp::install_stderr_sink(Box::new({
        // MCP 服务器的 stderr 尾部写进应用日志（stdio 服务器崩溃排查的主要证据）
        let session_log = Arc::clone(&session_log);
        move |server_id, tail| {
            session_log.write_app(LEVEL_INFO, "mcp_server_stderr", json!({ "serverId": server_id, "tail": tail }));
        }
    }));

    let runner = Arc::new(Runner::new(RunnerDeps {
        db: Arc::clone(&db),
        records: recorder.clone(),
        registry: Arc::clone(&registry),
        engine: Arc::clone(&engine),
        approvals: Arc::clone(&approvals),
        models: Arc::clone(&models),
        shell_tasks: Some(Arc::clone(&shell_tasks)),
        attachment_dir: data_dir.join("attachments").to_string_lossy().to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        log: Arc::clone(&session_log),
        app_version: services::APP_VERSION.to_string(),
        // MCP 说明段的注入依据：注册表里当前注册了多少个 mcp__ 工具。闭包每次构造
        // 提示词时求值，连接 / 断开 / 保存设置后下一轮对话即生效。
        mcp_tool_count: Box::new({
            let registry = Arc::clone(&registry);
            move || registry.specs().iter().filter(|s| s.name.starts_with("mcp__")).count()
        }),
        // 出站代理解析器（决策 D14）：每次建立模型会话即时从
        // settings 读出并经 craft-core 解析（保存即生效）。无持久化设置回落默认 system
        // 模式；custom 模式地址非法（遗留脏数据）按直连兜底——禁代理比走错代理安全。
        proxy_config: Box::new({
            let db = Arc::clone(&db);
            move || {
                let (mode, url) = db
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
        }),
        // 子代理配置（决策 D15 扩展）：每次派发与每次组装工具面时求值——
        // 关开关立即从工具面消失、改并发上限与自定义定义立即生效。
        // 无持久化设置时回落默认（开启、并发 4、无自定义）。
        subagent_config: Box::new({
            let db = Arc::clone(&db);
            move || {
                use craft_core::contract::{SubAgentSettings, CustomAgentProfile, SUBAGENT_CONFIG_KEY, SUBAGENT_PROFILES_KEY};
                let settings = db
                    .with(|conn| get_setting(conn, SUBAGENT_CONFIG_KEY))
                    .ok()
                    .flatten()
                    .and_then(|v| serde_json::from_value::<SubAgentSettings>(v).ok());
                let custom = db
                    .with(|conn| get_setting(conn, SUBAGENT_PROFILES_KEY))
                    .ok()
                    .flatten()
                    .and_then(|v| serde_json::from_value::<Vec<CustomAgentProfile>>(v).ok())
                    .unwrap_or_default();
                let settings = settings.unwrap_or_else(|| SubAgentSettings {
                    enabled: true,
                    max_concurrent: craft_agent::subagent::DEFAULT_MAX_CONCURRENT as i64,
                    builtin_profiles: Vec::new(),
                });
                craft_agent::subagent::SubAgentConfig::new(
                    settings.enabled,
                    settings.max_concurrent,
                    settings.builtin_profiles,
                    custom,
                )
            }
        }),
        // Shell 环境重捕（决策 D18）：run 启动时缓存过期才调用。读设置仍由壳层负责，
        // craft-agent 不碰 settings 表；失败写 app.log 并清缓存，run 照常进行。
        refresh_shell_env: Arc::new({
            let db = Arc::clone(&db);
            let log = Arc::clone(&session_log);
            move || {
                let shell = db
                    .with(|conn| get_setting(conn, "app"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("shellPath").and_then(Value::as_str).map(String::from))
                    .unwrap_or_default();
                services::apply_shell_env(&shell, Some(&log));
            }
        }),
    }));

    // 后台命令任务的唤醒回调：命令一结束就推会话（而不是等下次发言才发现）
    runner.wire_shell_task_wake();

    // 提供商层观测：重试等事件写入共享日志 model.log（thread 关联靠时间与 per-thread 流日志对照）
    models.set_log_hook({
        let session_log = Arc::clone(&session_log);
        Arc::new(move |message| session_log.write_model(LEVEL_INFO, "provider_event", message))
    });

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
    // 往返上限：读设置即时生效。0 = 不限制（默认，Runner 侧 `limit <= 0` 即不封顶），
    // 不能在此把 0 过滤成默认值——那样设置页的「0 表示不限制」就是假的。
    runner.set_max_iterations_resolver(Box::new({
        let db = Arc::clone(&db);
        move || {
            db.with(|conn| get_setting(conn, "app"))
                .ok()
                .flatten()
                .and_then(|v| v.get("maxIterations").and_then(Value::as_i64))
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
        log_level,
        session_log: Arc::clone(&session_log),
        shell_tasks,
        mcp_pool,
    })
}

/// 启动时把已启用的 MCP 服务器连起来并注册工具（规格第 1 节 `enabled` 的语义）。
///
/// 为什么必须在启动时做：`enabled = true` 若只在「保存设置的那一刻」连接，
/// 用户重启应用后所有服务器都停在 disconnected，必须在设置页逐个点连接——设置项
/// 形同虚设。这里走与保存设置同一条链路（`mcp_apply`），失败只影响该服务器
/// （状态停在 Failed + 可读错误，设置页显示红灯），不影响其余服务器。
fn connect_enabled_mcp_servers(state: &AppState) {
    for config in services::mcp_configs(state) {
        if config.enabled {
            services::mcp_apply(state, &config);
        } else {
            state.mcp_pool.mark_disabled(&config.id);
        }
    }
}

/// 注册全部内置工具（fs / apply_patch / shell / git / todo / doc / skill / agent）。
/// 清单在 `craft_tools::builtin_tools()` 单点维护；MCP 动态工具留桩（决策 D5）：
/// 后续接入时经同名 register 覆盖。
/// agent_* 是子代理编排工具（决策 D15）：主会话可见、子代理工具面里被 Runner 剔除；
/// 设置关闭子代理后由 Runner 从模型可见工具面里隐藏（工具本身仍注册，便于统一报错文案）。
fn register_builtin_tools(registry: &Registry) {
    for tool in craft_tools::builtin_tools() {
        registry.register(tool);
    }
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
        // 打开数据目录：路径固定取服务端装配时确定的数据目录，不接受客户端传入
        "openDataDir" => system::open_folder(&state.data_dir),
        "minimiseWindow" => system::minimise_window(app),
        "toggleMaximiseWindow" => system::toggle_maximise_window(app),
        "windowIsMaximised" => system::window_is_maximised(app),
        "closeWindow" => system::close_window(app),
        // ---------- 设置 ----------
        "settingsGet" => settings_get(state),
        "settingsSave" => settings_save(state, &a.value_at(0)),
        "shellsList" => shells_list(),
        "shellEnvRefresh" => shell_env_refresh(state),
        "personalizationGet" => personalization_get(state),
        "personalizationSave" => personalization_save(state, &a.str_at(0)),
        // ---------- 会话 ----------
        "threadList" => thread_list(state, &a.str_at(0), a.bool_at(1)),
        "threadSearch" => thread_search(state, &a.str_at(0), &a.str_at(1)),
        "threadCreate" => thread_create(state, &a.str_at(0), &a.str_at(1), &a.str_at(2)),
        "threadRename" => thread_rename(state, &a.str_at(0), &a.str_at(1)),
        "threadSetModel" => thread_set_model(state, &a.str_at(0), &a.str_at(1)),
        "threadSetReasoning" => thread_set_reasoning(state, &a.str_at(0), &a.str_at(1)),
        "threadArchive" => thread_set_archived(state, &a.str_at(0), a.bool_at(1)),
        "threadDelete" => thread_delete(state, &a.str_at(0)),
        "threadClearTemp" => thread_clear_temp(state, &a.str_at(0)),
        // 快照与翻页共用实现：limit=0 不限条数（全量）；翻页传 limit + beforeSeq 游标（保留最新 limit 条）
        "threadSnapshot" => thread_snapshot(state, &a.str_at(0), 0, 0),
        "threadSnapshotPage" => thread_snapshot(state, &a.str_at(0), a.i64_at(1), a.i64_at(2)),
        "threadMarkRead" => thread_mark_read(state, &a.str_at(0)),
        "setBadgeCount" => system::set_badge_count(app, a.value_at(0).as_i64()),
        // ---------- 子代理（决策 D15） ----------
        "subagentList" => subagent_list(state, &a.str_at(0)),
        "subagentSettingsGet" => subagent_settings_get(state),
        "subagentSettingsSave" => subagent_settings_save(state, &a.value_at(0)),
        "subagentProfilesList" => subagent_profiles_get(state),
        "subagentProfileCreate" => subagent_profile_create(state, &a.value_at(0)),
        "subagentProfileUpdate" => subagent_profile_update(state, &a.str_at(0), &a.value_at(1)),
        "subagentProfileDelete" => subagent_profile_delete(state, &a.str_at(0)),
        "subagentProfileSetEnabled" => subagent_profile_set_enabled(state, &a.str_at(0), a.bool_at(1)),
        // ---------- 对话 ----------
        "chatCreateAndStart" => chat_create_and_start(
            state,
            &a.str_at(0),
            &a.str_at(1),
            &a.str_at(2),
            a.value_at(3).as_array().map(Vec::as_slice).unwrap_or(&[]),
            &a.str_at(4),
            &a.str_at(5),
            a.bool_at(6),
            a.bool_at(7),
        ),
        "chatSend" => chat_send(
            state,
            &a.str_at(0),
            &a.str_at(1),
            a.value_at(2).as_array().map(Vec::as_slice).unwrap_or(&[]),
            a.bool_at(3),
            a.bool_at(4),
        ),
        "chatSteer" => chat_steer(state, &a.str_at(0), &a.str_at(1), a.value_at(2).as_array().map(Vec::as_slice).unwrap_or(&[]), a.bool_at(3)),
        "chatRegenerate" => chat_regenerate(state, &a.str_at(0)),
        // ---------- 计划 ----------
        // 计划审阅走审批卡（approval.* 分支），无独立计划通道
        // ---------- 输入预测 ----------
        "inputPredictionGet" => input_prediction_get(state, &a.str_at(0)),
        "chatCancel" => chat_cancel(state, &a.str_at(0)),
        "chatBashOutput" => chat_bash_output(state, &a.str_at(0), &a.str_at(1), a.i64_at(2)),
        "chatKillShell" => chat_kill_shell(state, &a.str_at(0), &a.str_at(1)),
        "chatCancelWait" => chat_cancel_wait(state, &a.str_at(0)),
        "chatDecideApproval" => {
            // async 服务：在当前异步上下文内直接驱动
            let approval_id = a.str_at(0);
            let allow = a.bool_at(1);
            let scope = a.str_at(2);
            // 第 3 参为修改意见（可选；仅计划审阅卡的「提出修改」使用）
            let feedback = a.value_at(3).as_str().map(|s| s.to_string());
            // dispatch 在 spawn_blocking 里——decide 需要 tokio 上下文：用 Handle::current 不可得，
            // 改为起一个临时 current-thread 运行时驱动（审批等待是 oneshot，开销可忽略）
            let approvals = Arc::clone(&state.approvals);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            rt.block_on(async move { approvals.decide(&approval_id, allow, &scope, feedback.as_deref()).await })?;
            Ok(json!(null))
        }
        // ---------- 工作区与文件 ----------
        "workspaceList" => workspace_list(state),
        "workspaceAdd" => workspace_add(state, &a.str_at(0), &a.str_at(1)),
        "workspaceAddProject" => workspace_add_project(state, &a.str_at(0), a.value_at(1).as_array().map(Vec::as_slice).unwrap_or(&[]), &a.str_at(2)),
        "workspaceSetTrust" => workspace_set_trust(state, &a.str_at(0), &a.str_at(1)),
        "workspaceRename" => workspace_rename(state, &a.str_at(0), &a.str_at(1)),
        "workspaceReplaceFolders" => workspace_replace_folders(state, &a.str_at(0), a.value_at(1).as_array().map(Vec::as_slice).unwrap_or(&[])),
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
        // 网络页公开端点探测（决策 D14 第 6 节修订）：id 空串全测，非空单行重测；
        // 行清单走单独的只读通道，界面据此固定渲染九行（探测只改单格状态，不增删行）
        "networkProbeTargets" => network_probe_targets(state),
        "networkProbeEndpoints" => network_probe_endpoints(state, &a.str_at(0)),
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
        // ---------- 存储（设置 → 数据与统计 → 存储，决策 D27） ----------
        "storageOverview" => storage_overview(state),
        "storageThreads" => storage_threads(state),
        "storageDbTables" => storage_db_tables(state),
        "storageCleanup" => storage_cleanup(state, &a.str_at(0)),
        "mcpList" => mcp_list(state),
        "mcpUpsert" => mcp_upsert(state, &a.value_at(0)),
        "mcpRemove" => mcp_remove(state, &a.str_at(0)),
        "mcpSetEnabled" => mcp_set_enabled(state, &a.str_at(0), a.bool_at(1)),
        "mcpTest" => mcp_test(state, &a.value_at(0)),
        "mcpConnect" => mcp_connect(state, &a.str_at(0)),
        "mcpDisconnect" => mcp_disconnect(state, &a.str_at(0)),
        "skillsList" => skills_list(state, &a.str_at(0)),
        "skillsListAll" => skills_list_all(state),
        "skillsSetEnabled" => skills_set_enabled(state, &a.str_at(0), a.bool_at(1), &a.str_at(2)),
        "skillsRemove" => skills_remove(state, &a.str_at(0), &a.str_at(1)),
        "skillsRead" => skills_read(state, &a.str_at(0), &a.str_at(1)),
        // ---------- 技能商店（决策 D12，spec 第 11 节） ----------
        "storeListMarkets" => store_list_markets(state),
        "storeAddMarket" => store_add_market(state, &a.str_at(0)),
        "storeRemoveMarket" => store_remove_market(state, &a.str_at(0)),
        "storeRefreshMarkets" => store_refresh_markets(state, &a.str_at(0)),
        "storeListPlugins" => store_list_plugins(state),
        "storeInstallPlugin" => store_install_plugin(state, &a.str_at(0)),
        "storeUpdatePlugin" => store_update_plugin(state, &a.str_at(0)),
        "storeRollbackPlugin" => store_rollback_plugin(state, &a.str_at(0)),
        "storeUninstallPlugin" => store_uninstall_plugin(state, &a.str_at(0)),
        "storeSetPluginEnabled" => store_set_plugin_enabled(state, &a.str_at(0), a.bool_at(1)),
        "storeDescribePlugin" => store_describe_plugin(state, &a.str_at(0)),
        "savePastedAttachment" => save_pasted_attachment(state, &a.str_at(0), &a.str_at(1)),
        "readAttachment" => read_attachment(state, &a.str_at(0)),
        _ => Err(format!("方法 {method} 尚未接线")),
    }
}

/// 应用入口：注册插件 → 装配 AppState 与内置临时会话目录 → 按主题装配主窗口 → 管理
/// 共享状态。窗口关闭（CloseRequested）与应用退出（ExitRequested）两处兜底取消在途
/// 运行并按取消落审计（AGENTS 6.2：不允许静默中断）。
///
/// macOS 菜单（app_menu）：Tauri 默认菜单的「关于」是原生预定义项、点击不发出事件，
/// 接不到应用内页面，故显式装一份打过补丁的菜单；菜单动作经 `craftwork:menu` 推送通道
/// 交给渲染层（App.tsx 打开设置 → 关于）。仅 macOS 装配，其他平台保持无菜单栏。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 托盘状态线程的信号通道：Bus 只投递「有 thread.updated」，由托盘线程合并后重查。
    // Sender 不是 Sync，包一层 Mutex 才能进 Bus 的 Sync 回调。
    let (tray_tx, tray_rx) = std::sync::mpsc::channel::<()>();
    let tray_signal: tray::TraySignal = Arc::new(Mutex::new(tray_tx));

    let builder = tauri::Builder::default()
        // 单实例必须第一个注册：第二次启动只唤出已有窗口，不新开进程。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init());

    #[cfg(target_os = "macos")]
    let builder = builder
        .menu(app_menu::build)
        .on_menu_event(app_menu::handle_event);

    builder
        .setup(move |app| {
            let state = Arc::new(
                build_app_state(&app.handle().clone(), Arc::clone(&tray_signal))
                    .map_err(std::io::Error::other)?,
            );
            // 启动确保内置临时会话目录
            let _ = services::ensure_conversation_workspace(&state);
            // 按主题偏好设好底色、最大化后再显示窗口（消除启动黑/白闪 + 启动即最大化）
            if let Some(window) = app.get_webview_window("main") {
                setup_main_window(&window, &state.db);
            }
            // MCP：把 enabled 的服务器连起来并注册工具。放到后台线程——stdio 服务器
            // 冷启动要等子进程握手（可能几百毫秒到数秒），在 setup 里同步做会拖住首屏。
            // 失败只让该服务器显示红灯，不影响启动。
            let state_for_mcp = Arc::clone(&state);
            std::thread::Builder::new()
                .name("craftwork-mcp-boot".into())
                .spawn(move || connect_enabled_mcp_servers(&state_for_mcp))
                .ok();
            app.manage(state);
            // 托盘在状态管理之后装配；失败时关闭窗口必须走旧语义，避免隐藏后无入口。
            match tray::setup(app.handle()) {
                Ok(()) => {
                    tray::start_worker(app.handle().clone(), tray_rx);
                    tray::refresh(app.handle());
                }
                Err(err) => {
                    if let Some(state) = app.try_state::<Arc<AppState>>() {
                        state.session_log.write_app(
                            LEVEL_ERROR,
                            "tray_setup_failed",
                            json!({ "error": err.to_string() }),
                        );
                    } else {
                        eprintln!("craftwork: 托盘创建失败: {err}");
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![craftwork_invoke])
        // 窗口关闭：默认隐藏到托盘（run / 后台命令 / MCP 继续）；设置开启「关闭窗口时退出应用」
        // 或托盘不可用时，保持旧的取消 + shutdown + 审计语义。
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                if tray::should_hide_on_close(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                    tray::on_window_hidden(window.app_handle());
                    return;
                }
                if let Some(state) = window.app_handle().try_state::<Arc<AppState>>() {
                    services::cancel_all_runs(state.inner());
                    // 后台命令任务已脱离 run，取消 run 带不走它们：关窗即整组杀，避免孤儿进程
                    state.runner.shutdown_shell_tasks();
                    // MCP：CloseAll 逐个断开（rmcp 的 transport.close 触发子进程退出），
                    // 之后再结束 craftwork-mcp 线程（规格第 5 节）
                    state.mcp_pool.shutdown();
                }
            }
            // 最小化 / 还原 / 聚焦后同步「显示主窗口 / 隐藏主窗口」标签。
            tauri::WindowEvent::Focused(_) | tauri::WindowEvent::Resized(_) => {
                tray::sync_toggle_label(window.app_handle());
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("craftwork 启动失败")
        .run(|app_handle, event| match event {
            // 应用退出（⌘Q / 托盘退出等）：同上兜底（窗口未关直接退出时 CloseRequested 不触发）
            tauri::RunEvent::ExitRequested { .. } => {
                if let Some(state) = app_handle.try_state::<Arc<AppState>>() {
                    services::cancel_all_runs(state.inner());
                    state.runner.shutdown_shell_tasks();
                    state.mcp_pool.shutdown();
                }
            }
            // macOS：点击 Dock 图标恢复窗口（隐藏到托盘后 Dock 图标仍保留）。
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => tray::show_main_window(app_handle),
            _ => {}
        });
}
