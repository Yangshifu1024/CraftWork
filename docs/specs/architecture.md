# CraftWork 架构规格

本文件描述系统的架构契约：分层、IPC、Agent 内核、策略、模型接入、存储与安全机制。决策背景见 `docs/adr/`；功能验收见 `features.md`；实现约束与命令见仓库根 `AGENTS.md`。

状态标注约定：未特别标注的能力均为当前已实现；「留桩」表示代码占位已建、能力未接入（M4：沙箱、自动更新下载替换、Streamable HTTP MCP、MCP 信任列表）。

## 1. 分层与进程模型

```text
┌──────────────────────────────────────────────────────────────┐
│ 前端（React + TypeScript）                                    │
│  三栏布局 / 聊天 / 面板 / 浮层 / 主题 / 键盘                   │
│  无系统访问权限，所有能力经绑定与事件访问                       │
├──────────────────────────────────────────────────────────────┤
│ IPC（Wails3 绑定 + 事件通道）                                  │
│  命令：类型化方法调用，请求-响应                                │
│  事件：cw:events 批量推送，33ms 合并                            │
├──────────────────────────────────────────────────────────────┤
│ Wails3 宿主层（Go）                                           │
│  窗口 / 单实例 / WebView / 资源服务器 / CSP / 生命周期          │
├──────────────────────────────────────────────────────────────┤
│ 应用服务层（Go）                                              │
│  service（绑定层）→ agent / tools / policy / model / mcp       │
│  workspace / doc / terminal / editor / store / secret          │
├──────────────────────────────────────────────────────────────┤
│ 基础设施                                                      │
│  SQLite（WAL）/ 文件系统 / PTY / keys.json / 备份目录           │
└──────────────────────────────────────────────────────────────┘
```

依赖方向自上而下。前端不直接访问数据库或文件系统；所有路径校验、命令分级、密钥读取都在 Go 侧完成。

进程模型：单实例（第二实例启动时回调里显式 `Restore/Show/Focus` 唤起主窗口）；每个会话最多一个 run goroutine；事件总线冲刷协程（33ms 节拍）；每个 PTY 一个读取协程；工具执行、模型请求、终端读取都在独立 goroutine，崩溃隔离到单次 run 或单个终端标签；WebView 崩溃不会带崩 Go 进程。

同一份代码可用 `-tags server` 编译为纯 HTTP 服务（无窗口，浏览器运行完整应用）；窗口、托盘、原生菜单、原生对话框、单实例在 server 模式是空实现，Agent、终端、文件、SQLite 等后端能力完整可用。

## 2. 目录与包职责

```text
main.go                  入口：embed frontend/dist，交给 internal/app
internal/app/            窗口、单实例、CSP、服务装配、关闭钩子
internal/service/        Wails 绑定层（App / Settings / Thread / Chat /
                         Workspace / Provider / Audit / Terminal / Document）
internal/agent/          Run 循环、审批状态机、授权缓存、提示词组装（prompt/）、
                         环境收集（envcontext）、历史重建（history）
internal/record/         事件写入与广播（落库 + 推送的唯一出口）
internal/platform/       原子写、哈希、环境清洗、子进程执行
internal/tools/          fs_* / shell_exec / git_* / todo_write / doc_*
internal/policy/         风险分级、审批档位、路径解析、命令分类
internal/model/          统一 Client、用户提供商目录、模型引用、OpenAI / Anthropic 适配器
internal/model/catalogdata/  models.dev 目录快照（构建期生成，运行时只读）
cmd/modelsync/           从 models.dev 同步并生成目录快照
internal/store/          SQLite、迁移、事件、审计、用量、设置、种子数据
internal/secret/         keys.json 读写与权限
internal/editor/         行级 diff（LCS + 上下文分组）
internal/doc/            格式注册表与 OOXML / PDF 适配器
internal/events/         事件类型与批量总线
internal/notify/         系统通知（macOS osascript、Linux notify-send）
internal/terminal/       PTY 会话管理
internal/update/         更新（版本、渠道、清单校验已实现；下载替换留桩）
internal/version/        构建信息
frontend/
├── bindings/            Wails3 生成的 TypeScript 绑定（提交入库，不手改）
└── src/
    ├── app/             AppShell、全局快捷键
    ├── features/        sidebar / chat / panel / overlays / statusbar
    ├── components/      Icon、BrandMark、Popover、Toast、Dialog
    ├── lib/             API、真实宿主绑定、Mock（仅 vite --mode mock）、类型、工具
    ├── stores/          app / threads / panel / workspace / terminal / ui
    └── styles/          样式分区 + highlight + app-extra + desktop
```

