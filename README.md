<p align="center">
  <img src="assets/appicon.png" width="96" height="96" alt="CraftWork" />
</p>

<h1 align="center">CraftWork</h1>

<p align="center">本地优先、可审计的桌面 Agent</p>

<p align="center">
  Go 内核 · mygo 原生 UI · 数据不出本机
</p>

---

## 这是什么

CraftWork 是一个完全本地运行的桌面 Agent。它在你的工作区里读文件、改代码、跑命令、看差异，也能读取文档、整理会议纪要、生成邮件草稿。

三条产品原则：

- **Go 内核**：Agent、工具、策略、存储、终端、密钥全部由 Go 执行。
- **本地优先**：会话、密钥、文件与日志保存在本机，不收集遥测。
- **可审计、可撤销**：写文件、执行命令、修改文档都先经过审批，并留下审计记录与备份。

> **桌面外壳已切换为 mygo 原生 UI**：Go GPU 自绘，应用内没有任何 WebView 与 React 前端；IPC 与绑定层随之消失，界面直接调用 Go 函数。决策与迁移过程见 [ADR-0014](docs/adr/0014-native-ui-via-mygo.md) 与[原生 UI 迁移留档](docs/native-ui/spec.md)。

## 核心特性

### 工作区会话与临时会话

- 工作区会话：绑定你添加的项目目录，文件、命令与 Git 都工作在该目录内。
- 临时会话：无需先选择工作区，侧栏「临时会话」组一键新建、开箱即聊；会话能力完整，产生的文件保存在应用内置的临时目录（`~/.craftwork/workspace/default`），目录被删除后重启自动重建。
- 会话列表按归属分组：临时会话置顶，其余按工作区分组。

### Code 模式

- 工作区文件树、Git 状态、代码编辑与搜索（mygo 代码编辑器）。
- 读文件、写文件、精确替换、`apply_patch` 补丁预演与逐块应用、Glob / Grep、Shell。
- Git 工具：状态、差异、提交记录与提交（默认不推送）。
- 统一 / 并排差异视图，支持在审批卡片中查看完整差异或补丁预览。
- 面板终端，libghostty 渲染、Go 侧真实 PTY。

### Work 模式

- Word / Excel 文本级读取与修改，PDF 文本提取；扫描件提示不支持 OCR。
- 文档写入前审批与备份，修改 Word 文本或 Excel 单元格时保留原有结构。
- 会议纪要、表格、邮件草稿等办公场景的对话流。
- 邮件与日历可通过本地 MCP 服务器（stdio）接入；草稿只在应用内生成，不自动发送。

### 对话与工具

- 流式文本、推理折叠、工具卡片、任务清单、错误卡片。
- 停止生成、重新生成、会话内切换模型。
- 运行中引导：任务执行中直接输入补充指令（回车 / Ctrl+Enter / 引导按钮），当前步骤结束后自动并入，运行结束则接力继续。
- 附件、拖放、`path:line` 引用、Markdown 表格与代码块高亮。
- 多提供商：OpenAI、Anthropic 与 OpenAI 兼容服务；应用不内置任何提供商，全部由你在设置里手动添加。
- 模型目录：从 models.dev 快照浏览并导入提供商与模型，自动带出上下文、最大输出、能力与价格；目录随二进制离线可用。
- 自定义提供商与模型：协议、端点、API Key 要求、模型 ID / 名称 / 上下文窗口 / 最大输出与能力。
- 错误分类（限流、认证、超时、上下文溢出等）、有限重试与失败转移；用量面板按目录价格估算费用。
- 快捷键：⌘K 命令面板、⌘N 新建会话、⌘B 面板开合、⌘W 关闭面板标签、⌘, 设置；Esc 一次退一层（运行中双击 Esc 停止、待审批 Esc 拒绝）。
- 系统通知：窗口失焦时的审批 / 完成 / 失败提醒，点击通知聚焦并跳到对应会话。

### 审批与安全

| 风险 | 含义 | 默认策略 |
| --- | --- | --- |
| R0 | 只读 | 自动允许，记录审计 |
| R1 | 写工作区 | 每次询问，可本会话 / 本工作区授权 |
| R2 | 执行 | 每次询问 |
| R3 | 网络或越界 | 每次询问，不提供永久允许 |
| R4 | 破坏性 | 阻止 |

