//! 子代理体系的运行时集成测试（决策 D15）。
//!
//! 为什么需要这一层：Store 测试覆盖了子会话与快照合并，craft-agent 单测覆盖了注册表与
//! 文案，但「父 run 派发 → 子 run 用不同工具面跑完 → 结论回到父历史」这条链路只有在
//! Runner 里才真实成立。本文件用可编程的假模型客户端驱动完整循环，断言：
//! - 前台派发：工具结果带回子代理结论，父会话里出现子代理卡片；
//! - 后台派发：立即返回启动回执，终态后报告自动注入父会话并触发一次唤醒 run；
//! - 工具面隔离：子代理的工具列表里没有 agent_* 与计划协议工具；
//! - 只读约束：explore 子代理的工具面没有写工具（不可见即不可调用）；
//! - 取消级联：父会话取消会连带取消在跑的子代理。

use std::collections::VecDeque;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use futures_util::stream;
use serde_json::json;

use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_agent::runner::{Runner, RunnerDeps};
use craft_agent::session_log::SessionLog;
use craft_agent::subagent::{AgentStatus, SubAgentConfig};
use craft_core::bus::Bus;
use craft_core::contract::Snapshot;
use craft_model::catalog::Catalog;
use craft_model::factory::ModelFactory;
use craft_model::types::{Chunk, Client, Request, Usage};
use craft_policy::Engine;
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_tools::registry::Registry;

/// 假模型客户端：按「请求特征 → 脚本队列」匹配产出。
///
/// 为什么不能只用一个全局队列：前台派发会让父 run 阻塞等子 run，子 run 的请求会
/// 穿插入父 run 的请求序列，全局顺序不可预测。因此脚本按 `Route` 分组，每个请求
/// 先按角色（父 / 子）与轮次判定走哪条队列。
struct ScriptedClient {
    /// 父会话脚本（按调用次序消费）
    parent: Arc<Mutex<VecDeque<Round>>>,
    /// 子代理脚本（按调用次序消费）
    child: Arc<Mutex<VecDeque<Round>>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

#[derive(Clone)]
struct Round {
    text: String,
    tool_calls: Vec<(String, String, serde_json::Value)>,
}

impl ScriptedClient {
    /// 父 / 子两套脚本：`parent` 用于主会话 run，`child` 用于子代理 run。
    fn new(parent: Vec<Round>, child: Vec<Round>, requests: Arc<Mutex<Vec<Request>>>) -> Self {
        Self {
            parent: Arc::new(Mutex::new(parent.into_iter().collect())),
            child: Arc::new(Mutex::new(child.into_iter().collect())),
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
        // 子代理的身份段由 craft-agent 的 prompt 写成「你是 CraftWork 的子代理」
        let is_child = req.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false);
        let queue = if is_child { &self.child } else { &self.parent };
        let round = queue.lock().unwrap().pop_front().unwrap_or(Round {
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
            parent: Arc::clone(&self.parent),
            child: Arc::clone(&self.child),
            requests: Arc::clone(&self.requests),
        })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    db: Arc<Db>,
    runner: Arc<Runner>,
    recorder: Recorder,
    requests: Arc<Mutex<Vec<Request>>>,
    thread_id: String,
}

/// 装配一个可跑的 Runner：隔离数据目录、假模型、完整内置工具集。
fn harness(_tag: &str, parent: Vec<Round>, child: Vec<Round>) -> Harness {
    harness_with_config(_tag, parent, child, SubAgentConfig::new(true, 4, Vec::new(), Vec::new()))
}

/// 带子代理配置的装配：用于验证自定义定义 / 内置默认值在派发时真正生效。
fn harness_with_config(
    _tag: &str,
    parent: Vec<Round>,
    child: Vec<Round>,
    subagent_config: SubAgentConfig,
) -> Harness {
    let dir = tempfile::TempDir::new().unwrap();
    let data_dir = dir.path().to_string_lossy().to_string();
    let db = Arc::new(Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    let recorder = Recorder::new(Arc::clone(&db), Arc::new(Bus::new(|_| {})));
    let engine = Arc::new(Mutex::new(Engine::default()));
    let grants = Arc::new(Grants::new(Arc::clone(&db)));
    let approvals = Arc::new(Approvals::new(Arc::clone(&db), recorder.clone(), Arc::clone(&grants)));
    let catalog = Arc::new(Catalog::new(Arc::clone(&db)));
    // 注册一个真实提供商：open_session 先查目录再构造客户端，若模型不在目录里会以
    // provider_unavailable 直接失败（client_override 在构造阶段才生效，帮不上忙）。
    register_stub_provider(&db);
    let models = Arc::new(ModelFactory::new(Arc::clone(&catalog), Box::new(NoKeys)));
    // Catalog 在 new() 时对 settings 做快照，写库之后必须 reload 才能解析到 stub/m1
    catalog.reload();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let scripted = ScriptedClient::new(parent, child, Arc::clone(&requests));
    models.set_client_override(Box::new(move |_| Some(scripted.clone_box())));
    let registry = Arc::new(Registry::new("", &data_dir, Arc::clone(&db)));
    register_tools(&registry);
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
        subagent_config: Box::new(move || subagent_config.clone()),
        refresh_shell_env: Arc::new(|| {}),
        shell_tasks: None,
    }));
    let thread_id = "th_parent_test".to_string();
    craft_store::threads::create_thread(
        &db.lock_conn(),
        craft_store::threads::NewThread {
            id: &thread_id,
            mode: "code",
            title: "测试会话",
            title_auto: false,
            model_id: "stub/m1",
            reasoning_level: "",
            workspace_id: "",
            status: "idle",
            archived: false,
        },
    )
    .unwrap();
    Harness { _dir: dir, db, runner, recorder, requests, thread_id }
}

