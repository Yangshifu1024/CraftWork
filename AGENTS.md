# AGENTS.md

本文件是本仓库的协作与实现约定，面向在本仓库内写代码的 Agent 与开发者。

`README.md` 面向使用者与贡献者，介绍功能、架构与贡献流程；本文件回答「在这个仓库里应该怎么改、怎么验证、哪些东西不能碰」。

## Agent skills

### Issue tracker

issue 以 markdown 形式存放在仓库 `docs/<feature>/` 下（spec 为 `spec.md`，issue 为 `issues/NN-<slug>.md`）。见 `docs/agents/issue-tracker.md`。

### Triage labels

使用默认五个分诊标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`，写在 issue 文件的 `Status:` 行。见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文布局：仓库根 `GLOSSARY.md` 术语表 + `docs/adr/` 决策记录 + `docs/specs/` 行为规格。见 `docs/agents/domain.md`。

---

## 1. 项目现状

CraftWork 是一个本地优先、可审计的桌面 Agent。M0–M3 功能（对话、Code 闭环、Work 模式文本级读写）已交付，**桌面外壳已完成 mygo 原生 UI 迁移**（`docs/adr/0014-native-ui-via-mygo.md`；迁移过程与各票留档见 `docs/native-ui/`，已随 10 号票收尾）：

- mygo v0.7.4 原生 UI（`package ui`，立即模式 view，GPU 自绘，应用内没有任何 WebView）。
- Go 1.27.3 后端：Agent 循环、工具、策略、审批、SQLite、终端、密钥；`internal/service` 是内部领域包，view 直调，没有 IPC 与绑定层。
- 界面：三栏外壳（侧栏 / 对话 / 面板）、浮层体系（命令面板 / 设置 / 数据 / 确认框）、聊天流式 Markdown、卡片与审批、文件树 / 编辑器 / 差异 / 终端面板、快捷键全表与窗口级 Esc 退出链、系统通知。
- 提供商与模型完全由用户添加；目录来自 models.dev 的构建期快照，应用不内置任何提供商。
- Wails3 + React 形态已删除（`frontend/`、绑定、Playwright、server 模式不入库，历史在 git）。

### 1.1 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面外壳与界面 | mygo v0.7.4 原生 UI（`package ui`，立即模式 view；GPU 自绘，无 WebView） |
| 对话渲染 | `ui/markdown`（流式）、`ui/editor`（代码）、`editor.DiffView`（差异） |
| 终端 | `plugins/terminal`（libghostty，自管 PTY，macOS 13+） |
| 图表 / 图标 | `ui/chart`；lucide SVG 由 `ui.Icon` 渲染（go:embed） |
| 后端 | Go 1.27.3、SQLite（modernc.org/sqlite）、内部领域包（`internal/service`） |

## 2. 环境与工具链

| 工具 | 版本 | 说明 |
| --- | --- | --- |
| Go | 1.27.3+ | 以 `go.mod` 的 `go` 指令为准 |
| mygo | v0.7.4 | 是 go.mod 的 tool 依赖，统一 `go tool mygo` 按锁定版本执行，无需单独安装 CLI |
| golangci-lint | v2.15.0 | Go lint 与格式化；配置见 `.golangci.yml`；安装 `go install github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.15.0`（落在 `$(go env GOPATH)/bin`，注意本机 GOPATH 不是 `~/go`） |

规则：

- **不要擅自安装或升级工具**；需要新工具或版本时先向用户提出请求。
- 使用 PATH 或 `$(go env GOPATH)/bin` 中已配置的版本，不通过临时环境变量切换。
- **mygo 升级是显式决策**：API 未到 1.0，升级前先读 mygo 仓库 `docs/ui/migration.md`，向用户提出并更新 `go.mod` 与本文件。

### 数据目录

- 默认 `~/.craftwork`：`craftwork.db`、`keys.json`、`backups/`、`attachments/`、`shell/`（mygo 窗口状态）。
- `CRAFTWORK_HOME=/tmp/...` 可隔离数据目录（含窗口状态），测试与冒烟必须使用隔离目录。

### 版本号

`mygo.json` 的 `version` 是发布产物（.app / DMG）的版本事实源；`internal/version` 的默认值与之保持一致，发版两处同改（规则见 `internal/version/version.go` 包注释）。

### 不能提交的生成物

`bin/`、`.task/`、`.DS_Store`。

## 3. 常用命令

| 目的 | 命令 |
| --- | --- |
| 桌面开发（编译失败保留旧进程） | `go tool mygo dev` |
| 打包 .app / DMG | `go tool mygo build` → `bin/darwin-<arch>/` |
| Go 测试（含 `ui.Tester` 无窗口 UI 用例与端到端冒烟） | `go test ./...` |
| Go 静态检查 | `go vet ./...` |
| Go lint / 格式化 | `golangci-lint run`（检查）；`golangci-lint fmt`（格式化）；`golangci-lint fmt --diff`（只检查） |
| 同步模型目录 | `go run ./cmd/modelsync`（检查漂移加 `-check`；生成 `internal/model/catalogdata/catalog.json.gz`） |
| GUI 冒烟（隔离目录） | `CRAFTWORK_HOME=/tmp/cw-smoke go tool mygo dev` |
| 常用任务快捷入口 | `task build` / `task test`（需另装 go-task；等价命令直接执行即可） |

## 4. 目录与职责

```text
main.go                  入口：装配交给 internal/app
internal/app/            进程装配：窗口、单实例、生命周期、服务装配、
                         通知与跳会话接线（端到端冒烟测试也在这里）
