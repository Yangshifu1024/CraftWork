# 贡献指南

感谢你愿意改进 CraftWork。本文件说明如何准备环境、修改代码、运行检查并提交改动。

写代码之前请先阅读 [AGENTS.md](AGENTS.md)：它列出了仓库分层、审批、安全、原生 UI 与数据的硬性约束。行为规格见 [docs/specs/](docs/specs/)（产品 / 功能 / 架构 / 提示词契约），决策背景与取舍见 [docs/adr/](docs/adr/)，术语以仓库根 [GLOSSARY.md](GLOSSARY.md) 为准；界面改动还需要对照[原生 UI 规格](docs/native-ui/spec.md)（`docs/specs/interface.md` 是 WebView 时代的历史规格，只作参照）。

## 环境准备

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| Go | 1.27.3+ | 以 `go.mod` 中的 `go` 指令为准 |
| macOS | 13+ | libghostty 终端要求；Windows / Linux 打包待实机验证 |

mygo 是 go.mod 的 tool 依赖（`go tool mygo`），golangci-lint 按需安装：

```bash
go install github.com/golangci/golangci-lint/v2/cmd/golangci-lint@v2.15.0
```

确认 `$(go env GOPATH)/bin`（或自定义的 `GOBIN`）在 `PATH` 中。

请不要在仓库内切换 Go 或 mygo 的版本，也不要通过临时环境变量指向其他安装位置；遇到版本问题时先提 Issue 说明情况。

## 本地开发

```bash
# 桌面开发模式（编译失败保留旧进程）
go tool mygo dev

# 使用隔离的数据目录，避免污染 ~/.craftwork
CRAFTWORK_HOME=/tmp/craftwork-dev go tool mygo dev

# 重新同步 models.dev 模型目录（生成文件需要提交）
go run ./cmd/modelsync
```

修改 Go 方法签名不需要任何代码生成步骤；界面直接调用 Go 函数，没有绑定层。

## 提交前检查

每次提交至少运行：

```bash
go test ./...
go vet ./...
golangci-lint run
go tool mygo build      # 产出可启动的 .app + DMG
```

涉及界面的改动还要额外确认：

- 在 1440×900 与 1100×800 两个窗口尺寸对照[原生 UI 规格](docs/native-ui/spec.md)。
- 三栏布局、48px 顶行、窄窗折叠、深浅主题都正常。
- 完整走一遍：发送消息 → 工具调用 → 审批允许 / 拒绝 → 差异面板 → 审计记录。
- 桌面端冒烟启动：`CRAFTWORK_HOME=/tmp/cw-smoke go tool mygo dev`；首次启动应显示“尚未添加提供商”的空状态。

## CI 检查内容

Pull Request 会运行 `.github/workflows/ci.yml`，包括：

| 任务 | 内容 |
| --- | --- |
| Go 测试与静态检查 | `go test ./...`、`go vet ./...`、`golangci-lint run`、`govulncheck ./...` |
| 桌面构建冒烟 | macOS runner 上 `go tool mygo build`，产物上传为 artifact |

CI 未通过时不要在 PR 中直接合并。漏洞扫描报出的问题需要升级依赖或在 PR 中说明处理方式。

## 代码约定

完整约束见 [AGENTS.md](AGENTS.md) 第 6 节，这里列出最容易踩错的几条：

- **分层**：业务逻辑放在对应领域包（`internal/agent`、`tools`、`policy`、`model`、`store` 等），`internal/service` 只做参数校验、编排与错误转换；界面 view 直调领域包，没有 IPC 与绑定层。
- **事件**：事件写入与广播统一走 `internal/record.Recorder`；后台结果回界面一律经 `win.Update`。
- **审批**：审批必须由用户显式决定，不允许在 Go 侧静默放行；审批决定与工具执行都要写入审计。
- **安全**：路径校验、命令分级、密钥读取、外链白名单都在 Go 侧完成，不信任界面传入的路径或命令。
- **原生 UI**：立即模式 view，状态放 app 结构体；观感取值只使用 `internal/ui/theme` 的主题常量；图标统一通过 `ui.Icon` 使用 lucide SVG；长列表给稳定 `Key` 并用 `ui.List`。
- **数据**：SQLite 是事实源；修改表结构要新增迁移版本，不要改已有迁移；写文件前先备份。
- **错误处理**：Go 侧返回可读错误，不 panic 到界面；界面调用失败要给出 toast 提示，不能只写日志。
- **测试**：策略、路径、事件、存储、Agent 循环、diff、UI 状态机等核心逻辑必须有 Go 单测；修复缺陷时补对应回归测试。

新增界面、工具或面板的步骤见 [AGENTS.md](AGENTS.md) 第 5 节。

## 分支与提交

1. Fork 仓库，并从 `main` 创建分支，例如 `feat/anthropic-adapter`、`fix/approval-timeout`。
2. 一次提交只做一件逻辑上的事；不要把无关的格式化或重构混进功能提交。
3. 提交信息使用中文，说明做了什么、为什么这样做。
4. 贡献使用 DCO（Developer Certificate of Origin），提交时追加签名：

```bash
git commit -s -m "feat: 增加 Anthropic 消息适配器"
```

签名会在提交信息末尾写入 `Signed-off-by: 你的名字 <你的邮箱>`，表示你确认有权提交该改动并同意以本仓库许可证发布。

## Pull Request

PR 描述至少包含：

- 改动内容与动机，关联的 Issue（如果有）。
- 验证方式：运行了哪些命令、覆盖了哪些场景。
- 影响的文档；如果修改了界面，附上两个窗口尺寸下的截图或录屏。
- 是否引入新的依赖、迁移或配置项。

不要提交以下生成物：

- `bin/`
- `.task/`
- `.DS_Store`

## 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 技术栈、分层、事件流、安全机制 | `docs/specs/architecture.md` |
| 系统提示词分节、缓存层级、文案与测试约束 | `docs/specs/prompt-contract.md` |
| 产品定位、范围、非功能指标、验收 | `docs/specs/product.md` |
| 功能与验收要点、工具与规则 | `docs/specs/features.md` |
| 界面观感约定（原生 UI） | `docs/native-ui/spec.md` |
| 推翻或新增架构与选型决策 | `docs/adr/` 新增编号并标注被取代的旧 ADR |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、本文件 |
| 开发约定、命令、陷阱 | `AGENTS.md` |

## 许可证

CraftWork 使用 [Apache-2.0](LICENSE) 许可证。提交贡献即表示你同意该贡献以相同许可证发布。

## 报告问题

- 功能缺陷与改进建议请提交 Issue，附上操作系统、版本、复现步骤与相关日志。
- 涉及密钥泄露或安全漏洞时，不要在公开 Issue 中粘贴密钥、令牌或完整数据目录；先通过私下渠道联系维护者，并立即轮换已泄露的密钥。