新增能力优先落到对应领域包，`internal/service` 只做参数校验、编排与错误转换。

## 3. Wails3 宿主

- 服务注册：App / Settings / Thread / Chat / Workspace / Provider / Audit / Terminal / Document。
- 窗口：`1440×900`，最小 `960×640`；macOS 隐式标题栏 + 48px 顶行，前端为红绿灯预留 82px；深色背景避免首帧白闪；`EnableFileDrop` 打开。
- 资源：`//go:embed all:frontend/dist`；`AssetFileServerFS` 附加中间件写入 CSP（生产 `default-src 'self'`，禁内联脚本与远程资源；开发额外放行 Vite 内联与本地 WebSocket）。
- 窗口 URL 固定为 `/`，禁止绝对 URL；外链唯一出口是 `AppService.OpenExternal`（仅 http/https）。
- 生产构建禁用 DevTools（`production` build tag）。
- 生命周期：启动打开数据库 → 清理未完成 run → 读密钥 → 建事件总线 → 装配服务 → 确保内置临时会话目录存在并把未绑定工作区的会话回填到该目录 → 恢复审批档位 → 建窗口。临时目录固定在数据目录 `workspace/default`（默认 `~/.craftwork/workspace/default`），删除后启动自动重建；初始化失败只降级临时会话能力，不阻断启动。活动工作区只是 UI 偏好，不写入任何策略状态。`OnShutdown` 逆序释放。
- 崩溃恢复：启动时把 `running` 的 run 标记 `interrupted`，挂起审批标记 `canceled`，对应会话标 `error` 并写入可重试的 `error` 事件。

## 4. IPC 契约

命令走绑定方法（前端 → Go）；事件只走 `cw:events` 单一批量通道（Go → 前端，ADR-0007）。

### 4.1 批量事件载荷

```ts
interface Batch {
  v: number;          // 契约版本，当前为 1
  threadId: string;
  events: Array<{
    seq: number;      // 会话内单调递增
    type: string;
    threadId?: string;
    runId?: string;
    ts: number;       // 毫秒
    data: any;
  }>;
}
```

### 4.2 事件类型

| 类型 | 载荷要点 |
| --- | --- |
| `run.started` | `messageId`、`modelId` |
| `text.delta` / `text.completed` | `partId`、`text` |
| `reasoning.delta` / `reasoning.completed` | `partId`、`text` |
| `tool.started` / `tool.completed` | `partId`、`toolCallId`、`name`、`args`、`risk`、`status`、`output`、`durationMs` |
| `approval.requested` | `partId`、`approvalId`、`toolCallId`、`tool`、`risk`、`target`、`facts`、`args`、`allowPermanent` |
| `approval.resolved` | `partId`、`approvalId`、`status`、`decision`、`scope` |
| `todo.updated` | `partId`、`items` |
| `user.message` | `messageId`、`text`、`attachments`、`via`；仅运行中引导广播（普通发送由前端乐观插入显示，不广播避免双份） |
| `error` | `partId`、`code`、`message`、`retryable` |
| `run.completed` / `run.canceled` / `run.failed` | `status`、可选 `code` / `message` |
| `thread.updated` | `threadId`、`status`、可选 `thread` |
| `usage.updated` | `inputTokens`、`outputTokens`、`contextTokens` 等 |
| `settings.updated` / `workspace.updated` | 变更摘要 |
| `context_summary` | 摘要与覆盖截止序号；持久化用，不进界面、不广播 |
| `terminal.output` / `terminal.exit` | `sessionId`、`data` / `exitCode` |

