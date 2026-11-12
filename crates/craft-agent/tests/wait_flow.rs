//! wait 工具的运行时集成测试（决策 D21，docs/arch.md §7）。
//!
//! 为什么需要这一层：`craft-tools` 单测覆盖 spec 与钳制，但「睡满返回 / 事件提前唤醒 /
//! 用户跳过后 run 继续 / 停止留痕」只有 Runner 的 `run_wait_tool` 分支里才真实成立。
//! 本文件用可编程假模型客户端驱动完整循环，断言：
//! - 到时返回：status=done、wokeBy=timeout、耗时真实；
//! - 事件唤醒：后台命令完成把在途 wait 打断，通知正文仍走既有注入点；
//! - 用户跳过：wait 记 canceled，run 继续推进到下一轮；
//! - 会话停止：wait 落 canceled 留痕（对「取消即丢弃」通用语义的例外）；
//! - steer 唤醒：运行中引导把在途 wait 打断并注入下一轮历史。
//!
//! 与 `shell_background.rs` 的差别：这里的等待卡在 Runner 异步分支里，交互（跳过 / 停止 /
//! steer）由独立观察线程在「wait 进入在途」后注入。

use std::collections::VecDeque;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use futures_util::stream;
use serde_json::json;

use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_agent::runner::{Runner, RunnerDeps, SteerItem};
use craft_agent::session_log::SessionLog;
use craft_agent::subagent::SubAgentConfig;
use craft_core::bus::Bus;
use craft_model::catalog::Catalog;
use craft_model::factory::ModelFactory;
use craft_model::types::{Chunk, Client, Request, Usage};
use craft_policy::Engine;
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_tools::registry::Registry;
use craft_tools::shell_tasks::ShellTasks;

#[derive(Clone)]
struct Round {
    text: String,
    tool_calls: Vec<(String, String, serde_json::Value)>,
}

