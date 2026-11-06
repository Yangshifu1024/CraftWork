//! 壳层服务分发集成测试：对话主链路（隔离目录）。
//! 覆盖：启动 ensure 临时目录 → 设置往返 → 会话创建/快照 → 首发原子路径（无模型失败零落库）
//! → 文件读写与 diff → 提供商 CRUD 与密钥。

use std::sync::Arc;

use serde_json::{json, Value};

// 直接复用壳层 crate 的服务函数（lib 目标）
use craftwork_lib::services;

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
    let registry = Arc::new(craft_tools::registry::Registry::new(&backup_dir, Arc::clone(&db)));
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
    let runner = Arc::new(craft_agent::runner::Runner::new(craft_agent::runner::RunnerDeps {
        db: Arc::clone(&db),
        records: recorder.clone(),
        registry: Arc::clone(&registry),
        engine: Arc::clone(&engine),
        approvals: Arc::clone(&approvals),
        models: Arc::clone(&models),
        attachment_dir: dir.path().join("attachments").to_string_lossy().to_string(),
        data_dir: data_dir.clone(),
        log: Arc::new(craft_agent::session_log::SessionLog::new(
            dir.path(),
            Arc::new(std::sync::atomic::AtomicU8::new(craft_agent::session_log::LEVEL_INFO)),
        )),
        app_version: "test".into(),
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
            log_level: Arc::new(std::sync::atomic::AtomicU8::new(craft_agent::session_log::LEVEL_INFO)),
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
    let saved = services::settings_save(&state, &json!({ "theme": "dark", "fontSize": 20, "panelWidth": 9999, "smoothFollow": true })).unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["fontSize"], 15, "越界字号回退默认");
    assert_eq!(saved["panelWidth"], 760, "越界面板宽度收口上限");
    assert_eq!(saved["smoothFollow"], true, "平滑跟随保存往返");
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
    assert_eq!(view["path"], std::fs::canonicalize(&p1).unwrap().to_string_lossy().to_string(), "首项为主要目录");

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
