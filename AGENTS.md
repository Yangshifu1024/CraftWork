# AGENTS.md

本文件是本仓库的协作与实现约定，面向在本仓库内写代码的 Agent 与开发者。

`README.md` 面向使用者与贡献者，介绍功能、架构与贡献流程；本文件回答「在这个仓库里应该怎么改、怎么验证、哪些东西不能碰」。

> **当前处于原生 UI 迁移期**（ADR-0014）：主导架构是 mygo 原生 UI；Wails3 + React 是双轨期遗留，只做最小维护，随收尾票删除。两边规则分开写，注意自己在哪条轨道上。

## Agent skills

### Issue tracker

issue 以 markdown 形式存放在仓库 `docs/<feature>/` 下（spec 为 `spec.md`，issue 为 `issues/NN-<slug>.md`）。见 `docs/agents/issue-tracker.md`。

### Triage labels

使用默认五个分诊标签：`needs-triage`、`needs-info`、`ready-for-agent`、`ready-for-human`、`wontfix`，写在 issue 文件的 `Status:` 行。见 `docs/agents/triage-labels.md`。

### Domain docs

单上下文布局：仓库根 `GLOSSARY.md` 术语表 + `docs/adr/` 决策记录 + `docs/specs/` 行为规格。见 `docs/agents/domain.md`。

---

## 1. 项目现状

CraftWork 是一个本地优先、可审计的桌面 Agent。M0–M3 已在 Wails3 + React 形态下交付（对话、Code 闭环、Work 模式文本级读写）；**主导工作现在是原生 UI 迁移**（`docs/adr/0014-native-ui-via-mygo.md`，规格与分阶段票见 `docs/native-ui/`）：

- **目标态**：mygo v0.7.4 原生 UI（`package ui`，GPU 自绘，应用内无任何 WebView）；Go 内核不变；IPC/绑定层消失，view 直调 Go 函数。
- **迁移进度**：P0 尖刺已通过（结论在 `docs/native-ui/issues/01-`，终端网格尺寸协商为已知风险，由 09 票处理）；P1 自 issue 02（后端解绑）起。
- **双轨期规则**：
  - 迁移工作只在 `feature/native-ui` 分支；**main 保持 Wails 架构可构建、可发布**，收尾票（`docs/native-ui/issues/10`）完成前不切换入口、不删 `frontend/`。
  - **`frontend/` 冻结**：只修阻断性 bug 保编译与测试绿，不做新功能——新界面能力一律做进原生 UI。
  - **后端内核两条轨道共用**：`internal/agent`、`tools`、`policy`、`store`、`secret`、`model`、`editor`、`events` 的改动照常在 main 评审，迁移分支及时合并。

### 1.1 技术栈

目标态（迁移完成后）：

| 层 | 选型 |
| --- | --- |
| 桌面外壳与界面 | mygo v0.7.4 原生 UI（`package ui`，立即模式 view；GPU 自绘，无 WebView） |
| 对话渲染 | `ui/markdown`（流式）、`ui/editor`（代码）、`editor.DiffView`（差异） |
| 终端 | `plugins/terminal`（libghostty，自管 PTY，macOS 13+） |
| 图表 / 图标 | `ui/chart`；lucide SVG 由 `ui.Icon` 渲染（go:embed） |
| 后端 | Go 1.27.3、SQLite（modernc.org/sqlite）、内部领域包（原 `internal/service`） |

双轨期遗留（main，P3 删除）：Wails3 v3.1.0 + 系统 WebView、React 19 + TS 5 + Vite 8、@assistant-ui/react、CodeMirror 6、xterm.js 6、lucide-react、自研 CSS token（`frontend/src/styles`，不用 Tailwind）。

## 2. 环境与工具链

