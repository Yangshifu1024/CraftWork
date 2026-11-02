# 贡献指南

感谢你愿意改进 CraftWork。本文件说明如何准备环境、修改代码、运行检查并提交改动。

写代码之前请先阅读 [AGENTS.md](AGENTS.md)：它列出了仓库分层、IPC、审批、安全、前端与数据的硬性约束。行为规格见 [docs/specs/](docs/specs/)（产品 / 功能 / 架构 / 界面 / 提示词契约），决策背景与取舍见 [docs/adr/](docs/adr/)，术语以仓库根 [GLOSSARY.md](GLOSSARY.md) 为准；界面改动还需要对照 [界面规格](docs/specs/interface.md)。

## 环境准备

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| Go | 1.27.3+ | 以 `go.mod` 中的 `go` 指令为准 |
| Node.js | 24.22.2+ | 以 Vite 8 与 `package.json` 为准，npm 随 Node.js 安装 |
| pnpm | 12.16.1 | 前端包管理器；`npm install -g pnpm`，版本锁定在 `package.json` 的 `packageManager` 字段 |
| Wails3 CLI | v3.0.0 | 桌面开发、构建与生成绑定 |

安装 Wails3 CLI：

```bash
go install github.com/wailsapp/wails/v3/cmd/wails3@v3.0.0
```

确认 `$(go env GOPATH)/bin`（或自定义的 `GOBIN`）在 `PATH` 中，然后检查环境：

```bash
wails3 doctor
```

安装前端依赖：

```bash
cd frontend
pnpm install
```

请不要在仓库内切换 Go、Node.js 或 Wails3 的版本，也不要通过临时环境变量指向其他安装位置；遇到版本问题时先提 Issue 说明情况。

## 本地开发

```bash
# 桌面开发模式（Go 后端 + Vite 前端）
wails3 dev

# 只启动前端，使用内存 Mock 后端
cd frontend && pnpm run dev
# http://127.0.0.1:9245/

# 重新同步 models.dev 模型目录（生成文件需要提交）
wails3 task models:sync

# 使用隔离的数据目录，避免污染 ~/.craftwork
CRAFTWORK_HOME=/tmp/craftwork-dev wails3 dev
```

修改 `internal/service/` 中导出方法的签名、参数或返回值后，必须重新生成 TypeScript 绑定，并提交生成结果：

```bash
wails3 generate bindings -clean=true -ts -i
```

`frontend/bindings/` 由工具生成，不要手工编辑。

## 提交前检查

每次提交至少运行：

```bash
go test ./...
go vet ./...
cd frontend && pnpm exec tsc --noEmit && pnpm run test && pnpm run build
wails3 build
```

浏览器端到端测试与截图回归：

```bash
cd frontend
pnpm exec playwright install chromium   # 首次运行
pnpm run test:e2e                  # 功能用例 + 截图对比
pnpm run test:e2e -- --grep-invert @screenshot   # 只跑功能用例
```

截图基线按操作系统保存；在非 macOS 上运行截图用例前需要生成对应平台基线。

也可以使用 Taskfile 中的聚合命令：

```bash
wails3 task test    # go test ./... + 前端单元测试与类型检查
wails3 task check   # go vet ./... + 前端类型检查
```

涉及界面的改动还要额外确认：

- 在 1440×900 与 1100×800 两个视口对照 [界面规格](docs/specs/interface.md) 的布局尺寸与视觉 token。
- 三栏布局、48px 顶行、27px 状态栏、窄窗口折叠、深浅主题都正常。
- 完整走一遍：发送消息 → 工具调用 → 审批允许 / 拒绝 → 差异面板 → 审计记录。
- 桌面端冒烟启动：`CRAFTWORK_HOME=/tmp/cw-smoke ./bin/CraftWork`；首次启动应显示“尚未添加提供商”的空状态。

## CI 检查内容

Pull Request 会运行 `.github/workflows/ci.yml`，包括：

| 任务 | 内容 |
| --- | --- |
| Go 测试与静态检查 | `go test ./...`、`go vet ./...`、`govulncheck ./...` |
| 前端类型检查与构建 | `ppnpm install --frozen-lockfile`、`pnpm run typecheck`、`pnpm run build`、`pnpm audit --audit-level=high` |
| 绑定漂移检查 | 重新生成 `frontend/bindings`，与仓库内容比对 |

CI 未通过时不要在 PR 中直接合并。依赖审计报出的高危问题需要升级依赖或在 PR 中说明处理方式。

## 代码约定

完整约束见 [AGENTS.md](AGENTS.md) 第 6 节，这里列出最容易踩错的几条：

- **分层**：业务逻辑放在对应领域包（`internal/agent`、`tools`、`policy`、`model`、`store` 等），`internal/service` 只做参数校验、编排与错误转换。
- **IPC**：命令走绑定方法，事件只走 `cw:events` 批量通道；文本增量在 Go 侧按 33ms 合并后推送。
- **审批**：审批必须跨 IPC 完成，不允许在 Go 侧静默放行；审批决定与工具执行都要写入审计。
- **安全**：路径校验、命令分级、密钥读取、外链白名单都在 Go 侧完成，不信任前端传入的路径或命令。
- **前端**：使用 assistant-ui headless primitives，不引入 Tailwind；颜色、间距、圆角只使用 `styles/base.css` 中的 CSS 变量；图标统一通过 `Icon` 组件使用 `lucide-react`。
- **数据**：SQLite 是事实源；修改表结构要新增迁移版本，不要改已有迁移；写文件前先备份。
- **错误处理**：Go 侧返回可读错误，不 panic 到界面；前端调用失败要给出 toast 提示，不能只写 `console.error`。
- **测试**：策略、路径、事件、存储、Agent 循环、diff 等核心逻辑必须有 Go 单测；修复缺陷时补对应回归测试。

新增服务、工具或面板的步骤见 [AGENTS.md](AGENTS.md) 第 5 节。

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
- 影响的文档；如果修改了界面，附上两个视口下的截图或录屏。
- 是否引入新的依赖、迁移或配置项。

不要提交以下生成物：

- `bin/`
- `frontend/dist/`
- `frontend/node_modules/`
- `.task/`
- `.DS_Store`

## 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 技术栈、分层、IPC、安全机制 | `docs/specs/architecture.md` |
| 系统提示词分节、缓存层级、文案与测试约束 | `docs/specs/prompt-contract.md` |
| 产品定位、范围、非功能指标、验收 | `docs/specs/product.md` |
| 功能与验收要点、工具与规则 | `docs/specs/features.md` |
| 布局、组件、交互、图标、视觉 | `docs/specs/interface.md` |
| 推翻或新增架构与选型决策 | `docs/adr/` 新增编号并标注被取代的旧 ADR |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、本文件 |
| 开发约定、命令、陷阱 | `AGENTS.md` |

## 许可证

CraftWork 使用 [Apache-2.0](LICENSE) 许可证。提交贡献即表示你同意该贡献以相同许可证发布。

## 报告问题

- 功能缺陷与改进建议请提交 Issue，附上操作系统、版本、复现步骤与相关日志。
- 涉及密钥泄露或安全漏洞时，不要在公开 Issue 中粘贴密钥、令牌或完整数据目录；先通过私下渠道联系维护者，并立即轮换已泄露的密钥。
