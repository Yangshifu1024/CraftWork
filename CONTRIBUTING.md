# 贡献指南

感谢你愿意改进 CraftWork。本文件说明如何准备环境、修改代码、运行检查并提交改动。

写代码之前请先阅读 [AGENTS.md](AGENTS.md)：它列出了仓库分层、审批、安全、界面与数据的硬性约束，以及 Agent skills 协作约定（issue tracker / triage 标签 / 领域文档）。行为规格按特性存放在 `docs/<feature>/spec.md`（配 `issues/` 本地票据），架构与决策背景见 [docs/arch.md](docs/arch.md)，术语以仓库根 [GLOSSARY.md](GLOSSARY.md) 为准。

## 环境准备

| 依赖 | 版本 | 说明 |
| --- | --- | --- |
| Rust | ≥ 1.90（`rust-toolchain.toml` 锁定具体版本） | 领域内核与 Tauri 壳 |
| Node.js | ≥ 20 | 渲染层构建与目录同步脚本 |
| pnpm | 最新稳定 | 唯一包管理器（渲染层） |
| macOS | — | 当前实测平台；Windows / Linux 打包配置已备，待实机验证 |

- 包管理只用 pnpm（渲染层）；Rust 依赖由 cargo workspace 管理，构建脚本许可在 `pnpm-workspace.yaml` 维护。
- 不要擅自安装或升级工具与依赖；需要新工具或大版本升级时先提 issue 说明情况。

## 本地开发

```bash
pnpm install

# 桌面开发模式
pnpm dev

# 使用隔离的数据目录，避免污染 ~/.craftwork
CRAFTWORK_HOME=/tmp/craftwork-dev pnpm dev

# 重新同步 models.dev 模型目录（生成文件需要提交）
node scripts/modelsync.mjs
node scripts/modelsync.mjs --check   # 只检查是否与上游一致
```

数据目录默认 `~/.craftwork`（数据库、密钥、备份、附件与内置临时会话目录），`CRAFTWORK_HOME` 可整体隔离；测试与冒烟必须使用隔离目录。多实例并存没有互斥，测试前先清理残留进程。

## 提交前检查

每次提交至少运行：

```bash
pnpm typecheck    # 渲染层类型检查
pnpm test:rust    # cargo test --workspace（领域内核）
pnpm build        # tauri build
pnpm dist:dir     # 打包冒烟（.app 可启动）
```

涉及界面的改动还要额外确认：

- `CRAFTWORK_HOME=/tmp/cw-<主题> pnpm dev` 隔离目录冒烟。
- 在 1440×900 窗口尺寸确认三栏布局与深浅主题。
- 完整走一遍：首次启动空状态 → 从模型目录添加提供商 → 配置密钥 → 发送消息 → 工具调用 → 审批允许 / 拒绝 → 创建项目对话框 → 数据面板 → 深浅主题切换。

## CI 检查内容

Pull Request 会运行 `.github/workflows/ci.yml`，包括：

| 任务 | 内容 |
| --- | --- |
| 渲染层类型检查 | `pnpm typecheck`（macOS runner） |
| Rust 检查矩阵 | macOS / Ubuntu / Windows 三平台 `cargo check / test / clippy`（非 macOS 允许失败） |

CI 未通过时不要在 PR 中直接合并。

## 代码约定

完整约束见 [AGENTS.md](AGENTS.md) 第 6 节，这里列出最容易踩错的几条：

- **分层**：领域 crate（`craft-core`、`craft-store`、`craft-policy`、`craft-agent`、`craft-model`、`craft-tools`、`craft-doc`）保持普通 Rust 模块、不依赖 Tauri（保证可 `cargo test`）；`src-tauri/src/services.rs` 只做参数校验、编排与错误转换。只有 `src-tauri` 允许接触 Tauri API。
- **事件**：事件写入与广播统一走 `craft-store` 的 `recorder::Recorder`（`seq` 分配与插入同事务）；渲染层状态壳只做「事件应用 + 动作」，`applyBatch` 是唯一事件入口。
- **通道**：渲染层一切系统操作经 `window.craftwork`（集中通道表）；新增能力在 `src/shared/channels.ts`（签名）、`src/renderer/src/lib/tauri-bridge.ts`（invoke 实现）与 `src-tauri/src/lib.rs`（dispatch 分支）三处同步接线，渲染层不使用 Node API。
- **审批**：审批必须由用户显式决定，不允许在后端侧静默放行；审批决定与工具执行都要写入审计。
- **安全**：路径校验（词法 + 符号链接真实路径）、命令分级、密钥读取、外链白名单都在 Rust 内核完成，不信任渲染层传入的路径或命令；密钥只存 Rust 进程与 `~/.craftwork/keys.json`（0600）。
- **数据**：SQLite 是事实源；修改表结构要新增迁移版本，不要改已有迁移；写文件前先备份；rusqlite 走 bundled 静态编译。
- **界面**：观感取值只使用 `styles.css` 的语义色 token（深浅两套主题同改），不写死色值；动效遵循 `prefers-reduced-motion`；图标用 lucide-react；列表渲染给稳定 `key`。
- **错误处理**：后端返回可读中文错误，界面 toast 展示；模型错误统一映射到既有错误码；后台异步失败要写日志或投递 `error` 事件，不能静默退出。
- **测试**：策略、路径、diff、存储、Agent 循环、工具、模型层等核心逻辑必须有 cargo test；修复缺陷时补对应回归测试。

新增界面、工具或提供商接入的步骤见 [AGENTS.md](AGENTS.md) 第 5 节。

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

- 改动内容与动机，关联的 issue 文件（`docs/<feature>/issues/` 下的文件，如果有）。
- 验证方式：运行了哪些命令、覆盖了哪些场景。
- 影响的文档；如果修改了界面，附上 1440×900 窗口尺寸下的截图或录屏。
- 是否引入新的依赖、迁移或配置项。

不要提交以下生成物：

- `out/`
- `dist/`
- `node_modules/`
- `.DS_Store`

## 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 行为规格（所属特性的行为变化） | 对应 `docs/<feature>/spec.md`（及其 issues 进展） |
| 推翻或新增架构与选型决策 | `docs/arch.md` 的架构章节与决策摘要同步更新 |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 协作约定（issue tracker / triage 标签 / 领域文档） | `docs/agents/` 对应文件 |
| 使用方式、功能、架构、贡献流程 | `README.md`、本文件 |
| 开发约定、命令、陷阱 | `AGENTS.md` |

## 许可证

CraftWork 使用 [Apache-2.0](LICENSE) 许可证。提交贡献即表示你同意该贡献以相同许可证发布。

## 报告问题

- 本仓库的 spec 与 issue 以本地 markdown 记录，**不使用 GitHub Issues**：每个 feature 一个目录 `docs/<feature>/`，行为规格写 `spec.md`，拆解的 issue 写 `issues/N-<slug>.md`（从 1 递增），完整约定见 [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)。
- issue 文件头部以行内字段记录 triage 标签（`needs-triage` / `needs-info` / `ready-for-agent` / `ready-for-human` / `wontfix`），词汇映射见 [docs/agents/triage-labels.md](docs/agents/triage-labels.md)。
- 描述问题请附上操作系统、版本、复现步骤与相关日志。
- 涉及密钥泄露或安全漏洞时，不要在公开渠道粘贴密钥、令牌或完整数据目录；先通过私下渠道联系维护者，并立即轮换已泄露的密钥。
