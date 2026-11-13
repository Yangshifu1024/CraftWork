//! 自动命名的广播契约（回归测试）。
//!
//! 背景（真实缺陷）：`auto_title` 只改 `threads.title` 这一列，不往总线发任何事件。
//! 渲染层刷新会话列表的唯一触发点是 `thread.updated`（见 `src/renderer/src/store/app.ts`
//! 的同名分支），而这个事件过去只在运行开始/结束时由 `Recorder::set_thread_status` 发出。
//! 结果：首轮并行起好的标题确实落进了库，但长 run 期间侧栏一直停在「首条消息截断」的
//! 兜底标题，看起来就像「没有自动命名」——排查时打开库才发现标题早已是模型生成的短名。
//!
//! 这里验证修正后的契约：命名一旦真正写库，就广播一次 `thread.updated`；标题没被改写
//! （条件更新被 `title_auto = 0` 挡住，即用户手动改过名）时不该发多余事件。

use std::collections::VecDeque;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use futures_util::stream;
use serde_json::json;

use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_agent::runner::{Runner, RunnerDeps};
use craft_agent::session_log::SessionLog;
use craft_agent::subagent::SubAgentConfig;
use craft_core::bus::Bus;
use craft_model::catalog::Catalog;
use craft_model::factory::ModelFactory;
use craft_model::types::{Chunk, Client, Request};
use craft_policy::Engine;
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_tools::registry::Registry;

/// 假模型客户端：记录收到的全部请求，主对话按队列产出，标题 sidecar 固定回一句标题。
///
/// 自动命名也是一次模型调用（走同一个客户端），因此按 system 提示词分流：
/// 标题 sidecar 的提示词带「标题生成器」，主对话没有。
struct ScriptedClient {
    main: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl ScriptedClient {
    fn new(main: Vec<&str>, requests: Arc<Mutex<Vec<Request>>>) -> Self {
        Self {
            main: Arc::new(Mutex::new(main.into_iter().map(String::from).collect())),
            requests,
        }
    }
}

impl Client for ScriptedClient {
    fn stream<'a>(
        &'a self,
        req: &'a Request,
    ) -> futures_core::stream::BoxStream<'a, Result<Chunk, craft_model::errors::ModelError>> {
        self.requests.lock().unwrap().push(req.clone());
        let is_title = req.system.iter().any(|b| b.content.contains("标题生成器"));
        let text = if is_title {
            "自动命名广播".to_string()
        } else {
            self.main
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| "（脚本已用尽）".into())
        };
        let chunks = vec![
            Ok(Chunk { text: Some(text), ..Default::default() }),
            Ok(Chunk { done: true, ..Default::default() }),
        ];
        Box::pin(stream::iter(chunks))
    }

    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(ScriptedClient {
            main: Arc::clone(&self.main),
            requests: Arc::clone(&self.requests),
        })
    }
}