struct NoKeys;
impl craft_model::factory::KeyProvider for NoKeys {
    fn get_for(&self, _provider_id: &str, _env_names: &[String]) -> String {
        String::new()
    }
}

/// 往 settings 写一个 stub 提供商（id=stub，模型 m1）：让 `open_session` 的目录解析通过，
/// 真正发起请求时由 `client_override` 换成脚本化客户端，不触网。
fn register_stub_provider(db: &Arc<Db>) {
    let provider = json!({
        "id": "stub", "name": "Stub", "kind": "openai-compatible",
        "endpoint": "https://stub.invalid/v1", "keyRequired": false,
        "models": [
            {
                "id": "m1", "name": "脚本模型", "contextK": 128, "maxTokens": 4096,
                "caps": { "stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false },
                "status": "", "source": "manual", "enabled": true,
            },
            // 第二个模型：验证「自定义子代理使用自己的模型」时能观察到切换
            {
                "id": "m2", "name": "评审模型", "contextK": 64, "maxTokens": 2048,
                "caps": { "stream": true, "tools": true, "vision": false, "reasoning": false, "structured": false },
                "status": "", "source": "manual", "enabled": true,
            },
        ],
    });
    db.with(|conn| {
        craft_store::records::set_setting(conn, "model.custom_providers", &json!([provider]))
    })
    .unwrap();
}

fn register_tools(registry: &Registry) {
    registry.register(Box::new(craft_tools::fs::FsRead));
    registry.register(Box::new(craft_tools::fs::FsWrite));
    registry.register(Box::new(craft_tools::fs::FsGrep));
    registry.register(Box::new(craft_tools::shell::ShellExec));
    registry.register(Box::new(craft_tools::todo::TodoWrite));
    registry.register(Box::new(craft_tools::plan::EnterPlan));
    registry.register(Box::new(craft_tools::plan::ExitPlan));
    registry.register(Box::new(craft_tools::agent::AgentSpawn));
    registry.register(Box::new(craft_tools::agent::AgentWait));
    registry.register(Box::new(craft_tools::agent::AgentList));
    registry.register(Box::new(craft_tools::agent::AgentClose));
}

impl Harness {
    /// 在独立的多线程运行时里发起一次运行，并等到「父会话空闲 + 全部子代理终态」。
    ///
    /// 为什么必须多线程：后台子代理与父 run 并发推进，父 run 的收尾还会触发唤醒 run；
    /// 单线程 runtime 下父 run 退出会连带销毁运行时，子代理就永远停在 running
    /// （测试基础设施问题，不是产品行为）。
    fn run_and_wait(&self, timeout_ms: u64) -> Snapshot {
        let seq = self.append_user_message("派发一个子代理");
        self.run_until_settled(seq, timeout_ms);
        self.snapshot(&self.thread_id)
    }

    /// 发起一次运行并等到全部活动停止（父会话空闲、子代理终态、无待投递报告）。
    fn run_until_settled(&self, trigger_seq: i64, timeout_ms: u64) {
        let runner = Arc::clone(&self.runner);
        let thread_id = self.thread_id.clone();
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .expect("测试运行时启动失败");
            let outcome = rt.block_on(async move {
                // run 必须在运行时上下文里发起：start_run 走 Handle::try_current 分支，
                // 由该运行时驱动全部子任务（含后台子代理与唤醒 run）。
                // run 必须在运行时上下文里发起：start_run 走 Handle::try_current 分支，
                // 由该运行时驱动全部子任务（含后台子代理与唤醒 run）。
                {
                    let runner = Arc::clone(&runner);
                    let thread_id = thread_id.clone();
                    tokio::task::spawn_blocking(move || runner.run(&thread_id, Some(trigger_seq)))
                        .await
                        .expect("发起运行失败")
                        .expect("启动运行失败");
                }
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
                // 与 shell_background::run_until_settled 同理：收敛条件需稳定保持 500ms。
                // 唤醒链路存在「报告已被消费、唤醒 run 尚未把会话标记为 running」的窗口
                // （spawn 排队 + start_run 的前置数据库操作），CI 慢机器上恰好采样到
                // 全空闲就过早收敛、漏掉唤醒 run。期间被打破就清零重等。
                let stable = std::time::Duration::from_millis(500);
                let mut settled_at: Option<tokio::time::Instant> = None;
                loop {
                    let parent_idle = !runner.is_running(&thread_id);
                    let children_settled = runner
                        .subagents()
                        .list(&thread_id)
                        .iter()
                        .all(|s| s.status.is_terminal());
                    let no_pending = !runner.subagents().has_reports(&thread_id);
                    if parent_idle && children_settled && no_pending {
                        let since = *settled_at.get_or_insert(tokio::time::Instant::now());
                        if tokio::time::Instant::now() - since >= stable {
                            return Ok(());
                        }
                    } else {
                        settled_at = None;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(format!(
                            "收敛超时（父空闲={parent_idle} 子终态={children_settled} 报告={:?}）",
                            runner.subagents().list(&thread_id).iter().map(|s| s.status).collect::<Vec<_>>()
                        ));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            });
            // 运行时在收敛后立刻销毁：先 drop 运行时的空转线程
            drop(rt);
            outcome
        });
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => panic!("{err}"),
            Err(_) => panic!("运行线程 panic"),
        }
    }

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

    fn snapshot(&self, thread_id: &str) -> Snapshot {
        craft_store::events::snapshot(&self.db.lock_conn(), thread_id, 0, 0).unwrap()
    }
}

