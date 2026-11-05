# CraftWork 架构文档

本文是架构事实源：分层结构、主业务流程、事件管道、安全模型与历史决策摘要。术语以仓库根 [GLOSSARY.md](../GLOSSARY.md) 为准；开发约定见 [AGENTS.md](../AGENTS.md)，行为规格与本地票据约定见 [docs/agents/issue-tracker.md](agents/issue-tracker.md)。

## 1. 定位与原则

CraftWork 是完全本地运行的桌面 Agent：在本机读文件、改代码、跑命令、处理文档。三条产品原则约束所有架构取舍：

- **本地内核**：Agent 循环、工具、策略、存储、密钥全部由本机 Rust 进程执行。
- **本地优先**：会话、密钥、文件与日志保存在本机，不收集遥测；应用不内置任何提供商。
- **可审计、可撤销**：写文件、执行命令、修改文档先经审批，留下审计记录与写前备份。

## 2. 分层架构

```mermaid
flowchart TB
    subgraph R["渲染层 src/renderer（React 19 + zustand + Tailwind v4）"]
        UI["侧栏 / 对话流卡片 / 右侧面板 / 浮层"]
        STORE["状态壳 store/app.ts<br/>applyBatch 唯一事件入口"]
        BRIDGE["tauri-bridge<br/>window.craftwork → invoke"]
    end
    subgraph S["应用壳 src-tauri（唯一接触 Tauri API 的层）"]
        DISPATCH["craftwork_invoke 统一分发<br/>spawn_blocking + dispatch"]
        SVC["services.rs 服务编排<br/>参数校验 / 错误转中文"]
        SYS["system.rs 系统能力<br/>对话框 / 外链唯一出口 / 剪贴板"]
    end
    subgraph D["领域内核 crates/craft-*（普通 Rust 模块，不依赖 Tauri）"]
        AGENT["craft-agent<br/>Run 循环 / 审批状态机 / 压缩"]
        TOOLS["craft-tools<br/>Registry + 16 内置工具"]
        POLICY["craft-policy<br/>风险分级 / 路径安全"]
        MODEL["craft-model<br/>SSE / 双协议适配 / 重试"]
        DOC["craft-doc<br/>docx / xlsx / pdf 文本级"]
        STORE2["craft-store<br/>SQLite / 事件 / Recorder"]
        CORE["craft-core<br/>事件总线 / 契约 / 密钥 / 平台"]
    end
    UI --> STORE --> BRIDGE -->|"invoke(method, args)"| DISPATCH --> SVC
    SVC --> AGENT & TOOLS & POLICY & MODEL & STORE2 & DOC
    AGENT --> TOOLS & POLICY & MODEL & STORE2
    TOOLS --> POLICY & DOC & CORE
    STORE2 --> CORE
    BUS["craft-core Bus 攒批 30ms"] -->|"emit craftwork:events"| STORE
    D --> DB[("SQLite ~/.craftwork/craftwork.db<br/>+ keys.json / backups/ / attachments/")]
```

| 层 | 目录 | 职责 | 关键约束 |
| --- | --- | --- | --- |
| 界面 | `src/renderer` | React 界面；事件驱动状态壳 | 不持有 Node 能力；颜色走语义 token；系统操作只经通道表 |
| 共享契约 | `src/shared` | `channels.ts` 通道表、`events.ts` 事件契约、`types.ts`、`ids.ts` | 与 `craft-core::contract` 经契约金样与 dispatch 对拍测试防漂移 |
| 应用壳 | `src-tauri` | 窗口装配、`craftwork_invoke` 分发、服务编排、系统能力 | 唯一允许 import Tauri 的层 |
| Agent | `crates/craft-agent` | Run 循环、审批状态机、授权缓存、历史重建、上下文压缩、自动命名、提示词组装、环境上下文 | 运行环境是值（RunEnv）不是全局态 |
| 工具 | `crates/craft-tools` | `Tool` trait（spec + execute）、Registry 统一横切（路径解析 / 输出截断 / 写前备份 / panic 隔离） | 同步执行，调用方负责放入阻塞线程 |
| 策略 | `crates/craft-policy` | 风险分级 R0~R4、审批档位、命令分类、路径安全解析、敏感附件名 | 档位只是引擎输入，不改变风险分级 |
| 模型 | `crates/craft-model` | 统一客户端：目录快照、SSE、OpenAI/Anthropic 适配、重试、错误归一化 | 错误必须映射为错误码，不抛原始 HTTP 错误 |
| 存储 | `crates/craft-store` | SQLite 打开/迁移、threads/workspaces/events/runs/approvals、审计/用量/设置/备份、Recorder | seq 分配与插入同事务；事件表是唯一事实源 |
| 基础 | `crates/craft-core` | 事件总线（30ms 攒批）、契约类型、ID、原子写/哈希/环境清洗/子进程、密钥、行级 diff | 被全部领域 crate 依赖 |
| 文档 | `crates/craft-doc` | docx / xlsx / pdf 文本级读写（低层 zip + XML） | 文本级：只改内容不重建排版 |

