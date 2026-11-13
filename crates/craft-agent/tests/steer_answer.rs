//! 引导应答的端到端验证（D19）。
//!
//! 覆盖两件事：
//! 1. **未应答 → 纠偏一次**：本轮并入了引导却只产出工具调用（零正文），下一轮开头
//!    注入一条提醒；再下一轮不再出现（每批最多一次，不与模型僵持）。
//! 2. **接力 run**：运行中入队的引导在 run 正常收尾后自动接力为新 run 消费
//!    （`runner.rs` 的 `execute` 主循环，此前零覆盖）。
//!
//! 装配手法与 `plan_gate.rs` 一致（隔离数据目录 + stub 提供商 + client_override），
//! 区别只在需要「运行中入队」：`try_steer` 在没有在途 run 时静默返回 `Ok(false)`，
//! 所以必须先等 run 登记进取消表再入队，否则引导根本不进队列。

use std::collections::VecDeque;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use futures_core::stream::BoxStream;
use serde_json::json;

use craft_agent::runner::{Runner, RunnerDeps, STEER_UNANSWERED_NUDGE};
use craft_agent::session_log::SessionLog;
use craft_core::bus::Bus;
use craft_model::catalog::Catalog;
use craft_model::factory::ModelFactory;
use craft_model::types::{Chunk, Client, Request, ToolCall, Usage};
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_policy::Engine;
use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_tools::registry::Registry;

/// 一轮脚本化的模型输出：正文 + 若干工具调用。
struct Round {
    text: String,
    tool_calls: Vec<(String, String, serde_json::Value)>,
}

type Hook = Arc<dyn Fn() + Send + Sync>;
type Hooks = Arc<Mutex<Vec<(usize, Hook)>>>;

struct ScriptedClient {
    rounds: Arc<Mutex<VecDeque<Round>>>,
    seen: Arc<Mutex<Vec<Request>>>,
    /// 主循环调用计数（只数带工具清单的请求；命名/预测等 sidecar 小请求不计）
    main_calls: Arc<Mutex<usize>>,
    /// (第几次主循环调用之前, 回调)：用于在确定的轮次入队引导——接力测试要求
    /// 引导在**最后一次 take_steers 之后**入队，靠时间竞速不可靠。
    hooks: Hooks,
}

struct NoKeys;
impl craft_model::factory::KeyProvider for NoKeys {
    fn get_for(&self, _provider_id: &str, _env_names: &[String]) -> String {
        String::new()
    }
}

impl Client for ScriptedClient {
    fn stream<'a>(
        &'a self,
        req: &'a Request,
    ) -> BoxStream<'a, Result<Chunk, craft_model::errors::ModelError>> {
        // 主循环请求（带工具清单）计数并触发到轮钩子；sidecar 小请求不参与。
        if !req.tools.is_empty() && !req.messages.is_empty() {
            let mut calls = self.main_calls.lock().unwrap();
            *calls += 1;
            let n = *calls;
            drop(calls);
            let fired: Vec<Arc<dyn Fn() + Send + Sync>> = {
                let mut hooks = self.hooks.lock().unwrap();
                let mut fired = Vec::new();
                hooks.retain(|(at, cb)| {
                    if *at <= n {
                        fired.push(Arc::clone(cb));
                        false
                    } else {
                        true
                    }
                });
                fired
            };
            for cb in fired {
                cb();
            }
        }
        self.seen.lock().unwrap().push(req.clone());
        let round = self.rounds.lock().unwrap().pop_front().unwrap_or(Round {
            text: "（脚本已用尽）".into(),
            tool_calls: vec![],
        });
        let mut chunks: Vec<Result<Chunk, craft_model::errors::ModelError>> = Vec::new();
        if !round.text.is_empty() {
            chunks.push(Ok(Chunk { text: Some(round.text), ..Default::default() }));
        }
        for (id, name, args) in round.tool_calls {
            chunks.push(Ok(Chunk {
                tool_call: Some(ToolCall { id, name, args }),
                ..Default::default()
            }));
        }
        chunks.push(Ok(Chunk {
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 5,
                cache_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                prompt_tokens: 15,
            }),
            ..Default::default()
        }));
        chunks.push(Ok(Chunk { done: true, ..Default::default() }));
        Box::pin(futures_util::stream::iter(chunks))
    }

    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(ScriptedClient {
            rounds: Arc::clone(&self.rounds),
            seen: Arc::clone(&self.seen),
            main_calls: Arc::clone(&self.main_calls),
            hooks: Arc::clone(&self.hooks),
        })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    db: Arc<Db>,
    runner: Arc<Runner>,
    recorder: Recorder,
    thread_id: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