### 4.3 合并、背压与取消

- 文本与推理增量按 33ms 合并（同一 `partId`）；缓冲 32 条立即冲刷；非增量事件立即推送保序。
- 终端输出独立合并，16KB 即推。
- 前端按 `seq` 去重；切换会话用快照替换，不重放事件。
- 大对象（文件内容、差异、终端历史）走命令按需拉取。
- 取消：`ChatService.Cancel` → `Runner.Cancel` → `context.CancelFunc`；取消等待中的审批与会话状态回收都由 `Runner.Cancel` 负责。运行记录的创建与会话状态切换同样收口在 Runner（`Runner.Run` / `createRunRow`）：服务层只写用户消息并触发，正常发送与接力运行共用同一写入序列。`shell_exec` 取消杀整个进程组（`Setpgid` + `SIGKILL`）；刷新或关窗时审批按 `canceled` 落库。
- 事件类型与载荷在 `internal/events/events.go` 与 `frontend/src/lib/types.ts` 两处保持一致；改服务方法签名后必须重新生成绑定。

## 5. 前端架构

- 运行时：`useExternalStoreRuntime({ messages, convertMessage, isRunning, onNew, onCancel, onReload })`；状态事实源在 Go，前端 store 是派生视图。
- **发送策略单一入口**：`threads.submit` 拥有完整策略（模型校验 → 图片门控 → 运行中转 steer → 自动建会话 → 发送）；runtime 退化为消息格式转换。新增发送路径（快捷指令、浮层等）必须接它。图片判断单源化为 `utils.isImagePath`，模型匹配抽成 `utils.matchModelInfo`。
- **激活工作区联动收口**：`stores/workspaceFlow.ts` 是切换 / 重同步 / 新建临时会话的唯一编排点；`FileContent` 携带 `workspaceId`，保存与差异应用显式传回打开时的工作区，不读「当前激活工作区」环境。
- **部件注册单表**：前端 `partRegistry.tsx` 单表声明「组件 + 折叠归属」，by_name 映射、WORK_KINDS 判断、分组兜底渲染全部派生；Go 侧 `agent.ToolCallPart` 单源定义 tool 部件形状（写入 / 读取共用）。新增部件走「Go 事件 + reducer handler + 注册表」三处。
- 状态归属：`app`（构建信息、设置、模式、主题、工作区、提供商）、`threads`（会话、消息部件、运行状态、待审批、附件）、`panel`（开关、宽度、全屏、模式标签）、`workspace`（文件树、Git 状态、编辑草稿、差异、文档预览）、`terminal`（会话、标签、输出缓冲）、`ui`（通知、确认、命令面板、设置、数据面板）。
- assistant-ui 部件映射：`text`/`reasoning` 用原语渲染；自定义部件以 `data-cw-*` 前缀注册到 `components.data.by_name`（`tool`、`approval`、`todo`、`error`、`draft`、`callout`、`notice`）。审批不复用 assistant-ui 的 `approval` 语义；用户消息不带 `status` 字段。
- 流式更新只修改目标消息的目标部件；重组件（面板、终端、编辑器、设置）用 `lazy()` 拆分；面板状态机为 docked / collapsed（< 1180px）/ overlay / maximized。
- 宿主判断：`isDesktop` 表示「后端在 Go 进程里」——桌面是 `wails:` / `wails.localhost`，server 模式是 http(s) 且非 mock 构建；只有 `vite --mode mock` 走 Mock。

## 6. Agent 内核

### 6.1 Run 生命周期

1. `ChatService.Send` 写入 `user_message` 事件，创建 `runs` 记录；会话状态置 `running`。
2. `Runner.execute` 从事件表重建历史，进入循环：调用模型流式接口 → 文本/推理增量推事件 → 工具调用走「策略评估 → 审批 → 执行 → 结果回填」→ 无工具调用则结束。
3. 完成更新 `runs.status = done`、会话 `idle`、记录用量，广播 `run.completed`；取消与失败分别广播 `run.canceled` / `run.failed` 并写 `error` 部件。