数据目录 `~/.craftwork`（`CRAFTWORK_HOME` 可隔离）：`craftwork.db`、`keys.json`（0600）、`backups/`、`attachments/`、`workspace/default`（内置临时会话目录）。

## 3. 主业务流程

### 3.1 发送一条消息（首发原子路径 + Run 循环）

```mermaid
sequenceDiagram
    autonumber
    participant U as 用户
    participant C as Composer（conversation.tsx）
    participant SVC as services.rs
    participant ST as craft-store
    participant RN as Runner（craft-agent）
    participant MC as 模型客户端（craft-model）
    participant BUS as Bus → emit

    U->>C: 输入并提交（Enter / 按钮）
    alt 草稿态首发
        C->>SVC: chatCreateAndStart（原子路径，失败零落库）
        SVC->>SVC: resolve(model_ref) 模型校验
        SVC->>ST: create_thread（短标题，title_auto=true）
        SVC->>ST: recorder.append(user_message) 写入并广播
        SVC->>RN: runner.run(thread_id, trigger_seq)
    else 会话内空闲发送
        C->>SVC: chatSend → append_user_message → runner.run
    else 运行中引导 steer
        C->>SVC: chatSteer → runner.try_steer 入队（「待送达」标记）
    end
    RN->>ST: create_run（预分配回复锚点 message_id）
    RN->>ST: snapshot + build_history 重建历史
    RN->>RN: collect_env_context + build_prompt（三层缓存）
    loop 每轮迭代（上限默认 300，实时读设置）
        RN->>MC: open_session_with_failover → stream(request)
        Note over RN: 首轮：上下文压缩检查（阈值 70%）+ 自动命名（并行）
        MC-->>RN: 流式片段（text/reasoning delta 只广播不落库）
        BUS-->>C: 攒批事件 → applyBatch 流式渲染
        RN->>ST: reasoning/text 部件整体落库 + record_usage
        alt 无 tool_calls —— 最终回答
            RN->>ST: finish_run(done)
        else 有 tool_calls
            RN->>RN: 逐个 run_tool（见 3.2 审批流）
        end
    end
    Note over RN: execute_run 返回且 has_steers → 接力新 run 消费 steer
```

要点：

- **首发原子路径**（`services.rs::chat_create_and_start`）：模型校验前置，任何一步失败零落库；侧栏与数据库因此天然不存在空会话。
- **用户消息先落事件表再启动 Run**；首发后 Run 启动失败不回滚（会话保留可重试）。
- **同会话同时只允许一个 run**：`Runner.run` 对旧 token 先 cancel 再登记新 token（新消息接管）。
- **失败转移**只在「建流前」发生：候选链 = 主模型 + `fallbackModel` 设置解析出的备选；流一旦建立，错误不再转移（交由 RetryClient 的同模型重试处理）。
- 四种终态：`done`（complete）/ `canceled`（取消）/ `error`（fail，先落 error 部件）/ `interrupted`（启动时 `interrupt_stale_runs` 兜底收尾上次未完成的 run）。

### 3.2 Run 状态机