- 三档审批：分级 / 严格 / 只读。
- 受限工作区直接拒绝网络访问，不提供任何永久授权。
- 写文件前自动备份，可在备份面板恢复。
- 审批决定、工具调用、文件写入全部落本地审计。

### 本地优先

- SQLite（WAL）保存会话、事件、审批、审计、用量与设置。
- 密钥保存在 `~/.craftwork/keys.json`（0600），只存在于 Go 进程。
- 界面全部本地绘制：不加载任何远程页面，外链经确认后交给系统浏览器。
- 不收集遥测，不上传代码或文档。

## 技术架构

```text
┌──────────────────────────────────────────────────────────┐
│ 原生界面  internal/ui（mygo 立即模式 view，GPU 自绘）      │
│ shell 三栏外壳 · sidebar · chat · panel · overlays        │
│ theme 主题 token · icons（lucide go:embed）               │
└───────────────┬──────────────────────────────────────────┘
                │ view 直调（无 IPC）      win.Update（主线程回调）
                │                          internal/events bus 订阅
┌───────────────▼──────────────────────────────────────────┐
│ mygo 宿主（v0.7.4）                                      │
│ 窗口 / 单实例 / 事件循环 / 通知 / 终端 / 对话框 / 剪贴板   │
└───────────────┬──────────────────────────────────────────┘
                │
┌───────────────▼──────────────────────────────────────────┐
│ Go 内部领域包                                            │
│ service → agent · tools · policy · model                 │
│         → store · secret · editor · notify               │
└───────────────┬──────────────────────────────────────────┘
                │
        SQLite / 文件系统 / PTY（libghostty）/ keys.json
```

| 层 | 目录 | 职责 |
| --- | --- | --- |
| 入口 | `main.go` | 启动应用 |
| 宿主 | `internal/app` | 窗口、单实例、生命周期、服务装配、通知接线 |
| 界面 | `internal/ui` | `shell / sidebar / chat / panel / overlays`，view 直调领域包 |
| 领域 | `internal/service`、`agent`、`tools`、`policy`、`model` | 参数校验与编排、Run 循环、工具、风险分级、模型接入 |
| 基础设施 | `internal/store`、`secret`、`editor`、`events`、`notify` | SQLite、密钥、差异、事件类型与总线、系统通知 |

### 关键设计

- **原生 UI，无 WebView**：mygo 立即模式 view 每帧直接构建界面，深浅色实时跟随系统；没有 CSS、没有前端构建链。
- **没有 IPC**：view 直调 Go 函数；后台结果统一经 `win.Update` 回主线程，`internal/events` 的类型表保留为内部契约。
- **审批跨 UI 线程**：审批请求落库并推送卡片，用户经 `DecideApproval` 决定；授权范围由服务端校验。
- **安全边界在 Go**：路径校验、命令分级、密钥读取、外链白名单都不依赖界面。
- **事件即事实**：SQLite 事件表按会话 `seq` 单调递增，界面是事件的投影，重启可完整恢复。

更完整的技术细节见 [架构规格](docs/specs/architecture.md)，设计决策的背景与取舍见 [docs/adr/](docs/adr/)。

## 快速开始

### 环境要求

- Go 1.27.3+（以 `go.mod` 的 `go` 指令为准）
- macOS 13+（Windows / Linux 打包待实机验证）

mygo 是 go.mod 的 tool 依赖，`go tool mygo` 直接按锁定版本执行，无需单独安装 CLI。首次裸 `go run` 会下载 libghostty 动态库到 `~/Library/Caches/mygo/natives/`（带 SHA-256 校验）；`mygo build` 产物内置该库，运行时不联网。

### 运行

```bash
go tool mygo dev
```

### 构建

```bash
go tool mygo build       # bin/darwin-<arch>/CraftWork.app + DMG
```

### 模型目录