- 模型调用往返上限可配置（`maxIterations`，默认 300，0 不限制）；单次模型请求超时 180 秒。
- 一个 run 的文本与工具调用合并为一条 assistant 消息；工具结果作为独立 `tool` 消息回填；推理内容不回传模型。
- 事件写入与广播统一经 `record.Recorder`；run 生命周期状态以 `runs` 表为准。运行正常结束后若仍有排队中的运行中引导，Runner 自动接力开启新 run（见 6.4）。
- **RunEnv 值化**：运行开始时由 Runner 从会话绑定的工作区一次性解析 `agent.RunEnv{WorkspacePath, Restricted, Purpose}`，沿 executeRun → history/prompt → 工具 Assess/Execute 与审批门控作为值下发；策略引擎、工具注册表与设置中都不保存工作区副本，并行运行互不覆盖，终端等 UI 动作按激活工作区显式解析。

### 6.2 上下文与历史

- 系统提示词分节组装与缓存分层见 `prompt-contract.md` 与 ADR-0013。
- 历史从事件表物化；压缩摘要（历史超过模型窗口 70% 触发）以 `context_summary` 事件持久化，重建时按覆盖截止序号裁剪并注入摘要 user 消息；摘要失败保留原始历史继续本轮。
- 附件：文件选择路径经 `policy.Resolve` 校验；粘贴内容由 `SavePastedAttachment` base64 写入 `attachments/`；文本限 1MB、图片限 5MB、单条最多 10 个、总计 12MB；`.env`、密钥与不支持的二进制不读取；附件内容带边界标记，按不可信数据处理。

### 6.4 运行中引导（steer）

`ChatService.Steer(threadID, text, attachments)` 在会话运行中追加一条用户指令：

1. 与 `Send` 共用消息落库路径（`user_message` 事件，原文先持久化），随后广播 `user.message` 事件让所有窗口立即显示（前端不做乐观插入，按 `messageId` 幂等去重）。
2. 消息挂入 Runner 的每会话内存投递队列（上限 10 条，超出明确拒绝）。入队以在途运行登记为准入条件，并与运行收尾在同一临界区注销：`TrySteer` 返回 false 即会话没有在途 run，服务层回退为普通发送。
3. 投递点在每轮迭代顶部（上一批工具执行完、下一次模型请求前），按消息 ID 去重后并入模型历史；同一批多个引导按到达顺序逐条并入。
4. 运行正常结束而队列非空时，Runner 自动创建接力 run 继续执行（同一 goroutine 与取消上下文，自动命名不重复触发，工时独立统计）。引导在 Steer 时已落库，接力 run 的历史重建会原样恢复，投递去重保证不重复发送。
5. 取消、失败与达到上限时未投递的引导随收尾丢弃，消息保留在会话记录中，随下一次运行的模型历史生效。
6. 已知取舍：不拆散工具批次注入（tool_call 与 tool_result 必须配对）、不做流中注入（无协议支持）；长工具执行期间的立即干预由双击 Esc 取消承担。回放语义上引导消息在快照中的位置取决于落库时机，可能与直播时的历史顺序略有差异，对话语义不受影响。

### 6.5 上下文占用与输出速度口径

- 上下文占用 = 最近一次主对话请求的 `input + output` token，每轮覆盖更新；run 中经 `usage.updated` 的 `contextTokens` 实时刷新。
- sidecar 调用（自动标题等）计入累计与费用，但 `usage.updated` 不带 `contextTokens`，避免压低水位。
- 会话累计 tokens（`threads.tokens`）保持累加语义，与占用口径分离。
- 输出速度 run 中按流式字符估算，轮次完成后显示真实值（`output ÷ 耗时`，耗时落 usage 表）。

## 7. 工具系统

```go
type Tool interface {
    Name() string
    Description() string
    Schema() map[string]any
    Execute(ctx context.Context, args map[string]any) Result
}
```