```mermaid
stateDiagram-v2
    [*] --> running: create_run + set_thread_status
    running --> awaiting: 审批请求（set_thread_status）
    awaiting --> running: 用户决定 / 取消
    running --> done: 无 tool_calls，最终回答
    running --> canceled: chatCancel / 新消息接管 / 窗口关闭
    running --> error: 流错误 / 超迭代上限 / 提供商不可用
    running --> running: 工具执行完毕继续下一轮
    done --> [*]
    canceled --> [*]
    error --> [*]
    interrupted --> [*]: 启动时 interrupt_stale_runs 兜底
```

### 3.3 审批与授权（三道防线）

```mermaid
sequenceDiagram
    autonumber
    participant RN as Runner
    participant RG as Registry（craft-tools）
    participant PE as PolicyEngine（craft-policy）
    participant AP as Approvals（craft-agent）
    participant GR as Grants（craft-agent）
    participant ST as craft-store
    participant C as 渲染层

    RN->>RG: assess(tool, args) 只读评估
    RG-->>RN: risk R0~R4 + 预构建 ToolCtx
    RN->>RN: emit tool.started（增量事件，不入库）
    RN->>PE: action(restricted, risk, level)
    alt Allow（R0 / 自由档位）
        RN->>RN: 直接进入执行
    else Deny（受限工作区 R3 / R4 破坏性）
        RN->>RN: 生成错误 ToolResult，不执行
    else Ask
        RN->>GR: allowed(tool, target)? 授权缓存查询
        alt 命中（scope=cache）
            RN->>RN: 跳过询问
        else 未命中
            RN->>ST: create_approval（decision IS NULL）
            RN->>ST: append(approval) 落库卡片 + set_thread_status(awaiting)
            ST-->>C: 广播 approval.requested → 审批卡片
            RN->>RN: 等待 oneshot（select! 与取消竞争）
            C->>RN: chatDecideApproval(approvalId, allow, scope)
            RN->>ST: resolve_approval（WHERE decision IS NULL，并发幂等）+ add_audit
            RN->>GR: 非 once 授权 → grants.grant 登记
            Note over GR: session 进内存；workspace 额外写 settings 表<br/>allowPermanent=false / R3 / R4 / 受限一律按 once
            RN-->>C: approval_resolved 事件更新卡片
        end
    end
    RN->>RG: spawn_blocking(execute)（panic 隔离 + 输出截断）
    RN->>ST: tool_call 部件落库 + add_audit
```

三道防线互相独立：工具自身参数校验；策略引擎决定是否审批；`craft-policy::resolve` 路径解析拒绝越界（词法校验 + 符号链接真实路径双重校验，不信任渲染层传来的路径）。审批必须显式决定，不允许后端静默放行；决定一经写入不可更改；取消、窗口关闭、应用退出统一按拒绝或取消落审计。

### 3.4 事件管道（写库 → 攒批 → 广播 → 应用）

```mermaid
flowchart LR
    W["领域 crate<br/>产生事件"] --> REC["Recorder.append<br/>事务内 MAX(seq)+1 分配 + INSERT<br/>并推进 threads.updated_at"]
    REC --> DB[("events 表<br/>线程内 seq 单调递增<br/>唯一事实源")]
    REC --> BUS["Bus（craft-core）<br/>内存 seq + 30ms 攒批窗口"]
    BUS -->|"emit craftwork:events<br/>Batch{v,threadId,events}"| APPLY["渲染层 applyBatch<br/>（唯一事件入口）"]
    APPLY --> UP["upsertPart / upsertAppend<br/>定位消息三级回落"]
    APPLY --> RT["thread.updated / workspace.updated<br/>refreshThreads / 清草稿槽"]
```

- **增量事件**（text.delta / reasoning.delta / tool.started）只广播不落库；落库的是部件整体（reasoning / text / tool_call 完成态）。
- 落库失败降级为纯广播（`append_or_warn`），保证界面不因存储故障卡死。
- 快照与在途事件合并：切换会话拉全量快照（权威序），事件流独有的 streaming 消息追加尾部（`mergeStreamedMessages`）。

### 3.5 停止与取消