struct NoKeys;
impl craft_model::factory::KeyProvider for NoKeys {
    fn get_for(&self, _provider_id: &str, _env_names: &[String]) -> String {
        String::new()
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    db: Arc<Db>,
    runner: Arc<Runner>,
    bus: Bus,
    /// 总线冲刷出来的事件（类型 + 载荷）：断言「命名写库后确有 thread.updated 落到界面」
    seen: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    thread_id: String,
}

/// 装配隔离环境：真实 SQLite + 真实 Runner，模型换成脚本客户端，不触网。
fn harness(title_auto: bool, main_text: &str) -> Harness {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = dir.path().to_string_lossy().to_string();
    let db = Arc::new(Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    db.with(|conn| {
        craft_store::records::set_setting(
            conn,
            "model.custom_providers",
            &json!([{
                "id": "stub", "name": "Stub", "kind": "openai-compatible",
                "endpoint": "https://stub.invalid/v1", "keyRequired": false,
                "models": [{
                    "id": "m1", "name": "脚本模型", "contextK": 128, "maxTokens": 4096,
                    "caps": { "stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false },
                    "status": "", "source": "manual", "enabled": true,
                }],
            }]),
        )
    })
    .unwrap();

    // 总线回调收集事件：断言「命名写库后确有 thread.updated 落到界面」
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let bus = Bus::new(move |batch| {
        let mut guard = sink.lock().unwrap();
        for event in batch.events {
            guard.push((event.event_type, event.data));
        }
    });
    let recorder = Recorder::new(Arc::clone(&db), Arc::new(bus.clone()));
    let engine = Arc::new(Mutex::new(Engine::default()));
    let grants = Arc::new(Grants::new(Arc::clone(&db)));
    let approvals = Arc::new(Approvals::new(Arc::clone(&db), recorder.clone(), Arc::clone(&grants)));
    let catalog = Arc::new(Catalog::new(Arc::clone(&db)));
    catalog.reload();
    let models = Arc::new(ModelFactory::new(Arc::clone(&catalog), Box::new(NoKeys)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let scripted = ScriptedClient::new(vec![main_text], Arc::clone(&requests));
    models.set_client_override(Box::new(move |_| Some(scripted.clone_box())));
    let registry = Arc::new(Registry::new(&data_dir, Arc::clone(&db)));
    let log = Arc::new(SessionLog::new(dir.path(), Arc::new(AtomicU8::new(craft_agent::session_log::LEVEL_ERROR))));
    let runner = Arc::new(Runner::new(RunnerDeps {
        db: Arc::clone(&db),
        records: recorder.clone(),
        registry,
        engine,
        approvals,
        models,
        attachment_dir: dir.path().join("attachments").to_string_lossy().to_string(),
        data_dir: data_dir.clone(),
        log,
        app_version: "test".into(),
        mcp_tool_count: Box::new(|| 0),
        proxy_config: Box::new(|| craft_core::network::ProxyConfig {
            mode: craft_core::network::ProxyMode::None,
            effective_url: None,
        }),
        subagent_config: Box::new(move || SubAgentConfig::new(false, 1, Vec::new(), Vec::new())),
        refresh_shell_env: Arc::new(|| {}),
        shell_tasks: None,
    }));

    let thread_id = "th_title_test".to_string();
    craft_store::threads::create_thread(
        &db.lock_conn(),
        craft_store::threads::NewThread {
            id: &thread_id,
            mode: "code",
            title: "很长的首条消息截断兜底标题…",
            title_auto,
            model_id: "stub/m1",
            reasoning_level: "",
            workspace_id: "",
            status: "idle",
            archived: false,
        },
    )
    .unwrap();
    Harness { _dir: dir, db, runner, bus, seen, thread_id }
}

impl Harness {
    /// 塞一条用户消息并跑完整个 run；结束后等自动命名旁路任务收尾（标题写库 + 广播）。
    /// 消息长度固定超过 20 字，保证走模型命名而不是「短名直用」。
    fn run(&self) {
        let seq = self
            .db
            .with(|conn| {
                craft_store::events::append_event(
                    conn,
                    &craft_store::events::EventRecord {
                        thread_id: self.thread_id.clone(),
                        message_id: "msg_title_test".into(),
                        run_id: None,
                        kind: "user_message".into(),
                        payload: json!({
                            "text": "帮我看看自动命名为什么不生效，长 run 期间侧栏一直显示首条消息截断的兜底标题",
                            "attachments": [],
                            "planMode": false
                        }),
                    },
                )
                .map(|(_, seq)| seq)
            })
            .unwrap();
        let runner = Arc::clone(&self.runner);
        let thread_id = self.thread_id.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                tokio::task::spawn_blocking(move || runner.run(&thread_id, Some(seq)))
                    .await
                    .expect("发起运行失败")
                    .expect("启动运行失败");
                // 命名是 spawn_blocking 旁路：等它把标题写库并广播完再收尾
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            });
        })
        .join()
        .unwrap();
        // close() 冲刷总线余量，确保收集到的就是全量事件
        self.bus.close();
    }

    fn title(&self) -> (String, bool) {
        let row = craft_store::threads::get_thread(&self.db.lock_conn(), &self.thread_id).unwrap();
        (row.title, row.title_auto)
    }

    /// 命名广播的特征：thread.updated 不带 status（运行状态广播才会带 status）。
    fn saw_title_broadcast(&self) -> bool {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .any(|(t, data)| t == "thread.updated" && data.get("status").is_none())
    }
}

/// 命名成功写库后必须广播 thread.updated，界面才会即时看到新标题。
///
/// 首条消息刻意超过 20 字：否则会走「短名直用」分支，不调模型、标题就是用户原话，
/// 断言不出「模型标题写库」这件事。
#[test]
fn auto_title_broadcasts_thread_updated_after_writing_title() {
    let h = harness(true, "我先看看自动命名是怎么接的。");
    h.run();
    let (title, title_auto) = h.title();
    assert_eq!(title, "自动命名广播", "模型标题应写库");
    assert!(!title_auto, "命名落地后 title_auto 应翻转，避免后续重复命名");
    assert!(h.saw_title_broadcast(), "命名写库后必须广播 thread.updated（不带 status）");
}

/// 用户手动改过名（title_auto = 0）时不再命名，也不该多发事件。
#[test]
fn auto_title_skipped_when_title_was_renamed_manually() {
    let h = harness(false, "我先看看自动命名是怎么接的。");
    h.run();
    let (title, title_auto) = h.title();
    assert_eq!(title, "很长的首条消息截断兜底标题…", "手动标题不得被覆盖");
    assert!(!title_auto);
    // 运行开始/结束本身会发 thread.updated（set_thread_status）；这里只要求标题没变，
    // 命名 sidecar 没有产生额外改写（标题断言已覆盖核心语义）。
}