- 风险等级、路径参数、命令参数、Work 模式与审批标题/摘要由 `tools.Spec` 一处声明；`Registry.Assess` 以 `RunEnv` 为首参解析目标并给出策略结论（注册表不保存工作区状态）；`Registry.Execute` 统一处理路径校验、输出截断与错误码；写前备份调用 `Call.Backup`。
- 内置工具与风险：`fs_read` / `fs_list` / `fs_glob` / `fs_grep`（R0）；`fs_write` / `fs_edit` / `apply_patch`（R1，备份 + 原子写）；`shell_exec`（R2/R3/R4，进程组、超时、环境清洗、32KB 截断）；`git_status` / `git_diff` / `git_log`（R0）；`git_commit`（R1，永不推送）；`todo_write`（R0）；`doc_read`（R0）/ `doc_edit` / `xlsx_set_cell`（R1）。
- `apply_patch` 解析统一 diff，支持 `dry_run` 预演与逐块应用；上下文不匹配时拒绝。
- `shell_exec` 与 MCP 子进程共用 `platform.SanitizedEnv`（移除 `*_API_KEY`、`*_TOKEN`、`*_SECRET`、`*_PASSWORD`、`AWS_*`、`GITHUB_*`、`OPENAI_*` 等）。
- 大输出写入本地文件并返回引用；二进制只返回类型、大小与 SHA-256。

## 8. 策略与路径安全

- 风险分级与档位见 ADR-0003；`policy.Engine` 只持有审批档位（真实全局设置），`Action` / `AllowsPermanent` 显式接收 `restricted` 参数，工作区状态来自 `RunEnv` 值传递而非全局副本。自由档位一律 `allow`（优先于只读、严格与受限判断），但 `AllowsPermanent` 仍为 false。
- 受限工作区：R3 直接拒绝；R1/R2 逐次审批；不提供永久授权，已有授权不生效。
- 路径解析 `policy.Resolve`：工作区 `EvalSymlinks` → 以工作区为根拼接 → 词法校验越界 → `EvalSymlinks` 后二次校验阻止符号链接逃逸 → 新文件解析父目录。拒绝路径穿越、符号链接逃逸、UNC 路径；前端传入的路径不是可信输入。
- 命令分类：危险命令 R4；联网命令 R3；普通命令 R2；解析失败默认询问或拒绝。
- Shell 前缀授权：取前两个片段（第二段是选项时只取第一段）；含管道、重定向、串联或 `sudo` / `bash -c` 包装的命令只允许精确匹配一次。
- 审计：每次工具调用写 `audit_log`（时间、会话、run、工具、风险、决定、范围、详情）；审批决定同时写 `approvals` 与 `audit_log`；审计面板读最近 200 条、可按决定过滤。

## 9. 模型接入

### 9.1 统一接口与协议

```go
type Client interface {
    Stream(ctx context.Context, req Request) (<-chan Chunk, error)
}
```

- 三种协议：`openai`（Chat Completions + SSE）、`anthropic`（Messages + SSE）、`openai-compatible`；自研 HTTP + SSE 客户端。Anthropic 在历史尾部追加缓存断点（合计三个断点，在提供商四断点限额内）。
- 统一错误码：`auth`、`rate_limit`、`timeout`、`context_overflow`、`provider_unavailable`、`invalid_request`、`canceled`；不向前端抛原始 HTTP 错误。
- 重试：429、5xx、超时与连接失败在建立流之前最多重试 3 次（指数退避）；失败转移按设置切换一次，仅限上述情形；工具调用过程中不转移，流建立后的错误直接失败并保留已产生内容。
- 密钥优先级：`keys.json` > 目录给出的环境变量 > 按提供商 ID 推导的环境变量（ADR-0005）。

### 9.2 模型引用与目录