internal/ui/             原生界面：theme（token）、shell（三栏外壳）、
                         sidebar / chat / panel / overlays（浮层）、icons；
                         view 直调内部领域包
internal/ui/theme/       base.css token 的 Go 翻译与主题派生（叶子包）
internal/service/        内部领域包：参数校验、编排与错误转换（原 Wails
                         绑定层，绑定语义已随迁移消失）
internal/agent/          Run 循环、审批状态机、授权缓存
internal/record/         事件写入与广播（落库 + 推送的唯一出口）
internal/platform/       原子写、哈希、环境清洗、子进程执行
internal/tools/          fs_* / shell_exec / git_status / todo_write
internal/policy/         风险分级、审批档位、路径解析、命令分类
internal/model/          统一 Client、用户提供商目录、模型引用、OpenAI / Anthropic 适配器
internal/model/catalogdata/  models.dev 目录快照（构建期生成，运行时只读）
cmd/modelsync/           从 models.dev 同步并生成目录快照
internal/store/          SQLite、迁移、事件、审计、用量、设置、种子数据
internal/secret/         keys.json 读写与权限
internal/editor/         行级 diff
internal/notify/         系统通知（mygo.NewNotification，点击跳会话）
internal/events/         事件类型表与内部事件总线（契约保留）
docs/                    adr（决策记录）、specs（行为规格）、native-ui（迁移留档）、agents（技能配置）
```

新增能力优先落到对应领域包；`internal/service` 只做参数校验、编排与错误转换，原生 UI 的 view 不得绕过它直接操作 store 或 agent。

## 5. 改动流程

1. **先读文档**：仓库根 `GLOSSARY.md`（术语）、`docs/specs/`（行为规格）、`docs/adr/`（决策记录）；改动与既有 ADR 冲突时先显式提出，不要静默推翻。
2. **改后端**：领域包保持普通 Go 方法；没有绑定代码生成这一步。
3. **改 UI**：遵守 6.4 节约定；观感取值来自 `internal/ui/theme`。
4. **补测试**：核心逻辑（策略、路径、事件、存储、Agent 循环、diff、UI 状态机）必须有 Go 单测；界面行为用 `ui.Tester` 用例与装配级端到端冒烟（`internal/app/app_e2e_test.go`）覆盖。
5. **跑验证**：见第 8 节。
6. **同步文档**：见第 9 节。

### 新增一个界面/能力

1. 数据与行为先落在领域包（`internal/service` 或更深的包），写成普通 Go 方法。
2. 界面放 `internal/ui/<区域>/`（shell / sidebar / chat / panel / overlays），view 函数从 app 状态构建；状态机写成纯函数便于单测。
3. 后台 goroutine 更新界面一律经 `win.Update(func(){...})`。
4. 颜色、字号、间距取自 `internal/ui/theme` 的主题常量，不写死值。
5. 图标在 `internal/ui/icons` 登记 lucide SVG 名称。
6. 补 `ui.Tester` 单测与必要的走查记录。

### 新增一个工具

1. 在 `internal/tools/` 实现 `Tool` 接口（`Spec/Execute`）。
2. 在 `Spec` 里声明风险等级、路径参数、命令参数、Work 模式与审批标题/摘要。
3. 在 `NewRegistry` 注册。
4. 路径解析、输出截断与错误码由 `Registry.Execute` 统一处理；写前备份调用 `Call.Backup`。
5. 补工具测试，走 `Registry.Assess` + `Registry.Execute` 接缝。

### 模型目录与提供商

1. 提供商与模型清单只来自 `internal/model/catalogdata/catalog.json.gz`，由 `cmd/modelsync` 从 models.dev 生成；**禁止在 Go 代码里新增内置提供商或模型清单**。
2. 同步规则在 `internal/model/catalogdata/rules.json`，只允许 `openai`、`anthropic`、`openai-compatible` 三种协议；Google / Gemini 原生协议、Azure、Bedrock、Vertex 一律排除。
3. 用户提供商保存在 `settings.model.custom_providers`；线程与设置保存 `providerID/modelID` 引用，不要只存原始模型 ID。
4. 首次启动没有默认模型；发送消息前必须由用户添加提供商并选择模型。
5. 修改目录后运行 `go run ./cmd/modelsync`，并提交生成的 gzip 与 manifest。

## 6. 实现约束（必须遵守）

### 6.1 UI 与数据流

- view 直调 Go 函数，**没有 IPC 与批量事件通道**；`internal/events` 的事件类型表保留为内部契约（`record` 落库 + bus 发布 → UI 订阅回调）。
- 后台事件到界面只经 `win.Update(func(){...})`（主线程执行并重绘，自动合并）；禁止在 view 外直接改 UI 状态。
- 大对象（文件内容、差异、终端历史）按需调用方法获取，不要常驻 app 状态。
- 界面状态机写成纯函数转移（参考移植的 threadReducer），可单测、可重放。

### 6.2 审批

- 审批必须显式决定：请求 → 卡片 → 用户决定 → 执行/跳过，**不允许 Go 侧静默放行**；决定与执行都走 `ChatService.DecideApproval` 语义。
- `once / session / workspace` 三种范围由服务端校验；`AllowPermanent` 为 false 时禁止 session/workspace。
- 受限工作区：R3 直接拒绝、不提供永久授权、已有授权不生效。
- 取消、窗口关闭、应用退出统一按拒绝或取消落审计。
- 审批决定与工具执行必须写 `audit_log`。
- 窗口级 Esc 退出链的层级次序（浮层模态 → 面板激活 tab → 聊天列两级）与消费语义由 `Shell.HandleWindowEscape` 统一，不要另起第二份分流逻辑。

### 6.3 安全

- **外链唯一出口**：http/https 经确认后走 `mygo.Shell.OpenExternal`；应用不加载任何远程页面。
- Markdown 不渲染原始 HTML（`ui/markdown` 默认按文本显示，模型常写的 `<b>/<code>` 等格式化标签除外）；图片默认只显示 alt 文本，加载需显式启用且仅限本地/受控来源。
- 所有路径校验在 Go 侧完成：`policy.Resolve` 先词法校验，再解析符号链接；不信任 UI 传来的路径。
- 密钥只存在 Go 进程与 `~/.craftwork/keys.json`；不进入日志、数据库、导出包。
- `shell_exec` 必须清洗敏感环境变量、杀进程组、限制输出。
- libghostty dylib 由 `mygo build` 打进 .app 并随应用签名（运行时不下载）；不要改用运行时网络获取。
- 系统通知只在窗口失焦时发送（开关在设置）；点击通知聚焦并跳会话，通知内容不带密钥与会话正文以外的敏感信息。

### 6.4 原生 UI 约定

- 立即模式：view 是每帧执行的普通函数；状态放 app 结构体；元素身份靠兄弟位置或 `Key` 保持，列表必须给稳定 `Key`。
- 样式没有 CSS：观感由 `internal/ui/theme` 主题常量 + 链式 setter 决定；深浅色实时切换来自 `ui.Theme`，不要自建第二套色板。
- 设置项必须立即生效；未接线的控件要禁用并标注里程碑，不要做「看起来能改但无效果」的控件。
- 长列表用 `ui.List`（虚拟化）+ 稳定 Key；聊天流式用 `markdown.Stream`，完成态切 `markdown.View`。
- 动效用 `Animate`/`Transition`；装饰性 Loop（扫光 / 脉冲）在 `theme.ReduceMotion(c)` 为真时停用（Animate / Transition 由 mygo 自动跟随系统偏好）。
- 窗口级快捷键只在 `RootView` 注册；焦点在面板可编辑区（终端 / 编辑器 / 过滤框）时不注册 Esc——mygo 对已注册快捷键不再投递给焦点元素，抢注册会吃掉终端的 Esc。
- 不写包装层封装 mygo API；版本锁定在 go.mod，升级走显式决策。

### 6.5 数据

- SQLite 是事实源；事件表按线程 `seq` 单调递增，写入在事务内完成。
- 事件写入与广播统一走 `internal/record.Recorder`，不要手写 `AppendEvent` + `Bus.Publish` 配对。
- 修改表结构时增加迁移版本，不要直接改已有迁移。
- 写入文件前备份；外部修改时用 `expectedHash` 阻止覆盖。
- 软删除、审计、用量等保留策略与 `docs/specs/product.md` 一致。

### 6.6 错误处理

- Go 侧返回可读错误；不要 panic 到界面（view 内的 recover 只兜底，不当流程用）。
- 模型调用错误统一使用 `internal/model` 的 `Error` 与错误码（`auth`、`rate_limit`、`timeout`、`context_overflow` 等）；新增提供商必须映射到既有错误码，不要向 UI 抛原始 HTTP 错误。
- UI 调用失败要 `c.Toast` 提示，不能只写日志。
- 后台 goroutine 失败要写日志并投递 `error` 事件，不能静默退出。

## 7. 已知坑

mygo 原生 UI（迁移全程实测，持续补充）：

- `ui.Split` 的宽度指针初值必须非零（如 280），0 会让左栏直接不可见。
- `WindowOptions.StateKey`（窗口状态记忆）与显式 `X/Y` 并存时，显式坐标被记忆值覆盖；两者二选一。
- 立即模式下，后台 goroutine 改 UI 状态必须经 `win.Update`，直接改不会重绘或引发数据竞争。
- 剪贴板 / 原生对话框在 view 帧内直调会死锁（等主循环而主循环正在跑帧），必须经 `win.Update` 或 goroutine 排到帧后。
- 绝对定位子元素会盖住普通兄弟；需要垫底时用 `track.Draw`。
- markdown.Stream / View 在收缩布局（Row 里不带 FillWidth 的 Column）里会塌成零宽，正文列必须显式 `FillWidth`。
- 窗口已注册的快捷键按键不再投给焦点元素（`claimed` 语义）：Esc 这类按键要做「焦点在面板可编辑区则不注册」的条件注册，否则终端 / vim 收不到 Esc；焦点状态取上一帧，存在一帧竞态（与 React 的逐事件判断同样近似，可接受）。
- 模态（DialogBase）打开时窗口级快捷键整体等待，Esc 归 `OverlayShortcut`：退出链的浮层层级不用自己写。
- `ui.Tester` 里动效按引擎时间推进，断言前要 `Advance` 动效时长；`CapturePage` 不含浮层，冒烟截图用 `screencapture -x -R`。
- libghostty 首次裸 `go run` 会下载 dylib 到 `~/Library/Caches/mygo/natives/`（SHA-256 校验）；打包应用永不下载。网络受限环境可设 `$MYGO_GHOSTTY_VT` 指向本地文件。
- 终端 PTY 网格尺寸协商随每次 paint 进行（layout → viewResize），惰性创建 + `Fill()` 即可，无需显式 Resize（01 号票观察到的「卡 80×24」经 09 号票证伪为早期帧暂态误读）。
- macOS 签名：`mygo build` 内置 Developer ID 签名与公证流程，需要证书时先向用户要，不要自己折腾 codesign。

通用：

- 应用没有任何内置提供商；不要写死 `openai`、`gemini` 等提供商分支，也不要提供「推荐模型」。
- 同一模型 ID 可以出现在多个提供商下，解析与选择必须使用 `model.Ref` / `FindModelByRef`；只有兼容旧数据时才用 `Resolve` 的原始 ID 回退。
- `catalog.json.gz` 是生成物，不要手改；更新目录用 `go run ./cmd/modelsync`。

## 8. 验证清单

提交前至少执行：

```bash
go test ./...
go vet ./...
golangci-lint run
go tool mygo build     # 产出可启动 .app + DMG
```

涉及 UI 时额外：

- `CRAFTWORK_HOME=/tmp/cw-<主题> go tool mygo dev` 隔离目录冒烟。
- 走查一遍：首次启动空状态 → 从模型目录添加提供商 → 配置密钥 → 选择模型 → 发送消息 → 工具调用 → 审批允许/拒绝 → 差异面板 → 终端 → 审计记录；1440×900 与 1100×800 两个窗口尺寸对照 `docs/native-ui/spec.md`（interface.md 的 CSS 视觉规格已标注为历史参照，行为对照不变）。

CI（`.github/workflows/ci.yml`）在 Ubuntu 上执行 `go test`、`go vet`、`golangci-lint run`、`govulncheck`，并在 macOS runner 上跑 `go tool mygo build` 冒烟（产物上传为 artifact）。本地提交前仍以本节命令为准。

## 9. 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 技术栈、分层、安全机制 | `docs/specs/architecture.md` |
| 系统提示词分节、缓存层级、文案与测试约束 | `docs/specs/prompt-contract.md` |
| 产品定位、范围、非功能指标、验收 | `docs/specs/product.md` |
| 功能与验收要点、工具与规则 | `docs/specs/features.md` |
| 界面观感约定（原生 UI） | `docs/native-ui/spec.md`；`docs/specs/interface.md` 是 WebView 时代的历史规格，只作参照，不再随实现更新 |
| 推翻或新增架构与选型决策 | `docs/adr/` 新增编号并标注被取代的旧 ADR |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、`CONTRIBUTING.md` |
| 开发约定、命令、陷阱 | 本文件 |
| 迁移留档的进度与决策 | 票的 `Status:` 与 `## Comments`（`docs/native-ui/issues/`） |

