//! 壳层服务分发集成测试：对话主链路（隔离目录）。
//! 覆盖：启动 ensure 临时目录 → 设置往返 → 会话创建/快照 → 首发原子路径（无模型失败零落库）
//! → 文件读写与 diff → 提供商 CRUD 与密钥。

use std::sync::Arc;

use serde_json::{json, Value};

// 直接复用壳层 crate 的服务函数（lib 目标）
use craftwork_lib::services;

/// 与 services::canon_folder 同一规则的期望值：canonicalize + 剥 Windows verbatim 前缀。
/// 存库形式是非 verbatim 干净路径（`\\?\C:\...` 会被路径校验的 UNC 守卫全灭）。
fn canon(p: &str) -> String {
    craft_policy::resolve::strip_verbatim(&std::fs::canonicalize(p).unwrap())
        .to_string_lossy()
        .to_string()
}

/// 最小装配 AppState（与启动装配 build_app_state 同构，但无 Tauri AppHandle 依赖）：
/// 隔离临时目录、空密钥存根、Bus 丢弃事件（本测试直接查返回值，不验证广播）。
fn test_state(tag: &str) -> (tempdir::TempDir, Arc<services::AppState>) {
    let dir = tempdir::TempDir::new(tag).unwrap();
    let data_dir = dir.path().to_string_lossy().to_string();
    let db = Arc::new(craft_store::db::Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    let secrets = Arc::new(craft_core::secret::SecretStore::new_empty(dir.path()));
    let engine = Arc::new(std::sync::Mutex::new(craft_policy::Engine::default()));
    let catalog = Arc::new(craft_model::catalog::Catalog::new(Arc::clone(&db)));
    let models = Arc::new(craft_model::factory::ModelFactory::new(
        Arc::clone(&catalog),
        Box::new(stub_keys()),
    ));
    let backup_dir = dir.path().join("backups").to_string_lossy().to_string();
    // 后台命令任务注册表：Registry 与 Runner 必须共用同一份，否则后台命令没有完成通知
    let shell_tasks = Arc::new(craft_tools::shell_tasks::ShellTasks::new());
    let registry = Arc::new(craft_tools::registry::Registry::new_with_shell_tasks(
        &backup_dir, "", Arc::clone(&db), Arc::clone(&shell_tasks),
    ));
    // 注册测试用到的内置工具：自定义子代理的 tools.Only 白名单会被过滤到注册表实际存在的
    // 名字上（防幽灵工具），空注册表会让 fs_read 这类名字全部消失。
    registry.register(Box::new(craft_tools::fs::FsRead));
    registry.register(Box::new(craft_tools::fs::FsWrite));
    registry.register(Box::new(craft_tools::shell::ShellExec));
    let grants = Arc::new(craft_agent::grants::Grants::new(Arc::clone(&db)));
    let recorder = craft_store::recorder::Recorder::new(
        Arc::clone(&db),
        Arc::new(craft_core::bus::Bus::new(|_| {})),
    );
    let approvals = Arc::new(craft_agent::approvals::Approvals::new(
        Arc::clone(&db),
        recorder.clone(),
        Arc::clone(&grants),
    ));
    let log_level = Arc::new(std::sync::atomic::AtomicU8::new(craft_agent::session_log::LEVEL_INFO));
    let session_log = Arc::new(craft_agent::session_log::SessionLog::new(
        dir.path(),
        Arc::clone(&log_level),
    ));
    let runner = Arc::new(craft_agent::runner::Runner::new(craft_agent::runner::RunnerDeps {
        db: Arc::clone(&db),
        records: recorder.clone(),
        registry: Arc::clone(&registry),
        engine: Arc::clone(&engine),
        approvals: Arc::clone(&approvals),
        models: Arc::clone(&models),
        attachment_dir: dir.path().join("attachments").to_string_lossy().to_string(),
        data_dir: data_dir.clone(),
        log: Arc::clone(&session_log),
        app_version: "test".into(),
        mcp_tool_count: Box::new(|| 0),
        // 测试装配：解析为直连（无代理干扰）；生产装配在 lib.rs 从 settings 即时读出
        proxy_config: Box::new(|| craft_core::network::ProxyConfig {
            mode: craft_core::network::ProxyMode::System,
            effective_url: None,
        }),
        // 子代理：测试默认开启、并发 4（与生产缺省一致）
        subagent_config: Box::new(craft_agent::subagent::SubAgentConfig::default),
        refresh_shell_env: Arc::new(|| {}),
        shell_tasks: Some(Arc::clone(&shell_tasks)),
    }));
    let attachment_dir = dir.path().join("attachments").to_string_lossy().to_string();
    (
        dir,
        Arc::new(services::AppState {
            db,
            secrets,
            engine,
            catalog,
            models,
            registry,
            runner,
            approvals,
            recorder,
            attachment_dir,
            data_dir,
            backup_dir,
            log_level,
            session_log,
            shell_tasks,
            mcp_pool: Arc::new(craft_mcp::McpPool::new()),

        }),
    )
}

/// 恒返回空串的密钥提供者：真实网络调用会失败，不影响配置面（设置/会话/提供商 CRUD）测试。
fn stub_keys() -> impl craft_model::factory::KeyProvider {
    struct Stub;
    impl craft_model::factory::KeyProvider for Stub {
        fn get_for(&self, _provider_id: &str, _env_names: &[String]) -> String {
            String::new()
        }
    }
    Stub
}

// 最小临时目录助手
mod tempdir {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Result<Self, String> {
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "cw-svc-{}-{}-{}-{}",
                tag,
                std::process::id(),
                n,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos()
            ));
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            Ok(TempDir(path))
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn shell_env_refresh_surfaces_capture_failure_instead_of_silent_fallback() {
    // 契约（spec §6.1）：选了一个读不出环境的 shell 时，refresh 必须把原因放进 error，
    // 不能静默回落成「看起来一切正常」。用不存在的路径模拟捕获失败——设置层会拒绝存
    // 这种值，故直接改库里的值绕过校验。
    let (_dir, state) = test_state("shell-refresh-fail");
    let missing = "/definitely/not/a/shell-cw-test";
    let mut settings = services::settings_get(&state).unwrap();
    settings["shellPath"] = json!(missing);
    state
        .db
        .with(|conn| craft_store::records::set_setting(conn, "app", &settings))
        .unwrap();

    let status = services::shell_env_refresh(&state).unwrap();
    assert_eq!(status["shellPath"], missing);
    assert_eq!(status["varCount"], 0);
    let err = status["error"].as_str().unwrap_or_default();
    assert!(!err.is_empty(), "捕获失败必须回传原因，实际：{status}");

    // 同时必须落盘：此前只有 eprintln!，GUI 双击启动时 stderr 无处可看，等于丢弃。
    let log = std::fs::read_to_string(std::path::Path::new(&state.data_dir).join("logs").join("app.log"))
        .unwrap_or_default();
    assert!(
        log.contains("shell_env_capture_failed"),
        "捕获失败应写 app.log，实际内容：{log}"
    );
}

#[test]
fn saved_empty_shell_stays_off_instead_of_defaulting() {
    // 全新数据目录默认登录 shell；一旦保存过设置，空串就是用户显式选的「不使用」，
    // 不能被默认值重新覆盖（D18 修订的边界）。
    let (_dir, state) = test_state("shell-default");
    let fresh = services::settings_get(&state).unwrap();
    assert_eq!(
        fresh["shellPath"],
        json!(services::default_login_shell()),
        "全新数据目录应使用登录 shell 默认值"
    );

    let mut settings = fresh;
    settings["shellPath"] = json!("");
    state
        .db
        .with(|conn| craft_store::records::set_setting(conn, "app", &settings))
        .unwrap();
    assert_eq!(services::settings_get(&state).unwrap()["shellPath"], json!(""), "显式空串必须保留");
    assert_eq!(services::effective_shell_path(&state.db), "");
}

#[test]
fn shell_env_refresh_without_selection_is_quiet() {
    // 未选中（空串）是正常状态，不该报错：默认「不使用」不能变成红色警告。
    // 全新数据目录现在会默认登录 shell，所以先显式保存空串来构造「不使用」。
    let (_dir, state) = test_state("shell-refresh-empty");
    let mut settings = services::settings_get(&state).unwrap();
    settings["shellPath"] = json!("");
    state
        .db
        .with(|conn| craft_store::records::set_setting(conn, "app", &settings))
        .unwrap();
    let status = services::shell_env_refresh(&state).unwrap();
    assert_eq!(status["shellPath"], "");
    assert_eq!(status["varCount"], 0);
    assert!(status["error"].is_null(), "未启用时 error 应为 null：{status}");
}

#[test]
fn open_folder_rejects_missing_directory() {
    // 只验证拒绝分支：目录不存在时不启动文件管理器进程，返回可读中文错误。
    // （存在分支会真的拉起访达，不在自动化测试里执行。）
    let err = craftwork_lib::system::open_folder("/definitely/not/a/dir-cw-test").unwrap_err();
    assert_eq!(err, "目录不存在");
}

#[test]
fn get_info_reports_build_metadata() {
    let (_dir, state) = test_state("info");
    let info = services::get_info(&state).unwrap();
    // 版本来自 Cargo 工作区版本（不是硬编码常量），构建期元数据由 build.rs 注入
    assert_eq!(info["version"].as_str(), Some(env!("CARGO_PKG_VERSION")), "版本应取 Cargo 工作区版本");
    assert_eq!(info["name"].as_str(), Some("CraftWork"));
    assert!(!info["gitHash"].as_str().unwrap_or_default().is_empty(), "Git 哈希不应为空");
    assert!(!info["buildTime"].as_str().unwrap_or_default().is_empty(), "编译时间不应为空");
    assert_eq!(info["dataDir"].as_str(), Some(state.data_dir.as_str()));
    // 数据库体积已从 getInfo 移到存储分节（D27），这里不再有该字段
    assert!(info.get("dbSizeBytes").is_none(), "数据库体积归存储页，AppInfo 不再携带");
}

/// 存储分节首屏：库体积为正、分类齐全、「其他」垫底、无任务时门禁为空。
#[test]
fn storage_overview_reports_database_and_categories() {
    let (_dir, state) = test_state("storage-overview");
    let view = services::storage_overview(&state).unwrap();
    assert!(view["database"]["bytes"].as_u64().unwrap_or(0) > 0, "库体积应为正数");
    let categories = view["categories"].as_array().expect("应返回分类列表");
    assert!(categories.iter().any(|c| c["kind"] == "logs"), "应含会话日志分类");
    assert_eq!(categories.last().unwrap()["kind"], "other", "「其他」必须垫底");
    assert_eq!(view["busyReason"].as_str(), Some(""), "无任务运行时门禁为空");
    assert_eq!(view["dataDir"].as_str(), Some(state.data_dir.as_str()));
}

/// 会话占用视图在空库上也要能出：没有会话时分组为空、总数归零，而不是报错。
#[test]
fn storage_threads_view_is_empty_without_sessions() {
    let (_dir, state) = test_state("storage-threads");
    let view = services::storage_threads(&state).unwrap();
    assert_eq!(view["threadTotal"].as_u64(), Some(0));
    assert!(view["projects"].as_array().unwrap().is_empty());
    assert!(view["orphanThreads"].as_array().unwrap().is_empty());
}

/// 清理动作：未知类型直接拒（不静默当成功），已知类型正常返回结果结构。
#[test]
fn storage_cleanup_rejects_unknown_kind_and_accepts_known() {
    let (_dir, state) = test_state("storage-cleanup");
    let err = services::storage_cleanup(&state, "nuke-everything").expect_err("未知清理项必须被拒");
    assert!(err.contains("未知的清理项"), "{err}");
    let result = services::storage_cleanup(&state, "logsOrphans").unwrap();
    assert_eq!(result["kind"].as_str(), Some("logsOrphans"));
    assert!(result["freedBytes"].is_u64());
    assert!(result["failures"].is_array());
}

#[test]
fn startup_ensure_and_settings_roundtrip() {
    let (_dir, state) = test_state("basic");
    // ensure 后 workspaceList 含临时目录
    let ws_id_value = services::ensure_conversation_workspace(&state).unwrap();
    let ws_id = ws_id_value.as_str().unwrap().to_string();
    let list = services::workspace_list(&state).unwrap();
    assert!(list.to_string().contains("临时会话"));
    assert!(list.to_string().contains(&ws_id));

    // 设置默认值与保存往返
    let settings = services::settings_get(&state).unwrap();
    assert_eq!(settings["theme"], "system");
    assert_eq!(settings["smoothFollow"], false, "平滑跟随默认关闭");
    assert_eq!(settings["quitOnClose"], false, "关闭窗口默认隐藏到托盘");
    let saved = services::settings_save(
        &state,
        &json!({ "theme": "dark", "fontSize": 20, "smoothFollow": true, "quitOnClose": true }),
    )
    .unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["fontSize"], 15, "越界字号回退默认");
    assert_eq!(saved["smoothFollow"], true, "平滑跟随保存往返");
    assert_eq!(saved["quitOnClose"], true, "关闭即退出开关保存往返");
    assert!(saved.get("panelWidth").is_none(), "面板宽度属会话内姿态，不在设置里（决策 D17）");
}