- 模型引用统一为 `providerID/modelID`（`model.Ref` / `FindModelByRef` 精确解析）；`model.Resolve` 的裸 ID 回退仅兼容旧数据；解析失败返回空，不回退默认模型。
- 目录快照由 `cmd/modelsync` 从 models.dev 生成（协议映射、端点覆盖与排除规则见 `catalogdata/rules.json`）；运行时 `//go:embed` 只读；应用不内置任何提供商（ADR-0008）。
- 价格单位美元 / 百万 token，写入 `usage.cost_micros`，界面标注「估算，非账单」。

### 9.3 会话标题

- 新会话 `title_auto = 1`，标题「新对话」；首条消息清洗后 ≤ 20 字符直接用原文；否则并行调用本次 run 实际使用的模型生成（8 秒超时、最多 128 输出 token）；失败回退首条消息提取（纯附件用文件名）。
- 写回带 `title_auto = 1 AND title = '新对话'` 条件，不覆盖手动改名；标题调用量照常计入用量。

## 10. MCP

- 传输：stdio 已实现（一行一个 JSON-RPC 消息）；Streamable HTTP 留桩。
- 生命周期：启动 → `initialize` → `tools/list` → 注册动态工具 → 关闭；健康检查与退避重连留桩。
- 工具命名 `mcp__服务器__工具`；`tools/call` 结果按不可信数据处理，调用走 R2 审批。
- 服务器配置在 `settings.mcp.servers`；启动时自动连接已启用服务器；子进程环境经清洗。
- 远程传输、服务器信任列表与工具级授权细化留桩。

## 11. 本地存储

SQLite（WAL），`busy_timeout=5000`，最大连接 4；迁移用 `PRAGMA user_version` 事务内逐版执行（v2 为会话增加 `title_auto`，v5 为工作区增加 `purpose`），失败拒绝启动并提示备份路径。

```sql
CREATE TABLE threads (
  id TEXT PRIMARY KEY, mode TEXT NOT NULL, title TEXT NOT NULL,
  title_auto INTEGER NOT NULL DEFAULT 0,
  model_id TEXT NOT NULL DEFAULT '', workspace_id TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'idle', archived INTEGER NOT NULL DEFAULT 0,
  tokens INTEGER NOT NULL DEFAULT 0, deleted_at INTEGER,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);

CREATE TABLE workspaces (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
  trust TEXT NOT NULL DEFAULT 'restricted', purpose TEXT NOT NULL DEFAULT 'project',
  created_at INTEGER NOT NULL, last_opened_at INTEGER NOT NULL
);

CREATE TABLE events (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, message_id TEXT NOT NULL,
  run_id TEXT, seq INTEGER NOT NULL, kind TEXT NOT NULL,
  payload TEXT NOT NULL, created_at INTEGER NOT NULL,
  UNIQUE(thread_id, seq)
);

CREATE TABLE runs (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, message_id TEXT NOT NULL,
  status TEXT NOT NULL, model_id TEXT NOT NULL DEFAULT '',
  started_at INTEGER NOT NULL, finished_at INTEGER,
  error_code TEXT, error_message TEXT
);

CREATE TABLE approvals (
  id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, run_id TEXT NOT NULL,
  tool_call_id TEXT NOT NULL, tool TEXT NOT NULL, risk TEXT NOT NULL,
  decision TEXT, scope TEXT, requested_at INTEGER NOT NULL, decided_at INTEGER
);

CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
  thread_id TEXT, run_id TEXT, tool TEXT NOT NULL, risk TEXT NOT NULL,
  decision TEXT NOT NULL, scope TEXT, detail TEXT
);

CREATE TABLE usage (
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, thread_id TEXT,
  model_id TEXT NOT NULL, input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0, cache_tokens INTEGER NOT NULL DEFAULT 0,
  reasoning_tokens INTEGER NOT NULL DEFAULT 0, cost_micros INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE backups (
  id TEXT PRIMARY KEY, path TEXT NOT NULL, backup_path TEXT NOT NULL,
  hash TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL
);
```

索引：`threads(mode, archived, updated_at)`、`events(thread_id, seq)`、`runs(thread_id, started_at)`、`approvals(thread_id, decision)`、`audit_log(ts)`、`usage(ts)`、`usage(model_id, ts)`、`backups(created_at)`。