/// 假模型客户端：按调用次序消费脚本（没有子代理，只有一条队列）。
struct ScriptedClient {
    rounds: Arc<Mutex<VecDeque<Round>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Client for ScriptedClient {
    fn stream<'a>(
        &'a self,
        req: &'a Request,
    ) -> futures_core::stream::BoxStream<'a, Result<Chunk, craft_model::errors::ModelError>> {
        self.requests.lock().unwrap().push(req.clone());
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
                tool_call: Some(craft_model::types::ToolCall { id, name, args }),
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
        Box::pin(stream::iter(chunks))
    }

    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(ScriptedClient {
            rounds: Arc::clone(&self.rounds),
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
    recorder: Recorder,
    requests: Arc<Mutex<Vec<Request>>>,
    shell_tasks: Arc<ShellTasks>,
    thread_id: String,
}

fn register_stub_provider(db: &Arc<Db>) {
    let provider = json!({
        "id": "stub", "name": "Stub", "kind": "openai-compatible",
        "endpoint": "https://stub.invalid/v1", "keyRequired": false,
        "models": [
            {
                "id": "m1", "name": "脚本模型", "contextK": 128, "maxTokens": 4096,
                "caps": {"stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false},
                "status": "", "source": "manual", "enabled": true,
            },
        ],
    });
    db.with(|conn| craft_store::records::set_setting(conn, "model.custom_providers", &json!([provider])))
        .unwrap();
}

fn harness(tag: &str, rounds: Vec<Round>) -> Harness {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = dir.path().to_string_lossy().to_string();
    let db = Arc::new(Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    let recorder = Recorder::new(Arc::clone(&db), Arc::new(Bus::new(|_| {})));
    // shell_exec 的 R2 分类不参与本文件断言，统一 free 档免审批（与 shell_background.rs 同理）
    let engine = Arc::new(Mutex::new(Engine::new(craft_policy::Level::Free)));
    let grants = Arc::new(Grants::new(Arc::clone(&db)));
    let approvals = Arc::new(Approvals::new(Arc::clone(&db), recorder.clone(), Arc::clone(&grants)));
    let catalog = Arc::new(Catalog::new(Arc::clone(&db)));
    register_stub_provider(&db);
    let models = Arc::new(ModelFactory::new(Arc::clone(&catalog), Box::new(NoKeys)));
    catalog.reload();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let scripted = ScriptedClient { rounds: Arc::new(Mutex::new(rounds.into_iter().collect())), requests: Arc::clone(&requests) };
    models.set_client_override(Box::new(move |_| Some(scripted.clone_box())));

    // Registry 与 Runner 共用同一份后台任务注册表（事件唤醒路径依赖这一点）
    let shell_tasks = Arc::new(ShellTasks::new());
    let registry = Arc::new(Registry::new_with_shell_tasks("", &data_dir, Arc::clone(&db), Arc::clone(&shell_tasks)));
    registry.register(Box::new(craft_tools::shell::ShellExec));
    registry.register(Box::new(craft_tools::wait::Wait));

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
        subagent_config: Box::new(|| SubAgentConfig::new(false, 1, Vec::new(), Vec::new())),
        refresh_shell_env: Arc::new(|| {}),
        shell_tasks: Some(Arc::clone(&shell_tasks)),
    }));
    runner.wire_shell_task_wake();
    let thread_id = format!("th_wait_{tag}");
    let ws_path = dir.path().join("ws");
    std::fs::create_dir_all(&ws_path).unwrap();
    let ws_path_str = ws_path.to_string_lossy().to_string();
    let ws = craft_store::workspaces::upsert_workspace(&db.lock_conn(), "wait-ws", &ws_path_str, "trusted", "project").unwrap();
    craft_store::workspaces::replace_workspace_folders(&db, &ws.id, &[ws_path_str]).unwrap();
    craft_store::threads::create_thread(
        &db.lock_conn(),
        craft_store::threads::NewThread {
            id: &thread_id,
            mode: "code",
            title: "wait 测试",
            title_auto: false,
            model_id: "stub/m1",
            reasoning_level: "",
            workspace_id: &ws.id,
            status: "idle",
            archived: false,
        },
    )
    .unwrap();
    Harness { _dir: dir, db, runner, recorder, requests, shell_tasks, thread_id }
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
                payload: json!({ "text": text, "attachments": [], "planMode": false }),
            })
            .unwrap()
            .1
    }

    fn snapshot(&self) -> craft_core::contract::Snapshot {
        craft_store::events::snapshot(&self.db.lock_conn(), &self.thread_id, 0, 0).unwrap()
    }

    fn wait_parts(&self) -> Vec<craft_core::contract::PartSnapshot> {
        self.snapshot()
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter(|p| p.kind == "tool" && p.payload["name"] == "wait")
            .cloned()
            .collect()
    }

    /// 在独立多线程运行时里发起一次运行，并等「空闲 + 无通知 + 无在跑任务」稳定收敛
    /// （与 shell_background.rs 同一套收敛纪律）。
    fn run_and_settle(&self, user_text: &str, timeout_ms: u64) {
        let seq = self.append_user_message(user_text);
        let runner = Arc::clone(&self.runner);
        let shell_tasks = Arc::clone(&self.shell_tasks);
        let thread_id = self.thread_id.clone();
        let requests = Arc::clone(&self.requests);
        let db = Arc::clone(&self.db);
        let dbg_thread = self.thread_id.clone();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .expect("测试运行时启动失败");
            let outcome = rt.block_on(async move {
                {
                    let runner = Arc::clone(&runner);
                    let thread_id = thread_id.clone();
                    tokio::task::spawn_blocking(move || runner.run(&thread_id, Some(seq)))
                        .await
                        .expect("发起运行失败")
                        .expect("启动运行失败");
                }
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
                let stable = std::time::Duration::from_millis(500);
                let mut settled_at: Option<tokio::time::Instant> = None;
                loop {
                    let idle = !runner.is_running(&thread_id);
                    let drained = !shell_tasks.has_notifications(&thread_id);
                    let no_running_tasks = shell_tasks.running_of(&thread_id).is_empty();
                    if idle && drained && no_running_tasks {
                        let since = *settled_at.get_or_insert(tokio::time::Instant::now());
                        if tokio::time::Instant::now() - since >= stable {
                            return Ok(());
                        }
                    } else {
                        settled_at = None;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        let tools: Vec<_> = craft_store::events::snapshot(&db.lock_conn(), &dbg_thread, 0, 0)
                            .unwrap()
                            .messages
                            .iter()
                            .flat_map(|m| m.parts.iter())
                            .filter(|p| p.kind == "tool")
                            .map(|p| (p.payload["name"].as_str().unwrap_or("").to_string(), p.payload["status"].as_str().unwrap_or("").to_string(), p.payload["output"].as_str().unwrap_or("").chars().take(120).collect::<String>()))
                            .collect();
                        return Err(format!(
                            "收敛超时（空闲={idle} 通知已清空={drained} 任务已结束={no_running_tasks} \
                             在途wait={} 请求数={} 工具行={tools:?})",
                            runner.wait_in_flight(&thread_id),
                            requests.lock().unwrap().len(),
                        ));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            });
            drop(rt);
            outcome
        });
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => panic!("{err}"),
            Err(_) => panic!("运行线程 panic"),
        }
    }
}