| 工具 | 版本 | 说明 |
| --- | --- | --- |
| Go | 1.27.3+ | 以 `go.mod` 的 `go` 指令为准 |
| mygo CLI | v0.7.4 | 原生 UI 主工具链；项目内统一 `go tool mygo`（go.mod 的 tool 指令）按锁定版本执行，不要升级或换装 |
| golangci-lint | v2.15.0 | Go lint 与格式化；配置见 `.golangci.yml`；安装 `go install github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.15.0`（落在 `$(go env GOPATH)/bin`，注意本机 GOPATH 不是 `~/go`） |
| Wails3 CLI | v3.0.0 | 双轨期遗留（main 的 dev/build/task）；用 `wails3 doctor` 检查 |
| Node.js / pnpm | 24.22.2+ / 12.16.1 | 双轨期遗留（frontend）；`packageManager` 字段锁定 pnpm 版本 |

规则：

- **不要擅自安装或升级工具**；需要新工具或版本时先向用户提出请求。
- 使用 PATH 或 `$(go env GOPATH)/bin` 中已配置的版本，不通过临时环境变量切换。
- **mygo 升级是显式决策**：API 未到 1.0，升级前先读 mygo 仓库 `docs/ui/migration.md`，向用户提出并更新 `go.mod` 与本文件。

### 数据目录

- 默认 `~/.craftwork`：`craftwork.db`、`keys.json`、`backups/`、`attachments/`（粘贴附件）。
- `CRAFTWORK_HOME=/tmp/...` 可隔离数据目录，测试与冒烟必须使用隔离目录；迁移分支运行原生 UI 时也必须隔离，避免与 Wails 版共写同一个库。

### server 模式（双轨期遗留，迁移后删除）

`-tags server` 把 Wails 版编译成纯 HTTP 服务（`wails3 task run:server` → `localhost:8080`，`/health` 检查），浏览器里运行完整应用。**迁移决策已确认放弃该能力**（ADR-0014：mygo 的 WASM 目标只能带 UI 不能带后端）；P3 收尾票删除相关代码与 Taskfile 任务。在那之前 main 上的 server 模式照旧可用：无内置认证，只在本机或可信内网用，对外必须前置认证与 TLS。

### 不能提交的生成物

`bin/`、`frontend/dist/`、`frontend/node_modules/`、`.task/`、`.DS_Store`、`build/`（mygo build 产物）。

双轨期遗留：`frontend/bindings/`（Wails 生成的 TS 绑定）**需要提交**，不要手改，用 `wails3 generate bindings` 重新生成；迁移完成后随 `frontend/` 一起删除。

## 3. 常用命令

通用（两条轨道共用）：

| 目的 | 命令 |
| --- | --- |
| Go 测试 | `go test ./...` |
| Go 静态检查 | `go vet ./...` |
| Go lint / 格式化 | `golangci-lint run`（检查）；`golangci-lint fmt`（格式化）；`golangci-lint fmt --diff`（只检查） |
| 同步模型目录 | `go run ./cmd/modelsync`（检查漂移加 `-check`；生成 `internal/model/catalogdata/catalog.json.gz`） |

原生 UI（迁移分支，在仓库根执行）：

| 目的 | 命令 |
| --- | --- |
| 开发（编译失败保留旧进程） | `go tool mygo dev` |
| 打包 .app / DMG | `go tool mygo build` |
| view 无窗口测试 | `go test ./...`（`ui.Tester` headless） |

双轨期遗留（main 上的 Wails 形态，P3 删除）：

| 目的 | 命令 |
| --- | --- |
| 桌面开发模式 | `wails3 dev` |
| 生产构建 / macOS 打包 | `wails3 build` → `bin/CraftWork`；`wails3 package` → `bin/CraftWork.app` |
| 生成绑定 | `wails3 generate bindings -clean=true -ts -i` |
| server 模式 | `wails3 task run:server` → `localhost:8080` |
| 纯 UI 走查（Mock，无 Go） | `cd frontend && pnpm run dev` → `http://127.0.0.1:9245/` |
| 前端检查 | `cd frontend && pnpm run lint`、`pnpm exec tsc --noEmit`、`pnpm run test`、`pnpm run build` |
| 浏览器端到端测试 | `PLAYWRIGHT_CHANNEL=msedge pnpm run test:e2e`（迁移后由 `ui.Tester` 测试替代） |
| 一键任务 | `wails3 task test` / `check` / `models:sync` |

