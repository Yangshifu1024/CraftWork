//! 工厂与目录集成测试：custom providers 解析、密钥校验、停用语义、回退链。
//! 用例语义对应 tests/factory.test.ts / model.test.ts（vitest 版的关键部分）。

use std::path::PathBuf;
use std::sync::Arc;

use craft_core::contract::event_type;
use craft_model::catalog::Catalog;
use craft_model::factory::{KeyProvider, ModelFactory};
use craft_store::db::Db;
use craft_store::records::set_setting;
use craft_model::types::PersistedModel;

struct StubKeys {
    keys: std::collections::HashMap<String, String>,
}

impl KeyProvider for StubKeys {
    fn get_for(&self, provider_id: &str, _env_names: &[String]) -> String {
        self.keys.get(provider_id).cloned().unwrap_or_default()
    }
}

fn tmp_db(tag: &str) -> (PathBuf, Arc<Db>) {
    let dir = std::env::temp_dir().join(format!(
        "cw-factory-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Arc::new(Db::open(&dir.join("test.db")).expect("打开测试库失败"));
    (dir, db)
}

fn model(id: &str, enabled: bool) -> PersistedModel {
    PersistedModel {
        id: id.into(),
        name: format!("模型 {id}"),
        context_k: 128,
        max_tokens: 4096,
        caps: craft_model::types::DEFAULT_CAPS,
        cost: None,
        status: String::new(),
        canonical_id: String::new(),
        release_date: String::new(),
        source: "manual".into(),
        enabled,
    }
}

fn provider_json(id: &str, kind: &str, endpoint: &str, key_required: bool, enabled: bool, models: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": id, "kind": kind, "endpoint": endpoint,
        "keyRequired": key_required, "note": "", "source": "manual",
        "catalogId": "", "catalogVersion": "", "envVars": [],
        "enabled": enabled, "models": models,
    })
}

fn install_providers(db: &Db, providers: serde_json::Value) {
    db.with(|conn| set_setting(conn, "model.custom_providers", &providers)).unwrap();
}