**临时会话与内置工作区**：临时会话不是「无工作区会话」，而是绑定应用内置临时目录（数据目录 `workspace/default`）的普通会话。工作区行的 `purpose = conversation` 标记它是内置目录（全库至多一条），据此驱动展示分类与系统提示词；`project` 是用户添加的项目目录，历史数据默认 `project`。运行机制完全复用现有链路：`policy.Resolve` 边界、审批、终端与文件树都照常工作在该目录内，`purpose = conversation` 的工作区固定 `trusted` 且不可移除、不可改信任、不可作为项目重复添加。启动时幂等 ensure 该目录与记录，并把历史遗留的空 `workspace_id` 会话一次性回填到它；系统提示词的会话规则节会向模型说明该目录「不是项目仓库，仅当用户要求保存文件时才使用」。

- 事件按会话 `seq` 单调递增，事务内分配序号（ADR-0006）；快照按 `thread_id` 读事件、`message_id` 分组，附 runs 状态与待审批。
- 备份：写前备份到 `~/.craftwork/backups/<文件名>.<时间戳>`，记录 SHA-256 与大小；恢复前确认并追加审计；每日数据库备份（`VACUUM INTO`）保留 7 份。
- 外部修改用 `expectedHash` 阻止覆盖；修改表结构增加迁移版本，不改已有迁移。

## 12. 文档、终端与编辑器

- **文档**（ADR-0010）：`doc.Format` 适配器（`Exts / Read / Replace`）承担格式差异；Word 解析 `word/document.xml`，替换要求完整位于同一格式片段内；Excel 解析工作簿、关系、共享字符串与工作表 XML，写 inlineStr 或数值节点；PDF 只读文本层（FlateDecode 与字面量/十六进制字符串），加密拒绝、无文本层提示不支持 OCR。写入前由工具层备份并走审批；`DocumentService.Preview` 用 `doc.Supported` 判断是否走文档解析。
- **终端**：`creack/pty` 启动用户 shell，`TerminalService` 管理创建、写入、调整与关闭；输出批量推送，前端 xterm.js 渲染；滚动缓冲 10,000 行；搜索用 `@xterm/addon-search`；用户终端不经审批，`shell_exec` 走策略引擎，两条路径独立；尺寸同步 ResizeObserver → FitAddon → `TerminalService.Resize`。
- **编辑器**：CodeMirror 6（行号、高亮、搜索、只读/编辑切换、IME），语言按扩展名懒加载；差异在 Go 侧计算（LCS + 3 行上下文，超过 20,000 行或 4,000,000 格子退化为整段替换），前端渲染统一/并排视图；保存前备份；`expectedHash` 阻止外部修改覆盖；二进制与超过 2MB 的截断文件禁止编辑。

## 13. 平台集成

| 能力 | 实现 |
| --- | --- |
| 文件对话框 | Wails3 `app.Dialog`（server 模式空实现） |
| 系统通知 | `internal/notify`（macOS `osascript`、Linux `notify-send`），窗口失焦时发送；权限被拒静默降级 |
| 剪贴板 | 前端优先 `navigator.clipboard`，失败回落 Go |
| 拖放 | 窗口 `EnableFileDrop` + 前端遮罩；附件内容由 Go 读取 |
| 单实例 | Wails3 SingleInstance + 回调显式唤起 |
| 窗口状态 | 尺寸、位置、分栏宽度、主题、最后工作区持久化到 settings |
| 外部链接 | `AppService.OpenExternal`，仅 http/https，前端先确认域名 |

## 14. 安全设计汇总