模型目录来自 [models.dev](https://models.dev)，在构建期同步并嵌入二进制，运行时不需要联网。更新目录后提交生成文件：

```bash
go run ./cmd/modelsync          # 重新生成 internal/model/catalogdata/catalog.json.gz
go run ./cmd/modelsync -check   # 只检查是否与上游一致
```

首次启动没有任何提供商；在「设置 → 模型与提供商」里从目录添加，或手动配置 Ollama、公司网关等端点。使用 `CRAFTWORK_HOME=/tmp/craftwork-dev` 可以隔离数据目录。

> 全部开发命令、目录约定、实现约束与常见坑见 [AGENTS.md](AGENTS.md)。

## 参与贡献

欢迎提交 Issue 与 Pull Request。完整的环境准备、代码约定、检查命令与提交规范见 [CONTRIBUTING.md](CONTRIBUTING.md)。

### 开始之前

1. 阅读 [AGENTS.md](AGENTS.md)：里面有命令、代码结构、审批 / 安全 / 原生 UI 约定。
2. 阅读 [架构规格](docs/specs/architecture.md)；界面观感约定见 [docs/native-ui/spec.md](docs/native-ui/spec.md)，涉及决策取舍时查 [docs/adr/](docs/adr/)。

### 流程

1. Fork 仓库并从 `main` 建分支。
2. 修改代码，补对应测试（核心逻辑 Go 单测；界面行为 `ui.Tester` 用例）。
3. 运行检查：

```bash
go test ./...
go vet ./...
golangci-lint run
go tool mygo build
```

4. 涉及 UI 时，在 1440×900 与 1100×800 两个窗口尺寸对照[原生 UI 规格](docs/native-ui/spec.md)，确认三栏布局、窄窗折叠与深浅主题。
5. 提交 PR，描述变更、验证方式与影响的文档。
6. 贡献使用 DCO：`git commit -s`。

CI 会运行 Go 测试与静态检查、依赖漏洞扫描，并在 macOS runner 上执行 `go tool mygo build` 冒烟；PR 合并前需要全部通过。

### 适合上手的方向

- M4：沙箱、自动更新（mygo 内置 Updater）、Streamable HTTP MCP。
- M3 深化：Word / Excel / PDF 的结构化解析与写入（当前为文本级）。
- 真实密钥验收：配置 OpenAI、Anthropic 或任意 OpenAI 兼容网关的密钥后跑通工具调用闭环。
- 跨平台：Windows（ConPTY 终端）与 Linux 打包的实机验证。

## 路线图

| 阶段 | 状态 | 目标 |
| --- | --- | --- |
| M0 骨架 | 已完成 | 三栏界面、流式对话、工具与审批、编辑器、差异、终端、设置、审计 |
| M1 对话 | 已完成 | 多提供商原生协议、上下文压缩、附件内容、通知 |
| M2 Code 闭环 | 已完成 | 命令前缀授权、补丁预演、Git 工具、终端搜索 |
| M3 Work 模式 | 已完成（文本级） | Word / Excel / PDF 文本级读写、文档差异与备份 |
| M4 安全与发布 | 部分完成 | MCP stdio 与更新清单校验已实现；沙箱、更新下载/替换、签名凭证与公证待完成 |
| M5 原生 UI | 已完成 | 桌面外壳切换 mygo 原生 UI，WebView 与 React 前端已移除；留档见 [docs/native-ui](docs/native-ui/spec.md) |

## 文档

- [产品规格](docs/specs/product.md)：定位、范围、非功能指标与验收
- [功能规格](docs/specs/features.md)：功能编号、优先级与验收要点
- [架构规格](docs/specs/architecture.md)：分层、事件流、Agent 内核、策略与存储
- [界面规格（历史）](docs/specs/interface.md)：WebView 时代的布局与视觉规格，保留作参照
- [系统提示词契约](docs/specs/prompt-contract.md)：提示词分节与缓存分层
- [决策记录（ADR）](docs/adr/)：架构与选型决策的背景与取舍
- [原生 UI 迁移](docs/native-ui/spec.md)：ADR-0014 的迁移规格、分阶段 issue 与验收留档
- [术语表](GLOSSARY.md)：仓库领域词汇基准
- [贡献指南](CONTRIBUTING.md)
- [开发与协作约定](AGENTS.md)

## 许可证

Apache-2.0，见 [LICENSE](LICENSE)；贡献流程使用 DCO（`git commit -s`）。

内置的模型目录数据来自 [models.dev](https://models.dev)，使用 MIT 许可；完整声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