/// 观察线程：等 wait 进入在途后执行一次动作（跳过 / 停止 / steer）。
fn spawn_wait_watcher(
    h: &Harness,
    action: impl FnOnce(&Runner, &str) + Send + 'static,
) -> std::thread::JoinHandle<()> {
    let runner = Arc::clone(&h.runner);
    let thread_id = h.thread_id.clone();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !runner.wait_in_flight(&thread_id) {
            if std::time::Instant::now() > deadline {
                panic!("wait 未进入在途（10s）");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        action(&runner, &thread_id);
    })
}

// 注意首词不能是 sleep：D21 起 shell_exec 前置拒绝 sleep 命令（is_sleep_command 只看首词）。
#[cfg(unix)]
const BG_WAKE_COMMAND: &str = "echo start >/dev/null; sleep 1; echo wait-wake-marker";
#[cfg(windows)]
const BG_WAKE_COMMAND: &str = "ping -n 3 127.0.0.1 >nul & echo wait-wake-marker";
#[cfg(not(any(unix, windows)))]
const BG_WAKE_COMMAND: &str = "echo wait-wake-marker";

/// 到时返回：done + wokeBy=timeout，耗时真实（不提前、不无限）。
#[test]
fn wait_sleeps_full_duration_returns_done() {
    let h = harness(
        "timeout",
        vec![
            Round {
                text: "先等 1 秒再继续。".into(),
                tool_calls: vec![(
                    "call_wait".into(),
                    "wait".into(),
                    json!({ "duration_ms": 1000, "purpose": "测试等待" }),
                )],
            },
            Round { text: "等完了，继续。".into(), tool_calls: vec![] },
        ],
    );
    h.run_and_settle("跑一个带等待的任务", 20_000);
    let parts = h.wait_parts();
    assert_eq!(parts.len(), 1, "应有一次 wait 调用：{}", parts.len());
    let payload = &parts[0].payload;
    assert_eq!(payload["status"], "done", "{}", payload);
    assert_eq!(payload["wokeBy"], "timeout", "{}", payload);
    assert_eq!(payload["title"], "等待");
    assert_eq!(payload["summary"], "测试等待", "摘要应取 purpose：{}", payload);
    let elapsed = payload["durationMs"].as_i64().unwrap_or_default();
    assert!(elapsed >= 900, "应睡满约 1s（实际 {elapsed}ms）：{}", payload);
    assert!(elapsed < 10_000, "不应超过请求时长太多（实际 {elapsed}ms）");
    // 结果正文引导继续，而不是鼓励再等
    assert!(payload["output"].as_str().unwrap().contains("继续你的下一步"));
}

/// 事件唤醒：后台命令完成把在途 wait 打断；通知正文仍走既有注入点（下一轮迭代开头）。
#[test]
fn wait_wakes_on_background_task_notification() {
    let h = harness(
        "bgwake",
        vec![
            Round {
                text: "后台跑命令，然后等它。".into(),
                tool_calls: vec![
                    (
                        "call_bg".into(),
                        "shell_exec".into(),
                        json!({ "command": BG_WAKE_COMMAND, "run_in_background": true }),
                    ),
                    ("call_wait".into(), "wait".into(), json!({ "duration_ms": 600_000 })),
                ],
            },
            Round { text: "后台命令完成，继续。".into(), tool_calls: vec![] },
        ],
    );
    h.run_and_settle("后台加等待", 20_000);
    let parts = h.wait_parts();
    assert_eq!(parts.len(), 1);
    let payload = &parts[0].payload;
    assert_eq!(payload["wokeBy"], "task_notification", "{}", payload);
    assert_eq!(payload["status"], "done", "{}", payload);
    assert!(payload["output"].as_str().unwrap().contains("后台命令任务"), "{}", payload);
    // 通知正文走既有注入点：后续请求里能看到 <task-notification>
    let requests = h.requests.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .any(|r| r.messages.iter().any(|m| m.content.contains("<task-notification>"))),
        "通知正文应在后续请求的历史里（唯一注入点）"
    );
}

