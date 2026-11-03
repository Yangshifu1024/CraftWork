# CraftWork 架构规格

> **形态变更声明（2026-11-04）**：桌面外壳已重构为 TypeScript + Electron（主进程承载领域内核，渲染层 React）。
> 当前形态的分层、命令与实现约束以仓库根 `README.md` 与 `AGENTS.md` 为准；
> 本文件其余部分描述的是迁移前的内核契约（事件流、Agent 循环、策略与存储语义），其中与形态无关的语义（事件契约、审批状态机、风险规则、存储 schema）仍然有效，形态相关的描述（Go 包、mygo 宿主、原生 UI）仅作历史参照。

本文件描述系统的架构契约：分层、事件流、Agent 内核、策略、模型接入、存储与安全机制。决策背景见 `docs/adr/`（原生 UI 迁移见 ADR-0014 与 `docs/native-ui/spec.md`）；功能验收见 `features.md`；实现约束与命令见仓库根 `AGENTS.md`。

状态标注约定：未特别标注的能力均为当前已实现；「留桩」表示代码占位已建、能力未接入（M4：沙箱、自动更新下载替换、Streamable HTTP MCP、MCP 信任列表）。

## 1. 分层与进程模型

```text
┌──────────────────────────────────────────────────────────────┐
│ 原生界面（internal/ui，mygo 立即模式 view）                    │
│  shell 三栏外壳 / sidebar / chat / panel / overlays / theme   │
│  view 每帧直接调用 Go 函数构建界面，GPU 自绘，无 WebView        │
├──────────────────────────────────────────────────────────────┤
│ mygo 宿主（v0.7.4）                                           │
│  窗口 / 单实例 / 事件循环 / 通知 / 对话框 / 剪贴板 / 终端       │
│  后台结果经 win.Update 回主线程；bus 订阅回调派发界面更新        │
├──────────────────────────────────────────────────────────────┤
│ 应用领域层（Go）                                              │
│  service（内部领域包）→ agent / tools / policy / model / mcp  │
│  → editor / doc / notify / store / secret                    │
├──────────────────────────────────────────────────────────────┤
│ 基础设施                                                      │
│  SQLite（WAL）/ 文件系统 / PTY（libghostty）/ keys.json / 备份 │
└──────────────────────────────────────────────────────────────┘
```

依赖方向自上而下。**没有 IPC**：界面与领域层在同一进程，view 直调普通 Go 方法（原 Wails 绑定层随迁移降级为内部领域包，`internal/service` 保留参数校验、编排与错误转换职责）。所有路径校验、命令分级、密钥读取都在 Go 侧完成；界面状态是数据库事件的投影。

进程模型：单实例（第二实例启动时回调里显式 `Restore/Show/Focus` 唤起主窗口）；每个会话最多一个 run goroutine；事件总线订阅回调把批次投给 `win.Update`（主线程执行并自动合并到下一帧）；终端由 `plugins/terminal`（libghostty）自管 PTY 读取协程；工具执行、模型请求都在独立 goroutine，崩溃隔离到单次 run。

## 2. 目录与包职责

```text
main.go                  入口：装配交给 internal/app
internal/app/            进程装配：窗口、单实例、生命周期、服务装配、
                         通知与跳会话接线；端到端冒烟测试（app_e2e_test.go）
internal/ui/             原生界面：theme（token 转发）、shell（三栏外壳与
                         窗口级快捷键 / Esc 退出链）、panelview / panel_diff /
                         panel_editor / panel_files / panel_docs / panel_terminal
internal/ui/theme/       base.css token 的 Go 翻译、深浅主题派生（叶子包）
internal/ui/icons/       lucide SVG 登记表（go:embed）
internal/ui/sidebar/     侧栏：模式切换、会话分组列表、工作区卡片
internal/ui/chat/        聊天列：reducer（事件归约）、composer、部件卡片、
                         markdown 流式渲染、快照分页
internal/ui/panel/       面板 tab 状态机（工作区分桶、singleton tab）
internal/ui/overlays/    浮层体系：命令面板、设置、数据、创建项目、
                         确认 / 输入 / 自由档位对话框、toast
internal/service/        内部领域包（原 Wails 绑定层）：App / Settings /
                         Thread / Chat / Workspace / Provider / Audit /
                         Document / MCP
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
internal/editor/         行级 diff 与差异视图（LCS + 上下文分组）
internal/doc/            格式注册表与 OOXML / PDF 适配器
internal/events/         事件类型表与内部事件总线（契约保留）
internal/notify/         系统通知（mygo.NewNotification，点击跳会话）
internal/update/         更新（版本、渠道、清单校验已实现；下载替换留桩）
internal/version/        构建信息（版本对齐策略见包注释）
assets/                  应用图标（appicon.png，mygo.json 的 icon 字段指向这里）
```