#[test]
fn first_message_atomic_path_no_model_failure_persists_nothing() {
    let (_dir, state) = test_state("chat");
    let ws_id = services::ensure_conversation_workspace(&state).unwrap().as_str().unwrap().to_string();

    // 模型不存在：失败且不产生会话
    let err = services::chat_create_and_start(&state, "code", &ws_id, "你好", &[], "nope/model", "", false, false).unwrap_err();
    assert!(err.contains("模型不存在"));
    let threads = services::thread_list(&state, "code", false).unwrap();
    assert_eq!(threads.as_array().unwrap().len(), 0, "失败首发零落库");
}

#[test]
fn thread_creation_and_snapshot_chain() {
    let (_dir, state) = test_state("thread");
    let ws_id = services::ensure_conversation_workspace(&state).unwrap().as_str().unwrap().to_string();

    // 直接构造「已解析模型」的提供商数据，使首发校验通过
    let provider = json!({
        "id": "stub", "name": "Stub", "kind": "openai-compatible",
        "endpoint": "https://stub.invalid/v1", "keyRequired": false,
        "models": [{ "id": "m1", "name": "模型一", "contextK": 8, "maxTokens": 512,
                     "caps": {"stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false},
                     "status": "", "source": "manual", "enabled": true }],
    });
    services::provider_save(&state, &provider).unwrap();
    assert!(services::provider_list(&state).unwrap().to_string().contains("\"stub/m1\""));

    // 首发：模型解析通过（连接 stub 端点会失败——run 的失败路径写 error 事件，不阻断首发本身）
    let result = services::chat_create_and_start(&state, "code", &ws_id, "帮我看看这个项目", &[], "stub/m1", "", false, false).unwrap();
    let thread_id = result["threadId"].as_str().unwrap().to_string();
    assert!(result["runId"].as_str().is_some());

    // 快照含用户消息；runner 后台失败（stub 端点不可达）完成后状态定局
    let mut messages: Vec<Value> = Vec::new();
    for _ in 0..50 {
        let snap = services::thread_snapshot(&state, &thread_id, 0, 0).unwrap();
        let thread_status = snap["thread"]["status"].as_str().unwrap_or_default().to_string();
        messages = snap["messages"].as_array().unwrap().clone();
        if thread_status == "error" || thread_status == "idle" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!messages.is_empty());
    let user_msg = messages
        .iter()
        .find(|m| m["role"] == "user")
        .expect("快照应含用户消息");
    assert_eq!(user_msg["parts"][0]["payload"]["text"], "帮我看看这个项目");

    // 会话列表显示自动标题
    let threads = services::thread_list(&state, "code", false).unwrap();
    assert!(threads.to_string().contains("帮我看看这个项目"));
}

#[test]
fn snapshot_replaces_legacy_tool_name_title_with_human_readable_one() {
    let (_dir, state) = test_state("tool-title");
    let ws_id = services::ensure_conversation_workspace(&state).unwrap().as_str().unwrap().to_string();
    let thread_id = thread_id_of(&services::thread_create(&state, "code", &ws_id, "读文件").unwrap());

    // 模拟旧历史：标题补齐之前落库的 tool_call，title 就是工具名 fs_read。
    let message_id = "msg_legacy";
    for (tool_call_id, name) in [("call_1", "fs_read"), ("call_2", "mcp__demo__search")] {
        state
            .recorder
            .append(&craft_store::events::EventRecord {
                thread_id: thread_id.clone(),
                message_id: message_id.into(),
                run_id: None,
                kind: "tool_call".into(),
                payload: json!({
                    "partId": format!("part_{tool_call_id}"),
                    "toolCallId": tool_call_id,
                    "name": name,
                    "args": {},
                    "risk": "R0",
                    "status": "done",
                    "output": "ok",
                    "isError": false,
                    "code": "",
                    "durationMs": 3,
                    "summary": "src/main.rs",
                    "title": name,
                }),
            })
            .unwrap();
    }

    let snap = services::thread_snapshot(&state, &thread_id, 0, 0).unwrap();
    let parts = snap["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == message_id)
        .unwrap()["parts"]
        .as_array()
        .unwrap()
        .clone();
    let title_of = |tool_call_id: &str| {
        parts
            .iter()
            .find(|p| p["payload"]["toolCallId"] == tool_call_id)
            .unwrap()["payload"]["title"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // 注册表里有内置标题：旧记录的工具名被换成中文标题
    assert_eq!(title_of("call_1"), "读取文件");
    // 注册表里没有的（MCP 动态工具）：保持原载荷，不做推测性替换
    assert_eq!(title_of("call_2"), "mcp__demo__search");
    // 工具名本身不被改写（审计与排查仍能定位到真实工具）
    let name_of = |tool_call_id: &str| {
        parts
            .iter()
            .find(|p| p["payload"]["toolCallId"] == tool_call_id)
            .unwrap()["payload"]["name"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(name_of("call_1"), "fs_read");
}

#[test]
fn file_write_and_diff() {
    let (_dir, state) = test_state("files");
    let ws_id = services::ensure_conversation_workspace(&state).unwrap().as_str().unwrap().to_string();

    let written = services::workspace_write_file(&state, &ws_id, "", "notes/a.txt", "第一行\n第二行", "").unwrap();
    assert!(written.is_null());

    let content = services::workspace_read_file(&state, &ws_id, "", "notes/a.txt").unwrap();
    assert_eq!(content["content"], "第一行\n第二行");
    let hash = content["hash"].as_str().unwrap().to_string();

    // expectedHash 不匹配：拒绝覆盖
    let err = services::workspace_write_file(&state, &ws_id, "", "notes/a.txt", "改写", "wrong-hash").unwrap_err();
    assert!(err.contains("外部修改"));

    // 一致：写入成功，且有备份记录
    services::workspace_write_file(&state, &ws_id, "", "notes/a.txt", "改写", &hash).unwrap();
    let diff = services::workspace_compute_diff(&state, "第一行\n第二行", "改写").unwrap();
    assert_eq!(diff["deletions"], 2);
    assert_eq!(diff["additions"], 1);
}

#[test]
fn provider_key_and_enable_disable() {
    let (_dir, state) = test_state("provider");
    let provider = json!({
        "id": "p1", "name": "P1", "kind": "openai", "endpoint": "https://api.p1.com/v1",
        "keyRequired": true,
        "models": [],
    });
    services::provider_save(&state, &provider).unwrap();
    services::provider_set_key(&state, "p1", "sk-secret-abcdef-1234").unwrap();
    let list = services::provider_list(&state).unwrap();
    let text = list.to_string();
    assert!(text.contains("file"), "密钥来源应为 file");
    assert!(text.contains("sk-se"), "遮罩应含前缀");
    assert!(!text.contains("sk-secret"), "完整密钥不得出现在列表返回中");

    services::provider_set_enabled(&state, "p1", false).unwrap();
    let list = services::provider_list(&state).unwrap();
    assert!(list.to_string().contains("\"enabled\":false"));

    services::provider_clear_key(&state, "p1").unwrap();
    let list = services::provider_list(&state).unwrap();
    assert!(list.to_string().contains("\"keySource\":\"\""));
}

#[test]
fn builtin_catalog_load_and_import() {
    let (_dir, state) = test_state("catalog");
    let meta = services::catalog_meta(&state).unwrap();
    assert!(meta["providerCount"].as_u64().unwrap() > 100);
    let providers = services::catalog_providers(&state, "openai").unwrap();
    assert!(!providers.as_array().unwrap().is_empty());
    // 导入 openai 目录全部模型
    let imported = services::catalog_import(&state, "openai", true, &[]).unwrap();
    assert!(imported.to_string().contains("gpt"), "导入应包含 openai 模型");
}

#[test]
fn provider_save_empty_id_generated_server_side_returns_single_object() {
    let (_dir, state) = test_state("provider-save-empty-id");
    let provider = json!({
        "id": "", "name": "手动提供商", "kind": "openai-compatible",
        "endpoint": "https://stub.invalid/v1", "keyRequired": true,
        "models": [{ "id": "m1", "name": "模型一", "contextK": 8, "maxTokens": 0,
                     "caps": {"stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false},
                     "status": "", "source": "manual", "enabled": true }],
    });
    // 迁移回归（migration-parity 票 01）：空 id 曾直接报「提供商 ID 不能为空」，手动新增提供商全废
    let view = services::provider_save(&state, &provider).unwrap();
    let id = view["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("custom_"), "服务端应生成 custom_* ID，实际 {id}");
    assert_eq!(view["name"], "手动提供商", "返回应为单个 ProviderView（对齐 channels 声明）");
    // 再以同一 id 保存走更新分支，仍返回单对象
    let updated = services::provider_save(&state, &json!({ "id": id, "name": "改名", "kind": "openai-compatible", "endpoint": "", "keyRequired": true, "models": [] })).unwrap();
    assert_eq!(updated["id"], id.as_str());
    assert_eq!(updated["name"], "改名");
}

/// 自定义请求头：保存时服务端校验（trim / 冲突名拒绝 / 换行拒绝），视图带出；
/// 未携带 headers 的部分更新（端点保存路径）保留现值；目录导入保留已存请求头。
#[test]
fn provider_custom_headers_save_validate_and_partial_update_preservation() {
    let (_dir, state) = test_state("provider-headers");
    // 保存带自定义头（网关路由头场景）
    let view = services::provider_save(&state, &json!({
        "id": "gw", "name": "网关", "kind": "openai-compatible",
        "endpoint": "https://gw.invalid/v1", "keyRequired": true,
        "headers": [{ "name": " x-opencode-session ", "value": " ses_1 " }],
        "models": [],
    })).unwrap();
    assert_eq!(view["headers"][0]["name"], "x-opencode-session", "保存时应 trim");
    assert_eq!(view["headers"][0]["value"], "ses_1");

    // 与密钥字段冲突：拒绝保存
    let err = services::provider_save(&state, &json!({
        "id": "gw", "name": "网关", "kind": "openai-compatible",
        "endpoint": "https://gw.invalid/v1", "keyRequired": true,
        "headers": [{ "name": "Authorization", "value": "Bearer x" }],
        "models": [],
    })).unwrap_err();
    assert!(err.contains("密钥字段冲突"), "{err}");

    // 值含换行（头注入）：拒绝保存
    let err = services::provider_save(&state, &json!({
        "id": "gw", "name": "网关", "kind": "openai-compatible",
        "endpoint": "https://gw.invalid/v1", "keyRequired": true,
        "headers": [{ "name": "X-A", "value": "v\nX-Evil: 1" }],
        "models": [],
    })).unwrap_err();
    assert!(err.contains("换行"), "{err}");

    // 不带 headers 的部分更新（端点保存路径）：保留现值
    let untouched = services::provider_save(&state, &json!({
        "id": "gw", "name": "网关", "kind": "openai-compatible",
        "endpoint": "https://gw2.invalid/v1", "keyRequired": true, "models": [],
    })).unwrap();
    assert_eq!(untouched["headers"][0]["name"], "x-opencode-session", "部分更新不得抹掉已存请求头");

    // 显式传空数组：整体清空（清空也是合法操作）
    let cleared = services::provider_save(&state, &json!({
        "id": "gw", "name": "网关", "kind": "openai-compatible",
        "endpoint": "https://gw2.invalid/v1", "keyRequired": true, "headers": [], "models": [],
    })).unwrap();
    assert!(cleared["headers"].as_array().unwrap().is_empty());

    // 目录导入：新导入的提供商无请求头，不继承其他提供商的配置
    let imported = services::catalog_import(&state, "openai", false, &["gpt-4o".into()]).unwrap();
    let imported_entry = imported.as_array().unwrap().iter().find(|p| p["id"] == json!("openai")).unwrap();
    assert!(imported_entry["headers"].as_array().unwrap().is_empty());
}

#[test]
fn workspace_add_empty_name_derivation_dedup_ordinal_and_single_object() {
    let (dir, state) = test_state("ws-add-project");
    let root = dir.path().join("a");
    std::fs::create_dir_all(root.join("proj")).unwrap();
    std::fs::create_dir_all(root.join("b").join("proj")).unwrap();
    std::fs::create_dir_all(root.join("c").join("proj")).unwrap();
    let p1 = root.join("proj").to_string_lossy().to_string();
    let p2 = root.join("b").join("proj").to_string_lossy().to_string();
    let p3 = root.join("c").join("proj").to_string_lossy().to_string();

    // 空名：从主要目录叶子名推导（migration-parity 票 02；此前空名直接报错，新建项目全废）
    let view = services::workspace_add_project(&state, "", json!([p1, p2]).as_array().unwrap(), "trusted").unwrap();
    assert_eq!(view["name"], "proj");
    assert_eq!(view["purpose"], "project");
    assert_eq!(view["trust"], "trusted");
    assert_eq!(view["folders"].as_array().unwrap().len(), 2, "返回单对象并携带文件夹列表");
    assert_eq!(view["path"], canon(&p1), "首项为主要目录");

    // 重名自动加序号（p3 与 p1 叶子名同为 proj）
    let view2 = services::workspace_add_project(&state, "", json!([p3]).as_array().unwrap(), "restricted").unwrap();
    assert_eq!(view2["name"], "proj (2)");

    // 请求内目录去重
    let dup = services::workspace_add_project(&state, "", json!([p1, p1]).as_array().unwrap(), "trusted");
    assert!(dup.is_err() && dup.unwrap_err().contains("重复"));

    // 已是其他工作区源文件夹
    let conflict = services::workspace_add_project(&state, "", json!([p1]).as_array().unwrap(), "trusted");
    assert!(conflict.is_err() && conflict.unwrap_err().contains("已被工作区"));
}

#[test]
fn workspace_replace_folders_appends_and_removes_additional_roots() {
    let (dir, state) = test_state("ws-edit-folders");
    let root = dir.path().join("a");
    let main = root.join("proj");
    let extra = root.join("extra");
    let later = root.join("later");
    for p in [&main, &extra, &later] {
        std::fs::create_dir_all(p).unwrap();
    }
    let main_s = main.to_string_lossy().to_string();
    let extra_s = extra.to_string_lossy().to_string();
    let later_s = later.to_string_lossy().to_string();
    let ws = services::workspace_add_project(&state, "演示", json!([main_s]).as_array().unwrap(), "trusted").unwrap();
    let ws_id = ws["id"].as_str().unwrap().to_string();

    // 追加附加目录：主目录不变，顺序按提交顺序
    let view = services::workspace_replace_folders(&state, &ws_id, json!([main_s, extra_s]).as_array().unwrap()).unwrap();
    let paths: Vec<&str> = view["folders"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec![canon(&main_s), canon(&extra_s)]);
    assert_eq!(view["path"], canon(&main_s), "主要目录未变");

    // 再追加一个、并移除中间那个：整体替换语义
    let view = services::workspace_replace_folders(&state, &ws_id, json!([main_s, later_s]).as_array().unwrap()).unwrap();
    let paths: Vec<&str> = view["folders"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec![canon(&main_s), canon(&later_s)], "附加目录被整体替换");
}

#[test]
fn workspace_replace_folders_keeps_own_roots_accepted() {
    // 编辑场景必须排除自身占用：否则「原地保存、一个都不改」会被「已被工作区使用」误伤
    let (dir, state) = test_state("ws-edit-self");
    let root = dir.path().join("a");
    let main = root.join("proj");
    let extra = root.join("extra");
    for p in [&main, &extra] {
        std::fs::create_dir_all(p).unwrap();
    }
    let main_s = main.to_string_lossy().to_string();
    let extra_s = extra.to_string_lossy().to_string();
    let ws = services::workspace_add_project(&state, "演示", json!([main_s, extra_s]).as_array().unwrap(), "trusted").unwrap();
    let ws_id = ws["id"].as_str().unwrap().to_string();

    let view = services::workspace_replace_folders(&state, &ws_id, json!([main_s, extra_s]).as_array().unwrap())
        .expect("原地保存自身目录必须通过");
    assert_eq!(view["folders"].as_array().unwrap().len(), 2);
}

#[test]
fn workspace_replace_folders_rejects_other_workspace_directory() {
    let (dir, state) = test_state("ws-edit-conflict");
    let root = dir.path().join("a");
    let first = root.join("one");
    let second = root.join("two");
    let stolen = root.join("stolen");
    for p in [&first, &second, &stolen] {
        std::fs::create_dir_all(p).unwrap();
    }
    let first_s = first.to_string_lossy().to_string();
    let second_s = second.to_string_lossy().to_string();
    let stolen_s = stolen.to_string_lossy().to_string();
    let a = services::workspace_add_project(&state, "A", json!([first_s]).as_array().unwrap(), "trusted").unwrap();
    services::workspace_add_project(&state, "B", json!([stolen_s]).as_array().unwrap(), "trusted").unwrap();
    let a_id = a["id"].as_str().unwrap().to_string();

    let err = services::workspace_replace_folders(&state, &a_id, json!([second_s, stolen_s]).as_array().unwrap()).unwrap_err();
    assert!(err.contains("已被工作区"), "实际：{err}");
}

#[test]
fn workspace_replace_folders_rejects_main_change_with_bound_worktree() {
    let (dir, state) = test_state("ws-edit-wt-guard");
    let repo = git_repo(dir.path());
    let plain = dir.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let repo_s = repo.to_string_lossy().to_string();
    let plain_s = plain.to_string_lossy().to_string();
    let ws = services::workspace_add_project(&state, "wtproj", json!([repo_s]).as_array().unwrap(), "trusted").unwrap();
    let ws_id = ws["id"].as_str().unwrap().to_string();
    let thread = services::thread_create(&state, "code", &ws_id, "任务").unwrap();
    services::ensure_thread_worktree(&state, &thread_id_of(&thread)).unwrap();

    // 换主要目录：被绑定工作树的会话挡下
    let err = services::workspace_replace_folders(&state, &ws_id, json!([plain_s, repo_s]).as_array().unwrap()).unwrap_err();
    assert!(err.contains("git 工作树"), "实际：{err}");

    // 只动附加目录不受守卫影响
    let view = services::workspace_replace_folders(&state, &ws_id, json!([repo_s, plain_s]).as_array().unwrap())
        .expect("只追加附加目录应通过");
    assert_eq!(view["folders"].as_array().unwrap().len(), 2);
}

#[test]
fn workspace_replace_folders_rejects_conversation_workspace() {
    let (_dir, state) = test_state("ws-edit-conv");
    // ensure 返回的是工作区 id 字符串（不是对象）
    let conv_id = services::ensure_conversation_workspace(&state).unwrap();
    let conv_id = conv_id.as_str().unwrap().to_string();
    let err = services::workspace_replace_folders(&state, &conv_id, json!(["/tmp"]).as_array().unwrap()).unwrap_err();
    assert!(err.contains("临时会话目录"), "实际：{err}");
}

#[test]
fn workspace_add_rejects_builtin_conversation_directory() {
    let (_dir, state) = test_state("ws-add-project-guard");
    let _ = services::ensure_conversation_workspace(&state).unwrap();
    let list = services::workspace_list(&state).unwrap();
    let conv_path = list
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["purpose"] == "conversation")
        .unwrap()["path"]
        .as_str()
        .unwrap()
        .to_string();
    let err = services::workspace_add_project(&state, "", json!([conv_path]).as_array().unwrap(), "trusted").unwrap_err();
    assert!(err.contains("临时会话目录"), "实际：{err}");
}

#[test]
fn skills_list_all_reports_workspace_owner() {
    let (dir, state) = test_state("skills-list-all");
    // 用户级技能：<数据目录>/skills/demo/SKILL.md
    let user_dir = dir.path().join("skills").join("demo");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(
        user_dir.join("SKILL.md"),
        "---\nname: user-demo\ndescription: 用户级技能\n---\n正文\n",
    )
    .unwrap();

    // 项目工作区：<工作区>/.craftwork/skills/proj-demo/SKILL.md
    let ws_root = dir.path().join("proj");
    let proj_dir = ws_root.join(".craftwork").join("skills").join("proj-demo");
    std::fs::create_dir_all(&proj_dir).unwrap();
    std::fs::write(
        proj_dir.join("SKILL.md"),
        "---\nname: proj-demo\ndescription: 工作区级技能\n---\n正文\n",
    )
    .unwrap();
    let ws_path = ws_root.to_string_lossy().to_string();
    let created = services::workspace_add_project(&state, "演示项目", json!([ws_path]).as_array().unwrap(), "trusted").unwrap();
    let ws_id = created["id"].as_str().unwrap().to_string();

    let views = services::skills_list_all(&state).unwrap();
    let views = views.as_array().unwrap();
    let find = |name: &str| views.iter().find(|v| v["name"] == name).unwrap_or_else(|| panic!("缺少技能 {name}"));

    let user = find("user-demo");
    assert_eq!(user["workspaceId"], "", "用户级技能无归属工作区");
    assert_eq!(user["workspaceName"], "");
    assert_eq!(user["scope"], "user");

    let proj = find("proj-demo");
    assert_eq!(proj["workspaceId"], ws_id.as_str(), "工作区级技能带归属 id");
    assert_eq!(proj["workspaceName"], "演示项目");
    assert_eq!(proj["scope"], "workspace");

    // 管理操作沿用条目自带的归属 id 即可通过根校验（启停往返）
    let proj_path = proj["path"].as_str().unwrap().to_string();
    let next = services::skills_set_enabled(&state, &proj_path, false, &ws_id).unwrap();
    let disabled = next.as_array().unwrap().iter().find(|v| v["path"] == proj_path.as_str()).unwrap();
    assert_eq!(disabled["enabled"], false);
}

/// 插件缓存技能接入管理链路（D12）：管理页可见（限定名
/// 注入、source=marketplace）、单技能禁用沿用 skills.disabled（缓存绝对路径天然兼容）、
/// 删除拒绝（缓存是只读产物，错误信息说明插件并引导走商店卸载）。
#[test]
fn skills_list_all_includes_plugin_skills_and_rejects_plugin_delete() {
    let (dir, state) = test_state("skills-plugin");
    // 插件缓存：版本目录布局（install_path 指向当前版本目录，其下 skills/ 纳入扫描）
    let plugin_dir = dir
        .path()
        .join("skills-marketplace")
        .join("cache")
        .join("mk")
        .join("foo")
        .join("1.0.0");
    let skill_dir = plugin_dir.join("skills").join("demo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "---\nname: demo\ndescription: 插件技能\n---\n正文\n").unwrap();
    // 造市场 + 插件行：craftwork 无 rusqlite 显式依赖（本票不动 Cargo.lock），拼接 SQL
    // 与迁移 V14 的 skill_plugins 列对应；外键已启用，先造 marketplaces 行
    {
        let conn = state.db.lock_conn();
        conn.execute(
            "INSERT INTO marketplaces (id, source, repo_url, created_at) VALUES ('mk', 'mk', 'https://example.com/mk', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            &format!(
                "INSERT INTO skill_plugins (id, marketplace_id, name, current_version, versions, install_path, source_kind, source_ref, enabled, installed_at, updated_at) \
                 VALUES ('foo@mk', 'mk', 'foo', '1.0.0', '[\"1.0.0\"]', '{}', 'entry-relative', './x', 1, 1, 1)",
                plugin_dir.to_string_lossy().replace('\'', "''")
            ),
            [],
        )
        .unwrap();
    }

    // 管理页可见：限定名注入 + source=marketplace + 用户级归属
    let views = services::skills_list_all(&state).unwrap();
    let views = views.as_array().unwrap();
    let plugin_skill = views.iter().find(|v| v["name"] == "foo:demo").expect("插件技能在管理页可见");
    assert_eq!(plugin_skill["source"], "marketplace");
    assert_eq!(plugin_skill["workspaceId"], "", "插件缓存装在用户级，无工作区归属");

    // 单技能禁用沿用 skills.disabled 路径数组（缓存绝对路径天然兼容）
    let path = plugin_skill["path"].as_str().unwrap().to_string();
    let next = services::skills_set_enabled(&state, &path, false, "").unwrap();
    let disabled = next.as_array().unwrap().iter().find(|v| v["path"] == path.as_str()).unwrap();
    assert_eq!(disabled["enabled"], false);

    // 删除拒绝：缓存只读，错误信息说明插件并引导走商店卸载或禁用
    let err = services::skills_remove(&state, &path, "").unwrap_err();
    assert!(err.contains("foo") && (err.contains("只读") || err.contains("卸载")), "实际：{err}");
    // 拒绝后技能目录未被破坏（缓存完整性保持）
    assert!(skill_dir.join("SKILL.md").is_file());
}

#[test]
fn settings_save_level_takes_effect_without_restart() {
    let (_dir, state) = test_state("level-hot");
    // migration-parity 票 04：引擎只在启动时按设置初始化，保存后必须同步热切换
    services::settings_save(&state, &json!({ "approvalLevel": "free" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "free");
    services::settings_save(&state, &json!({ "approvalLevel": "graded" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "graded");
    // 已移除的历史档位（strict/readonly）与非法值一律回落默认档（graded）
    services::settings_save(&state, &json!({ "approvalLevel": "readonly" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "graded");
    services::settings_save(&state, &json!({ "approvalLevel": "strict" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "graded");
    services::settings_save(&state, &json!({ "approvalLevel": "bogus" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "graded");
}

#[test]
fn model_reasoning_declaration_persists_and_survives_unrelated_edit() {
    let (_dir, state) = test_state("reasoning-decl");
    let provider = json!({
        "id": "gw", "name": "GW", "kind": "openai-compatible",
        "endpoint": "https://gw.invalid/v1", "keyRequired": false,
        "models": [{ "id": "m1", "name": "网关模型", "contextK": 8, "maxTokens": 512,
                     "caps": {"stream": true, "tools": true, "vision": false, "reasoning": true, "structured": false},
                     "status": "", "source": "manual", "enabled": true }],
    });
    services::provider_save(&state, &provider).unwrap();

    // 手动声明档位：写入 reasoning_kind/levels 并出现在视图中
    // （caps 用 4 字段子集——模型弹窗无 stream 勾选，服务端经 PartialCaps 归一补全）
    let view = services::provider_update_model(&state, "gw", "m1", &json!({
        "id": "m1", "name": "网关模型", "contextK": 8, "maxTokens": 512,
        "caps": {"tools": true, "vision": false, "reasoning": true, "structured": false},
        "reasoningKind": "effort", "reasoningLevels": ["low", "high", "max"],
    })).unwrap();
    assert!(view.to_string().contains(r#""reasoningLevels":["low","high","max"]"#));
    assert!(view.to_string().contains(r#""stream":true"#), "缺省 stream 应归一为 true");

    // 不携带档位字段的编辑（改显示名）：保留已声明的档位，不被抹掉
    let view = services::provider_update_model(&state, "gw", "m1", &json!({
        "id": "m1", "name": "改名", "contextK": 8, "maxTokens": 512,
        "caps": {"stream": true, "tools": true, "vision": false, "reasoning": true, "structured": false},
    })).unwrap();
    assert!(view.to_string().contains(r#""reasoningLevels":["low","high","max"]"#));

    // 显式清除：kind 空串 + 空数组
    let view = services::provider_update_model(&state, "gw", "m1", &json!({
        "id": "m1", "name": "改名", "contextK": 8, "maxTokens": 512,
        "caps": {"stream": true, "tools": true, "vision": false, "reasoning": true, "structured": false},
        "reasoningKind": "", "reasoningLevels": [],
    })).unwrap();
    assert!(view.to_string().contains(r#""reasoningLevels":[]"#));
}

#[test]
fn model_update_renames_manual_model_id_and_rewires_default_model() {
    let (_dir, state) = test_state("model-rename-id");
    let caps = json!({"stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false});
    let provider = json!({
        "id": "gw", "name": "GW", "kind": "openai-compatible",
        "endpoint": "https://gw.invalid/v1", "keyRequired": false,
        "models": [
            { "id": "m1", "name": "模型一", "contextK": 8, "maxTokens": 512,
              "caps": caps, "status": "", "source": "manual", "enabled": true },
            { "id": "m2", "name": "模型二", "contextK": 8, "maxTokens": 512,
              "caps": caps, "status": "", "source": "manual", "enabled": true },
        ],
    });
    services::provider_save(&state, &provider).unwrap();
    services::settings_save(&state, &json!({ "defaultModel": "gw/m1", "fallbackModel": "gw/m2" })).unwrap();

    // 手动模型改 ID：成功；default 引用同步改写，fallback（指向别的模型）不受影响
    let view = services::provider_update_model(&state, "gw", "m1", &json!({
        "id": "m1-renamed", "name": "模型一", "contextK": 8, "maxTokens": 512, "caps": caps,
    })).unwrap();
    assert!(view.to_string().contains(r#""id":"m1-renamed""#));
    assert!(!view.to_string().contains(r#""id":"m1""#), "旧 ID 应消失");
    let settings = services::settings_get(&state).unwrap();
    assert_eq!(settings["defaultModel"], "gw/m1-renamed");
    assert_eq!(settings["fallbackModel"], "gw/m2");

    // 同名不改 ID 的编辑照常工作
    let view = services::provider_update_model(&state, "gw", "m1-renamed", &json!({
        "id": "m1-renamed", "name": "改名", "contextK": 8, "maxTokens": 512, "caps": caps,
    })).unwrap();
    assert!(view.to_string().contains(r#""name":"改名""#));

    // 改成同提供商下已存在的 ID：拒绝
    let dup = services::provider_update_model(&state, "gw", "m1-renamed", &json!({
        "id": "m2", "name": "模型一", "contextK": 8, "maxTokens": 512, "caps": caps,
    }));
    assert!(dup.is_err() && dup.unwrap_err().contains("已存在"), "应报 ID 冲突");

    // 目录来源模型改 ID：拒绝（目录事实源，改 ID 等于换了模型）
    let catalog_provider = json!({
        "id": "cat", "name": "Cat", "kind": "openai",
        "endpoint": "https://cat.invalid/v1", "keyRequired": false,
        "models": [{ "id": "gpt-x", "name": "GPT X", "contextK": 8, "maxTokens": 512,
                     "caps": caps, "status": "", "source": "catalog", "enabled": true }],
    });
    services::provider_save(&state, &catalog_provider).unwrap();
    let locked = services::provider_update_model(&state, "cat", "gpt-x", &json!({
        "id": "gpt-y", "name": "GPT X", "contextK": 8, "maxTokens": 512, "caps": caps,
    }));
    assert!(locked.is_err() && locked.unwrap_err().contains("不可修改 ID"), "目录来源应拒绝改 ID");
}

// ---------- worktree ----------

/// 「主仓库已登记该工作树」的判定：porcelain 输出的 worktree 路径与目标路径两边都
/// canonicalize（解析符号链接与 Windows 8.3 短名）后比较。Windows runner 的 TEMP 是
/// 短名形式（如 RUNNER~1），而 git 输出解析后的长名，仅归一化分隔符匹配不上。
fn porcelain_contains_path(stdout: &str, target: &std::path::Path) -> bool {
    stdout.lines().filter_map(|line| line.strip_prefix("worktree ")).any(|entry| {
        std::fs::canonicalize(entry).is_ok_and(|resolved| resolved == target)
    })
}

/// 在测试目录里造一个真实 git 仓库（git init + 首次提交，worktree add 需要 HEAD 存在）。
fn git_repo(dir: &std::path::Path) -> std::path::PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        assert!(out.status.success(), "git {args:?} 失败：{}", String::from_utf8_lossy(&out.stderr));
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@craftwork.local"]);
    run(&["config", "user.name", "test"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-qm", "init"]);
    repo
}

/// 注册「已解析模型」的存根提供商（照抄 thread_creation 测试的做法，让首发校验通过）。
fn stub_provider(state: &services::AppState) {
    let provider = json!({
        "id": "stub", "name": "Stub", "kind": "openai-compatible",
        "endpoint": "https://stub.invalid/v1", "keyRequired": false,
        "models": [{ "id": "m1", "name": "模型一", "contextK": 8, "maxTokens": 512,
                     "caps": {"stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false},
                     "status": "", "source": "manual", "enabled": true }],
    });
    services::provider_save(state, &provider).unwrap();
}

fn thread_id_of(view: &Value) -> String {
    view["id"].as_str().unwrap().to_string()
}

#[test]
fn worktree_ensure_rejects_work_mode_conversation_and_non_git_project() {
    let (dir, state) = test_state("wt-reject");
    // 内置临时目录（purpose=conversation）：不是 git 仓库
    let conv = services::ensure_conversation_workspace(&state).unwrap().as_str().unwrap().to_string();
    let t_conv = services::thread_create(&state, "code", &conv, "临时任务").unwrap();
    let err = services::ensure_thread_worktree(&state, &thread_id_of(&t_conv)).unwrap_err();
    assert!(err.contains("不是 git 仓库"), "实际：{err}");

    // 普通目录（非 git）项目工作区
    let plain = dir.path().join("plain-proj");
    std::fs::create_dir_all(&plain).unwrap();
    let ws = services::workspace_add_project(&state, "plain", json!([plain.to_string_lossy().to_string()]).as_array().unwrap(), "trusted").unwrap();
    let t_plain = services::thread_create(&state, "code", ws["id"].as_str().unwrap(), "任务").unwrap();
    let err = services::ensure_thread_worktree(&state, &thread_id_of(&t_plain)).unwrap_err();
    assert!(err.contains("不是 git 仓库"), "实际：{err}");

    // 真实 git 仓库 + work 模式会话：没有 Git 能力
    let repo = git_repo(dir.path());
    let ws = services::workspace_add_project(&state, "gitproj", json!([repo.to_string_lossy().to_string()]).as_array().unwrap(), "trusted").unwrap();
    let t_work = services::thread_create(&state, "work", ws["id"].as_str().unwrap(), "文档任务").unwrap();
    let err = services::ensure_thread_worktree(&state, &thread_id_of(&t_work)).unwrap_err();
    assert!(err.contains("Work 模式"), "实际：{err}");
}

#[test]
fn worktree_ensure_creates_binds_and_is_idempotent() {
    let (dir, state) = test_state("wt-create");
    let repo = git_repo(dir.path());
    let ws = services::workspace_add_project(&state, "wtproj", json!([repo.to_string_lossy().to_string()]).as_array().unwrap(), "trusted").unwrap();
    let thread = services::thread_create(&state, "code", ws["id"].as_str().unwrap(), "任务").unwrap();
    let id = thread_id_of(&thread);

    let (path, branch) = services::ensure_thread_worktree(&state, &id).unwrap();
    assert!(std::path::Path::new(&path).is_dir(), "工作树目录已创建");
    assert!(branch.starts_with("craftwork/"), "分支名 craftwork/<短码>：{branch}");
    // 数据目录统一管理：<data_dir>/worktrees/<会话id>
    assert_eq!(path, dir.path().join("worktrees").join(&id).to_string_lossy().to_string());

    // 幂等：二次调用返回同一绑定，不重复创建
    let (path2, branch2) = services::ensure_thread_worktree(&state, &id).unwrap();
    assert_eq!((path.clone(), branch.clone()), (path2, branch2));

    // 绑定落库（ThreadView 透出 worktreePath/worktreeBranch）
    let view = services::thread_snapshot(&state, &id, 0, 0).unwrap();
    assert_eq!(view["thread"]["worktreePath"], path.as_str());
    assert_eq!(view["thread"]["worktreeBranch"], branch.as_str());

    // 主仓库的 worktree 登记 + 分支存在
    let out = std::process::Command::new("git").args(["worktree", "list", "--porcelain"]).current_dir(&repo).output().unwrap();
    let target = std::fs::canonicalize(&path).expect("工作树目录应存在且可解析");
    assert!(
        porcelain_contains_path(&String::from_utf8_lossy(&out.stdout), &target),
        "主仓库应登记该工作树；porcelain 输出：\n{}\ntarget：{}",
        String::from_utf8_lossy(&out.stdout),
        target.display()
    );
    let out = std::process::Command::new("git").args(["branch", "--list", &branch]).current_dir(&repo).output().unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).trim().is_empty(), "分支应存在");
}

#[test]
fn worktree_ensure_rebuilds_after_directory_loss_reusing_branch() {
    let (dir, state) = test_state("wt-rebuild");
    let repo = git_repo(dir.path());
    let ws = services::workspace_add_project(&state, "wtproj", json!([repo.to_string_lossy().to_string()]).as_array().unwrap(), "trusted").unwrap();
    let thread = services::thread_create(&state, "code", ws["id"].as_str().unwrap(), "任务").unwrap();
    let id = thread_id_of(&thread);

    let (path, branch) = services::ensure_thread_worktree(&state, &id).unwrap();
    // 在工作树里留一个提交（分支历史不为空），目录丢失后重建应复用分支
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git").args(args).current_dir(&path).output().unwrap();
        assert!(out.status.success(), "git {args:?} 失败：{}", String::from_utf8_lossy(&out.stderr));
    };
    run(&["config", "user.email", "test@craftwork.local"]);
    run(&["config", "user.name", "test"]);
    std::fs::write(std::path::Path::new(&path).join("w.txt"), "worktree\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-qm", "wt-commit"]);

    std::fs::remove_dir_all(&path).unwrap();
    let (rebuilt, rebuilt_branch) = services::ensure_thread_worktree(&state, &id).unwrap();
    assert_eq!(rebuilt, path, "重建路径稳定");
    assert_eq!(rebuilt_branch, branch, "重建复用原分支");
    // 分支历史提交不丢
    let out = std::process::Command::new("git").args(["log", "--oneline", "-2"]).current_dir(&rebuilt).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("wt-commit"), "分支提交保留");
}

#[test]
fn worktree_send_failure_rolls_back_created_thread() {
    let (_dir, state) = test_state("wt-atomic");
    stub_provider(&state);
    // 启动装配先建内置临时目录（应用启动时必经；否则空串回落不到工作区）
    services::ensure_conversation_workspace(&state).unwrap();
    // 临时槽（空串回落 conversation 工作区）：勾选 worktree 首发必失败
    let err = services::chat_create_and_start(&state, "code", "", "任务", &[], "stub/m1", "", false, true).unwrap_err();
    assert!(err.contains("不是 git 仓库"), "实际：{err}");
    let threads = services::thread_list(&state, "code", false).unwrap();
    assert_eq!(threads.as_array().unwrap().len(), 0, "worktree 失败回滚刚建的空会话（失败零落库）");
}

#[test]
fn thread_delete_cleans_up_bound_worktree_and_branch() {
    let (dir, state) = test_state("wt-delete");
    let repo = git_repo(dir.path());
    let ws = services::workspace_add_project(&state, "wtproj", json!([repo.to_string_lossy().to_string()]).as_array().unwrap(), "trusted").unwrap();
    let thread = services::thread_create(&state, "code", ws["id"].as_str().unwrap(), "任务").unwrap();
    let id = thread_id_of(&thread);
    let (path, branch) = services::ensure_thread_worktree(&state, &id).unwrap();
    // 删除前固定工作树真实路径：删除后目录不存在，无法再 canonicalize 与 git 输出对齐
    let target = std::fs::canonicalize(&path).expect("工作树目录应存在且可解析");

    services::thread_delete(&state, &id).unwrap();
    assert!(!std::path::Path::new(&path).exists(), "工作树目录已清理");
    let out = std::process::Command::new("git").args(["branch", "--list", &branch]).current_dir(&repo).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty(), "分支已删除");
    let out = std::process::Command::new("git").args(["worktree", "list", "--porcelain"]).current_dir(&repo).output().unwrap();
    assert!(!porcelain_contains_path(&String::from_utf8_lossy(&out.stdout), &target), "主仓库 worktree 登记已移除");
}

/// 删除会话必须把它按 id 命名的文件一并清掉（日志 / 后台输出 / 计划正文），
/// 否则存储页会立刻把刚删掉的会话报成「孤儿」。
#[test]
fn thread_delete_removes_session_files() {
    let (_dir, state) = test_state("thread-files");
    let ws_id = services::ensure_conversation_workspace(&state)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let id = thread_id_of(&services::thread_create(&state, "code", &ws_id, "待删").unwrap());
    // 另一个会话的文件必须活着（防止误删同前缀的兄弟）
    let sibling = thread_id_of(&services::thread_create(&state, "code", &ws_id, "保留").unwrap());

    let data = std::path::Path::new(&state.data_dir);
    let logs = data.join("logs");
    let shell_tasks = data.join("shell-tasks");
    let plans = data.join("plans");
    for dir in [&logs, &shell_tasks, &plans] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(logs.join(format!("{id}.log")), b"log").unwrap();
    std::fs::write(logs.join(format!("{sibling}.log")), b"keep").unwrap();
    std::fs::create_dir_all(shell_tasks.join(&id)).unwrap();
    std::fs::create_dir_all(shell_tasks.join(&sibling)).unwrap();
    let plan_path = plans.join("plan_x.md");
    std::fs::write(&plan_path, b"plan").unwrap();
    state
        .db
        .with(|conn| {
            let plan = craft_store::plans::create_plan(conn, &id, "run-x")?;
            craft_store::plans::set_plan_file(conn, &plan.id, &plan_path.to_string_lossy())
        })
        .unwrap();

    services::thread_delete(&state, &id).unwrap();

    assert!(!logs.join(format!("{id}.log")).exists(), "该会话日志应删除");
    assert!(!shell_tasks.join(&id).exists(), "该会话后台输出应删除");
    assert!(!plan_path.exists(), "该会话计划正文应删除");
    assert!(logs.join(format!("{sibling}.log")).exists(), "其他会话日志不得误删");
    assert!(shell_tasks.join(&sibling).exists(), "其他会话后台输出不得误删");
}

/// 一键清理临时会话：只删当前模式、未归档、非运行中的线程；running/awaiting 跳过；审计落一条。
#[test]
fn thread_clear_temp_deletes_idle_and_skips_running_awaiting_and_archived() {
    let (_dir, state) = test_state("clear-temp");
    // 启动装配先建内置临时目录；conversation 工作区 id 由返回值取得（ensure 幂等）
    let ws_id = services::ensure_conversation_workspace(&state)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let mk = |title: &str, mode: &str| -> String {
        thread_id_of(&services::thread_create(&state, mode, &ws_id, title).unwrap())
    };
    let idle_code = mk("普通会话", "code");
    let running_code = mk("运行中", "code");
    let awaiting_code = mk("等审批", "code");
    let archived_code = mk("已归档", "code");
    let work_thread = mk("跨模式", "work");
    state.recorder.set_thread_status(&running_code, Some("run-x"), "running").unwrap();
    state.recorder.set_thread_status(&awaiting_code, Some("run-y"), "awaiting").unwrap();
    services::thread_set_archived(&state, &archived_code, true).unwrap();

    let result = services::thread_clear_temp(&state, "code").unwrap();
    assert_eq!(result["removed"], json!(1), "只删 idle 的 code 未归档会话");
    assert_eq!(result["skipped"], json!(2), "running + awaiting 跳过");

    let alive = |id: &str| craft_store::threads::get_thread(&state.db.lock_conn(), id).is_ok();
    assert!(!alive(&idle_code), "普通会话已删");
    assert!(alive(&running_code), "运行中跳过");
    assert!(alive(&awaiting_code), "等待审批跳过");
    assert!(alive(&archived_code), "归档不动（与侧栏可见列表一致）");
    assert!(alive(&work_thread), "跨模式不动");

    let audits = craft_store::records::list_audit(&state.db.lock_conn(), 10).unwrap();
    assert!(
        audits.iter().any(|a| a.tool == "thread" && a.decision == "deleted" && a.detail.contains("清理临时会话")),
        "清理落审计：{:?}",
        audits.iter().map(|a| (a.tool.as_str(), a.decision.as_str())).collect::<Vec<_>>()
    );
}

// ---------- 个性化（<数据目录>/AGENTS.md 读写） ----------

#[test]
fn personalization_roundtrip_and_first_read_missing_file() {
    let (dir, state) = test_state("personal-roundtrip");
    // 首次读取：文件不存在，exists=false 空内容，不报错
    let first = services::personalization_get(&state).unwrap();
    assert_eq!(first["exists"], json!(false));
    assert_eq!(first["content"], json!(""));
    assert_eq!(first["size"], json!(0));

    // 保存 → 覆盖读取往返；文件落在 <数据目录>/AGENTS.md（envcontext 注入同一文件）
    let saved = services::personalization_save(&state, "## 语言约束\n1. 使用中文").unwrap();
    assert_eq!(saved["exists"], json!(true));
    assert_eq!(saved["content"], json!("## 语言约束\n1. 使用中文"));
    assert_eq!(saved["size"], json!(saved["content"].as_str().unwrap().len()));
    let on_disk = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
    assert_eq!(on_disk, "## 语言约束\n1. 使用中文");

    // 再次保存覆盖（不是追加）
    let saved = services::personalization_save(&state, "second").unwrap();
    assert_eq!(saved["content"], json!("second"));
    let on_disk = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
    assert_eq!(on_disk, "second");
}

#[test]
fn personalization_over_limit_rejected_and_empty_clears_file() {
    let (dir, state) = test_state("personal-limit");
    let agents_md = || dir.path().join("AGENTS.md");
    // 超过 100KB 拒绝且不落盘
    let big = "x".repeat(100 * 1024 + 1);
    let err = services::personalization_save(&state, &big).unwrap_err();
    assert!(err.contains("上限"), "超限错误应可读：{err}");
    assert!(!agents_md().exists(), "超限拒绝不应落盘");
    // 恰好 100KB 允许（边界）
    let exact = "y".repeat(100 * 1024);
    services::personalization_save(&state, &exact).unwrap();
    // 清空保存 = 删除文件；再读取 exists=false
    let cleared = services::personalization_save(&state, "").unwrap();
    assert_eq!(cleared["exists"], json!(false));
    assert!(!agents_md().exists(), "清空后文件应删除");
    // 纯空白等价清空
    services::personalization_save(&state, "内容").unwrap();
    let cleared = services::personalization_save(&state, "   \n  ").unwrap();
    assert_eq!(cleared["exists"], json!(false));
    assert!(!agents_md().exists(), "空白保存同样应删除文件");
}

// ---------- 技能商店（决策 D12；票 #7） ----------

const STORE_MARKET: &str = "demo-market";
const STORE_PLUGIN: &str = "demo";

/// 直接放置市场快照（不 clone：服务层校验只放行 https:// 与 owner/repo，测试经磁盘
/// 构造「已添加 + 已刷新」状态）。形态 2 Claude 兼容清单 + 条目相对路径来源 + 一个技能。
fn place_store_snapshot(data_dir: &std::path::Path) -> std::path::PathBuf {
    let snapshot = data_dir.join("skills-marketplace").join("marketplaces").join(STORE_MARKET);
    let market_json = format!(
        r#"{{"name":"{STORE_MARKET}","plugins":[{{"name":"{STORE_PLUGIN}","description":"演示插件","version":"1.0.0","source":"./"}}]}}"#
    );
    let skill = snapshot.join("skills").join("demo-skill").join("SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::create_dir_all(snapshot.join(".claude-plugin")).unwrap();
    std::fs::write(snapshot.join(".claude-plugin").join("marketplace.json"), market_json).unwrap();
    std::fs::write(skill, "---\nname: demo-skill\ndescription: 演示技能\n---\n正文").unwrap();
    snapshot
}

/// 直插已装插件行 + 缓存目录（不经 orchestrator：安装编排已在 craft-marketplace
/// 集成测试覆盖，本测试关注服务层列表组装与卸载刷新）。
fn install_store_plugin(data_dir: &std::path::Path, state: &services::AppState) {
    state
        .db
        .with(|conn| {
            craft_marketplace::store::add_marketplace(
                conn,
                craft_marketplace::store::NewMarketplace {
                    id: STORE_MARKET,
                    source: STORE_MARKET,
                    repo_url: "https://github.com/demo/market",
                },
            )
        })
        .unwrap();
    let version_dir = data_dir
        .join("skills-marketplace")
        .join("cache")
        .join(STORE_MARKET)
        .join(STORE_PLUGIN)
        .join("1.0.0");
    let skill = version_dir.join("skills").join("demo-skill").join("SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(skill, "---\nname: demo-skill\ndescription: 演示技能\n---\n正文").unwrap();
    state
        .db
        .with(|conn| {
            craft_marketplace::store::insert_plugin(
                conn,
                craft_marketplace::store::NewPlugin {
                    id: "demo@demo-market",
                    marketplace_id: STORE_MARKET,
                    name: STORE_PLUGIN,
                    current_version: "1.0.0",
                    versions: &["1.0.0".to_string()],
                    install_path: version_dir.to_string_lossy().as_ref(),
                    source_kind: craft_marketplace::store::source_kind::ENTRY_RELATIVE,
                    source_ref: "./",
                    source_sha: Some("abc1234"),
                },
            )
        })
        .unwrap();
}

#[test]
fn store_add_market_rejects_invalid_inputs_with_readable_errors() {
    let (_dir, state) = test_state("store-invalid");
    // 空输入
    let err = services::store_add_market(&state, "   ").unwrap_err();
    assert!(err.contains("不能为空"), "实际：{err}");
    // 本地目录路径：留桩报「暂不支持」
    let err = services::store_add_market(&state, "/tmp/local-market").unwrap_err();
    assert!(err.contains("暂不支持"), "实际：{err}");
    // file:// 与 http:// 同为「暂不支持」
    assert!(services::store_add_market(&state, "file:///tmp/x").unwrap_err().contains("暂不支持"));
    assert!(services::store_add_market(&state, "http://example.com/r").unwrap_err().contains("暂不支持"));
    // 既非 https:// 也非 owner/repo 的形态：格式不支持
    let err = services::store_add_market(&state, "just-a-name").unwrap_err();
    assert!(err.contains("格式不支持"), "实际：{err}");
    // 列表仍为空（未产生任何市场）
    let result = services::store_list_markets(&state).unwrap();
    assert_eq!(result["markets"].as_array().unwrap().len(), 0);
    assert!(result["recommended"].as_array().unwrap().len() >= 5, "推荐源候选常驻");
}

#[test]
fn store_channels_reject_bad_plugin_ids_and_missing_market() {
    let (_dir, state) = test_state("store-bad-id");
    // 插件 id 必须是 名称@市场id（rsplit 最后一个 @）
    for bad in ["no-at-sign", "@market-only", "name-only@"] {
        let err = services::store_install_plugin(&state, bad).unwrap_err();
        assert!(err.contains("不合法"), "id {bad:?} 应被拒：{err}");
    }
    // 市场 id 为空
    assert!(services::store_remove_market(&state, "").unwrap_err().contains("不能为空"));
    // 不存在的市场移除：可读错误
    let err = services::store_remove_market(&state, "ghost").unwrap_err();
    assert!(err.contains("不存在"), "实际：{err}");
}

#[test]
fn store_describe_plugin_falls_back_to_snapshot_for_uninstalled_plugin() {
    let (dir, state) = test_state("store-describe");
    let plugin_id = format!("{STORE_PLUGIN}@{STORE_MARKET}");
    // 快照不存在（从未刷新）：可读错误提示先刷新
    let err = services::store_describe_plugin(&state, &plugin_id).unwrap_err();
    assert!(err.contains("请先刷新"), "实际：{err}");

    // 放置快照后：未安装插件的离线描述（条目元数据 + 来源目录技能扫描）
    let snapshot = place_store_snapshot(dir.path());
    let detail = services::store_describe_plugin(&state, &plugin_id).unwrap();
    assert_eq!(detail["installed"], json!(false));
    assert_eq!(detail["marketplaceId"], json!(STORE_MARKET));
    assert_eq!(detail["description"], json!("演示插件"), "描述取自市场条目");
    assert_eq!(detail["hasHooks"], json!(false));
    let skills = detail["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 1, "快照来源目录扫描出技能：{detail}");
    assert_eq!(skills[0]["name"], json!("demo-skill"));
    assert!(skills[0]["path"].as_str().unwrap().contains("demo-skill"));
    let _ = snapshot;
}

#[test]
fn store_describe_plugin_reads_installed_cache_with_versions() {
    let (dir, state) = test_state("store-describe-installed");
    place_store_snapshot(dir.path());
    install_store_plugin(dir.path(), &state);
    let detail = services::store_describe_plugin(&state, &format!("{STORE_PLUGIN}@{STORE_MARKET}")).unwrap();
    assert_eq!(detail["installed"], json!(true));
    assert_eq!(detail["currentVersion"], json!("1.0.0"));
    assert_eq!(detail["versions"], json!(["1.0.0"]));
    assert_eq!(detail["sourceSha"], json!("abc1234"), "来源 pin 透出");
    assert_eq!(detail["skills"].as_array().unwrap().len(), 1, "技能清单读自安装缓存");
}

#[test]
fn store_list_plugins_merges_snapshot_entries_and_uninstall_refreshes_list() {
    let (dir, state) = test_state("store-list-uninstall");
    place_store_snapshot(dir.path());
    install_store_plugin(dir.path(), &state);

    // 装了插件：快照条目与已装行合并为一条（installed=true、版本历史、来源 pin、更新状态）
    let list = services::store_list_plugins(&state).unwrap();
    let views = list.as_array().unwrap();
    assert_eq!(views.len(), 1, "快照条目与已装行按名合并，不重复：{list}");
    let view = &views[0];
    assert_eq!(view["id"], json!("demo@demo-market"));
    assert_eq!(view["installed"], json!(true));
    assert_eq!(view["version"], json!("1.0.0"));
    assert_eq!(view["versions"], json!(["1.0.0"]));
    assert_eq!(view["sourceSha"], json!("abc1234"));
    assert_eq!(view["updateStatus"]["state"], json!("none"), "同版本同 sha 不误报更新");
    assert_eq!(view["hasHooks"], json!(false));

    // 卸载后列表刷新：条目仍在（上游清单未删），回到 installed=false
    let list = services::store_uninstall_plugin(&state, &format!("{STORE_PLUGIN}@{STORE_MARKET}")).unwrap();
    let views = list.as_array().unwrap();
    assert_eq!(views.len(), 1, "卸载后条目仍在");
    assert_eq!(views[0]["installed"], json!(false), "卸载后回到未装态");
    assert_eq!(views[0]["version"], json!("1.0.0"), "未装条目版本取市场侧解析值");
    // 磁盘缓存目录已删除
    let cache = dir.path().join("skills-marketplace").join("cache").join(STORE_MARKET).join(STORE_PLUGIN);
    assert!(!cache.exists(), "卸载删除全部版本目录");

    // 停用一个未装插件：可读错误（行不存在）
    let err = services::store_set_plugin_enabled(&state, &format!("{STORE_PLUGIN}@{STORE_MARKET}"), false).unwrap_err();
    assert!(err.contains("不存在"), "实际：{err}");
}

// ---------- 网络页公开端点探测（决策 D14修订） ----------

/// 起一个极简 HTTP mock 服务器：对每个连接回固定状态码（无响应体）后即关闭连接。
/// 线程阻塞在 accept（listener 随线程持有），测试进程结束时一并回收，不 join。
fn mock_http_server(status: u16) -> std::net::SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 1024];
            // 小 GET 请求一次 read 足够（只读不解析，读到什么无所谓）
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let resp = format!("HTTP/1.1 {status} MOCK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
        }
    });
    addr
}

/// 直连配置（None 模式显式清空代理含环境变量）：本地 mock 端点不受环境代理干扰。
fn direct_probe_proxy() -> craft_core::network::ProxyConfig {
    craft_core::network::ProxyConfig {
        mode: craft_core::network::ProxyMode::None,
        effective_url: None,
    }
}

/// 探测判定核心：任意 HTTP 状态码（含 4xx/5xx）都算连通成功——探测网络可达性而非
/// API 语义，401（无密钥时真实端点的典型响应）与 404 都应报成功并透出状态码。
#[test]
fn network_probe_counts_any_http_status_as_success() {
    let proxy = direct_probe_proxy();
    for status in [401u16, 404] {
        let addr = mock_http_server(status);
        let out = services::network_probe_url(&proxy, &format!("http://{addr}/"));
        assert!(out.error.is_none(), "HTTP {status} 应算连通成功，实际：{:?}", out.error);
        assert_eq!(out.http_status, Some(status), "HTTP {status} 状态码应透出");
        assert!(out.latency_ms >= 0);
    }
}

/// 探测判定核心：连接失败（端口无监听者）算失败，错误为可读中文，状态码为空。
#[test]
fn network_probe_unreachable_endpoint_fails_with_readable_error() {
    let proxy = direct_probe_proxy();
    // 先 bind 拿一个空闲端口再释放：该端口此后无监听者，连接必然被拒
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let out = services::network_probe_url(&proxy, &format!("http://{addr}/"));
    assert!(out.http_status.is_none(), "连接失败不应有状态码");
    let error = out.error.expect("不可达端口应报失败");
    assert!(
        error.contains("连接失败") || error.contains("连接超时"),
        "错误应为可读中文（连接失败/连接超时），实际：{error}"
    );
}

/// 探测清单的每个 id 都必须存在于编译期嵌入的目录快照里：`network_probe_endpoints`
/// 对目录里查不到的 id 静默跳过，拼写漂移（如把 `zhipuai` 写成 `zhipu`）只会让网络页
/// 少一行、不报任何错，靠这条测试拦下。
#[test]
fn probe_provider_ids_exist_in_catalog() {
    let (_dir, state) = test_state("probe-ids");
    let missing: Vec<&str> = services::PROBE_PROVIDER_IDS
        .iter()
        .copied()
        .filter(|pid| state.catalog.builtin_provider(pid).is_none())
        .collect();
    assert!(missing.is_empty(), "探测清单里这些 id 不在目录快照中：{missing:?}");
    // 国内常用三家必须在清单内且端点非空（端点为空的行后端会跳过、界面不渲染）
    for pid in ["zhipuai", "deepseek", "minimax-cn"] {
        let provider = state.catalog.builtin_provider(pid).unwrap_or_else(|| panic!("目录缺少 {pid}"));
        assert!(!provider.endpoint.trim().is_empty(), "{pid} 的端点不应为空");
    }
}

/// 探测行清单（D14 修订）：界面在探测**之前**就要把九行固定画出来，行集合与顺序必须与
/// 声明清单一致、id 唯一、端点非空。这条通道是「固定行」的唯一来源——若它漏行或多行，
/// 界面会固定渲染出错误行数，比旧实现（随探测结果走）更难察觉。
#[test]
fn probe_targets_follow_declared_order_and_are_unique() {
    let (_dir, state) = test_state("probe-targets");
    let rows = services::network_probe_targets(&state).unwrap();
    let rows = rows.as_array().expect("行清单应是数组");
    assert!(!rows.is_empty(), "行清单不应为空");
    assert_eq!(rows.len(), services::PROBE_PROVIDER_IDS.len(), "行数应与声明清单一致（目录缺失的 id 除外）");

    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap_or_default()).collect();
    let declared: Vec<&str> = services::PROBE_PROVIDER_IDS.to_vec();
    assert_eq!(ids, declared, "行顺序应为声明序");
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "行 id 必须唯一（重复 id 会让界面按 key 合并出错）");

    for row in rows {
        assert!(!row["name"].as_str().unwrap_or_default().is_empty(), "行名不应为空：{row}");
        assert!(!row["url"].as_str().unwrap_or_default().is_empty(), "行端点不应为空：{row}");
    }
}

#[test]
fn subagent_settings_roundtrip_and_dispatch() {    let (_dir, state) = test_state("subagent-settings");
    // 子代理配置已迁到独立键：Settings 里的旧字段保持默认（不再被写入路径消费）
    let settings = services::settings_get(&state).unwrap();
    assert_eq!(settings["subagentsEnabled"], true, "旧字段仍在 Settings 里（兼容形状）");
    assert_eq!(settings["subagentMaxConcurrent"], 4, "旧字段默认值");
    // 旧字段走 settingsSave 不再改变子代理行为：新键仍为空时按兼容回落读旧字段，
    // 因此这里读到 false 是预期的迁移语义（旧库升级不丢用户已关掉的开关）；
    // 一旦用户在新分节保存过，就只认新键、旧字段彻底失效。
    services::settings_save(&state, &json!({ "subagentsEnabled": false, "subagentMaxConcurrent": 99 })).unwrap();
    let fallback = services::subagent_settings_get(&state).unwrap();
    assert_eq!(fallback["enabled"], false, "新键为空时兼容回落旧字段");
    assert_eq!(fallback["maxConcurrent"], 8, "回落时同样钳制 1~8");

    // 新分节保存后即与旧字段脱钩
    services::subagent_settings_save(&state, &json!({ "enabled": true, "maxConcurrent": 3 })).unwrap();
    services::settings_save(&state, &json!({ "subagentsEnabled": false })).unwrap();
    let config = services::subagent_settings_get(&state).unwrap();
    assert_eq!(config["enabled"], true, "新键已写入后不再被旧字段覆盖");
    assert_eq!(config["maxConcurrent"], 3);
    // 封顶值只读随行（D15 的面板补注）：渲染层上限输入的最大值
    // 与「0 = 封顶 N」文案的唯一事实源
    assert_eq!(config["maxIterationsCap"], 500);

    // 子代理状态列表通道：空会话返回空数组
    let list = services::subagent_list(&state, "th_unknown").unwrap();
    assert!(list.as_array().unwrap().is_empty());
}

#[test]
fn max_iterations_defaults_to_unlimited_and_accepts_positive() {
    let (_dir, state) = test_state("max-iterations");
    // 默认 0 = 不限制（不是 300 这类隐形上限）
    let settings = services::settings_get(&state).unwrap();
    assert_eq!(settings["maxIterations"], 0, "最大工具轮数默认 0（不限制）");

    // 默认值经保存往返后仍是 0（未被回落成隐形上限）
    let saved = services::settings_save(&state, &json!({ "maxIterations": 0 })).unwrap();
    assert_eq!(saved["maxIterations"], 0);

    // 显式设正值仍然生效
    let saved = services::settings_save(&state, &json!({ "maxIterations": 120 })).unwrap();
    assert_eq!(saved["maxIterations"], 120);

    // 越界回落 0（不是回落 300）
    let saved = services::settings_save(&state, &json!({ "maxIterations": 5000 })).unwrap();
    assert_eq!(saved["maxIterations"], 0, "越界值回落不限制，而非隐形上限");
}

#[test]
fn subagent_settings_isolated_key_and_legacy_fallback() {
    let (_dir, state) = test_state("subagent-config");
    // 默认值（新键为空、旧键也是默认）：开启、并发 4、内置两型补齐
    let settings = services::subagent_settings_get(&state).unwrap();
    assert_eq!(settings["enabled"], true);
    assert_eq!(settings["maxConcurrent"], 4);
    let builtin = settings["builtinProfiles"].as_array().unwrap();
    assert_eq!(builtin.len(), 2, "内置两型必须补齐");
    assert_eq!(builtin[0]["agentType"], "general");
    assert_eq!(builtin[1]["agentType"], "explore");

    // 保存：并发越界钳制、轮数钳制
    services::subagent_settings_save(
        &state,
        &json!({
            "enabled": false,
            "maxConcurrent": 99,
            "builtinProfiles": [
                { "agentType": "general", "model": "", "maxIterations": 30 },
                { "agentType": "explore", "model": "", "maxIterations": 999 },
            ],
        }),
    )
    .unwrap();
    let saved = services::subagent_settings_get(&state).unwrap();
    assert_eq!(saved["enabled"], false);
    assert_eq!(saved["maxConcurrent"], 8, "并发上限钳制到 8");
    assert_eq!(saved["builtinProfiles"][0]["maxIterations"], 30);
    assert_eq!(saved["builtinProfiles"][1]["maxIterations"], 500, "轮数钳制到封顶值（500）");

    // 与 Settings 彻底分离：settingsGet 不含子代理字段的写入路径
    let app_settings = services::settings_get(&state).unwrap();
    assert_eq!(app_settings["subagentsEnabled"], true, "旧字段保持默认（不再被写入）");
}

#[test]
fn subagent_profile_crud_and_validation() {
    let (_dir, state) = test_state("subagent-profiles");
    // 新建：合法
    let created = services::subagent_profile_create(
        &state,
        &json!({
            "name": "Reviewer",
            "description": "只做代码评审",
            "rolePrompt": "你是评审子代理。",
            "model": "",
            "maxIterations": 20,
            "tools": { "kind": "readOnly" },
        }),
    )
    .unwrap();
    assert_eq!(created["name"], "reviewer", "名称归一为小写");
    assert_eq!(created["maxIterations"], 20);
    let id = created["id"].as_str().unwrap().to_string();

    // 保留字拒绝
    let err = services::subagent_profile_create(&state, &json!({ "name": "general" })).unwrap_err();
    assert!(err.contains("内置"), "应提示占用内置类型：{err}");
    // 重名拒绝
    let err = services::subagent_profile_create(&state, &json!({ "name": "reviewer" })).unwrap_err();
    assert!(err.contains("已被其他"), "{err}");
    // 非法名称拒绝
    assert!(services::subagent_profile_create(&state, &json!({ "name": "1bad" })).is_err());
    // 未知模型拒绝
    assert!(
        services::subagent_profile_create(&state, &json!({ "name": "other", "model": "nope/none" })).is_err()
    );

    // 更新：未提供的字段保持原值
    let updated = services::subagent_profile_update(&state, &id, &json!({ "description": "改过的简介" })).unwrap();
    assert_eq!(updated["description"], "改过的简介");
    assert_eq!(updated["name"], "reviewer", "未传 name 时保持原值");
    assert_eq!(updated["maxIterations"], 20);

    // only 档：不存在的工具名被过滤
    let only = services::subagent_profile_update(
        &state,
        &id,
        &json!({ "tools": { "kind": "only", "names": ["fs_read", "no_such_tool"] } }),
    )
    .unwrap();
    let names = only["tools"]["names"].as_array().unwrap();
    assert_eq!(names.len(), 1, "幽灵工具应被过滤");
    assert_eq!(names[0], "fs_read");

    // 启停
    let list = services::subagent_profile_set_enabled(&state, &id, false).unwrap();
    assert_eq!(list[0]["enabled"], false);
    // 删除
    let list = services::subagent_profile_delete(&state, &id).unwrap();
    assert!(list.as_array().unwrap().is_empty());
    assert!(services::subagent_profile_delete(&state, &id).is_err(), "重复删除报不存在");
}

// ---------- MCP ----------

#[test]
fn mcp_upsert_list_remove_round_trip() {
    let (_dir, state) = test_state("mcp-crud");

    // 新增 stdio 服务器
    let list = services::mcp_upsert(
        &state,
        &json!({
            "name": "demo",
            "transport": "stdio",
            "command": "/nonexistent/craftwork-mcp-demo",
            "args": ["--port", "1"],
            "timeoutMs": 3000
        }),
    )
    .unwrap();
    let servers = list.as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["name"], "demo");
    assert_eq!(servers[0]["transport"], "stdio");
    assert_eq!(servers[0]["enabled"], true);
    assert_eq!(servers[0]["envCount"], 0, "env 只回传条数");
    assert!(!servers[0]["id"].as_str().unwrap().is_empty());

    // 名称重复拒绝
    assert!(services::mcp_upsert(
        &state,
        &json!({ "name": "demo", "transport": "stdio", "command": "node" })
    )
    .is_err());

    // 缺命令拒绝
    assert!(services::mcp_upsert(&state, &json!({ "name": "bad", "transport": "stdio" })).is_err());

    // 编辑：改名字
    let id = servers[0]["id"].as_str().unwrap().to_string();
    let list = services::mcp_upsert(
        &state,
        &json!({ "id": id, "name": "demo2", "transport": "stdio", "command": "/nonexistent/x" }),
    )
    .unwrap();
    let servers = list.as_array().unwrap();
    assert_eq!(servers[0]["name"], "demo2");
    assert_eq!(servers[0]["id"], id.as_str(), "编辑保留原 id");

    // 删除
    let list = services::mcp_remove(&state, &id).unwrap();
    assert!(list.as_array().unwrap().is_empty());
    assert!(services::mcp_remove(&state, &id).is_err(), "重复删除报不存在");
}

/// 旧留桩数据（决策 D5 时代只有 name/command/args）必须能读出来升级，
/// 否则升级后用户已配置的服务器整片消失。
#[test]
fn mcp_legacy_stub_rows_are_upgraded() {
    let (_dir, state) = test_state("mcp-legacy");
    // 直接写旧形状
    state
        .db
        .with(|conn| craft_store::records::set_setting(
            conn,
            "mcp.servers",
            &json!([{
                "id": "mcp_legacy0000000",
                "name": "old-server",
                "command": "node",
                "args": ["server.js"],
                "enabled": true
            }]),
        ))
        .unwrap();
    let list = services::mcp_list(&state).unwrap();
    let servers = list.as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["name"], "old-server");
    assert_eq!(servers[0]["transport"], "stdio", "旧数据按「有 command 即 stdio」升级");
    assert_eq!(servers[0]["timeoutMs"], 30000, "超时回落默认值");
}

/// 未启用配置的启动：列表为空、不报错。
#[test]
fn mcp_list_empty_is_ok() {
    let (_dir, state) = test_state("mcp-empty");
    let list = services::mcp_list(&state).unwrap();
    assert!(list.as_array().unwrap().is_empty());
}

// ---------- MCP 工具注册规划（票 02） ----------

fn mcp_config(id: &str, name: &str) -> craft_mcp::ServerConfig {
    let mut config = craft_mcp::ServerConfig::new(name.to_string(), craft_mcp::Transport::Stdio);
    config.id = id.to_string();
    config.command = "node".into();
    config
}

fn mcp_descriptor(tool_name: &str) -> craft_mcp::ToolDescriptor {
    craft_mcp::ToolDescriptor {
        tool_name: tool_name.to_string(),
        title: None,
        description: String::new(),
        input_schema: json!({ "type": "object", "properties": {} }),
        read_only_hint: false,
        destructive_hint: false,
        idempotent_hint: false,
        meta_timeout_ms: None,
    }
}

/// 单服务器工具数上限必须真的生效：规格第 12 节「超出不注册」。
/// 旧实现算完上限后 `let _ =` 丢掉，等于没有上限。
#[test]
fn mcp_plan_caps_tools_per_server() {
    let limit = craft_tools::mcp::MAX_TOOLS_PER_SERVER;
    let descriptors: Vec<craft_mcp::ToolDescriptor> =
        (0..limit + 5).map(|index| mcp_descriptor(&format!("tool_{index}"))).collect();
    let planned = services::plan_mcp_tools(&[(mcp_config("mcp_a", "alpha"), descriptors)]);
    assert_eq!(planned.len(), limit, "单服务器最多注册 {limit} 个工具");
    assert_eq!(planned[0].model_name, "mcp__alpha__tool_0");
    assert_eq!(planned[limit - 1].model_name, format!("mcp__alpha__tool_{}", limit - 1));
}

/// 归一后撞名 fail closed：第二个同名工具不注册，而不是静默覆盖。
#[test]
fn mcp_plan_drops_colliding_names() {
    // 两个配置名归一后相同（空格 → 下划线），工具名必然撞车
    let planned = services::plan_mcp_tools(&[
        (mcp_config("mcp_a", "my server"), vec![mcp_descriptor("read")]),
        (mcp_config("mcp_b", "my_server"), vec![mcp_descriptor("read")]),
    ]);
    assert_eq!(planned.len(), 1, "撞名时只保留第一个");
    assert_eq!(planned[0].server_id, "mcp_a");
}

/// 模型工具名第一段必须来自**配置名**：服务器自述名变化不能让工具名漂移
/// （授权缓存与历史重建都依赖名字稳定）。
#[test]
fn mcp_plan_names_tools_from_config_name() {
    let planned = services::plan_mcp_tools(&[(mcp_config("mcp_a", "我的服务器"), vec![mcp_descriptor("echo")])]);
    assert_eq!(planned.len(), 1);
    // 归一名与自述名无关，配置名「我的服务器」全是非 ASCII → 归一为 unknown
    assert_eq!(planned[0].model_name, "mcp__unknown__echo");
    assert_eq!(planned[0].server_name, "我的服务器", "展示名仍是用户填的原名");

    let planned = services::plan_mcp_tools(&[(mcp_config("mcp_a", "alpha"), vec![mcp_descriptor("echo")])]);
    assert_eq!(planned[0].model_name, "mcp__alpha__echo");
}

// ---------- MCP 编辑不丢配置（票 06） ----------

/// 凭据值只存在本地库、不回传前端（规格第 9 节），因此前端编辑时拿不到原值。
/// 若服务层按「字段缺失即空」处理，改一句名字就会把 env / headers / cwd 全冲掉。
#[test]
fn mcp_edit_keeps_env_headers_and_cwd() {
    let (_dir, state) = test_state("mcp-edit-keeps");
    services::mcp_upsert(
        &state,
        &json!({
            "name": "keep-me",
            "transport": "stdio",
            "command": "/nonexistent/craftwork-mcp-edit",
            "env": { "API_KEY": "secret-value" },
            "cwd": "/tmp/workdir"
        }),
    )
    .unwrap();
    let id = services::mcp_list(&state).unwrap()[0]["id"].as_str().unwrap().to_string();

    // 只改名字，不带 env / cwd（前端就是这样提交的：凭据值拿不到）
    services::mcp_upsert(&state, &json!({ "id": id, "name": "renamed", "transport": "stdio", "command": "/nonexistent/craftwork-mcp-edit" }))
        .unwrap();

    let list = services::mcp_list(&state).unwrap();
    assert_eq!(list[0]["name"], "renamed");
    assert_eq!(list[0]["envCount"], 1, "改名不应冲掉已存凭据");
    assert_eq!(list[0]["cwd"], "/tmp/workdir", "改名不应冲掉工作目录");
}

/// 显式传空对象 = 用户确实要清空（与「字段缺失 = 保留」配套，否则没法删凭据）。
#[test]
fn mcp_edit_can_clear_env_explicitly() {
    let (_dir, state) = test_state("mcp-edit-clear");
    services::mcp_upsert(
        &state,
        &json!({ "name": "clear-me", "transport": "stdio", "command": "/nonexistent/craftwork-mcp-edit", "env": { "API_KEY": "v" } }),
    )
    .unwrap();
    let id = services::mcp_list(&state).unwrap()[0]["id"].as_str().unwrap().to_string();

    services::mcp_upsert(
        &state,
        &json!({ "id": id, "name": "clear-me", "transport": "stdio", "command": "/nonexistent/craftwork-mcp-edit", "env": {} }),
    )
    .unwrap();
    assert_eq!(services::mcp_list(&state).unwrap()[0]["envCount"], 0, "显式空对象应清空凭据");
}

/// 编辑一个已停用的服务器，保存后必须仍是停用（前端不再硬编码 enabled=true）。
#[test]
fn mcp_edit_keeps_disabled_state() {
    let (_dir, state) = test_state("mcp-edit-disabled");
    services::mcp_upsert(
        &state,
        &json!({ "name": "off", "transport": "stdio", "command": "/nonexistent/craftwork-mcp-edit", "enabled": false }),
    )
    .unwrap();
    let id = services::mcp_list(&state).unwrap()[0]["id"].as_str().unwrap().to_string();

    services::mcp_upsert(&state, &json!({ "id": id, "name": "off2", "transport": "stdio", "command": "/nonexistent/craftwork-mcp-edit" }))
        .unwrap();
    assert_eq!(services::mcp_list(&state).unwrap()[0]["enabled"], false, "编辑不应静默启用停用服务器");
}

// ---------- MCP 连接生命周期（票 05） ----------
/// 启动时应连接所有 enabled 的服务器：`mcp_apply` 真的被调用过，
/// 状态从 disconnected 走到 failed（命令不存在），停用的服务器停在 disabled。
#[test]
fn mcp_apply_connects_enabled_and_marks_disabled() {
    let (_dir, state) = test_state("mcp-apply");
    let mut enabled = mcp_config("mcp_on", "on-server");
    enabled.command = "/nonexistent/craftwork-mcp-on".into();
    enabled.timeout_ms = 1000;
    let mut disabled = mcp_config("mcp_off", "off-server");
    disabled.enabled = false;

    services::mcp_apply(&state, &enabled);
    services::mcp_apply(&state, &disabled);

    assert_eq!(
        state.mcp_pool.runtime_status("mcp_on").status,
        craft_mcp::ServerStatus::Failed,
        "启用的服务器应尝试连接（命令不存在 → 失败），而不是停在未连接"
    );
    assert_eq!(
        state.mcp_pool.runtime_status("mcp_off").status,
        craft_mcp::ServerStatus::Disabled,
        "停用的服务器不应连接"
    );
}

/// 退出时 `shutdown` 之后池内不再产出任何工具（等价于「连接都断开了」）。
#[test]
fn mcp_pool_shutdown_clears_registered_tools() {
    let (_dir, state) = test_state("mcp-shutdown");
    state.mcp_pool.shutdown();
    assert!(state.mcp_pool.connected_tools().is_empty(), "shutdown 后不应再有已连接工具");
}

/// chatCancelWait 无在途等待时拒绝（D21）：防重复点击与竞态，通道层给出可读错误。
#[test]
fn chat_cancel_wait_without_in_flight_is_rejected() {
    let (_dir, state) = test_state("cancel-wait-idle");
    let err = services::chat_cancel_wait(&state, "th_no_wait_here").unwrap_err();
    assert!(err.contains("没有等待中的任务"), "应给出可读错误：{err}");
}