/// 用户跳过：wait 记 canceled，run 继续推进到下一轮（不终止会话）。
#[test]
fn wait_user_skip_continues_run() {
    let h = harness(
        "skip",
        vec![
            Round {
                text: "等一个不会来的事件。".into(),
                tool_calls: vec![(
                    "call_wait".into(),
                    "wait".into(),
                    json!({ "duration_ms": 60_000, "purpose": "等一个不会来的事件" }),
                )],
            },
            Round { text: "已跳过等待，直接检查。".into(), tool_calls: vec![] },
        ],
    );
    let watcher = spawn_wait_watcher(&h, |runner, thread_id| {
        assert!(runner.cancel_wait(thread_id), "在途 wait 应可跳过");
    });
    h.run_and_settle("等然后跳过", 20_000);
    watcher.join().expect("观察线程 panic");
    let parts = h.wait_parts();
    assert_eq!(parts.len(), 1);
    let payload = &parts[0].payload;
    assert_eq!(payload["status"], "canceled", "{}", payload);
    assert_eq!(payload["wokeBy"], "user_cancel", "{}", payload);
    let output = payload["output"].as_str().unwrap_or_default();
    assert!(output.contains("等待被用户跳过"), "{}", output);
    assert!(output.contains("原目的：等一个不会来的事件"), "取消文案应带 purpose：{}", output);
    assert!(output.contains("请立即继续核实"), "{}", output);
    // run 继续推进：wait 返回后脚本第 2 轮被消费（同一条 assistant 消息继续追加正文，
    // 新请求至少再发一次）。round2 的正文已作为 text 部件落在 wait 行之后。
    let snap = h.snapshot();
    let parts: Vec<_> = snap
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .map(|p| (p.kind.as_str(), p.payload["text"].as_str().unwrap_or_default()))
        .collect();
    let wait_idx = parts.iter().position(|(k, _)| *k == "tool").expect("应有 wait 工具行");
    assert!(
        parts[wait_idx..].iter().any(|(k, t)| *k == "text" && t.contains("已跳过等待")),
        "跳过后应继续到下一轮正文：{parts:?}"
    );
    assert!(h.requests.lock().unwrap().len() >= 2, "跳过后应再发起一次模型请求");
}

/// 会话停止：在途 wait 落 canceled 留痕（对「取消即丢弃」通用语义的 D21 例外）。
#[test]
fn wait_run_stop_leaves_canceled_row() {
    let h = harness(
        "stop",
        vec![Round {
            text: "等一个长事件。".into(),
            tool_calls: vec![("call_wait".into(), "wait".into(), json!({ "duration_ms": 60_000 }))],
        }],
    );
    let watcher = spawn_wait_watcher(&h, |runner, thread_id| {
        assert!(runner.cancel(thread_id), "运行中的会话应可取消");
    });
    h.run_and_settle("等然后停止", 20_000);
    watcher.join().expect("观察线程 panic");
    let parts = h.wait_parts();
    assert_eq!(parts.len(), 1, "停止后 wait 行应留痕");
    let payload = &parts[0].payload;
    assert_eq!(payload["status"], "canceled", "{}", payload);
    assert_eq!(payload["wokeBy"], "run_canceled", "{}", payload);
    assert!(payload["output"].as_str().unwrap().contains("随会话停止"), "{}", payload);
}

/// steer 唤醒：运行中引导把在途 wait 打断，正文在下一轮迭代注入。
#[test]
fn wait_steer_wakes_wait() {
    let h = harness(
        "steer",
        vec![
            Round {
                text: "先等一段时间。".into(),
                tool_calls: vec![("call_wait".into(), "wait".into(), json!({ "duration_ms": 60_000 }))],
            },
            Round { text: "收到引导，直接继续。".into(), tool_calls: vec![] },
        ],
    );
    let watcher = spawn_wait_watcher(&h, |runner, thread_id| {
        let ok = runner
            .try_steer(
                thread_id,
                SteerItem {
                    message_id: "msg_steer_wait_test".into(),
                    text: "别等了，直接看结果".into(),
                    attachments: Vec::new(),
                    trigger_seq: 0,
                },
            )
            .expect("steer 入队失败");
        assert!(ok, "运行中的会话应接受引导");
    });
    h.run_and_settle("等然后引导", 20_000);
    watcher.join().expect("观察线程 panic");
    let parts = h.wait_parts();
    assert_eq!(parts.len(), 1);
    let payload = &parts[0].payload;
    assert_eq!(payload["wokeBy"], "steer", "{}", payload);
    assert_eq!(payload["status"], "done", "{}", payload);
    // 引导正文注入下一轮历史（唯一注入点）
    let requests = h.requests.lock().unwrap().clone();
    assert!(
        requests.iter().any(|r| r.messages.iter().any(|m| m.content.contains("别等了，直接看结果"))),
        "引导正文应在后续请求的历史里"
    );
}