#[test]
fn 内置目录_加载与元数据() {
    let (dir, db) = tmp_db("builtin");
    let catalog = Catalog::new(Arc::clone(&db));
    let (generated_at, version, provider_count, model_count) = catalog.builtin_meta();
    assert!(!generated_at.is_empty());
    assert!(!version.is_empty());
    assert!(provider_count > 100, "models.dev 快照应含大量提供商");
    assert!(model_count > 1000);
    // 搜索
    assert!(!catalog.search_builtin_providers("openai").is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 解析_引用优先_原始ID兼容() {
    let (dir, db) = tmp_db("resolve");
    install_providers(
        &db,
        serde_json::json!([
            provider_json("prov1", "openai", "https://api.p1.com/v1", false, true,
                serde_json::json!([model("m1", true), model("shared", true)])),
            provider_json("prov2", "anthropic", "https://api.p2.com", false, true,
                serde_json::json!([model("shared", true)])),
        ]),
    );
    let catalog = Catalog::new(Arc::clone(&db));
    // 引用解析
    let info = catalog.resolve("prov1/m1").unwrap();
    assert_eq!((info.provider.as_str(), info.id.as_str()), ("prov1", "m1"));
    // 原始 ID 全局匹配（兼容旧数据）
    let info = catalog.resolve("m1").unwrap();
    assert_eq!(info.provider, "prov1");
    // 规范化
    let normalized = catalog.normalize("shared").unwrap();
    assert!(normalized.reference.starts_with("prov"));
    assert_eq!(normalized.model_name, "模型 shared");
    // 找不到 → None（不静默回退）
    assert!(catalog.resolve("nope").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 客户端构造_缺密钥与停用分支() {
    let (dir, db) = tmp_db("branches");
    install_providers(
        &db,
        serde_json::json!([
            provider_json("needkey", "openai", "https://a.com/v1", true, true, serde_json::json!([model("m", true)])),
            provider_json("disabled", "openai", "https://b.com/v1", false, false, serde_json::json!([model("m", true)])),
            provider_json("modeloff", "openai", "https://c.com/v1", false, true, serde_json::json!([model("m", false)])),
            provider_json("ok", "anthropic", "https://d.com", false, true, serde_json::json!([model("m", true)])),
        ]),
    );
    let keys = StubKeys {
        keys: [("ok".to_string(), "sk-test".to_string())].into_iter().collect(),
    };
    let factory = ModelFactory::new(Catalog::new(Arc::clone(&db)), Box::new(keys));

    let expect_err = |factory: &ModelFactory, reference: &str| -> craft_model::errors::ModelError {
        match factory.client(reference) {
            Ok(_) => panic!("{reference} 应返回错误"),
            Err(err) => err,
        }
    };
    // 模型不存在
    let err = expect_err(&factory, "nope/x");
    assert_eq!(err.code, "provider_unavailable");
    // 缺密钥
    let err = expect_err(&factory, "needkey/m");
    assert_eq!(err.code, "auth");
    assert!(err.message.contains("尚未配置密钥"));
    // 提供商停用
    let err = expect_err(&factory, "disabled/m");
    assert_eq!(err.code, "provider_unavailable");
    assert!(err.message.contains("已停用"));
    // 模型停用
    let err = expect_err(&factory, "modeloff/m");
    assert!(err.message.contains("已停用"));
    // 正常构造（anthropic 分支）
    assert!(factory.client("ok/m").is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 回退链_停用与同引用与缺密钥候选不参与() {
    let (dir, db) = tmp_db("fallback");
    install_providers(
        &db,
        serde_json::json!([
            provider_json("cur", "openai", "https://a.com/v1", false, true, serde_json::json!([model("m", true)])),
            provider_json("fb", "openai", "https://b.com/v1", false, true, serde_json::json!([model("m", true), model("off", false)])),
            provider_json("needkey", "openai", "https://c.com/v1", true, true, serde_json::json!([model("m", true)])),
            provider_json("disabled", "openai", "https://d.com/v1", false, false, serde_json::json!([model("m", true)])),
        ]),
    );
    let keys = StubKeys::default();
    let factory = ModelFactory::new(Catalog::new(Arc::clone(&db)), Box::new(keys));

    // 无 resolver → 空
    assert_eq!(factory.fallback_model("cur/m"), "");
    // resolver 返回空 → 空
    factory.set_fallback_resolver(Box::new(|| String::new()));
    assert_eq!(factory.fallback_model("cur/m"), "");
    // 正常回退
    factory.set_fallback_resolver(Box::new(|| "fb/m".into()));
    assert_eq!(factory.fallback_model("cur/m"), "fb/m");
    // 同引用 → 空
    factory.set_fallback_resolver(Box::new(|| "cur/m".into()));
    assert_eq!(factory.fallback_model("cur/m"), "");
    // 候选模型停用 → 空
    factory.set_fallback_resolver(Box::new(|| "fb/off".into()));
    assert_eq!(factory.fallback_model("cur/m"), "");
    // 候选提供商停用 → 空
    factory.set_fallback_resolver(Box::new(|| "disabled/m".into()));
    assert_eq!(factory.fallback_model("cur/m"), "");
    // 候选需要密钥且不可得 → 空
    factory.set_fallback_resolver(Box::new(|| "needkey/m".into()));
    assert_eq!(factory.fallback_model("cur/m"), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 协议类型_费用分支用() {
    let (dir, db) = tmp_db("kind");
    install_providers(
        &db,
        serde_json::json!([
            provider_json("p1", "anthropic", "https://a.com", false, true, serde_json::json!([])),
            provider_json("p2", "openai-compatible", "https://b.com/v1", false, true, serde_json::json!([])),
        ]),
    );
    let factory = ModelFactory::new(Catalog::new(Arc::clone(&db)), Box::new(StubKeys::default()));
    assert_eq!(factory.provider_kind("p1"), "anthropic");
    assert_eq!(factory.provider_kind("p2"), "openai-compatible");
    assert_eq!(factory.provider_kind("none"), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 事件类型常量_与契约对齐() {
    // 烟雾：event_type 常量可访问（contract 与 events.ts 对齐由黄金文件测试保障）
    assert_eq!(event_type::RUN_STARTED, "run.started");
}

impl Default for StubKeys {
    fn default() -> Self {
        Self { keys: std::collections::HashMap::new() }
    }
}
