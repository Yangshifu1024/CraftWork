//! 计划模式硬门卡的端到端验证。
//!
//! 规则：分级档下计划态的写类工具（`ToolSpec.mutates = true`）不得执行——此前只有提示词
//! 软约束，模型可以在「声称调研」的同时直接改文件。这里用脚本化模型真实走一遍 Runner，
//! 断言工具确实被拦下、磁盘确实没被改，且放行范围（只读 / 协议 / todo_write）不受影响。
//!
//! 自由档是唯一的例外（D8 2026-11-10 修订）：free 在策略引擎里本就是全量豁免，门卡不启用，
//! 但 `exit_plan` 的无条件审批不变——两个方向各有一条用例钉住（见文件末尾两条 free_level_*）。
//!
//! 装配手法与 `subagent_flow.rs` 一致（隔离数据目录 + stub 提供商 + client_override），
//! 区别仅在用户消息带 `planMode: true`，让 run 以计划态启动。

use std::collections::VecDeque;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};

use futures_core::stream::BoxStream;
use serde_json::json;

use craft_agent::approvals::Approvals;
use craft_agent::grants::Grants;
use craft_agent::runner::{Runner, RunnerDeps};
use craft_agent::session_log::SessionLog;
use craft_core::bus::Bus;
use craft_core::contract::Snapshot;
use craft_model::catalog::Catalog;
use craft_model::factory::ModelFactory;
use craft_model::types::{Chunk, Client, Request, ToolCall, Usage};
use craft_policy::{Engine, Level};
use craft_store::db::Db;
use craft_store::recorder::Recorder;
use craft_tools::registry::Registry;

/// 一轮脚本化的模型输出：正文 + 若干工具调用。
struct Round {
    text: String,
    tool_calls: Vec<(String, String, serde_json::Value)>,
}

/// 假模型客户端：按脚本队列依次产出，并把收到的请求存下来供断言
/// （`crates/craft-model/src/integrity.rs` 模块头：端到端断言「交给提供商的消息序列形状」）。
struct ScriptedClient {
    rounds: Arc<Mutex<VecDeque<Round>>>,
    seen: Arc<Mutex<Vec<Request>>>,
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
        Box::new(ScriptedClient { rounds: Arc::clone(&self.rounds), seen: Arc::clone(&self.seen) })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    db: Arc<Db>,
    runner: Arc<Runner>,
    recorder: Recorder,
    approvals: Arc<Approvals>,
    thread_id: String,
    workspace: std::path::PathBuf,
    /// 假模型客户端收到的全部请求（按调用顺序），断言「交给提供商的消息序列」用
    requests: Arc<Mutex<Vec<Request>>>,
    /// 会话日志目录（断言 history_integrity_repaired 的有无）
    log_dir: std::path::PathBuf,
}

/// 装配 Runner：隔离数据目录、stub 提供商、脚本化模型、真实内置工具。
/// 审批档位默认分级（被门卡拦截的路径）；自由档用例经 [`harness_with_level`] 指定。
fn harness(rounds: Vec<Round>) -> Harness {
    harness_with_level(rounds, Engine::default())
}