> 双轨期内修改 Go 服务导出方法签名后，仍须重新生成绑定并同步 `desktop.ts`/`mock.ts`，否则 main 的前端类型与后端不一致。

## 4. 目录与职责

```text
main.go                  入口：装配交给 internal/app（双轨期仍 embed frontend/dist）
internal/app/            进程装配：窗口、单实例、生命周期、服务装配
internal/ui/             （迁移新增）原生界面：theme / shell / sidebar / chat /
                         panel / overlays；view 直调内部领域包
internal/service/        双轨期为 Wails 绑定层；迁移中降级为内部领域包
                         （参数校验、编排与错误转换留在这一层）
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
internal/events/         事件类型与（迁移后内部）事件总线
frontend/                双轨期遗留：React 界面，冻结待删（见 1.1）
docs/                    adr（决策记录）、specs（行为规格）、native-ui（迁移）、agents（技能配置）
```

新增能力优先落到对应领域包；`internal/service` 只做参数校验、编排与错误转换，原生 UI 的 view 不得绕过它直接操作 store 或 agent。

## 5. 改动流程

1. **先读文档**：仓库根 `GLOSSARY.md`（术语）、`docs/specs/`（行为规格）、`docs/adr/`（决策记录）、迁移工作先读 `docs/native-ui/spec.md`；改动与既有 ADR 冲突时先显式提出，不要静默推翻。
2. **改后端**：领域包的改动两条轨道共用，保持 service 方法签名稳定；迁移分支不引入新的绑定方法。
3. **改 UI**：原生 UI 遵守 6.4 节约定；双轨期 `frontend/` 冻结，只做阻断性修复。
4. **补测试**：核心逻辑（策略、路径、事件、存储、Agent 循环、diff、UI 状态机/reducer）必须有 Go 单测；迁移中移植的纯函数（threadReducer、sidePane）逐行对齐既有用例。
5. **跑验证**：见第 8 节。
6. **同步文档**：见第 9 节。

### 新增一个原生 UI 界面/能力

1. 数据与行为先落在领域包（`internal/service` 或更深的包），写成普通 Go 方法。
2. 界面放 `internal/ui/<区域>/`（shell / sidebar / chat / panel / overlays），view 函数从 app 状态构建；状态机写成纯函数便于单测。
3. 后台 goroutine 更新界面一律经 `win.Update(func(){...})`。
4. 颜色、字号、间距取自 `internal/ui/theme.go` 的主题常量，不写死值。
5. 图标在 `internal/ui/icons.go` 登记 lucide SVG 名称。
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

### 双轨期遗留流程（frontend，冻结）

`frontend/` 不新增功能。确需修阻断性 bug 时：保持既有 class 名与 CSS token（视觉规格见 `docs/specs/interface.md`），类型检查必须通过；新增 API 需同步 `desktop.ts` 与 `mock.ts`（`Api` 类型从 `desktopApi` 推导，不要手写第二份接口签名），并重新生成绑定。

## 6. 实现约束（必须遵守）

### 6.1 UI 与数据流

- view 直调 Go 函数，**没有 IPC 与批量事件通道**；`internal/events` 的事件类型表保留为内部契约（`record` 落库 + bus 发布 → UI 订阅回调）。
- 后台事件到界面只经 `win.Update(func(){...})`（主线程执行并重绘，自动合并）；禁止在 view 外直接改 UI 状态。
- 大对象（文件内容、差异、终端历史）按需调用方法获取，不要常驻 app 状态。
- 界面状态机写成纯函数转移（参考移植的 threadReducer / sidePane），可单测、可重放。

### 6.2 审批