## 10. 当前留桩

- 自动更新：下载 / 替换与回滚未接线；mygo 内置 `Updater`（Ed25519 签名 + 增量）待接入（M4）。
- 沙箱与 Streamable HTTP MCP：留桩（M4）。
- M3 限制：Word 只支持同一格式片段内的文本替换；Excel 不计算公式；PDF 只读文本层，不支持 OCR 与编辑（文档富预览、逐块 diff 应用不在原生 UI 范围，见 `docs/native-ui/spec.md` 裁剪清单）。
- 真实密钥验收：OpenAI、Anthropic 或任意 OpenAI 兼容网关需要本地配置密钥后各跑一次工具调用闭环（迁移后界面已跑通假端点全链，真实网关待用户本地验收）。
- 跨平台：Windows（原生 UI + ConPTY 终端）与 Linux 打包需要实机验证；仅 macOS 实测。

## 11. 提交规范

- 提交信息使用中文，一次提交只做一件逻辑上的事。
- 提交前运行第 8 节的检查；不要提交 `bin/`、`node_modules/`。
- 贡献使用 DCO：`git commit -s`。
- 界面视觉与交互的变更先更新 `docs/native-ui/spec.md`（观感约定）与受影响的行为 spec，再改实现；决策变更走 `docs/adr/`。
