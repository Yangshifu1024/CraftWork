//! 壳层服务分发集成测试：对话主链路（隔离目录）。
//! 覆盖：启动 ensure 临时目录 → 设置往返 → 会话创建/快照 → 首发原子路径（无模型失败零落库）
//! → 文件读写与 diff → 提供商 CRUD 与密钥。

use std::sync::Arc;

use serde_json::{json, Value};

// 直接复用壳层 crate 的服务函数（lib 目标）
use craftwork_lib::services;

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
        }),
    )
}

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
fn 启动ensure与设置往返() {
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
    let saved = services::settings_save(&state, &json!({ "theme": "dark", "fontSize": 20, "panelWidth": 9999 })).unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["fontSize"], 15, "越界字号回退默认");
    assert_eq!(saved["panelWidth"], 760, "越界面板宽度收口上限");
}

#[test]
fn 首发原子路径_无模型失败零落库() {
    let (_dir, state) = test_state("chat");
    let ws_id = services::ensure_conversation_workspace(&state).unwrap().as_str().unwrap().to_string();

    // 模型不存在：失败且不产生会话
    let err = services::chat_create_and_start(&state, "code", &ws_id, "你好", &[], "nope/model").unwrap_err();
    assert!(err.contains("模型不存在"));
    let threads = services::thread_list(&state, "code", false).unwrap();
    assert_eq!(threads.as_array().unwrap().len(), 0, "失败首发零落库");
}

#[test]
fn 会话创建与快照链路() {
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
    let result = services::chat_create_and_start(&state, "code", &ws_id, "帮我看看这个项目", &[], "stub/m1").unwrap();
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
fn 文件写入与diff() {
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
fn 提供商密钥与启停() {
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
fn 内置目录加载与导入() {
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
fn 提供商保存空id由服务端生成并返回单对象() {
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

#[test]
fn 创建项目空名推导重名序号与返回单对象() {
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
fn 创建项目拒绝内置临时会话目录() {
    let (dir, state) = test_state("ws-add-project-guard");
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
fn 设置保存档位即时生效无需重启() {
    let (_dir, state) = test_state("level-hot");
    // migration-parity 票 04：引擎只在启动时按设置初始化，保存后必须同步热切换
    services::settings_save(&state, &json!({ "approvalLevel": "readonly" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "readonly");
    services::settings_save(&state, &json!({ "approvalLevel": "free" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "free");
    // 非法值回落默认档（graded）
    services::settings_save(&state, &json!({ "approvalLevel": "bogus" })).unwrap();
    assert_eq!(state.engine.lock().unwrap().level().as_str(), "graded");
}