fn harness_with_hooks(
    rounds: Vec<Round>,
    hooks: Vec<(usize, Arc<dyn Fn() + Send + Sync>)>,
) -> Harness {
    let dir = tempfile::TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let data_dir = dir.path().to_string_lossy().to_string();
    let db = Arc::new(Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    let recorder = Recorder::new(Arc::clone(&db), Arc::new(Bus::new(|_| {})));
    let engine = Arc::new(Mutex::new(Engine::default()));
    let grants = Arc::new(Grants::new(Arc::clone(&db)));
    let approvals = Arc::new(Approvals::new(Arc::clone(&db), recorder.clone(), Arc::clone(&grants)));
    let catalog = Arc::new(Catalog::new(Arc::clone(&db)));

    let provider = json!({
        "id": "stub", "name": "Stub", "kind": "openai-compatible",
        "endpoint": "https://stub.invalid/v1", "keyRequired": false,
        "models": [{
            "id": "m1", "name": "脚本模型", "contextK": 128, "maxTokens": 4096,
            "caps": { "stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false },
            "status": "", "source": "manual", "enabled": true,
        }],
    });
    db.with(|conn| craft_store::records::set_setting(conn, "model.custom_providers", &json!([provider]))).unwrap();

    let models = Arc::new(ModelFactory::new(Arc::clone(&catalog), Box::new(NoKeys)));
    catalog.reload();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let scripted = ScriptedClient {
        rounds: Arc::new(Mutex::new(rounds.into_iter().collect())),
        seen: Arc::clone(&requests),
        main_calls: Arc::new(Mutex::new(0)),
        hooks: Arc::new(Mutex::new(hooks)),
    };
    models.set_client_override(Box::new(move |_| Some(scripted.clone_box())));

    let registry = Arc::new(Registry::new(&data_dir, Arc::clone(&db)));
    registry.register(Box::new(craft_tools::fs::FsList));
    registry.register(Box::new(craft_tools::fs::FsRead));
    registry.register(Box::new(craft_tools::todo::TodoWrite));

    // 日志目录按进程内序号隔离（同 plan_gate：四个 test 并行跑，共用目录会互相 remove）
    use std::sync::atomic::AtomicU32;
    static LOG_SEQ: AtomicU32 = AtomicU32::new(0);
    let log_dir = std::path::Path::new("/tmp/steer-home").join(format!(
        "run-{}-{}",
        std::process::id(),
        LOG_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&log_dir);
    std::fs::create_dir_all(&log_dir).unwrap();
    let log = Arc::new(SessionLog::new(
        &log_dir,
        Arc::new(AtomicU8::new(craft_agent::session_log::LEVEL_DEBUG)),
    ));
    let runner = Arc::new(Runner::new(RunnerDeps {
        db: Arc::clone(&db),
        records: recorder.clone(),
        registry,
        engine,
        approvals: Arc::clone(&approvals),
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
        subagent_config: Box::new(|| craft_agent::subagent::SubAgentConfig::new(true, 4, Vec::new(), Vec::new())),
        refresh_shell_env: Arc::new(|| {}),
        shell_tasks: None,
    }));

    let ws_id = craft_store::workspaces::upsert_workspace(
        &db.lock_conn(),
        "ws",
        workspace.to_str().unwrap(),
        "trusted",
        "",
    )
    .unwrap()
    .id;
    let thread_id = "th_steer_answer".to_string();
    craft_store::threads::create_thread(
        &db.lock_conn(),
        craft_store::threads::NewThread {
            id: &thread_id,
            mode: "code",
            title: "引导应答",
            title_auto: false,
            model_id: "stub/m1",
            reasoning_level: "",
            workspace_id: &ws_id,
            status: "idle",
            archived: false,
        },
    )
    .unwrap();

    Harness { _dir: dir, db, runner, recorder, thread_id, requests }
}

impl Harness {
    fn append_user_message(&self, text: &str) -> i64 {
        let message_id = craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE);
        self.recorder
            .append(&craft_store::events::EventRecord {
                thread_id: self.thread_id.clone(),
                message_id,
                run_id: None,
                kind: "user_message".into(),
                payload: json!({ "text": text, "attachments": [], "planMode": false, "via": null }),
            })
            .unwrap()
            .1
    }

    /// 发起运行并等到会话空闲（含接力 run 全部收尾）。
    /// 需要「运行中入队引导」时用 `harness_with_hooks` 在主循环第 N 次调用上挂钩子——
    /// 靠时间竞速不可靠：`try_steer` 在没有在途 run 时静默返回 `Ok(false)`，
    /// 而接力又要求引导在最后一次 `take_steers` 之后才进队列。
    fn run_and_wait(&self, trigger_seq: i64) {
        let runner = Arc::clone(&self.runner);
        let thread_id = self.thread_id.clone();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("测试运行时启动失败");
            rt.block_on(async move {
                tokio::task::spawn_blocking({
                    let runner = Arc::clone(&runner);
                    let thread_id = thread_id.clone();
                    move || runner.run(&thread_id, Some(trigger_seq))
                })
                .await
                .expect("发起运行失败")
                .expect("启动运行失败");

                // 等本次运行从取消表里消失（含接力 run 全部收尾）
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    if !runner.is_running(&thread_id) {
                        break;
                    }
                    if tokio::time::Instant::now() > deadline {
                        panic!("运行超时未收尾");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
        });
        handle.join().expect("运行线程 panic");
    }

    /// 主循环的模型请求（sidecar 小请求不带工具清单，以 `tools` 非空区分）。
    fn main_requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|req| !req.tools.is_empty())
            .cloned()
            .collect()
    }
}

/// 未应答的引导 → 下一轮开头注入纠偏，且同一批只注入一次。
#[test]
fn unanswered_steer_injects_nudge_once() {
    // 时序关键：钩子在模型 `stream()` 被调用时触发，此时**本轮的 take_steers 已经跑过**，
    // 所以入队的引导在**下一轮开头**才被并入。脚本按此排布：
    //   第 1 轮：钩子入队引导（本轮不并入）
    //   第 2 轮：并入引导，但零正文只调工具 → 记未应答
    //   第 3 轮：开头应注入纠偏，然后给出正文收尾
    let holder: Arc<Mutex<Option<Arc<Runner>>>> = Arc::new(Mutex::new(None));
    let h_holder = Arc::clone(&holder);
    let h = harness_with_hooks(
        vec![
            Round {
                text: "先看看有什么。".into(),
                tool_calls: vec![("call_list".into(), "fs_list".into(), json!({ "path": "." }))],
            },
            // 这一轮并入了引导，却一句话没说
            Round {
                text: String::new(),
                tool_calls: vec![("call_todo".into(), "todo_write".into(), json!({
                    "items": [{ "text": "先做点别的", "status": "done" }]
                }))],
            },
            Round { text: "收到，我改成先补测试再动手。".into(), tool_calls: vec![] },
        ],
        vec![(
            1,
            Arc::new(move || {
                let runner = h_holder.lock().unwrap().clone().expect("runner 已注册");
                let item = craft_agent::runner::SteerItem {
                    message_id: craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE),
                    text: "改成先补测试".into(),
                    attachments: Vec::new(),
                    trigger_seq: 0,
                };
                assert!(matches!(runner.try_steer("th_steer_answer", item), Ok(true)), "运行中入队应成功");
            }) as Arc<dyn Fn() + Send + Sync>,
        )],
    );
    *holder.lock().unwrap() = Some(Arc::clone(&h.runner));

    let seq = h.append_user_message("先跑一遍任务");
    h.run_and_wait(seq);

    let reqs = h.main_requests();
    assert!(reqs.len() >= 3, "至少三轮主循环请求，实际 {}", reqs.len());

    // 纠偏出现在「未应答那一轮的下一轮」请求里
    let third = reqs.get(2).expect("应有第三轮请求");
    assert!(
        third.messages.iter().any(|m| m.content.contains(STEER_UNANSWERED_NUDGE)),
        "未应答的下一次请求应带上纠偏"
    );

    // 只注入一次：最终历史里纠偏消息恰好一条（它会一直留在历史里，
    // 所以不能按「出现在几个请求里」判定）
    let nudge_count = reqs
        .last()
        .map(|r| {
            r.messages
                .iter()
                .filter(|m| m.content.contains(STEER_UNANSWERED_NUDGE))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(nudge_count, 1, "纠偏消息在最终历史里应恰好一条，实际 {nudge_count}");

    // 引导本身已并入历史（用户视角内容不丢）
    assert!(
        reqs.last().unwrap().messages.iter().any(|m| m.content.contains("改成先补测试")),
        "引导文本应并入历史"
    );
}

/// 运行中入队的引导在 run 收尾后接力为新 run，并留下投递回执。
#[test]
fn relay_run_consumes_queued_steer() {
    // 关键：接力只在「execute_run 返回后队列仍非空」时发生，而队列在每个迭代开头
    // 就被 take_steers 取空。所以引导必须在**最后一次 take_steers 之后**入队——
    // 即在最后一轮模型调用期间。脚本共 2 轮主循环，钩子挂在第 2 轮。
    let holder: Arc<Mutex<Option<Arc<Runner>>>> = Arc::new(Mutex::new(None));
    let h_holder = Arc::clone(&holder);
    let h = harness_with_hooks(
        vec![
            // 第 1 轮：说句话 + 调工具
            Round {
                text: "先看一眼目录。".into(),
                tool_calls: vec![("call_list".into(), "fs_list".into(), json!({ "path": "." }))],
            },
            // 第 2 轮：本轮结束后 execute_run 返回，队列里有引导 → 接力
            Round { text: "这一轮就到这里。".into(), tool_calls: vec![] },
            // 接力 run 的轮次
            Round { text: "收到补充要求，继续。".into(), tool_calls: vec![] },
        ],
        vec![(
            2,
            Arc::new(move || {
                let runner = h_holder.lock().unwrap().clone().expect("runner 已注册");
                let item = craft_agent::runner::SteerItem {
                    message_id: craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE),
                    text: "还要看 README".into(),
                    attachments: Vec::new(),
                    trigger_seq: 0,
                };
                assert!(matches!(runner.try_steer("th_steer_answer", item), Ok(true)), "运行中入队应成功");
            }) as Arc<dyn Fn() + Send + Sync>,
        )],
    );
    *holder.lock().unwrap() = Some(Arc::clone(&h.runner));

    let seq = h.append_user_message("看看这个项目");
    h.run_and_wait(seq);

    // 接力 run 存在：runs 表里该会话多于一条 run（首轮 + 接力）
    let run_count: i64 = h
        .db
        .with(|conn| {
            conn.query_row("SELECT COUNT(*) FROM runs WHERE thread_id = ?1", [&h.thread_id], |r| r.get(0))
                .map_err(Into::into)
        })
        .unwrap();
    assert!(run_count >= 2, "应有接力 run，实际 {run_count} 条");

    // 引导文本被接力 run 并入历史
    let reqs = h.main_requests();
    assert!(
        reqs.iter().any(|r| r.messages.iter().any(|m| m.content.contains("还要看 README"))),
        "接力 run 应把引导并入历史"
    );
}