/// 前台派发（显式 run_in_background=false，D31 后前台是显式选择）：
/// 工具结果里带回子代理结论，父会话出现一张 completed 的子代理卡。
#[test]
fn foreground_spawn_returns_child_result_and_renders_card() {
    let h = harness(
        "subagent-foreground",
        vec![
            Round {
                text: "我先派一个探查子代理。".into(),
                tool_calls: vec![(
                    "call_spawn".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "调研入口",
                        "prompt": "读一下项目的入口在哪里",
                        "agent_type": "explore",
                        "name": "explorer",
                        "run_in_background": false,
                    }),
                )],
            },
            Round { text: "子代理确认入口在 src/main.tsx。".into(), tool_calls: vec![] },
        ],
        vec![Round { text: "入口在 src/main.tsx。".into(), tool_calls: vec![] }],
    );
    let snap = h.run_and_wait(20_000);

    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert_eq!(cards.len(), 1, "前台派发应有一张子代理卡");
    assert_eq!(cards[0].payload["status"], "completed");
    assert_eq!(cards[0].payload["name"], "explorer");
    assert_eq!(cards[0].payload["agentType"], "explore");
    assert_eq!(cards[0].payload["notify"], false, "前台结果走工具结果，不重复回报");
    assert!(
        cards[0].payload["result"].as_str().unwrap_or_default().contains("src/main.tsx"),
        "卡片应带结论正文：{:?}",
        cards[0].payload["result"]
    );

    let child_id = cards[0].payload["agentId"].as_str().unwrap().to_string();
    let child = h.snapshot(&child_id);
    assert!(child.messages.iter().any(|m| m.role == "user"), "子会话应有派发正文");
    assert!(child.messages.iter().any(|m| m.role == "assistant"), "子会话应有子代理产出");

    // 父 run 的第二轮请求里能看到子代理结论（工具结果进历史）
    let requests = h.requests.lock().unwrap().clone();
    let parent_requests: Vec<&Request> = requests
        .iter()
        .filter(|r| !r.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false))
        .collect();
    assert!(parent_requests.len() >= 2, "父 run 至少两轮请求（sidecar 小请求也会计入），实际 {}", parent_requests.len());
    let tool_texts: Vec<&str> = parent_requests[1]
        .messages
        .iter()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.as_str())
        .collect();
    assert!(
        tool_texts.iter().any(|t| t.contains("src/main.tsx") && t.contains("subagent_name")),
        "工具结果应带回子代理结论与 usage 尾注：{tool_texts:?}"
    );
}

/// 子代理工具面隔离：子 run 的请求里没有 agent_* 与计划协议工具，explore 也没有写工具。
#[test]
fn child_tool_surface_excludes_recursion_plan_and_writes() {
    let h = harness(
        "subagent-tools",
        vec![
            Round {
                text: "派一个只读子代理。".into(),
                tool_calls: vec![(
                    "call_spawn".into(),
                    "agent_spawn".into(),
                    json!({ "description": "只读探查", "prompt": "看看代码结构", "agent_type": "explore" }),
                )],
            },
            Round { text: "已经拿到结构。".into(), tool_calls: vec![] },
        ],
        vec![Round { text: "结构如下……".into(), tool_calls: vec![] }],
    );
    h.run_and_wait(20_000);
    let requests = h.requests.lock().unwrap().clone();
    let child_req = requests
        .iter()
        .find(|r| r.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false))
        .expect("应有一次子代理请求");
    let names: Vec<&str> = child_req.tools.iter().map(|t| t.name.as_str()).collect();
    for banned in [
        "agent_spawn",
        "agent_wait",
        "agent_list",
        "agent_close",
        "enter_plan",
        "exit_plan",
        "fs_write",
        "shell_exec",
    ] {
        assert!(!names.contains(&banned), "子代理工具面不应包含 {banned}：{names:?}");
    }
    assert!(names.contains(&"fs_read"), "explore 应有只读工具：{names:?}");
    assert!(child_req.system[0].content.contains("子代理"));
    let dynamic = &child_req.system[2].content;
    assert!(dynamic.contains("# 你的任务"), "子代理提示词应包含任务段");
    assert!(dynamic.contains("子代理契约"), "子代理提示词应包含契约段");
    assert!(!dynamic.contains("计划模式"), "子代理不注入计划段");

    let parent_req = requests
        .iter()
        .find(|r| !r.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false))
        .expect("应有一次父会话请求");
    let parent_names: Vec<&str> = parent_req.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(parent_names.contains(&"agent_spawn"));
    assert!(parent_names.contains(&"fs_write"));
    assert!(parent_names.contains(&"enter_plan"), "主会话保留计划协议工具");
}