- 审批必须显式决定：请求 → 卡片 → 用户决定 → 执行/跳过，**不允许 Go 侧静默放行**；「跨 IPC」变为「跨 UI 线程」，决定与执行仍走 `ChatService.DecideApproval` 语义。
- `once / session / workspace` 三种范围由服务端校验；`AllowPermanent` 为 false 时禁止 session/workspace。
- 受限工作区：R3 直接拒绝、不提供永久授权、已有授权不生效。
- 取消、窗口关闭、应用退出统一按拒绝或取消落审计。
- 审批决定与工具执行必须写 `audit_log`。

### 6.3 安全

- **外链唯一出口**：http/https 经确认后走 `mygo.Shell.OpenExternal`；应用不加载任何远程页面。
- Markdown 不渲染原始 HTML（`ui/markdown` 默认按文本显示，模型常写的 `<b>/<code>` 等格式化标签除外）；图片默认只显示 alt 文本，加载需显式启用且仅限本地/受控来源。
- 所有路径校验在 Go 侧完成：`policy.Resolve` 先词法校验，再解析符号链接；不信任 UI 传来的路径。
- 密钥只存在 Go 进程与 `~/.craftwork/keys.json`；不进入日志、数据库、导出包。
- `shell_exec` 必须清洗敏感环境变量、杀进程组、限制输出。
- libghostty dylib 由 `mygo build` 打进 .app 并随应用签名（运行时不下载）；不要改用运行时网络获取。

### 6.4 原生 UI 约定

- 立即模式：view 是每帧执行的普通函数；状态放 app 结构体；元素身份靠兄弟位置或 `Key` 保持，列表必须给稳定 `Key`。
- 样式没有 CSS：观感由 `internal/ui/theme.go` 主题常量 + 链式 setter 决定；深浅色实时切换来自 `ui.Theme`，不要自建第二套色板。
- 设置项必须立即生效；未接线的控件要禁用并标注里程碑，不要做「看起来能改但无效果」的控件。
- 长列表用 `ui.List`（虚拟化）+ 稳定 Key；聊天流式用 `markdown.Stream`，完成态切 `markdown.View`。
- 动效用 `Animate`/`Transition`；遵循系统 `ReduceMotion`。
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

mygo 原生 UI（P0 实测，持续补充）：

- `ui.Split` 的宽度指针初值必须非零（如 280），0 会让左栏直接不可见。
- `WindowOptions.StateKey`（窗口状态记忆）与显式 `X/Y` 并存时，显式坐标被记忆值覆盖；两者二选一。
- `plugins/terminal` 的 PTY 网格在 `terminal.New` 后 0.5 秒内由视图尺寸协商，错过则卡在 80×24；本机（Retina 双屏）实测协商仍可能失效——处理方案与验证见 `docs/native-ui/issues/01-`，终端票（09）实施时优先解决。
- 立即模式下，后台 goroutine 改 UI 状态必须经 `win.Update`，直接改不会重绘或引发数据竞争。
- libghostty 首次裸 `go run` 会下载 dylib 到 `~/Library/Caches/mygo/natives/`（SHA-256 校验）；打包应用永不下载。网络受限环境可设 `$MYGO_GHOSTTY_VT` 指向本地文件。
- macOS 签名：`mygo build` 内置 Developer ID 签名与公证流程，需要证书时先向用户要，不要自己折腾 codesign。

双轨期遗留（Wails/前端，随 `frontend/` 删除失效）：

- Wails3 没有默认 CSP、不拦截窗口导航；单实例只通知不唤起窗口（回调里 `Restore/Show/Focus`）。
- server 模式下窗口、托盘、原生菜单、原生对话框、单实例都是空实现；页面是构建产物无 HMR；生产 CSP 是 `connect-src 'self'`（`internal/app/csp_prod.go`）。
- `isDesktop` 表示「后端在 Go 进程里」；Mock 的定位是「伪造数据，不复刻行为规则」，行为规则以 Go 为唯一事实源，e2e 失败先区分产品 bug 还是 Mock 演砸。
- xterm 通过 canvas 渲染，`fontFamily` 必须传具体字体栈并在末尾补 Nerd Font 回退。
- Vite HMR 与 Zustand 并存时模式切换可能出现旧 store 实例；验证时整页重载。

两条轨道通用：