- 停止按钮（armed 二次确认 800ms）与运行中双击 Esc 共享 armed 态，都走 `chatCancel` → `Runner.cancel`：从 `cancels` 表取出该会话的 `CancellationToken` 并 cancel；流消费与审批等待处 `tokio::select!` 感知，分别进入 `canceled` 终态与审批取消收尾。
- 窗口关闭 / 应用退出：`on_window_event(CloseRequested)` 与 `RunEvent::ExitRequested` 两处调 `cancel_all_runs`（逐会话 cancel + 审计 decision="canceled"）；异步收尾未完成的由下次启动 `interrupt_stale_runs` 标记 interrupted 兜底。
- 运行中引导 steer：入队暂存（上限 `MAX_STEER_QUEUE`），当前工具批次结束后并入下一次模型请求；run 正常结束后若有未消费 steer，自动接力为新 run。

### 3.6 快照装载与草稿

- **快照**：切换会话 → `threadSnapshot` → `craft-store::events::snapshot`：按 seq 升序读事件，按 messageId 归组重建消息与部件；审批状态回填以 approvals 表 `decision IS NULL` 为准；runs 表给无事件 run 补占位消息（权威信号 = `trigger_seq + 1`）。
- **草稿槽**：未发首条消息的会话意图，不写事件表、不进会话列表；按「模式 × 工作区」分槽存 localStorage 单键 `craftwork-drafts`（内容跨重启恢复）；项目删除经 `WORKSPACE_UPDATED{removed}` 事件清对应槽。

## 4. 存储模型

SQLite（rusqlite bundled + WAL，单连接 Mutex 串行）是事实源；消息与部件不另存副本，从事件表重建。

| 表 | 职责 |
| --- | --- |
| `events` | 会话事件，`(thread_id, seq)` 单调递增；写入在事务内完成（seq 分配与插入同事务） |
| `threads` | 会话行（标题、状态、title_auto 标记、updated_at 随事件推进） |
| `workspaces` / `workspace_folders` | 工作区与多源文件夹（首个为主要目录）、信任档（trusted / restricted） |
| `runs` | Run 生命周期（status 终态、message_id 回复锚点、trigger_seq） |
| `approvals` | 审批事实（请求载荷 + 决定；`WHERE decision IS NULL` 做并发幂等） |
| `audit_log` | 工具调用与审批决定（自由档位下是事后追责唯一手段） |
| `usage` / `backups` / `settings` | 用量（含费用估算）、写前备份（SHA-256 登记）、设置（含 workspace_grants） |

schema 变更只在 `craft-store/src/db.rs` 的 `MIGRATIONS` 追加版本，已发布版本号语义不得改动；与历史 Node 版数据库双向兼容（golden 样本对拍测试保障）。

## 5. 安全模型

| 机制 | 实现 |
| --- | --- |
| 风险分级 | R0 读 / R1 写工作区 / R2 执行 / R3 网络或越界 / R4 破坏性；参数可提升风险 |
| 审批档位 | graded / strict / readonly / free；free 是唯一放行一切的档位，切换需「能力清单 + 风险勾选」确认门；档位切换即时生效 |
| 授权范围 | once / session / workspace；`allowPermanent=false`、R3、R4、受限工作区一律不给永久授权 |
| 受限工作区 | R3 直接拒绝、R1/R2 逐次审批、已有授权不生效、不提供永久授权 |
| 路径安全 | `craft-policy::resolve`：词法校验（越界 / 反斜杠 / 双斜杠）→ 符号链接真实路径解析 → 必须落在工作区根内 |
| 命令执行 | `shell_exec`：独立进程组、环境白名单清洗、超时杀进程组（含孙进程）、有界输出缓冲 + 截断；不是交互式终端（已移除，决策 D1） |
| 密钥 | 只存 Rust 进程与 `~/.craftwork/keys.json`（0600，损坏降级启动）；不进日志、数据库、渲染层 |
| 界面隔离 | CSP + capabilities 开启；不加载远程页面；外链唯一出口经 `open_external` 校验（http/https）交系统浏览器；`on_navigation` 拦截就地导航 |
| 数据保护 | 写前备份到 `backups/` 并记 SHA-256；数据面板可恢复；恢复前对当前文件再备份 |

## 6. 模型接入