/// agent_wait 没有可等待目标时必须是可读错误，而不是静默成功。
/// 背景：模型把后台任务误当成子代理的等待对象时会连续调用 agent_wait，旧行为返回成功，
/// 界面只留下「等待子代理 × N / 完成」，用户误以为后台任务在等子代理
/// （术语边界与失败路径见 docs/subagent/spec.md §4.1 / §9）。
#[test]
fn wait_without_subagents_is_error_with_guidance() {
    let h = harness(
        "subagent-wait-empty",
        vec![
            Round {
                text: "先看看有没有可等待的子代理。".into(),
                tool_calls: vec![
                    ("call_wait_empty".into(), "agent_wait".into(), json!({})),
                    ("call_wait_ghost".into(), "agent_wait".into(), json!({ "names": ["ghost"] })),
                ],
            },
            Round { text: "没有可等待的子代理，结束本轮。".into(), tool_calls: vec![] },
        ],
        vec![],
    );
    let snap = h.run_and_wait(20_000);
    let waits: Vec<_> = snap
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter(|p| p.kind == "tool" && p.payload["name"] == "agent_wait")
        .collect();
    assert_eq!(waits.len(), 2, "应有两次 agent_wait 工具部件：{}", waits.len());
    for part in &waits {
        assert_eq!(part.payload["status"], "error", "空目标必须是错误：{}", part.payload);
        assert_eq!(part.payload["isError"], true, "空目标必须带 isError：{}", part.payload);
    }
    let output_of = |id: &str| -> String {
        waits
            .iter()
            .find(|p| p.payload["toolCallId"] == id)
            .unwrap_or_else(|| panic!("找不到 {id} 的工具部件"))
            .payload["output"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    // 未指定 names：说明没有在跑的目标 + 后台任务不需要等待
    let empty = output_of("call_wait_empty");
    assert!(empty.contains("没有在跑的子代理"), "空等待应说明没有在跑的目标：{empty}");
    assert!(empty.contains("只等待 agent_spawn 派出且尚未结束的子代理"), "应点名本工具的等待对象：{empty}");
    assert!(empty.contains("自动通知"), "应说明后台任务不需要等待：{empty}");
    assert!(empty.contains("bash_output"), "应给出读输出的替代路径：{empty}");
    // 指定了不存在的 names：点名找不到的对象
    let ghost = output_of("call_wait_ghost");
    assert!(ghost.contains("找不到子代理：ghost"), "未知名应点名：{ghost}");
    assert!(ghost.contains("不等待后台任务"), "应划清与后台任务的边界：{ghost}");
}

/// agent_wait 省略 names 时只等仍在跑的条目，不被本会话历史终态「秒回成功」。
/// 背景：登记列表含终态条目（finish 只 update 不 remove），不过滤会让「任一终态即返回」
/// 被历史终态立即满足——只要派发过任何一个已完成的子代理，不写 names 的 wait 就永远
/// 等了个寂寞还回成功。显式 names 保留含终态查找（查询状态仍有用），全部已终态时注明
/// 「已终结，无需等待」。
#[test]
fn wait_without_names_ignores_terminal_agents() {
    let h = harness(
        "subagent-wait-terminal",
        vec![
            // 第 1 轮：前台派发 explorer（D31 后前台是显式选择），阻塞至终态、结论回带
            Round {
                text: "先派一个前台子代理。".into(),
                tool_calls: vec![(
                    "call_spawn_fg".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "前台调研",
                        "prompt": "调研入口",
                        "agent_type": "explore",
                        "name": "explorer",
                        "run_in_background": false,
                    }),
                )],
            },
            // 第 2 轮：不写 names 的 wait——此时没有在跑的子代理，必须是可读错误
            Round {
                text: "再试试空等。".into(),
                tool_calls: vec![("call_wait_no_names".into(), "agent_wait".into(), json!({}))],
            },
            // 第 3 轮：显式点名已终结的 explorer——立即返回状态并注明无需等待
            Round {
                text: "查一下它的状态。".into(),
                tool_calls: vec![(
                    "call_wait_named_done".into(),
                    "agent_wait".into(),
                    json!({ "names": ["explorer"] }),
                )],
            },
            // 第 4 轮：后台派 digger（保持 running 窗口）
            Round {
                text: "再后台派一个。".into(),
                tool_calls: vec![(
                    "call_spawn_bg".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "后台统计",
                        "prompt": "统计文件",
                        "agent_type": "explore",
                        "name": "digger",
                        "run_in_background": true,
                    }),
                )],
            },
            // 第 5 轮：不写 names 的 wait——此时 digger 在跑，应等到它终态才返回
            Round {
                text: "等它跑完。".into(),
                tool_calls: vec![("call_wait_running".into(), "agent_wait".into(), json!({}))],
            },
            Round { text: "都结束了。".into(), tool_calls: vec![] },
        ],
        // explorer 消费第 1 轮，digger 消费第 2 轮
        vec![
            Round { text: "入口在 src/main.tsx。".into(), tool_calls: vec![] },
            Round { text: "共 42 个文件。".into(), tool_calls: vec![] },
        ],
    );
    let snap = h.run_and_wait(20_000);
    let waits: Vec<_> = snap
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter(|p| p.kind == "tool" && p.payload["name"] == "agent_wait")
        .collect();
    assert_eq!(waits.len(), 3, "应有三次 agent_wait 工具部件：{}", waits.len());
    let output_of = |id: &str| -> String {
        waits
            .iter()
            .find(|p| p.payload["toolCallId"] == id)
            .unwrap_or_else(|| panic!("找不到 {id} 的工具部件"))
            .payload["output"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    let status_of = |id: &str| -> String {
        waits
            .iter()
            .find(|p| p.payload["toolCallId"] == id)
            .unwrap()
            .payload["status"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };

    // 1) 无在跑目标：可读错误，不是假成功
    assert_eq!(status_of("call_wait_no_names"), "error", "无在跑目标必须是错误");
    let no_names = output_of("call_wait_no_names");
    assert!(no_names.contains("没有在跑的子代理"), "应说明没有在跑目标：{no_names}");

    // 2) 显式 names 查已终结子代理：立即成功返回状态，注明无需等待
    assert_eq!(status_of("call_wait_named_done"), "done");
    let named_done = output_of("call_wait_named_done");
    assert!(named_done.contains("explorer"), "应包含目标名：{named_done}");
    assert!(named_done.contains("已完成"), "应返回终态状态：{named_done}");
    assert!(named_done.contains("已终结，无需等待"), "全部终态应注明无需等待：{named_done}");

    // 3) 有在跑目标：等到终态才返回（不是被历史终态秒回），结果含 digger 的终态
    assert_eq!(status_of("call_wait_running"), "done");
    let wait_running = output_of("call_wait_running");
    assert!(wait_running.contains("digger"), "等待结果应包含 digger：{wait_running}");
    assert!(wait_running.contains("已完成"), "digger 应已终态：{wait_running}");
    assert!(!wait_running.contains("等待超时"), "不应是超时返回：{wait_running}");
}

/// 后台派发：立即返回启动回执（工具结果不含结论），终态后自动唤醒父会话并注入报告。
#[test]
fn background_spawn_wakes_parent_with_report() {
    let h = harness(
        "subagent-background",
        vec![
            Round {
                text: "后台派一个。".into(),
                tool_calls: vec![(
                    "call_bg".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "后台统计",
                        "prompt": "统计文件数量",
                        "agent_type": "explore",
                        "run_in_background": true,
                    }),
                )],
            },
            Round { text: "已交给后台子代理，我先做别的。".into(), tool_calls: vec![] },
            // 唤醒 run：读到报告后收尾
            Round { text: "后台子代理报告：共 42 个文件。".into(), tool_calls: vec![] },
        ],
        vec![Round { text: "共 42 个文件。".into(), tool_calls: vec![] }],
    );
    let snap = h.run_and_wait(20_000);
    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert_eq!(cards.len(), 1, "后台派发应有子代理卡");
    assert_eq!(cards[0].payload["notify"], true, "后台派发终态需要回报");
    // 后台回执不含结论（工具结果只说明已启动）
    let tool_output = snap
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .find(|p| p.kind == "tool")
        .and_then(|p| p.payload["output"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(tool_output.contains("后台启动"), "回执应说明已后台启动：{tool_output}");
    assert!(!tool_output.contains("42 个文件"), "后台回执不应提前给出结论：{tool_output}");

    // 等后台子代理终态 + 唤醒 run 收敛
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20_000);
    loop {
        let list = h.runner.subagents().list(&h.thread_id);
        if list.iter().all(|s| s.status.is_terminal()) && !h.runner.is_running(&h.thread_id) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "后台子代理未收敛：{:?}",
                list.iter().map(|s| (s.agent_name.clone(), s.status)).collect::<Vec<_>>()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let requests = h.requests.lock().unwrap().clone();
    let has_report = requests.iter().any(|r| {
        r.messages
            .iter()
            .any(|m| m.role == "user" && m.content.contains("【子代理结果") && m.content.contains("42 个文件"))
    });
    assert!(has_report, "唤醒 run 的历史里应有子代理报告");

    // 唤醒后父会话里能看到「子代理回来了」的锚点消息（via=subagent_report）
    let snap = h.snapshot(&h.thread_id);
    let anchor = snap.messages.iter().any(|m| {
        m.role == "user"
            && m.parts
                .iter()
                .any(|p| p.payload.get("via").and_then(|v| v.as_str()) == Some("subagent_report"))
    });
    assert!(anchor, "应有子代理回报的锚点用户消息");
}

/// 缺省派发即后台（D31）：不传 run_in_background 的行为与显式 true 一致——
/// 立即返回启动回执、终态后报告注入父历史。串行工具循环下前台缺省会让
/// 「同批多派」排队等终态，并行能力形同虚设，这是默认翻转的根因。
#[test]
fn spawn_defaults_to_background() {
    let h = harness(
        "subagent-default-bg",
        vec![
            Round {
                text: "派一个子代理。".into(),
                tool_calls: vec![(
                    "call_spawn".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "默认派发",
                        "prompt": "统计文件数量",
                        "agent_type": "explore",
                    }),
                )],
            },
            Round { text: "已交给子代理，我先做别的。".into(), tool_calls: vec![] },
            // 唤醒 run：读到报告后收尾
            Round { text: "子代理报告：完成统计。".into(), tool_calls: vec![] },
        ],
        vec![Round { text: "共 42 个文件。".into(), tool_calls: vec![] }],
    );
    let snap = h.run_and_wait(20_000);

    // 子代理卡按后台形态渲染：notify=true（终态需要回报）
    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert_eq!(cards.len(), 1, "缺省派发应有子代理卡");
    assert_eq!(cards[0].payload["notify"], true, "缺省派发是后台形态，终态需要回报");
    assert_eq!(cards[0].payload["status"], "completed");

    // 工具结果是启动回执，不含结论
    let tool_output = snap
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .find(|p| p.kind == "tool" && p.payload["name"] == "agent_spawn")
        .and_then(|p| p.payload["output"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(tool_output.contains("后台"), "缺省回执应说明已后台启动：{tool_output}");
    assert!(!tool_output.contains("42 个文件"), "缺省回执不应提前给出结论：{tool_output}");

    // 报告注入父历史（唤醒链路与显式 true 一致）
    let requests = h.requests.lock().unwrap().clone();
    let has_report = requests.iter().any(|r| {
        r.messages
            .iter()
            .any(|m| m.role == "user" && m.content.contains("【子代理结果") && m.content.contains("42 个文件"))
    });
    assert!(has_report, "缺省派发的报告应注入父历史");
}

/// 子代理 nudge（issue 1 P4）：主会话本 run 检索类调用达到阈值却从未派发时，
/// 下一轮请求的 dynamic 块注入一次派发考虑提醒（哨兵保证只提醒一次）。
/// fs_read 不计入检索口径，fs_grep 等宽面检索才计。
#[test]
fn subagent_nudge_after_repeated_retrieval_without_spawn() {
    // 一轮里 6 次 fs_grep（达到 SUBAGENT_NUDGE_THRESHOLD），下一迭代应触发提醒
    let greps: Vec<(String, String, serde_json::Value)> = (0..6)
        .map(|i| {
            (
                format!("call_grep_{i}"),
                "fs_grep".into(),
                json!({ "pattern": "demo", "path": "." }),
            )
        })
        .collect();
    let h = harness(
        "subagent-nudge",
        vec![
            Round { text: "我先大范围检索一下。".into(), tool_calls: greps },
            Round { text: "检索完成，整理结论。".into(), tool_calls: vec![] },
        ],
        vec![],
    );
    let seq = h.append_user_message("帮我检索整理");
    h.run_until_settled(seq, 20_000);

    let requests = h.requests.lock().unwrap().clone();
    let parent_requests: Vec<&Request> = requests
        .iter()
        .filter(|r| !r.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false))
        .collect();
    assert!(parent_requests.len() >= 2, "应至少两轮父请求");
    // 第 2 轮请求（检索计数达标后的下一迭代）注入提醒
    let second_dynamic = &parent_requests[1].system[2].content;
    assert!(
        second_dynamic.contains("尚未派发过任何子代理"),
        "检索达标且从未派发时应注入 nudge：{second_dynamic}"
    );
    // 哨兵：第 2 轮只注入一次，若还有后续轮不重复（本脚本无第 3 轮，钉住注入位置即可）
    let first_dynamic = &parent_requests[0].system[2].content;
    assert!(!first_dynamic.contains("尚未派发过任何子代理"), "首轮（0 次检索）不应注入");
}

/// 父会话取消：在跑的子代理被级联取消（不留孤儿烧 token）。
#[test]
fn cancelling_parent_cascades_to_child_agents() {
    let h = harness(
        "subagent-cancel",
        vec![
            Round {
                text: "派一个然后我取消。".into(),
                tool_calls: vec![(
                    "call_spawn".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "长任务",
                        "prompt": "做很久的事",
                        "agent_type": "explore",
                        "run_in_background": true,
                    }),
                )],
            },
            // 父 run 第二轮：明确表示还在等（保持运行窗口）
            Round { text: "已派发，我等它。".into(), tool_calls: vec![] },
        ],
        vec![Round { text: "子代理结论".into(), tool_calls: vec![] }],
    );
    let runner = Arc::clone(&h.runner);
    let thread_id = h.thread_id.clone();
    let seq = h.append_user_message("派发");
    let outcome = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("测试运行时启动失败");
        rt.block_on(async move {
            {
                let runner = Arc::clone(&runner);
                let thread_id = thread_id.clone();
                tokio::task::spawn_blocking(move || runner.run(&thread_id, Some(seq)))
                    .await
                    .expect("发起运行失败")
                    .expect("启动运行失败");
            }
            // 等子代理登记
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10_000);
            while runner.subagents().list(&thread_id).is_empty() {
                if tokio::time::Instant::now() >= deadline {
                    return Err("子代理未登记".to_string());
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            // 取消父会话：级联应把子代理收口
            runner.cancel(&thread_id);
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10_000);
            loop {
                let list = runner.subagents().list(&thread_id);
                if list.iter().all(|s| s.status.is_terminal()) {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "子代理未被级联取消：{:?}",
                        list.iter().map(|s| s.status).collect::<Vec<_>>()
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
    })
    .join()
    .expect("取消测试线程 panic");
    if let Err(err) = outcome {
        panic!("{err}");
    }
    let list = h.runner.subagents().list(&h.thread_id);
    assert!(
        matches!(
            list[0].status,
            AgentStatus::Interrupted | AgentStatus::Closed | AgentStatus::Completed
        ),
        "取消后子代理应处于终态：{:?}",
        list[0].status
    );
}

/// 自定义子代理：派发时用定义里的模型与迭代上限，且工具面按定义裁剪。
#[test]
fn custom_agent_uses_its_own_model_and_iteration_cap() {
    use craft_core::contract::{CustomAgentProfile, CustomAgentTools};
    // 自定义「reviewer」：只读 + 指定模型 stub/m2 + 轮数 20
    let custom = vec![CustomAgentProfile {
        id: "agent_reviewer".into(),
        name: "reviewer".into(),
        description: "只做代码评审".into(),
        role_prompt: "你是评审子代理，只读代码并按风险排序列出问题。".into(),
        model: "stub/m2".into(),
        max_iterations: 20,
        tools: CustomAgentTools::ReadOnly,
        enabled: true,
    }];
    let h = harness_with_config(
        "subagent-custom",
        vec![
            Round {
                text: "派一个评审子代理。".into(),
                tool_calls: vec![(
                    "call_review".into(),
                    "agent_spawn".into(),
                    json!({ "description": "评审入口", "prompt": "评审一下入口文件", "agent_type": "reviewer" }),
                )],
            },
            Round { text: "评审完成。".into(), tool_calls: vec![] },
        ],
        vec![Round { text: "发现两个风险点。".into(), tool_calls: vec![] }],
        SubAgentConfig::new(true, 4, Vec::new(), custom),
    );
    let snap = h.run_and_wait(20_000);

    // 卡片：类型是 custom，名字取定义名
    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].payload["agentType"], "custom");
    assert_eq!(cards[0].payload["model"], "stub/m2", "卡片记录定义里的模型");
    assert_eq!(cards[0].payload["status"], "completed");

    // 子会话请求：用定制模型 + 定制角色提示词 + 只读工具面
    let requests = h.requests.lock().unwrap().clone();
    let child_req = requests
        .iter()
        .find(|r| r.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false))
        .expect("应有一次子代理请求");
    assert_eq!(child_req.model, "m2", "子代理请求用定义里的模型");
    let dynamic = &child_req.system[2].content;
    assert!(dynamic.contains("你是评审子代理"), "定义的角色提示词进入提示词：{dynamic}");
    let names: Vec<&str> = child_req.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(!names.contains(&"fs_write"), "只读档不应有写工具：{names:?}");
    assert!(names.contains(&"fs_read"));

    // 子会话落库了档案快照（历史可解释「当时按什么跑的」）
    let child_id = cards[0].payload["agentId"].as_str().unwrap();
    let row = craft_store::threads::get_thread(&h.db.lock_conn(), child_id).unwrap();
    assert_eq!(row.agent_type, "custom");
    let profile: serde_json::Value = serde_json::from_str(&row.agent_profile).expect("档案快照应为合法 JSON");
    assert_eq!(profile["typeName"], "reviewer");
    assert_eq!(profile["model"], "stub/m2");
    assert_eq!(profile["maxIterations"], 20);
}