新增能力优先落到对应领域包，`internal/service` 只做参数校验、编排与错误转换；view 不得绕过 service 直接操作 store 或 agent。

## 3. mygo 宿主

- 应用配置：`mygo.json`（name / identifier / version / icon / out）；图标 `assets/appicon.png` 构建期生成 `.icns` 与 DMG。
- 窗口：`1440×900`，最小 `960×640`；`TitleBarHiddenInset` 隐藏标题栏，界面自画 48px 顶行拖拽区并避让红绿灯；底色按设置主题在首帧前设定，避免白闪；`StateKey` 记忆窗口位置与尺寸。
- 界面内容：`ui.View(cwui.RootView(shell))` 纯 Go view；窗口没有页面可加载，应用内不出现任何 WebView。
- 外链唯一出口是 `mygo.Shell.OpenExternal`（仅 http/https，界面先确认域名）；应用不加载任何远程页面。
- 生命周期：单实例锁 → 数据目录（含 `shell/` 窗口状态子目录）→ 打开数据库 → 清理未完成 run → 读密钥 → 建事件总线 → 装配策略 / 工具 / 服务 → 恢复审批档位与外观 → 确保内置临时会话目录存在并把未绑定工作区的会话回填到该目录 → 建窗口 → 侧栏与聊天列启动首拉。临时目录固定在数据目录 `workspace/default`（默认 `~/.craftwork/workspace/default`），删除后启动自动重建；初始化失败只降级临时会话能力，不阻断启动。活动工作区只是 UI 偏好，不写入任何策略状态。
- 崩溃恢复：启动时把 `running` 的 run 标记 `interrupted`，挂起审批标记 `canceled`，对应会话标 `error` 并写入可重试的 `error` 事件。
- 退出：`OnWillQuit` 关闭事件总线；窗口关闭回收面板终端进程（SIGHUP + 2 秒补杀）。

## 4. 事件流契约

**没有 IPC 与绑定**：界面读状态走方法调用；领域层到界面的通知走「落库 + bus 发布 + 主线程派发」（ADR-0007 的类型表保留为内部契约）。

```text
领域层 → record.Recorder（事务落库）→ events.Bus（发布）
       → app.emitter（win.Update 派发）→ chat reducer（主线程状态写入）
```

### 4.1 批次载荷