```mermaid
flowchart LR
    REF["providerID/modelID 引用"] --> F["ModelFactory.client"]
    F --> RES["catalog.resolve<br/>（models.dev 构建期快照 + 用户提供商）"]
    RES -->|"停用即拦截<br/>PROVIDER_UNAVAILABLE"| EN["enabled 检查"]
    EN --> KEY["SecretStore 取密钥<br/>（required 且缺失 → AUTH）"]
    KEY --> PROTO{"协议分发"}
    PROTO -->|"anthropic"| AN["AnthropicClient<br/>（缓存断点 + 交替约束）"]
    PROTO -->|"openai / openai-compatible"| OA["OpenAICompatClient"]
    AN & OA --> RC["RetryClient<br/>3 次尝试 / 700ms 指数退避封顶 5s<br/>仅对未产出片段的可重试错误"]
    RC --> SSE["SSE 流（craft-model::sse）"]
```

- 提供商与模型目录来自 `assets/catalog/catalog.json.gz`（models.dev 构建期快照，`include_bytes!` 嵌入）；用户自定义提供商存 settings 的 `model.custom_providers`；应用不内置任何可用提供商。
- 错误归一化：HTTP 状态码 → 基础错误码，关键词二次修正（如 400/401 也可能报限流、流内错误无状态码）；界面只见 `auth / rate_limit / timeout / context_overflow` 等错误码。
- 上下文占用 = 最近一次主对话请求 `input + output`，每轮覆盖更新；历史超过模型窗口 70% 触发压缩（会话模型生成摘要，落 `context_summary` 事件，失败不阻断）。
- 系统提示词三层缓存：identity（无缓存标记）/ stable（跨会话一致）/ dynamic（会话内一致），Anthropic 侧映射为前缀缓存断点。

## 7. 架构决策摘要

完整背景见 git 历史中的原 ADR 与迁移计划（已归档）；以下为仍约束当前开发的决策摘要。

| # | 决策 | 要点 |
| --- | --- | --- |
| ADR-0001 | 新建会话延迟落库与草稿槽 | 新建只是渲染层草稿，首发才原子落库；侧栏与数据库天然无空会话；Run 启动失败不回滚（用户消息先落事件表是仓库原则） |
| ADR-0002 | 采用 Tauri 2 重构桌面外壳 | Electron → Tauri 2：主进程领域逻辑 Rust 重写，渲染层原样保留；数据层双向兼容可回滚；连带决策：移除终端、Agent 自研、craft- 前缀 |
| D1 | 移除终端功能 | 不引入 PTY/xterm；`shell_exec` 保留命令执行能力；WKWebView 中文 IME 风险与 node-pty 跨平台打包障碍一并消除；**不要重新引入 PTY 相关代码** |
| D2 | Agent 侧完全自研 | 参考 openai/codex 的设计与工程拆分（非代码复制）；不引入 rig / genai / goose；基础设施 crate（tokio、rusqlite、reqwest 等）仍用成熟第三方库 |
| D3 | crate 命名 craft- 前缀 | 无前缀的 `core` 遮蔽 Rust 内置 crate，`::core::` 绝对路径连带 derive 宏失效（实测 E0432），硬冲突无规避价值 |
| D4 | 右侧面板保留「文件 / 文档」两个标签 | 终端标签移除后结构不动，默认标签为「文件」 |
| D5 | MCP（stdio 客户端）留桩 | 设置页可存服务器配置，连接能力不实现；rmcp 接入单独立项 |
| D6 | 先出未签名包；updater 链路未接线 | tauri-plugin-updater 未引入、无签名公钥，仅设置字段留存；凭据到位后另立项接线 |
| D7 | 仅保障 macOS 实测 | Win / Linux 实机走查单独立项（D1 后打包障碍已消失，配置已备） |

## 8. 已知留桩与限制

- 自动更新：updater 未接线（D6）。
- 沙箱与 MCP：设置可存配置，连接留桩（D5）。
- Work 模式：Word 只支持同一格式片段内文本替换；Excel 不计算公式；PDF 只读文本层（无 OCR / 编辑）。
- 会话删除为物理删除且对运行中 / 等待审批的会话无守卫（GLOSSARY 已知限制）。
- 跨平台：NSIS / AppImage 配置已备，Windows / Linux 实机走查待排期。