- 应用没有任何内置提供商；不要写死 `openai`、`gemini` 等提供商分支，也不要提供「推荐模型」。
- 同一模型 ID 可以出现在多个提供商下，解析与选择必须使用 `model.Ref` / `FindModelByRef`；只有兼容旧数据时才用 `Resolve` 的原始 ID 回退。
- `catalog.json.gz` 是生成物，不要手改；更新目录用 `go run ./cmd/modelsync`。

## 8. 验证清单

通用（提交前必跑）：

```bash
go test ./...
go vet ./...
golangci-lint run
```

迁移分支额外：

```bash
go tool mygo build          # 产出可启动 .app
CRAFTWORK_HOME=/tmp/cw-native go tool mygo dev   # 隔离目录冒烟
```

冒烟走查：首次启动空状态 → 从模型目录添加提供商 → 配置密钥 → 选择模型 → 发送消息 → 工具调用 → 审批允许/拒绝 → 差异面板 → 终端 → 审计记录；1440×900 与 1100×800 两个窗口尺寸对照 `docs/native-ui/spec.md`（原生 UI 阶段不再对照 interface.md 的 CSS 细节，行为对照不变）。

双轨期遗留（main 上动 Wails/前端代码时）：

```bash
cd frontend && pnpm run lint && pnpm exec tsc --noEmit && pnpm run build
wails3 build
PLAYWRIGHT_CHANNEL=msedge pnpm run test:e2e -- --grep @screenshot
```

CI（`.github/workflows/ci.yml`）目前跑 `go test`、`go vet`、`govulncheck`、frontend 检查与绑定漂移；P3 收尾票改为 Go 侧 + `mygo build` 冒烟，删除 Playwright 与绑定漂移检查。本地提交前仍以本节命令为准。

## 9. 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 技术栈、分层、安全机制 | `docs/specs/architecture.md` |
| 系统提示词分节、缓存层级、文案与测试约束 | `docs/specs/prompt-contract.md` |
| 产品定位、范围、非功能指标、验收 | `docs/specs/product.md` |
| 功能与验收要点、工具与规则 | `docs/specs/features.md` |
| 布局、组件、交互、图标、视觉 | `docs/specs/interface.md`（双轨期）；原生 UI 的观感约定进 `docs/native-ui/spec.md` |
| 推翻或新增架构与选型决策 | `docs/adr/` 新增编号并标注被取代的旧 ADR |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、`CONTRIBUTING.md` |
| 开发约定、命令、陷阱 | 本文件 |
| 原生 UI 迁移的进度与决策 | 票的 `Status:` 与 `## Comments`（`docs/native-ui/issues/`）；影响范围的决策更新 `docs/adr/0014` |

## 10. 当前留桩

- 迁移相关：编辑器/终端/diff 按裁剪清单验收（`docs/native-ui/spec.md`）；终端网格尺寸协商问题（09 票）；自动更新改接线 mygo 内置 `Updater`（Ed25519 签名 + 增量），取代原 Wails 更新留桩。
- M3 限制：Word 只支持同一格式片段内的文本替换；Excel 不计算公式；PDF 只读文本层，不支持 OCR 与编辑（文档富预览不在迁移范围）。
- 真实密钥验收：OpenAI、Anthropic 或任意 OpenAI 兼容网关需要本地配置密钥后各跑一次工具调用闭环（原生 UI MVP 后重跑）。
- 跨平台：Windows（WebView2 → 原生 UI + ConPTY 终端）与 Linux 打包需要实机验证；迁移前仅 macOS 实测。

## 11. 提交规范

- 提交信息使用中文，一次提交只做一件逻辑上的事。
- 提交前运行第 8 节的检查；不要提交 `bin/`、`frontend/dist/`、`node_modules/`、`build/`。
- 贡献使用 DCO：`git commit -s`。
- 界面视觉与交互的变更先更新规格（双轨期 `docs/specs/interface.md`，原生 UI `docs/native-ui/spec.md`），再改实现；行为变更同步受影响的 spec，决策变更走 `docs/adr/`。