```go
type Batch struct {
    V        int
    ThreadID string
    Events   []*Envelope // Seq 会话内单调递增
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
| `user.message` | `messageId`、`text`、`attachments`、`via`；仅运行中引导广播（普通发送由聊天列乐观插入显示，不广播避免双份） |
| `error` | `partId`、`code`、`message`、`retryable` |
| `run.completed` / `run.canceled` / `run.failed` | `status`、可选 `code` / `message` |
| `thread.updated` | `threadId`、`status`、可选 `thread` |
| `usage.updated` | `inputTokens`、`outputTokens`、`contextTokens` 等 |
| `settings.updated` / `workspace.updated` | 变更摘要 |
| `context_summary` | 摘要与覆盖截止序号；持久化用，不进界面、不广播 |
| `terminal.output` / `terminal.exit` | 契约保留（TerminalService 已随迁移删除；面板终端由 plugins/terminal 自管 PTY，不产生这两类事件） |

### 4.3 派发、合并与取消

- 事件写入与广播统一经 `record.Recorder`；部件事件按 Kind 映射类型表发布，生命周期事件 `Emit` 只广播不落库。
- 后台批次经 `win.Update(func)` 投递：mygo 自动合并到下一帧，界面按帧消费，无需节流。
- 消费端按 `seq` 去重；切换会话用快照替换，不重放事件；其他会话有变更时联动侧栏刷新。
- 大对象（文件内容、差异、终端历史）按需调用方法获取，不进事件。
- 取消：`ChatService.Cancel` → `Runner.Cancel` → `context.CancelFunc`；取消等待中的审批与会话状态回收都由 `Runner.Cancel` 负责。运行记录的创建与会话状态切换同样收口在 Runner（`Runner.Run` / `createRunRow`）：服务层只写用户消息并触发，正常发送与接力运行共用同一写入序列。`shell_exec` 取消杀整个进程组（`Setpgid` + `SIGKILL`）；关窗或退出时审批按 `canceled` 落库。
- 事件类型以 `internal/events/events.go` 为唯一事实源；聊天列 reducer（`internal/ui/chat/reducer.go`）是契约的消费端，两者一起改。

## 5. 原生界面架构

- **立即模式**：view 是每帧执行的普通函数，状态放 app 结构体（`Shell` / `Sidebar` / `Chat` / `Pane` / overlays 单例）；没有虚拟 DOM，元素身份靠兄弟位置或 `Key` 保持。
- **主线程约定**：界面状态只在主线程（view 帧或 `win.Update` 回调）读写，不放锁；后台 goroutine 一律经 `win.Update` 回主线程。
- **组合根**：`RootView(shell)` 每帧依次画外壳、处理窗口级快捷键（⌘K/⌘N/⌘B/⌘W/⌘,/Esc 退出链）、画浮层。模态（DialogBase）打开时窗口快捷键整体等待，Esc 归浮层。
- **Esc 退出链**：浮层模态（命令面板 / 设置 / 数据 / 图片预览 / 全局对话框）在上层自消费 Esc；剩余层级由 `Shell.HandleWindowEscape` 统一——面板激活 tab（焦点在面板可编辑区时整链跳过：焦点持有 `FocusWithin` 时根视图不注册 Esc，保证终端 / 编辑器把 Esc 当输入）→ 聊天列两级（运行中双击 Esc 停止，800ms 确认窗；空闲待审批 Esc 拒绝最后一条，范围 once）。IME 组合期按键以 TextComposition 直送焦点控件，不产生 keyDown，天然到不了快捷键。
- **聊天列**：`reducer.go` 逐事件归约（23 种事件 handler，纯函数可单测）；流式正文 `markdown.Stream`、完成态 `markdown.View`；部件卡片（思考 / 工具 / 审批 / 待办 / 错误）与「已工作」折叠按 work 单表语义切段。
- **面板**：`Pane` 按「工作区桶 × singleton tab」管理（文件树 / 编辑器 / 差异 / 终端 / 文档）；tab 按「工作区 + 线程」过滤显隐；窄窗（<1100）自动收起，Split 拖拽 320–760、双击恢复默认宽。
- **浮层体系**：`overlays` 包级单例 + Backend 服务门面 + Hooks 跨视图回调；同类请求同时最多一个，新请求覆盖旧请求（旧回调按取消收尾）。
- **主题**：`theme.Apply` 每帧套用；深浅色实时跟随系统，显式主题覆盖；动效走 `Animate`/`Transition`（自动跟随系统减少动效偏好），装饰性 Loop 在 `theme.ReduceMotion` 时停用。

## 6. Agent 内核

### 6.1 Run 生命周期

1. `ChatService.Send` 写入 `user_message` 事件，创建 `runs` 记录；会话状态置 `running`。
2. `Runner.execute` 从事件表重建历史，进入循环：调用模型流式接口 → 文本/推理增量推事件 → 工具调用走「策略评估 → 审批 → 执行 → 结果回填」→ 无工具调用则结束。
3. 完成更新 `runs.status = done`、会话 `idle`、记录用量，广播 `run.completed`；取消与失败分别广播 `run.canceled` / `run.failed` 并写 `error` 部件。

- 模型调用往返上限可配置（`maxIterations`，默认 300，0 不限制）；单次模型请求超时 180 秒。
- 一个 run 的文本与工具调用合并为一条 assistant 消息；工具结果作为独立 `tool` 消息回填；推理内容不回传模型。
- 事件写入与广播统一经 `record.Recorder`；run 生命周期状态以 `runs` 表为准。运行正常结束后若仍有排队中的运行中引导，Runner 自动接力开启新 run（见 6.4）。
- **RunEnv 值化**：运行开始时由 Runner 从会话绑定的工作区一次性解析 `agent.RunEnv{WorkspacePath, Roots, Restricted, Purpose}`（Roots 是全部源文件夹，第一个为主要目录），沿 executeRun → history/prompt → 工具 Assess/Execute 与审批门控作为值下发；策略引擎、工具注册表与设置中都不保存工作区副本，并行运行互不覆盖，终端等 UI 动作按激活工作区显式解析。

### 6.2 上下文与历史

- 系统提示词分节组装与缓存分层见 `prompt-contract.md` 与 ADR-0013。
- 历史从事件表物化；压缩摘要（历史超过模型窗口 70% 触发）以 `context_summary` 事件持久化，重建时按覆盖截止序号裁剪并注入摘要 user 消息；摘要失败保留原始历史继续本轮。
- 附件：文件选择路径经 `policy.Resolve` 校验；粘贴内容由 `SavePastedAttachment` base64 写入 `attachments/`；文本限 1MB、图片限 5MB、单条最多 10 个、总计 12MB；`.env`、密钥与不支持的二进制不读取；附件内容带边界标记，按不可信数据处理。

### 6.4 运行中引导（steer）

`ChatService.Steer(threadID, text, attachments)` 在会话运行中追加一条用户指令：

1. 与 `Send` 共用消息落库路径（`user_message` 事件，原文先持久化），随后广播 `user.message` 事件让所有窗口立即显示（聊天列按 `messageId` 幂等去重）。
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
- 路径解析 `policy.Resolve`：工作区 `EvalSymlinks` → 以工作区为根拼接 → 词法校验越界 → `EvalSymlinks` 后二次校验阻止符号链接逃逸 → 新文件解析父目录。拒绝路径穿越、符号链接逃逸、UNC 路径；界面传入的路径不是可信输入。
- 多根解析 `policy.ResolveAny` / `ResolveExisting`：按根顺序（主要目录优先）逐根做同样的词法与符号链接校验；主要目录不可用时报错，附加目录不可用时跳过；「存在优先」变体在目标真实存在于某个根时命中该根，全部不存在时回落词法判定（新建文件落主要目录）。链接逃逸直接拒绝，不尝试后续根。
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
- 统一错误码：`auth`、`rate_limit`、`timeout`、`context_overflow`、`provider_unavailable`、`invalid_request`、`canceled`；不向界面抛原始 HTTP 错误。
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

- **文档**（ADR-0010）：`doc.Format` 适配器（`Exts / Read / Replace`）承担格式差异；Word 解析 `word/document.xml`，替换要求完整位于同一格式片段内；Excel 解析工作簿、关系、共享字符串与工作表 XML，写 inlineStr 或数值节点；PDF 只读文本层（FlateDecode 与字面量/十六进制字符串），加密拒绝、无文本层提示不支持 OCR。写入前由工具层备份并走审批；文档预览用 `doc.Supported` 判断是否走文档解析。
- **终端**：`plugins/terminal`（libghostty）惰性创建，tab 首帧起用户 shell；PTY 网格尺寸随每次 paint 协商（layout → viewResize），视图 `Fill()` 即可；深浅两套主题按窗口外观选择；窗口关闭 / 应用退出回收终端进程（SIGHUP，插件 2 秒后补杀）。用户终端不经审批，`shell_exec` 走策略引擎，两条路径独立。
- **编辑器**：`ui/editor`（行号、语法高亮、只读/编辑切换、IME）；差异在 Go 侧计算（LCS + 3 行上下文，超过 20,000 行或 4,000,000 格子退化为整段替换），`editor.DiffView` 渲染统一/并排视图与词级高亮；保存前备份；`expectedHash` 阻止外部修改覆盖；二进制与超过 2MB 的截断文件禁止编辑。

## 13. 平台集成

| 能力 | 实现 |
| --- | --- |
| 文件对话框 | `mygo.Dialog`（打开 / 保存 / 消息），在后台 goroutine 调用（帧内直调会死锁） |
| 系统通知 | `internal/notify`（`mygo.NewNotification`），窗口失焦时发送；未打包运行或权限被拒静默降级；点击聚焦窗口并跳会话 |
| 剪贴板 | `mygo.Clipboard`；经 `win.Update` / goroutine 调用（帧内直调会死锁） |
| 拖放 | 元素 `DroppedFiles`；附件内容由 Go 读取 |
| 单实例 | `mygo.App.RequestSingleInstanceLock` + `OnSecondInstance` 回调显式唤起 |
| 窗口状态 | 位置与尺寸由 `StateKey` 记忆到 `~/.craftwork/shell/`；主题、最后工作区持久化到 settings |
| 外部链接 | `mygo.Shell.OpenExternal`，仅 http/https，界面先确认域名 |
| 深浅色 | `ui.Theme` 实时跟随系统；显式主题经 `theme.SetAppearance` 覆盖 |

## 14. 安全设计汇总

| 面 | 措施 |
| --- | --- |
| 界面 | 无 WebView：不加载任何远程页面；Markdown 不渲染原始 HTML；图片不加载远程地址 |
| 导航 | 外链只走系统浏览器（`OpenExternal` 唯一出口）；应用内无第三方页面 |
| 路径 | 词法校验 + 符号链接解析；越界提升 R3 |
| 命令 | 分类与危险模式；清洗环境变量；进程组清理 |
| 密钥 | keys.json 0600/0700；只存 Go 进程；界面只拿遮罩与来源（ADR-0005） |
| 提示注入 | 工具输出、文件、文档、邮件、MCP 描述视为不可信；策略独立于模型（ADR-0003） |
| 文档 | 不执行宏、脚本、外链；写入前备份；每次审批（ADR-0010） |
| 审计 | 工具、审批、写入、配置变更落库 |
| 日志 | 脱敏；不记录密钥、令牌、完整对话 |
| 通知 | 只在窗口失焦时发送；内容为状态短语，不含密钥与对话正文 |
| 沙箱 | macOS Seatbelt / Windows 受限令牌（留桩）；当前依赖审批与路径校验 |
| 更新 | 清单签名、SHA-256 校验已实现；下载、原子替换、回滚与内置公钥留桩（需发布服务器与签名密钥） |

## 15. 性能设计

| 场景 | 手段 |
| --- | --- |
| 冷启动 | Go 直接自绘首帧（无 WebView 初始化）；窗口底色首帧前设定避免白闪 |
| 流式渲染 | 事件经 `win.Update` 按帧合并；聊天列只更新目标部件；`markdown.Stream` 词级 reveal |
| 大文本 | `ui/editor` 按需渲染；8MB 读取上限；二进制只返回元信息 |
| 文件树 | 逐层加载；忽略依赖目录；过滤时扁平化匹配 |
| 终端 | libghostty GPU 渲染；PTY 网格按视图尺寸协商 |
| 数据库 | WAL、索引、批量事务、分页 |
| 动效 | Animate / Transition 只在值变化时画帧；窗口不可见时不画帧 |
| 列表 | 侧栏分页（每组默认条数 +「显示更多」）；面板长列表用 `ui.List` 虚拟化 |

## 16. 测试策略

| 层级 | 范围 | 工具 |
| --- | --- | --- |
| Go 单元 | 路径逃逸、风险分级、事件归约（chat reducer）、diff、SQLite 迁移与快照、提示词契约 | `go test ./...` |
| Go 界面 | `ui.Tester` 无窗口驱动真实 view：审批矩阵、Esc 退出链、tab 状态机、主题切换、拖拽钳制 | `go test ./...` |
| Go 端到端 | 装配链冒烟（`internal/app/app_e2e_test.go`）：假模型端点 → 发消息 → 工具 → 审批 → 审计 | `go test` |
| Go 集成 | 假模型 + 临时工作区 + 临时数据库，覆盖审批允许/拒绝、工具不执行、审计 | `go test` |
| 桌面冒烟 | `go tool mygo dev` 隔离目录全链路走查（提供商 → 消息 → 工具 → 审批 → 差异 → 终端 → 重启恢复） | 手动（`CRAFTWORK_HOME` 隔离） |
| 安全用例 | 路径穿越、符号链接、命令注入、外链拦截、HTML 不执行 | 专项用例 |
| 性能基准 | 冷启动、流式帧率、10MB 文件、终端输出、数据库查询 | `testing.B`、benchstat |

关键回归用例：审批拒绝后工具不执行；R3/R4 无永久授权；文档写入前备份；取消后无残留进程；导出包不含密钥；Markdown 中的 `<script>` 不执行；焦点在终端时 Esc 不关闭面板 tab。

## 17. 构建与发布

- 开发：`go tool mygo dev`（编译失败保留旧进程）；界面无 HMR，改完即重启。
- 生产：`go tool mygo build` → `bin/darwin-<arch>/CraftWork.app` + DMG；无 cgo，可交叉编译（macOS 签名与 DMG 仅在 macOS 上执行）。
- 签名与公证：`mygo build` 内置 Developer ID 签名与公证流程；需要开发者证书与发布密钥（未配置时 adhoc 签名、跳过公证）。
- 版本：`mygo.json` 的 `version` 是发布产物版本事实源，`internal/version` 默认值与之对齐（规则见包注释）。
- 更新：`internal/update` 已实现版本、渠道、HTTPS 地址、SHA-256 与 Ed25519 签名校验；下载、原子替换、回滚与内置公钥留桩（需发布服务器与签名密钥；接线目标为 mygo 内置 Updater）。
- 产物目标与发布准入见 `product.md` 第 6.5 节与第 8 节。