| 面 | 措施 |
| --- | --- |
| WebView | 严格 CSP；窗口只加载 `/`；禁绝对 URL；生产禁 DevTools（ADR-0009） |
| 导航 | 外链只走系统浏览器；应用内不加载第三方页面 |
| HTML | Markdown 不渲染原始 HTML；图片不加载远程地址；用户内容按文本处理 |
| 路径 | 词法校验 + 符号链接解析；越界提升 R3 |
| 命令 | 分类与危险模式；清洗环境变量；进程组清理 |
| 密钥 | keys.json 0600/0700；只存 Go 进程；前端只拿遮罩与来源（ADR-0005） |
| 提示注入 | 工具输出、文件、文档、邮件、网页、MCP 描述视为不可信；策略独立于模型（ADR-0003） |
| 文档 | 不执行宏、脚本、外链；写入前备份；每次审批（ADR-0010） |
| 审计 | 工具、审批、写入、配置变更落库 |
| 日志 | 脱敏；不记录密钥、令牌、完整对话 |
| 沙箱 | macOS Seatbelt / Windows 受限令牌（留桩）；当前依赖审批与路径校验 |
| 更新 | 清单签名、SHA-256 校验已实现；下载、原子替换、回滚与内置公钥留桩 |

## 15. 性能设计

| 场景 | 手段 |
| --- | --- |
| 冷启动 | Go 启动与 WebView 初始化并行；首帧只渲染骨架；编辑器/终端/设置按需加载 |
| 流式渲染 | Go 33ms 合并；前端只更新目标部件；Markdown 解析可延迟 |
| 大文本 | CodeMirror 可视区域渲染；8MB 读取上限；二进制只返回元信息 |
| 文件树 | 逐层加载；忽略依赖目录；过滤时扁平化匹配 |
| 终端 | xterm 渲染 + 16KB 合并；滚动缓冲上限 |
| 数据库 | WAL、索引、批量事务、分页 |
| 内存 | 消息分页（每页 80 条）与 `content-visibility` 离屏隔离；终端输出上限 200KB；图片懒加载 |
| 首屏体积 | 主包约 267KB gzip；FilesPanel / TerminalPanel / 浮层独立分块 |

## 16. 测试策略

| 层级 | 范围 | 工具 |
| --- | --- | --- |
| Go 单元 | 路径逃逸、风险分级、事件合并、diff、SQLite 迁移与快照、提示词契约 | `go test ./...` |
| Go 集成 | 假模型 + 临时工作区 + 临时数据库，覆盖审批允许/拒绝、工具不执行、审计 | `go test` |
| 前端类型 | 绑定与组件类型 | `tsc --noEmit` |
| 前端单元 | store reducer、事件归约、diff 渲染 | Vitest |
| 浏览器端到端 | Mock 后端：布局、发送、审批、设置 | Playwright |
| 桌面 / server 冒烟 | `wails3 build` 后启动；`wails3 task run:server` + `/health` | 手动 / 脚本（建议 `CRAFTWORK_HOME` 隔离） |
| 视觉回归 | 同视口截图对比（基线按操作系统保存） | `npm run test:e2e -- --grep @screenshot` |
| 安全用例 | 路径穿越、符号链接、命令注入、外链拦截、HTML 不执行 | 专项用例 |
| 性能基准 | 冷启动、流式帧率、10MB 文件、终端输出、数据库查询 | `testing.B`、benchstat |

关键回归用例：审批拒绝后工具不执行；R3/R4 无永久授权；文档写入前备份；取消后无残留进程；导出包不含密钥；Markdown 中的 `<script>` 不执行。

## 17. 构建与发布

- 开发：`wails3 dev`（HMR + Go 热重启）；浏览器完整应用：`wails3 task run:server`；纯 UI 走查：`npm run dev`（Mock）。
- 生产：`wails3 build` → `bin/craftwork`（自动构建前端、生成绑定、`go build -tags production`）。
- 签名与公证已接入 Wails 工具链（`wails3 tool sign [--notarize]`、`--format dmg`）；需要开发者证书与发布密钥。
- 更新：`internal/update` 已实现版本、渠道、HTTPS 地址、SHA-256 与 Ed25519 签名校验；下载、原子替换、回滚与内置公钥留桩（需发布服务器与签名密钥）。
- 产物目标与发布准入见 `product.md` 第 6.5 节与第 8 节。