/// 自定义定义被停用后派发报错，并列出可用类型（含内置两型）。
#[test]
fn disabled_custom_agent_rejects_spawn_with_available_names() {
    use craft_core::contract::{CustomAgentProfile, CustomAgentTools};
    let custom = vec![CustomAgentProfile {
        id: "agent_off".into(),
        name: "reviewer".into(),
        description: String::new(),
        role_prompt: String::new(),
        model: String::new(),
        max_iterations: 0,
        tools: CustomAgentTools::ReadOnly,
        enabled: false,
    }];
    let h = harness_with_config(
        "subagent-custom-off",
        vec![
            Round {
                text: "派一个停用的子代理。".into(),
                tool_calls: vec![(
                    "call_off".into(),
                    "agent_spawn".into(),
                    json!({ "description": "评审", "prompt": "评审", "agent_type": "reviewer" }),
                )],
            },
            Round { text: "拿不到那个子代理。".into(), tool_calls: vec![] },
        ],
        vec![],
        SubAgentConfig::new(true, 4, Vec::new(), custom),
    );
    let snap = h.run_and_wait(20_000);
    // 没有子代理卡片（派发在早于建会话处就失败）
    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert!(cards.is_empty(), "停用定义不应产生子代理");
    // 工具结果里是可读错误，且列出内置类型
    let tool_output = snap
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .find(|p| p.kind == "tool")
        .and_then(|p| p.payload["output"].as_str())
        .unwrap_or_default()
        .to_string();
    assert!(tool_output.contains("未知的子代理类型"), "{tool_output}");
    assert!(tool_output.contains("general"), "错误文案应列出可用类型：{tool_output}");
}