/// 指定审批档位的装配。档位是门卡的唯一开关：分级档拦写类工具，自由档整道门卡不启用
/// （D8 2026-11-10 修订），所以两条路径必须各自有用例钉住。
fn harness_with_level(rounds: Vec<Round>, engine: Engine) -> Harness {
    let dir = tempfile::TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let data_dir = dir.path().to_string_lossy().to_string();
    let db = Arc::new(Db::open(dir.path().join("craftwork.db").as_path()).unwrap());
    let recorder = Recorder::new(Arc::clone(&db), Arc::new(Bus::new(|_| {})));
    let engine = Arc::new(Mutex::new(engine));
    let grants = Arc::new(Grants::new(Arc::clone(&db)));
    let approvals = Arc::new(Approvals::new(Arc::clone(&db), recorder.clone(), Arc::clone(&grants)));
    let catalog = Arc::new(Catalog::new(Arc::clone(&db)));

    // stub 提供商：让目录能解析到 stub/m1（否则 open_session 直接 provider_unavailable）
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
    };
    models.set_client_override(Box::new(move |_| Some(scripted.clone_box())));

    let registry = Arc::new(Registry::new("", &data_dir, Arc::clone(&db)));
    registry.register(Box::new(craft_tools::fs::FsList));
    registry.register(Box::new(craft_tools::fs::FsRead));
    registry.register(Box::new(craft_tools::fs::FsWrite));
    registry.register(Box::new(craft_tools::fs::FsEdit));
    registry.register(Box::new(craft_tools::shell::ShellExec));
    registry.register(Box::new(craft_tools::todo::TodoWrite));
    registry.register(Box::new(craft_tools::plan::EnterPlan));
    registry.register(Box::new(craft_tools::plan::ExitPlan));

    // 日志落到固定根下的唯一子目录且开 DEBUG：TempDir 在 panic 时会被清理，固定根
    // 便于事后排查「运行超时未收尾」类挂起（看 run 停在了哪一步）；子目录按进程内
    // 序号隔离——四个用例并行跑，共用一个目录会互相 remove（实测挂在 create_dir_all）。
    use std::sync::atomic::AtomicU32;
    static LOG_SEQ: AtomicU32 = AtomicU32::new(0);
    let log_dir = std::path::Path::new("/tmp/pg-home").join(format!(
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

    // 会话绑定到隔离工作区（trusted：让写类操作不被受限工作区规则额外拦掉，
    // 本测试唯一要验证的是计划门卡）。upsert 不收 id——行 ID 由存储层生成，
    // 必须拿返回值回填 thread.workspace_id，否则 get_workspace 查不到（曾因参数
    // 错位把 id 传进 name 位，run 全部落到 workspace_missing 提前失败）。
    let ws_id = craft_store::workspaces::upsert_workspace(
        &db.lock_conn(),
        "ws",
        workspace.to_str().unwrap(),
        "trusted",
        "",
    )
    .unwrap()
    .id;
    let thread_id = "th_plan_gate".to_string();
    craft_store::threads::create_thread(
        &db.lock_conn(),
        craft_store::threads::NewThread {
            id: &thread_id,
            mode: "code",
            title: "计划门卡",
            title_auto: false,
            model_id: "stub/m1",
            reasoning_level: "",
            workspace_id: &ws_id,
            status: "idle",
            archived: false,
        },
    )
    .unwrap();

    Harness { _dir: dir, db, runner, recorder, approvals, thread_id, workspace, requests, log_dir }
}

impl Harness {
    /// 追加一条「计划模式」用户消息（planMode=true 让本 run 以计划态启动），返回其 seq。
    fn append_plan_user_message(&self, text: &str) -> i64 {
        let message_id = craft_core::ids::new_id(craft_core::ids::id_prefix::MESSAGE);
        self.recorder
            .append(&craft_store::events::EventRecord {
                thread_id: self.thread_id.clone(),
                message_id,
                run_id: None,
                kind: "user_message".into(),
                payload: json!({ "text": text, "attachments": [], "planMode": true }),
            })
            .unwrap()
            .1
    }

    /// 追加一条普通用户消息（planMode=false）：自动进入计划门（D32）的触发场景——
    /// run 不以计划态启动，模型直接动手写文件。
    fn append_plain_user_message(&self, text: &str) -> i64 {
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

    /// 在独立运行时里发起运行并等到会话空闲。
    ///
    /// `Runner::run` 是非阻塞的（起后台任务后立即返回），所以不能只等线程：
    /// 必须轮询 `is_running` 直到本次运行真正结束，否则读快照时工具还没执行。
    /// 运行时也不能提前 drop——run 的后台任务挂在它上面。
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
                // 等本次运行从取消表里消失（收尾完成）
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

    fn snapshot(&self) -> Snapshot {
        craft_store::events::snapshot(&self.db.lock_conn(), &self.thread_id, 0, 0).unwrap()
    }

    /// 收集所有工具结果（名称、输出、是否错误）。
    fn tool_outputs(&self) -> Vec<(String, String, bool)> {
        self.snapshot()
            .messages
            .iter()
            .flat_map(|m| m.parts.iter())
            .filter(|p| p.kind == "tool")
            .map(|p| {
                (
                    p.payload["name"].as_str().unwrap_or_default().to_string(),
                    p.payload["output"].as_str().unwrap_or_default().to_string(),
                    p.payload["isError"].as_bool().unwrap_or(false),
                )
            })
            .collect()
    }

    /// 读取会话日志里 `history_integrity_repaired` 的行数（配对修复的观测断言用）。
    fn integrity_repair_log_lines(&self) -> usize {
        let path = self.log_dir.join("logs").join(format!("{}.log", self.thread_id));
        std::fs::read_to_string(path)
            .map(|content| content.lines().filter(|l| l.contains("history_integrity_repaired")).count())
            .unwrap_or(0)
    }

    /// 主循环的模型请求（按顺序）：sidecar 小请求（命名/预测/压缩）不带工具清单，
    /// 以 `tools` 非空区分。断言「交给提供商的消息序列」只看主循环请求。
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

/// 计划态下写文件被拦：工具报错、提示 exit_plan，磁盘上确实没有产生文件。
#[test]
fn plan_mode_blocks_fs_write_and_leaves_disk_untouched() {
    let h = harness(vec![
        Round {
            text: "我先写个文件。".into(),
            tool_calls: vec![(
                "call_write".into(),
                "fs_write".into(),
                json!({ "path": "blocked.txt", "content": "不该被写入" }),
            )],
        },
        Round { text: "计划模式下调不动写工具，我先提交计划。".into(), tool_calls: vec![] },
    ]);
    let seq = h.append_plan_user_message("调研一下这个项目");
    h.run_and_wait(seq);

    let outputs = h.tool_outputs();
    let write = outputs
        .iter()
        .find(|(name, _, _)| name == "fs_write")
        .expect("应有 fs_write 的工具结果");
    assert!(write.2, "计划态下 fs_write 应返回错误结果");
    assert!(
        write.1.contains("计划模式") && write.1.contains("exit_plan"),
        "拦截说明应指出计划模式并给出 exit_plan 出路：{}",
        write.1
    );
    assert!(
        !h.workspace.join("blocked.txt").exists(),
        "计划态下文件绝不应真的被写入磁盘"
    );
}

/// 自由档 + 计划态：门卡不启用，写类工具照常执行并真的落盘。
///
/// 2026-11-10 用户裁决：free 的语义就是「放行一切操作」（`Engine::action` 在受限工作区判定
/// 之前就返回 Allow），门卡曾是唯一不认档位的地方。本用例钉住豁免，并连带钉住 shell_exec：
/// 它在会话 th_a8365685cc319a17 里被一刀切拦下多次，其中多数是 R0 只读命令。
#[test]
fn free_level_plan_mode_executes_write_tools() {
    let h = harness_with_level(
        vec![
            Round {
                text: "自由档下我直接写文件。".into(),
                tool_calls: vec![
                    (
                        "call_write".into(),
                        "fs_write".into(),
                        json!({ "path": "free.txt", "content": "自由档写入" }),
                    ),
                    (
                        "call_shell".into(),
                        "shell_exec".into(),
                        // 命令必须按平台选：未配置 shell 时执行器是 cmd（Windows）
                        // 或 /bin/sh（Unix），`cat` 在 cmd 下不存在（D25）。
                        json!({ "command": if cfg!(windows) { "type free.txt" } else { "cat free.txt" } }),
                    ),
                ],
            },
            Round { text: "写完了。".into(), tool_calls: vec![] },
        ],
        Engine::new(Level::Free),
    );
    let seq = h.append_plan_user_message("直接改吧");
    h.run_and_wait(seq);

    let outputs = h.tool_outputs();
    let write = outputs.iter().find(|(n, _, _)| n == "fs_write").expect("应有 fs_write 结果");
    assert!(!write.2, "自由档下计划态不应拦截写工具：{}", write.1);
    assert!(
        h.workspace.join("free.txt").exists(),
        "自由档下文件应真的落盘"
    );
    let shell = outputs.iter().find(|(n, _, _)| n == "shell_exec").expect("应有 shell_exec 结果");
    assert!(!shell.2, "自由档下计划态不应拦截命令：{}", shell.1);
    assert!(
        !shell.1.contains("当前处于计划模式"),
        "自由档不应再出现门卡文案：{}",
        shell.1
    );
}

/// 自由档 + 计划态：exit_plan 的无条件审批不受豁免影响（D8 核心边界，自由档也弹卡）。
///
/// 与上一条成对：豁免只针对「计划态写拦截」，协议握手照旧。若无此用例，将来有人把
/// 门卡判据顺手挪到 exit_plan 分支上，自由档就会静默退化成「计划不需要批准」。
#[test]
fn free_level_still_requires_exit_plan_approval() {
    let h = harness_with_level(
        vec![
            Round {
                text: "计划做好了，提交审阅。".into(),
                tool_calls: vec![(
                    "call_exit".into(),
                    "exit_plan".into(),
                    json!({ "plan": "第一步：调研；第二步：实施。" }),
                )],
            },
            Round { text: "计划被拒了，等待指示。".into(), tool_calls: vec![] },
        ],
        Engine::new(Level::Free),
    );

    // 后台决定者：等 exit_plan 的审批卡出现就点「忽略」（与分级档用例同一手法）
    let approvals = Arc::clone(&h.approvals);
    let db = Arc::clone(&h.db);
    let decider = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let (approval_id, tool): (String, String) = loop {
            let found: Option<(String, String)> = db
                .with(|conn| {
                    conn.query_row(
                        "SELECT json_extract(payload,'$.approvalId'), json_extract(payload,'$.tool') \
                         FROM events WHERE kind='approval' ORDER BY seq DESC LIMIT 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(craft_store::db::StoreError::from)
                })
                .ok();
            if let Some(found) = found {
                break found;
            }
            assert!(std::time::Instant::now() < deadline, "等待审批卡超时");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(tool, "exit_plan", "自由档下计划审阅卡仍应发出");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(approvals.decide(&approval_id, false, "once", None)).expect("忽略决定应成功");
    });

    let seq = h.append_plan_user_message("做个计划");
    h.run_and_wait(seq);
    decider.join().expect("决定线程 panic");

    let resolved: String = h
        .db
        .with(|conn| {
            conn.query_row(
                "SELECT json_extract(payload,'$.status') FROM events WHERE kind='approval_resolved' ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(craft_store::db::StoreError::from)
        })
        .unwrap();
    assert_eq!(resolved, "denied", "自由档下计划审阅仍必须经用户决定");

    let status = h
        .db
        .with(|conn| Ok(craft_store::threads::get_thread(conn, &h.thread_id).unwrap().status))
        .unwrap();
    assert_eq!(status, "idle", "忽略计划后应回到空闲等待用户输入");
}

/// 计划态下只读工具与 todo_write 照常可用（调研与更新待办是计划模式的本职工作）。
#[test]
fn plan_mode_allows_readonly_and_todo_tools() {
    let h = harness(vec![
        Round {
            text: "先看看目录，同时更新待办。".into(),
            tool_calls: vec![
                ("call_list".into(), "fs_list".into(), json!({ "path": "." })),
                (
                    "call_todo".into(),
                    "todo_write".into(),
                    json!({"items": [
                        {"text": "退出计划模式（提交计划待批准）", "status": "pending"},
                        {"text": "调研项目结构", "status": "active"},
                    ]}),
                ),
            ],
        },
        Round { text: "调研完成。".into(), tool_calls: vec![] },
    ]);
    let seq = h.append_plan_user_message("调研一下这个项目");
    h.run_and_wait(seq);

    let outputs = h.tool_outputs();
    let list = outputs.iter().find(|(n, _, _)| n == "fs_list").expect("应有 fs_list 结果");
    assert!(!list.2, "只读工具在计划态下应放行：{}", list.1);
    let todo = outputs.iter().find(|(n, _, _)| n == "todo_write").expect("应有 todo_write 结果");
    assert!(!todo.2, "计划态首项合规的 todo_write 应放行：{}", todo.1);
}

/// 计划态下 todo_write 首项不是「退出计划模式」时被拒绝（规则一）。
#[test]
fn plan_mode_rejects_todo_without_exit_plan_first() {
    let h = harness(vec![
        Round {
            text: "更新待办。".into(),
            tool_calls: vec![(
                "call_todo".into(),
                "todo_write".into(),
                json!({"items": [
                    {"text": "调研项目结构", "status": "active"},
                    {"text": "退出计划模式", "status": "pending"},
                ]}),
            )],
        },
        Round { text: "待办格式不对，我调整一下。".into(), tool_calls: vec![] },
    ]);
    let seq = h.append_plan_user_message("调研一下这个项目");
    h.run_and_wait(seq);

    let outputs = h.tool_outputs();
    let todo = outputs.iter().find(|(n, _, _)| n == "todo_write").expect("应有 todo_write 结果");
    assert!(todo.2, "首项不是退出计划模式时应拒绝：{}", todo.1);
    assert!(todo.1.contains("第一项"), "错误应指明第一项约束：{}", todo.1);
}

/// 计划审阅被「忽略」（拒绝且无反馈）：本轮立即终止，会话按正常完成回到 idle，
/// 模型不再获得下一轮——用户点忽略后不该看到任务还在转（曾因此被感知为「卡死」）。
#[test]
fn plan_review_ignore_stops_run_and_returns_to_idle() {
    let h = harness(vec![
        Round {
            text: "计划做好了，提交审阅。".into(),
            tool_calls: vec![(
                "call_exit".into(),
                "exit_plan".into(),
                json!({ "plan": "第一步：调研；第二步：实施。" }),
            )],
        },
        // 若实现错误地继续迭代，模型会产出这一轮——用它的文本做「未发生」断言
        Round { text: "第二轮不该发生。".into(), tool_calls: vec![] },
    ]);

    // 后台决定者：等审批卡出现（此时 pending 必已登记——emit 在 insert 之后）就点「忽略」
    let approvals = Arc::clone(&h.approvals);
    let db = Arc::clone(&h.db);
    let decider = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let approval_id = loop {
            let found: Option<String> = db
                .with(|conn| {
                    conn.query_row(
                        "SELECT json_extract(payload,'$.approvalId') FROM events WHERE kind='approval' ORDER BY seq DESC LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(craft_store::db::StoreError::from)
                })
                .ok();
            if let Some(id) = found {
                break id;
            }
            assert!(std::time::Instant::now() < deadline, "等待审批卡超时");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        // decide 是 async（pending 为 tokio Mutex）：与 services dispatch 同款临时运行时驱动
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(approvals.decide(&approval_id, false, "once", None)).expect("忽略决定应成功");
    });

    let seq = h.append_plan_user_message("做个计划");
    h.run_and_wait(seq);
    decider.join().expect("决定线程 panic");

    // 会话回到 idle（正常完成收尾；不是 canceled 也不是 error）
    let status = h
        .db
        .with(|conn| Ok(craft_store::threads::get_thread(conn, &h.thread_id).unwrap().status))
        .unwrap();
    assert_eq!(status, "idle", "忽略后应回到空闲等待用户输入");

    // 模型没有下一轮：第二轮脚本文本绝不出现在对话流
    let texts: Vec<String> = h
        .snapshot()
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter(|p| p.kind == "text")
        .map(|p| p.payload["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        !texts.iter().any(|t| t.contains("第二轮不该发生")),
        "忽略后模型不应再被调用，实际文本：{texts:?}"
    );

    // 审批以 denied 终态落库（决定链路闭环）
    let resolved: String = h
        .db
        .with(|conn| {
            conn.query_row(
                "SELECT json_extract(payload,'$.status') FROM events WHERE kind='approval_resolved' ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(craft_store::db::StoreError::from)
        })
        .unwrap();
    assert_eq!(resolved, "denied", "忽略应落库为 denied 终态");
}

/// 本次报错的直接回归（`crates/craft-model/src/integrity.rs` 模块头）：计划审阅点「提出修改」后，
/// 反馈必须排在 exit_plan 工具结果**之后**进入下一轮请求——此前反馈插在调用与结果
/// 之间，OpenAI 系直接 400 拒绝（`tool_calls` 后必须紧跟全部 tool 结果），且修复
/// 必须发生在根因（入历史位置）而不是靠请求出口兜底掩盖（出口触发即日志为证）。
#[test]
fn plan_review_feedback_lands_after_tool_results_in_next_request() {
    let feedback_text = "请把第一步拆细一点，并补充回滚方案。";
    let h = harness(vec![
        Round {
            text: "计划做好了，提交审阅。".into(),
            tool_calls: vec![(
                "call_exit".into(),
                "exit_plan".into(),
                json!({ "plan": "第一步：调研；第二步：实施。" }),
            )],
        },
        Round { text: "收到反馈，我修订一下计划。".into(), tool_calls: vec![] },
    ]);

    // 后台决定者：审批卡出现即点「提出修改」（拒绝带反馈）
    let approvals = Arc::clone(&h.approvals);
    let db = Arc::clone(&h.db);
    let decider = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let approval_id = loop {
            let found: Option<String> = db
                .with(|conn| {
                    conn.query_row(
                        "SELECT json_extract(payload,'$.approvalId') FROM events WHERE kind='approval' ORDER BY seq DESC LIMIT 1",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(craft_store::db::StoreError::from)
                })
                .ok();
            if let Some(id) = found {
                break id;
            }
            assert!(std::time::Instant::now() < deadline, "等待审批卡超时");
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(approvals.decide(&approval_id, false, "once", Some(feedback_text)))
            .expect("提出修改的决定应成功");
    });

    let seq = h.append_plan_user_message("做个计划");
    h.run_and_wait(seq);
    decider.join().expect("决定线程 panic");

    // Run 正常推进到第二轮：反馈后的修订轮真实发生
    let all_requests = h.main_requests();
    assert_eq!(all_requests.len(), 2, "提出修改后应有第二轮模型请求");
    let second = &all_requests[1].messages;

    // 不变量在请求序列上成立（用产品同款整理函数验证：无修复即合规）
    let report = craft_model::integrity::repair_tool_call_integrity(second.clone());
    assert!(
        report.fixes.is_empty(),
        "第二轮请求的消息序列应天然合规（根因修复生效，而非出口兜底）：{:?}",
        report.fixes
    );

    // 关键顺序：带调用的 assistant → exit_plan 工具结果（紧邻）→ 用户反馈（其后）
    let call_pos = second
        .iter()
        .position(|m| m.role == "assistant" && m.tool_calls.iter().any(|c| c.name == "exit_plan"))
        .expect("第二轮请求应包含 exit_plan 的调用消息");
    let result_pos = second
        .iter()
        .position(|m| m.role == "tool" && m.tool_call_id == "call_exit")
        .expect("第二轮请求应包含 exit_plan 的工具结果");
    let feedback_pos = second
        .iter()
        .position(|m| m.role == "user" && m.content.contains(feedback_text))
        .expect("用户反馈必须出现在下一轮请求里，不能凭空消失");
    assert_eq!(result_pos, call_pos + 1, "工具结果必须紧跟带调用的 assistant 消息");
    assert!(feedback_pos > result_pos, "用户反馈必须排在工具结果之后");

    // 用户输入不消失：反馈消息已落库，快照可见
    let snapshot_texts: Vec<String> = h
        .snapshot()
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| p.payload["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        snapshot_texts.iter().any(|t| t.contains(feedback_text)),
        "反馈应落库为真实用户消息：{snapshot_texts:?}"
    );

    // 根因修复的可观察证据：请求出口没有触发任何配对修复（有触发会写 history_integrity_repaired）
    assert_eq!(h.integrity_repair_log_lines(), 0, "根因修掉后不应由请求出口兜底修复");
}

/// 后台批量决定者：把随后出现的 `expected` 张审批卡逐张批准（once 授权，同一路径
/// 多次写入会多次弹卡）。分级档下非计划态的 fs_write 是 R1 → Ask，D32 的非计划态
/// 用例需要真实批准才能推进；被自动门 / 硬门卡拦下的调用不弹卡、不消耗名额。
fn spawn_batch_approver(h: &Harness, expected: usize) -> std::thread::JoinHandle<()> {
    let approvals = Arc::clone(&h.approvals);
    let db = Arc::clone(&h.db);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let mut decided: Vec<String> = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while decided.len() < expected {
            let cards: Vec<String> = db
                .with(|conn| {
                    let mut stmt = conn
                        .prepare("SELECT json_extract(payload,'$.approvalId') FROM events WHERE kind='approval' ORDER BY seq ASC")
                        .map_err(craft_store::db::StoreError::from)?;
                    let rows: Vec<String> = stmt
                        .query_map([], |row| row.get(0))
                        .map_err(craft_store::db::StoreError::from)?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(craft_store::db::StoreError::from)?;
                    Ok(rows)
                })
                .unwrap();
            for id in cards {
                if !decided.contains(&id) {
                    rt.block_on(approvals.decide(&id, true, "once", None)).expect("批准决定应成功");
                    decided.push(id);
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "等待审批卡超时（已决定 {} / {expected}）",
                decided.len()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    })
}

/// 自动进入计划门（D32）：非计划态 run 内第 3 个**不同文件**的写入在执行前被拦下，
/// 自动进入计划态——a、b 落盘，c 不落盘；拦截说明列出已改文件与 exit_plan 出路；
/// 下一轮请求按新状态重建（system 含计划模式段）。
#[test]
fn auto_enters_plan_on_third_distinct_file() {
    let h = harness(vec![
        Round {
            text: "开始改。".into(),
            tool_calls: vec![
                ("call_a".into(), "fs_write".into(), json!({ "path": "a.txt", "content": "A" })),
                ("call_b".into(), "fs_write".into(), json!({ "path": "b.txt", "content": "B" })),
                ("call_c".into(), "fs_write".into(), json!({ "path": "c.txt", "content": "C" })),
            ],
        },
        Round { text: "已进入计划模式，我先整理计划。".into(), tool_calls: vec![] },
    ]);
    let seq = h.append_plain_user_message("把三个文件都改了");
    let decider = spawn_batch_approver(&h, 2);
    h.run_and_wait(seq);
    decider.join().expect("决定线程 panic");

    // a、b 落盘，c 不落盘（被拦的写入不落盘）
    assert!(h.workspace.join("a.txt").exists(), "前两个文件应正常落盘");
    assert!(h.workspace.join("b.txt").exists(), "前两个文件应正常落盘");
    assert!(!h.workspace.join("c.txt").exists(), "第 3 个文件的写入必须被拦下，磁盘不应有 c.txt");

    let outputs = h.tool_outputs();
    let write_c = outputs.iter().find(|(n, _, _)| n == "fs_write" && !n.is_empty()).expect("应有 fs_write 结果");
    let _ = write_c;
    let blocked = outputs
        .iter()
        .find(|(_, out, is_err)| *is_err && out.contains("c.txt"))
        .expect("被拦的 fs_write(c.txt) 应有错误结果");
    assert!(
        blocked.1.contains("计划模式") && blocked.1.contains("exit_plan") && blocked.1.contains("a.txt") && blocked.1.contains("b.txt"),
        "拦截说明应给出计划模式、已改清单与 exit_plan 出路：{}",
        blocked.1
    );
    // 3 次 fs_write 调用：a、b 成功，c 被拦
    let write_results: Vec<&(String, String, bool)> = outputs.iter().filter(|(n, _, _)| n == "fs_write").collect();
    assert_eq!(write_results.len(), 3, "应有三次 fs_write 工具结果");
    assert!(!write_results[0].2 && !write_results[1].2, "前两次写入应成功");
    assert!(write_results[2].2, "第三次写入应被拦为错误");

    // 下一轮请求按计划态重建：system 含计划模式段
    let requests = h.main_requests();
    assert!(requests.len() >= 2, "应有至少两轮主循环请求");
    let last = requests.last().unwrap();
    assert!(
        last.system.iter().any(|b| b.content.contains("计划模式")),
        "自动进入后的下一轮请求应含计划模式提示词：{:?}",
        last.system.iter().map(|b| b.content.chars().take(80).collect::<String>()).collect::<Vec<_>>()
    );
}

/// 免计划额度内的写入不受干扰：2 个不同文件（含同一文件写两次只算一个）全部成功，
/// 无自动进入、无拦截文案。
#[test]
fn two_distinct_files_do_not_auto_enter_plan() {
    let h = harness(vec![
        Round {
            text: "改两个文件。".into(),
            tool_calls: vec![
                ("call_a1".into(), "fs_write".into(), json!({ "path": "a.txt", "content": "first" })),
                ("call_b".into(), "fs_write".into(), json!({ "path": "b.txt", "content": "B" })),
                ("call_a2".into(), "fs_write".into(), json!({ "path": "a.txt", "content": "second" })),
            ],
        },
        Round { text: "改完了。".into(), tool_calls: vec![] },
    ]);
    let seq = h.append_plain_user_message("改两个文件");
    let decider = spawn_batch_approver(&h, 3);
    h.run_and_wait(seq);
    decider.join().expect("决定线程 panic");

    let outputs = h.tool_outputs();
    let writes: Vec<&(String, String, bool)> = outputs.iter().filter(|(n, _, _)| n == "fs_write").collect();
    assert_eq!(writes.len(), 3);
    for w in &writes {
        assert!(!w.2, "免计划额度内的写入不应被拦：{}", w.1);
        assert!(!w.1.contains("已自动进入计划模式"), "不应出现自动进入文案：{}", w.1);
    }
    assert!(h.workspace.join("a.txt").exists() && h.workspace.join("b.txt").exists());
    let content = std::fs::read_to_string(h.workspace.join("a.txt")).unwrap();
    assert_eq!(content, "second", "同一文件写两次都应执行（计数只算一个，不拦第二次）");
    // 下一迭代注入软提醒（已改 2 个不同文件、从未 enter_plan）
    let requests = h.main_requests();
    let last = requests.last().unwrap();
    assert!(
        last.system.iter().any(|b| b.content.contains("已修改 2 个不同的文件")),
        "改满 2 个文件后下一轮应有软提醒"
    );
}

/// 自由档整道自动门不启用（与硬门卡同一豁免，D8）：三个文件全部落盘、无拦截。
#[test]
fn free_level_does_not_auto_enter_plan() {
    let h = harness_with_level(
        vec![
            Round {
                text: "自由档直接改。".into(),
                tool_calls: vec![
                    ("call_a".into(), "fs_write".into(), json!({ "path": "a.txt", "content": "A" })),
                    ("call_b".into(), "fs_write".into(), json!({ "path": "b.txt", "content": "B" })),
                    ("call_c".into(), "fs_write".into(), json!({ "path": "c.txt", "content": "C" })),
                ],
            },
            Round { text: "改完了。".into(), tool_calls: vec![] },
        ],
        Engine::new(Level::Free),
    );
    let seq = h.append_plain_user_message("改三个文件");
    h.run_and_wait(seq);

    for name in ["a.txt", "b.txt", "c.txt"] {
        assert!(h.workspace.join(name).exists(), "自由档下 {name} 应直接落盘");
    }
    let outputs = h.tool_outputs();
    for (_, out, is_err) in outputs.iter().filter(|(n, _, _)| n == "fs_write") {
        assert!(!is_err, "自由档不应拦截写入：{out}");
        assert!(!out.contains("已自动进入计划模式"), "自由档不应出现自动进入文案：{out}");
    }
}

/// 自动进入 → exit_plan 批准 → 继续实施：c 最终落盘，plans 行 approved=1 且正文
/// 按提交原文落盘（钉住「自动进入也建计划行」——批准产物与手动路径一致）。
#[test]
fn auto_plan_approval_persists_plan_and_continues() {
    let plan_text = "第一步：完成 a 与 b（已做）；第二步：写 c；第三步：验证。";
    let h = harness(vec![
        Round {
            text: "开始改。".into(),
            tool_calls: vec![
                ("call_a".into(), "fs_write".into(), json!({ "path": "a.txt", "content": "A" })),
                ("call_b".into(), "fs_write".into(), json!({ "path": "b.txt", "content": "B" })),
                ("call_c".into(), "fs_write".into(), json!({ "path": "c.txt", "content": "C" })),
            ],
        },
        // 计划态下提交计划（审批卡 → 后台决定者批准）
        Round {
            text: "提交计划。".into(),
            tool_calls: vec![("call_exit".into(), "exit_plan".into(), json!({ "plan": plan_text }))],
        },
        // 批准后继续实施（fs_write 是 R1，分级档弹卡 → 决定者再批）
        Round {
            text: "计划已批准，继续写 c。".into(),
            tool_calls: vec![("call_c2".into(), "fs_write".into(), json!({ "path": "c.txt", "content": "C" }))],
        },
        Round { text: "全部完成。".into(), tool_calls: vec![] },
    ]);

    // 后台决定者：依次批准全部 4 张审批卡（a、b 的 R1 写入 ×2 + exit_plan + 批准后的 c）
    let decider = spawn_batch_approver(&h, 4);

    let seq = h.append_plain_user_message("改三个文件");
    h.run_and_wait(seq);
    decider.join().expect("决定线程 panic");

    // 批准后继续实施：c 最终落盘
    assert!(h.workspace.join("c.txt").exists(), "计划批准后 c 应落盘（第 3 个文件的写入放行）");
    // plans 行：approved=1 且 file_path 非空，文件内容为提交的计划原文
    let (approved, file_path): (i64, String) = h
        .db
        .with(|conn| {
            conn.query_row(
                "SELECT approved, file_path FROM plans WHERE thread_id = ?1 ORDER BY id DESC LIMIT 1",
                [&h.thread_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(craft_store::db::StoreError::from)
        })
        .unwrap();
    assert_eq!(approved, 1, "自动进入产生的计划行在批准后应置 approved=1");
    assert!(!file_path.is_empty(), "计划正文应落盘并回填 file_path");
    let saved = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(saved, plan_text, "计划文件内容应为模型提交的计划原文");
}

/// 零噪音回归（`crates/craft-model/src/integrity.rs` 模块头）：正常会话的请求序列
/// 不被整理改动——没有多余的空工具消息、没有顺序被意外重排的内容，也不写修复日志。
#[test]
fn normal_plan_run_requests_stay_compliant_without_repair_log() {
    let h = harness(vec![
        Round {
            text: "先看看目录。".into(),
            tool_calls: vec![("call_list".into(), "fs_list".into(), json!({ "path": "." }))],
        },
        Round { text: "目录看完了，没有异常。".into(), tool_calls: vec![] },
    ]);
    let seq = h.append_plan_user_message("调研一下这个项目");
    h.run_and_wait(seq);

    let all_requests = h.main_requests();
    assert_eq!(all_requests.len(), 2);
    for (i, req) in all_requests.iter().enumerate() {
        let report = craft_model::integrity::repair_tool_call_integrity(req.messages.clone());
        assert!(
            report.fixes.is_empty(),
            "第 {} 轮请求应零修复（正常历史不被改动）：{:?}",
            i + 1,
            report.fixes
        );
    }
    // 第二轮请求的历史里，第一轮的 fs_list 调用与其结果仍保持「调用 → 结果」紧邻
    let second = &all_requests[1].messages;
    let call_pos = second
        .iter()
        .position(|m| m.role == "assistant" && m.tool_calls.iter().any(|c| c.id == "call_list"))
        .expect("第二轮请求应包含第一轮的工具调用");
    assert_eq!(second[call_pos + 1].role, "tool");
    assert_eq!(second[call_pos + 1].tool_call_id, "call_list");
    assert_eq!(h.integrity_repair_log_lines(), 0, "正常请求不应产生修复日志噪音");
}
