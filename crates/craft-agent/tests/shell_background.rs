//! 后台命令任务的运行时集成测试（`crates/craft-agent/src/shell_tasks.rs` 模块头）。
//!
//! 为什么需要这一层：`craft-tools` 的单测覆盖了「启动即返回」「读尾部」「整组杀」，
//! 但「后台任务完成后自动通知模型并唤醒会话」只有 Runner 里才真实成立。本文件用可编程
//! 的假模型客户端驱动完整循环，断言：
//! - 后台启动的工具结果里带任务号，且父 run 立刻结束（不阻塞等命令）；
//! - 命令结束后 Runner 起唤醒 run，通知正文进了父会话历史；
//! - 通知正文只给路径与状态，不灌 stdout 全文。
//!
//! 与 `subagent_flow.rs` 的差别：那条链路的并发来自子代理，这里来自 shell 等待线程；
//! 断言同样靠「父会话空闲 + 无待投递通知」收敛。
//!
//! 平台说明：后台命令必须比父 run 活得更久，本测试才能稳定走到「会话空闲后唤醒」路径。
//! 因此用平台特定的延迟命令（Unix `sleep 1; echo ...`，Windows `ping -n 3 ... & echo ...`）；
//! 若命令瞬间结束，通知会被父 run 的下一次迭代消费——这是合法产品行为，但本测试观察的是
//! 唤醒 run，必须让两条路径确定分离。测试引擎设为 free，避免延迟命令的 R2 分类触发审批。

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
    // 延迟命令通常是 R2（sleep / ping），本测试只验证后台任务生命周期，不验证审批。
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

    // 关键：Registry 与 Runner 共用同一份后台任务注册表，否则工具写入的任务
    // Runner 永远看不到，也就没有完成通知（这正是生产装配必须走 new_with_shell_tasks 的原因）。
    let shell_tasks = Arc::new(ShellTasks::new());
    let registry = Arc::new(Registry::new_with_shell_tasks(&data_dir, Arc::clone(&db), Arc::clone(&shell_tasks)));
    registry.register(Box::new(craft_tools::shell::ShellExec));
    registry.register(Box::new(craft_tools::shell::BashOutput));
    registry.register(Box::new(craft_tools::shell::KillShell));

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
    // 后台任务唤醒回调必须在 Arc 之后装配（回调要持有 Runner 自己）
    runner.wire_shell_task_wake();
    let thread_id = format!("th_bg_{tag}");
    // 绑定真实工作区：shell_exec 的 cwd 从工作区解析，空工作区会在评估期判越界
    let ws_path = dir.path().join("ws");
    std::fs::create_dir_all(&ws_path).unwrap();
    let ws_path_str = ws_path.to_string_lossy().to_string();
    let ws = craft_store::workspaces::upsert_workspace(&db.lock_conn(), "bg-ws", &ws_path_str, "trusted", "project").unwrap();
    craft_store::workspaces::replace_workspace_folders(&db, &ws.id, &[ws_path_str]).unwrap();
    craft_store::threads::create_thread(
        &db.lock_conn(),
        craft_store::threads::NewThread {
            id: &thread_id,
            mode: "code",
            title: "后台任务测试",
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

    /// 发起一次运行，等到「会话空闲 + 无待投递后台通知」且该状态稳定保持一小段。
    ///
    /// 为什么多线程运行时：后台任务的等待线程是 std::thread，但唤醒 run 要跑在 tokio 上；
    /// 单线程 runtime 在父 run 结束时会被销毁，唤醒 run 就永远起不来（测试基建问题）。
    ///
    /// 为什么要稳定窗口：唤醒链路存在「通知已被消费（drained=true）→ 唤醒 run 尚未把
    /// 会话标记为 running」的窗口（spawn 排队 + start_run 的前置数据库操作），CI 慢机器
    /// 上这个窗口可达几十毫秒，恰好采样到「空闲且已清空」就会过早收敛、漏掉唤醒 run。
    /// 因此要求收敛状态连续稳定保持 500ms 才返回；期间被打破（唤醒 run 真的起来了）
    /// 就清零重等。唤醒 run 一旦运行至少跨多个采样间隔，不会被窗口漏掉。
    fn run_until_settled(&self, timeout_ms: u64) {
        let seq = self.append_user_message("跑一个后台命令");
        let runner = Arc::clone(&self.runner);
        let shell_tasks = Arc::clone(&self.shell_tasks);
        let thread_id = self.thread_id.clone();
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
                    // 延迟命令还在跑时不算收敛：否则会在后台任务完成前提前返回，
                    // 后续断言看不到通知与唤醒 run（本文件用延迟命令稳定复现唤醒路径）。
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
                        return Err(format!(
                            "收敛超时（空闲={idle} 通知已清空={drained} 后台任务已结束={no_running_tasks}）"
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

/// 后台命令启动即返回，命令结束后通知进历史并触发唤醒 run。
/// 首词不能是 sleep：D21 起 shell_exec 前置拒绝 sleep 命令（is_sleep_command 只看首词）。
#[cfg(unix)]
const BG_WAKE_COMMAND: &str = "echo bg-start >/dev/null; sleep 1; echo bg-done-marker";
#[cfg(windows)]
const BG_WAKE_COMMAND: &str = "ping -n 3 127.0.0.1 >nul & echo bg-done-marker";
#[cfg(not(any(unix, windows)))]
const BG_WAKE_COMMAND: &str = "echo bg-done-marker";

#[test]
fn background_shell_task_notifies_and_wakes_parent() {
    let h = harness(
        "notify",
        vec![
            Round {
                text: "我在后台等这个命令。".into(),
                tool_calls: vec![(
                    "call_bg".into(),
                    "shell_exec".into(),
                    json!({ "command": BG_WAKE_COMMAND, "run_in_background": true }),
                )],
            },
            // 第一轮之后的收尾轮（后台任务的回执已返回，父 run 正常结束）
            Round { text: "已交给后台，先去忙别的。".into(), tool_calls: vec![] },
            // 唤醒 run：读到通知后汇报
            Round { text: "后台命令已完成。".into(), tool_calls: vec![] },
        ],
    );
    h.run_until_settled(20_000);

    let snap = craft_store::events::snapshot(&h.db.lock_conn(), &h.thread_id, 0, 0).unwrap();
    // 工具行带任务号（前端据此显示「后台运行」与读取输出入口）
    let tool_parts: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "tool").collect();
    assert_eq!(tool_parts.len(), 1, "应只有一次 shell_exec 调用");
    let task_id = tool_parts[0].payload["backgroundTaskId"].as_str().unwrap_or_default().to_string();
    assert!(task_id.starts_with("bash_"), "工具行应带后台任务号：{:?}", tool_parts[0].payload);

    // 通知正文进了历史：唤醒 run 的请求里能看到任务号与输出路径
    let requests = h.requests.lock().unwrap().clone();
    let saw_notification = requests.iter().any(|r| {
        r.messages
            .iter()
            .any(|m| m.content.contains("<task-notification>") && m.content.contains(&task_id))
    });
    assert!(saw_notification, "唤醒 run 的请求里应包含完成通知（任务 {task_id}）");

    // 通知只给路径与状态，不灌命令 stdout 全文。
    // 注意别用「命令原文不出现」这种断言：command 段本来就含命令字符串。
    // 真正的判据是「输出文件里的内容行没有被内联」——命令 echo 出的那行只在文件里。
    let notification_text = requests
        .iter()
        .flat_map(|r| r.messages.iter())
        .find(|m| m.content.contains("<task-notification>"))
        .map(|m| m.content.clone())
        .expect("应有通知正文");
    assert!(notification_text.contains("<output-file>"), "应给出输出文件路径：{notification_text}");
    let output_path = h
        .shell_tasks
        .get(&h.thread_id, &task_id)
        .map(|t| t.output_path)
        .expect("任务仍在注册表里");
    let file_text = std::fs::read_to_string(&output_path).unwrap_or_default();
    assert_eq!(file_text.trim(), "bg-done-marker", "输出确实写进了文件");
    // 通知正文长度远小于「命令 + 全量输出」拼接，且不含输出行
    assert!(
        !notification_text.lines().any(|line| line.trim() == "bg-done-marker"),
        "通知不应内联 stdout 内容行：{notification_text}"
    );

    // 唤醒 run 确实跑起来了：由它发起的新请求里带着通知正文（上面已断言通知进了历史），
    // 且它自己的模型调用产生了 assistant 正文（脚本第 2/3 轮被旁路请求消耗，正文内容不固定，
    // 因此只断言"多了一条 assistant 消息"这个事实，不锁具体文案）。
    //
    // 为什么轮询等待而不是收敛后立即断言：唤醒 run 的产物（模型轮 + assistant 落库）
    // 相对「会话空闲」有固有滞后（spawn 排队 + 模型轮 + 落库），CI 慢机器上收敛判定
    // 可能先于落库完成。超时仍未出现时把链路各环节的状态一并打出，定位丢失路径。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let assistant_count = loop {
        let snap = craft_store::events::snapshot(&h.db.lock_conn(), &h.thread_id, 0, 0).unwrap();
        let count = snap.messages.iter().filter(|m| m.role == "assistant").count();
        if count >= 2 {
            break count;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "唤醒 run 未产生第二条 assistant 消息（10s）：assistant={count} \
                 请求数={} 通知队列={} 任务状态={:?} 消息数={}",
                h.requests.lock().unwrap().len(),
                h.shell_tasks.has_notifications(&h.thread_id),
                h.shell_tasks.get(&h.thread_id, &task_id).map(|t| t.state),
                snap.messages.len(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    // 循环已保证 assistant_count >= 2；请求断言补全链路的另一半
    assert!(assistant_count >= 2, "唤醒 run 的 assistant 消息应已落库");
    assert!(!h.requests.lock().unwrap().is_empty(), "唤醒 run 应至少发起一次模型请求");
}