impl Harness {
    /// 某会话某类事件的载荷列表（seq 升序）——断言事件表原始形状用，不经快照合并。
    fn events_of_kind(&self, thread_id: &str, kind: &str) -> Vec<serde_json::Value> {
        self.db
            .with(|conn| {
                let mut stmt = conn
                    .prepare("SELECT payload FROM events WHERE thread_id = ?1 AND kind = ?2 ORDER BY seq ASC")
                    .map_err(craft_store::db::other_err)?;
                let rows: Vec<String> = stmt
                    .query_map([thread_id, kind], |row| row.get::<_, String>(0))
                    .map_err(craft_store::db::other_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(craft_store::db::other_err)?;
                Ok(rows.iter().map(|raw| serde_json::from_str(raw).unwrap()).collect())
            })
            .unwrap()
    }

    /// 最后一次子代理请求（收尾指令的注入次数断言用：history 累积，最后请求里
    /// 含指令文本的 user 消息条数即注入次数）。
    fn last_child_request(&self) -> Request {
        let requests = self.requests.lock().unwrap().clone();
        requests
            .into_iter()
            .rev()
            .find(|r| r.system.first().map(|b| b.content.contains("子代理")).unwrap_or(false))
            .expect("应至少有一次子代理请求")
    }
}

/// 运行中进度可观测（D15 的面板补注）：子代理每完成一次工具调用，
/// 父会话就追加一条同一部件 id 的中间态事件——工具次数与已用轮数实时递增、载荷含
/// 三个新契约字段（iterations / maxIterations / parentThreadId）；终态事件仍从数据库
/// 统计真值覆盖计数，最后一次模型调用（无工具的最终回答）也计入轮数。
#[test]
fn progress_events_stream_to_parent_during_child_run() {
    let h = harness(
        "subagent-progress",
        vec![
            Round {
                text: "派一个子代理做两次检索。".into(),
                tool_calls: vec![(
                    "call_spawn".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "两次检索",
                        "prompt": "读两个文件",
                        "agent_type": "general",
                        "name": "digger",
                    }),
                )],
            },
            Round { text: "子代理已经完成检索。".into(), tool_calls: vec![] },
        ],
        vec![
            Round {
                text: "先读第一个文件。".into(),
                tool_calls: vec![("c1".into(), "fs_read".into(), json!({ "path": "/tmp/a.txt" }))],
            },
            Round {
                text: "再读第二个文件。".into(),
                tool_calls: vec![("c2".into(), "fs_read".into(), json!({ "path": "/tmp/b.txt" }))],
            },
            Round { text: "检索完成。".into(), tool_calls: vec![] },
        ],
    );
    let snap = h.run_and_wait(20_000);

    // 快照侧：多次事件仍合并为一张卡（partId 不变）
    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert_eq!(cards.len(), 1, "中间态多次事件不增加卡片数");
    assert_eq!(cards[0].payload["status"], "completed");
    assert_eq!(cards[0].payload["toolUses"], 2, "终态以数据库统计真值覆盖");
    assert_eq!(cards[0].payload["iterations"], 3, "轮数含最后一轮（无工具的最终回答）");
    assert_eq!(cards[0].payload["parentThreadId"], h.thread_id.as_str());
    assert!(cards[0].payload["maxIterations"].as_i64().unwrap() > 0, "生效上限写进载荷");

    // 事件表侧：running + 两次中间态 + 最后一轮 + 终态，全部同一部件 id
    let events = h.events_of_kind(&h.thread_id, "subagent");
    assert!(events.len() >= 5, "应至少有 running + 2 中间态 + 最后一轮 + 终态，实际 {}", events.len());
    let part_id = cards[0].payload["partId"].as_str().unwrap().to_string();
    for payload in &events {
        assert_eq!(payload["partId"].as_str().unwrap_or_default(), part_id, "全部事件同一部件 id（合并语义）");
    }
    // 中间态事件：轮数与工具次数随调用递增
    let iterations: Vec<i64> = events.iter().map(|p| p["iterations"].as_i64().unwrap_or(0)).collect();
    let tool_uses: Vec<i64> = events.iter().map(|p| p["toolUses"].as_i64().unwrap_or(0)).collect();
    assert_eq!(iterations[0], 0, "running 卡：尚未发起轮次");
    assert_eq!(iterations[1], 1, "第一次工具调用后已用 1 轮");
    assert_eq!(tool_uses[1], 1);
    assert_eq!(iterations[2], 2, "第二次工具调用后已用 2 轮");
    assert_eq!(tool_uses[2], 2);
    assert_eq!(tool_uses.last().unwrap(), &2, "终态仍以数据库统计覆盖（两次工具调用）");
}

/// 软上限收尾 + 触顶收尾（D15 的面板补注）：上限配得很小时（3 轮），
/// 子 run 第一轮开始就满足「剩余 ≤ 20」——子会话出现恰好一条可见收尾提示、历史注入恰好
/// 一次收尾指令；触顶后失败信息写明「已用满 N 轮，上限 M」，触顶前最后文本作为部分结论带回。
#[test]
fn wrapup_notice_injected_once_and_cap_failure_keeps_partial_result() {
    let config = SubAgentConfig::new(
        true,
        4,
        vec![craft_core::contract::BuiltinAgentProfile {
            agent_type: "general".into(),
            model: String::new(),
            max_iterations: 3,
        }],
        Vec::new(),
    );
    let h = harness_with_config(
        "subagent-wrapup",
        vec![
            Round {
                text: "派一个子代理做长任务。".into(),
                tool_calls: vec![(
                    "call_spawn".into(),
                    "agent_spawn".into(),
                    json!({
                        "description": "长任务",
                        "prompt": "多轮检索",
                        "agent_type": "general",
                        "name": "looper",
                    }),
                )],
            },
            Round { text: "拿到子代理的失败结论。".into(), tool_calls: vec![] },
        ],
        vec![
            Round {
                text: "第 1 轮工作。".into(),
                tool_calls: vec![("c1".into(), "fs_read".into(), json!({ "path": "/tmp/a" }))],
            },
            Round {
                text: "第 2 轮工作。".into(),
                tool_calls: vec![("c2".into(), "fs_read".into(), json!({ "path": "/tmp/b" }))],
            },
            Round {
                text: "第 3 轮：结论如下，已把部分成果整理完毕。".into(),
                tool_calls: vec![("c3".into(), "fs_read".into(), json!({ "path": "/tmp/c" }))],
            },
        ],
        config,
    );
    let snap = h.run_and_wait(20_000);

    // 父会话卡片：触顶按失败收尾，触顶前最后文本作为部分结论带回
    let cards: Vec<_> = snap.messages.iter().flat_map(|m| m.parts.iter()).filter(|p| p.kind == "subagent").collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].payload["status"], "failed");
    let result = cards[0].payload["result"].as_str().unwrap_or_default();
    assert!(result.contains("结论如下"), "触顶前最后文本应作为部分结论带回：{result}");

    let child_id = cards[0].payload["agentId"].as_str().unwrap().to_string();
    // 子会话：可见收尾提示恰好一条（用户正开着详情 tab 就能看到「开始收尾」）
    let notices = h.events_of_kind(&child_id, "notice");
    assert_eq!(notices.len(), 1, "收尾提示只写一次");
    assert!(notices[0]["text"].as_str().unwrap_or_default().contains("收尾"));
    // 子会话：触顶失败信息写明「已用满 N 轮，上限 M」
    let errors = h.events_of_kind(&child_id, "error");
    assert_eq!(errors.len(), 1, "触顶应有且只有一条失败转移");
    let message = errors[0]["message"].as_str().unwrap_or_default();
    assert!(message.contains("已用满 3 轮"), "{message}");
    assert!(message.contains("上限 3 轮"), "{message}");

    // 历史注入：最后一个子请求里含收尾指令的 user 消息恰好一条（哨兵保证只注入一次；
    // history 累积让后续每轮请求都能看到同一条消息，条数不随轮数增长）
    let last_child = h.last_child_request();
    let wrapup_count = last_child
        .messages
        .iter()
        .filter(|m| m.role == "user" && m.content.contains("停止扩张范围"))
        .count();
    assert_eq!(wrapup_count, 1, "收尾指令只注入一次");
    assert!(last_child.messages.iter().any(|m| m.content.contains("剩余约 3 轮")), "指令应写明剩余轮数");
}
