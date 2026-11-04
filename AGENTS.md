# AGENTS.md

本文件是本仓库的协作与实现约定，面向在本仓库内写代码的 Agent 与开发者。

`README.md` 面向使用者与贡献者，介绍功能、架构与贡献流程；本文件回答「在这个仓库里应该怎么改、怎么验证、哪些东西不能碰」。

## 1. 项目现状

CraftWork 是一个本地优先、可审计的桌面 Agent，采用 **TypeScript + Electron** 形态：

- Electron 主进程承载全部领域逻辑（Agent 循环、工具、策略、SQLite、终端、密钥），普通 TypeScript 模块，无绑定层。
- 渲染层 React 19 + zustand + Tailwind CSS v4；主/渲染进程经 `contextBridge` 与集中通道表通信，渲染层不持有任何 Node 能力。
- SQLite 使用 Node 内置 `node:sqlite`（零原生依赖）；终端使用 `node-pty` 真实 PTY + xterm.js。
- 提供商与模型完全由用户添加；目录来自 models.dev 的构建期快照，应用不内置任何提供商。

## 1.1 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面外壳 | Electron 45（main / preload / renderer 三段构建，electron-vite） |
| 界面 | React 19、zustand、Tailwind CSS v4（`@theme inline` 语义色 token）、radix-ui、cmdk、lucide-react |
| 对话渲染 | streamdown（流式 Markdown，不渲染原始 HTML） |
| 终端 | node-pty（真实 PTY）+ @xterm/xterm |
| 后端 | TypeScript 5.9（strict）、Node 内置 `node:sqlite`、zod |
| 测试 | vitest |
| 打包 | electron-builder |

规则：

- **不要擅自安装或升级工具**；需要新工具或新的大版本依赖时先向用户提出请求。
- 包管理只用 pnpm；原生模块的构建许可在 `pnpm-workspace.yaml` 的 `onlyBuiltDependencies` 里维护。
- `node:sqlite` 必须经 `createRequire(import.meta.url)` 引入，避免打包器把内置模块改写成 npm 包。

### 数据目录

- 默认 `~/.craftwork`：`craftwork.db`、`keys.json`、`backups/`、`attachments/`、`workspace/default`（内置临时会话目录）。
- `CRAFTWORK_HOME=/tmp/...` 可隔离数据目录，测试与冒烟必须使用隔离目录。

### 版本号

`package.json` 的 `version` 是发布产物（.app）的版本事实源；`src/main/index.ts` 的 `APP_VERSION` 与之保持一致，发版两处同改。

### 不能提交的生成物

`out/`、`dist/`、`node_modules/`、`.DS_Store`。

## 2. 环境与工具链

| 工具 | 说明 |
| --- | --- |
| Node ≥ 24 | `node:sqlite` 稳定可用；Electron 45 内置 Node 24 |
| pnpm | 唯一包管理器；`pnpm-workspace.yaml` 承载构建脚本许可 |
| electron-vite | 开发/构建：`pnpm dev` / `pnpm build`（main、preload、renderer 三段） |
| electron-builder | 打包：`pnpm dist:dir`（.app 目录）/ `pnpm dist`（含签名流程时再开） |

## 3. 常用命令

| 目的 | 命令 |
| --- | --- |
| 桌面开发 | `pnpm dev` |
| 单元测试 | `pnpm test`（vitest，`tests/` 目录） |
| 类型检查 | `pnpm typecheck`（node 侧 + web 侧两个 tsconfig） |
| 构建产物 | `pnpm build`（输出 `out/`） |
| 打包 .app | `pnpm dist:dir` → `dist/mac-arm64/CraftWork.app` |
| 同步模型目录 | `node scripts/modelsync.mjs`（检查漂移加 `--check`；生成快照并重写内嵌模块） |
| GUI 冒烟（隔离目录） | `CRAFTWORK_HOME=/tmp/cw-smoke pnpm dev` |

## 4. 目录与职责

```text
src/shared/              跨进程共享：事件契约（events.ts）、类型（types.ts）、
                         IPC 通道表（channels.ts）、ID 生成（ids.ts）
src/main/index.ts        入口：单实例、数据目录、服务装配、窗口、通知
src/main/core/           基础设施：事件总线、Recorder、平台能力（原子写/哈希/
                         环境清洗/子进程）、密钥、行级 diff
src/main/store/          SQLite：打开/迁移、threads、workspaces、events（快照
                         重建）、runs/approvals、审计/用量/设置/备份
src/main/policy/         风险分级与审批档位、命令分类、路径安全解析、敏感附件名
src/main/tools/          工具框架（Registry）与内置工具：fs_*、apply_patch、
                         shell_exec、git_*、doc_*、xlsx_set_cell、todo_write
src/main/model/          统一模型客户端：类型、错误归一化、目录（内置快照 +
                         用户提供商）、SSE 骨架、OpenAI/Anthropic 适配器、重试、工厂
src/main/agent/          Run 循环、审批状态机、授权缓存、历史重建、上下文压缩、
                         自动命名、系统提示词组装、附件加载、环境上下文
src/main/services/       面向界面的编排层：设置、会话、对话、提供商、工作区、
                         审计、应用（外链/剪贴板/附件）、MCP、终端
src/main/ipc.ts          IPC handler 全量接线（通道表 ↔ 服务方法）
src/main/doc/            文档读写：docx / xlsx / pdf 文本级处理
src/preload/index.ts     contextBridge 桥（方法清单与通道表一一对应）
src/renderer/            React 界面：主题（styles.css）、侧栏、对话流、右侧面板、
                         浮层（命令面板/设置/数据/对话框/toast）、zustand 状态壳
tests/                   vitest 单测（policy / editor / store / model / tools /
                         title / context）
scripts/                 modelsync.mjs（拉取 models.dev 目录、按规则过滤、生成快照与
                         logo 资产并内嵌为 TS 模块）
assets/catalog/          models.dev 快照（catalog.json.gz + manifest + rules）
docs/                    adr（决策记录）、specs（行为规格）
```

新增能力优先落到对应领域包；`src/main/services` 只做参数校验、编排与错误转换，渲染层不得绕过通道表直接假设服务实现。

## 5. 改动流程

1. **先读文档**：仓库根 `GLOSSARY.md`（术语）、`docs/specs/`（行为规格）、`docs/adr/`（决策记录）；改动与既有 ADR 冲突时先显式提出，不要静默推翻。
2. **改主进程**：领域包保持普通 TypeScript 模块，不 import Electron（`core/`、`store/`、`policy/`、`agent/`、`model/`、`tools/` 保持可单测；只有 `index.ts`、`ipc.ts`、`services/app.ts`、`services/terminal.ts` 允许接触 Electron API）。
3. **改界面**：状态壳（zustand）只做事件应用与动作；观感取值一律走 `src/renderer/src/styles.css` 的语义色 token，不写死色值。
4. **补测试**：核心逻辑（策略、路径、diff、存储、Agent、工具、模型层）必须有 vitest 单测。
5. **跑验证**：见第 8 节。
6. **同步文档**：见第 9 节。

### 新增一个界面/能力

1. 数据与行为先落在领域包（`services` 或更深的包），写成普通函数。
2. 在 `src/shared/channels.ts` 的 `CraftworkApi` 增加方法签名；`src/preload/index.ts` 的方法清单与 `src/main/ipc.ts` 同步接线。
3. 界面组件放 `src/renderer/src/components/` 或 `overlays/`；状态进 zustand store，事件经 `applyBatch` 应用。
4. 颜色、字号、间距取自语义 token（`bg-background`、`text-foreground-subtle` 等），不写死值。
5. 图标用 lucide-react，按需导入。

### 新增一个工具

1. 在 `src/main/tools/` 实现 `Tool`（`spec` + `execute`）。
2. 在 `spec` 里声明风险等级、路径参数、命令参数、Work 模式可用性与标题/摘要。
3. 在 `src/main/index.ts` 注册进 Registry。
4. 路径解析、输出截断与写前备份由 Registry 统一处理；写前备份调用 `ctx.backup`。
5. 补工具测试，走 `Registry.assess` + `Registry.execute` 接缝。

### 模型目录与提供商

1. 提供商与模型清单只来自 `src/main/model/catalogdata.ts`（由 `scripts/modelsync.mjs` 生成）；**禁止在代码里新增内置提供商或模型清单**。
2. 同步规则在 `assets/catalog/rules.json`，只允许 `openai`、`anthropic`、`openai-compatible` 三种协议；Google/Gemini 原生协议、Azure、Bedrock、Vertex 一律排除。
3. 用户提供商保存在 settings 的 `model.custom_providers`；会话与设置保存 `providerID/modelID` 引用，不要只存原始模型 ID。
4. 首次启动没有默认模型；发送消息前必须由用户添加提供商并选择模型。
5. 更新目录后运行 `node scripts/modelsync.mjs`，并提交快照、manifest 与内嵌模块。

## 6. 实现约束（必须遵守）

### 6.1 界面与数据流

- 渲染层状态壳只做「事件应用 + 动作」；会话消息从快照装载、由事件流更新（`applyBatch` 是唯一事件入口）。
- 所有系统操作经 `window.craftwork`（通道表）；渲染层不使用 Node API。
- 大对象（文件内容、差异、目录树）按需调用方法获取，不常驻全局状态。
- 列表渲染给稳定 `key`；消息流用 `use-stick-to-bottom` 贴底。

### 6.2 审批

- 审批必须显式决定：请求 → 卡片 → 用户决定 → 执行/跳过，**不允许主进程侧静默放行**。
- `once / session / workspace` 三种范围由服务端校验；`allowPermanent` 为 false 时 session/workspace 一律按 once 处理。
- 受限工作区：R3 直接拒绝、不提供永久授权、已有授权不生效。
- 取消、窗口关闭、应用退出统一按拒绝或取消落审计。
- 审批决定与工具执行必须写 `audit_log`。

### 6.3 安全

- **外链唯一出口**：http/https 经 `AppService.openExternal` 校验后交给系统浏览器；渲染层不加载任何远程页面。
- Markdown 不渲染原始 HTML（streamdown 默认按文本处理）。
- 所有路径校验在主进程完成：`policy/resolve` 先词法校验，再解析符号链接真实路径；不信任渲染层传来的路径。
- 密钥只存在主进程与 `~/.craftwork/keys.json`（0600）；不进入日志、数据库、渲染层。
- `shell_exec` 必须清洗环境变量（白名单 + 敏感词剔除）、独立进程组、超时杀树、输出截断。
- CSP、`setWindowOpenHandler` 全 deny、`will-navigate` 拦截保持开启。

### 6.4 界面约定

- 深浅两套主题（zai-light / zai-dark）都定义在 `styles.css`；语义色 token 经 `@theme inline` 映射，**新增颜色必须两套同改**。
- 设置项必须立即生效；未接线的控件要禁用并标注里程碑，不要做「看起来能改但无效果」的控件。
- 动效遵循 `prefers-reduced-motion`；运行态统一用文字扫光（`animated-gradient-text`），不旋转图标。
- 窗口级快捷键集中在 `src/renderer/src/hooks/shortcuts.ts`；Esc 退出链为「浮层一层 → 输入框不拦截」。

### 6.5 数据

- SQLite 是事实源；事件表按会话 `seq` 单调递增，写入在事务内完成（seq 分配与插入同事务）。
- 事件写入与广播统一走 `core/record.Recorder`，不要手写 `appendEvent` + `bus.emit` 配对。
- 修改表结构时在 `store/db.ts` 追加迁移版本，不要直接改已有迁移。
- 写入文件前备份；外部修改时用 `expectedHash` 阻止覆盖。

### 6.6 错误处理

- 主进程返回可读中文错误；渲染层 toast 展示（`cleanErrorMessage` 剥 IPC 包装）。
- 模型调用错误统一使用 `model/errors.ts` 的错误码（`auth`、`rate_limit`、`timeout`、`context_overflow` 等）；新增提供商必须映射到既有错误码，不要向界面抛原始 HTTP 错误。
- 后台异步失败要写日志或投递 `error` 事件，不能静默退出。

## 7. 已知坑

Electron 与工具链：

- preload 产物是 ESM（`index.mjs`），窗口里引用路径必须写对。
- 打包后 renderer 经 `file://` 加载，`electron.vite.config.ts` 的 renderer `base` 必须是 `"./"`，否则白屏。
- Tailwind v4 的语义色必须经 `@theme inline` 映射（token 值用 `var(--zai-*)`），直接在普通选择器里定义 `--color-*` 不会生成工具类。
- `-webkit-app-region` 的可拖区域合成是扁平矩形减法：`no-drag` 祖先的矩形会从可拖形状里整体减掉，其后代再用 `drag` 声明也无法覆盖回来（macOS 实测）。所以可拖区域不能嵌在 `no-drag` 容器里「翻身」；正确模式是 app-region 默认值为 `no-drag`，只在顶层条上声明 `drag`，再对其内部不可拖的控件（标题文字、按钮）声明 `no-drag` 挖洞。
- pnpm 12 的原生模块构建许可在 `pnpm-workspace.yaml`（`allowBuilds` 映射），不在 package.json；package.json 里的 `pnpm` 字段整块不再被读取。
- electron 45 起不再声明 postinstall 脚本，二进制改为「首次使用时懒下载」；electron-vite 只认 `node_modules/electron/path.txt`，读不到就抛 `Electron uninstall`。根项目 `prepare` 脚本已接入官方 `install-electron` 命令补齐二进制（幂等），删除 `node_modules` 重装后无需手动处理；个别环境需要手动补跑时执行 `node node_modules/electron/install.js`。
- pnpm 供应链策略会拒绝发布不满最短时间的包版本；遇到时退回上一个稳定小版本。
- Electron 的单实例锁按 userData 目录；dev 与打包产物同时运行会互踢，测试前先杀干净残留进程。
- `node-pty` 在 renderer 不可用；只能在主进程经 `createRequire` 引入。
- 系统通知、对话框、剪贴板等 Electron API 版本间有同步/异步差异（如 `clipboard.readText` 返回 Promise），升级时逐个核对。

通用：

- 应用没有任何内置提供商；不要写死 `openai`、`gemini` 等提供商分支，也不要提供「推荐模型」。
- 同一模型 ID 可以出现在多个提供商下，解析与选择必须使用 `provider/model` 引用；只有兼容旧数据时才用原始 ID 回退。
- `catalogdata.ts` 与 `catalog.json.gz` 是生成物，不要手改；更新目录用 `node scripts/modelsync.mjs`。

## 8. 验证清单

提交前至少执行：

```bash
pnpm typecheck
pnpm test
pnpm build      # 产物构建成功
pnpm dist:dir   # 打包冒烟（.app 可启动）
```

涉及界面时额外：

- `CRAFTWORK_HOME=/tmp/cw-<主题> pnpm dev` 隔离目录冒烟。
- 走查一遍：首次启动空状态 → 从模型目录添加提供商 → 配置密钥 → 发送消息 → 工具调用 → 审批允许/拒绝 → 终端 → 数据面板 → 深浅主题切换。

## 9. 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、`CONTRIBUTING.md` |
| 开发约定、命令、陷阱 | 本文件 |

## 10. 当前留桩

- 自动更新：下载 / 替换与回滚未实现。
- 沙箱与 MCP（stdio 客户端）：设置页可存储服务器配置，连接能力留桩。
- Work 模式限制：Word 只支持同一格式片段内的文本替换；Excel 不计算公式；PDF 只读文本层，不支持 OCR 与编辑。
- 真实密钥验收：OpenAI、Anthropic 或任意 OpenAI 兼容网关需要本地配置密钥后各跑一次工具调用闭环。
- 跨平台：Windows（ConPTY 终端）与 Linux 打包需要实机验证；仅 macOS 实测。

## 11. 提交规范

- 提交信息使用中文，一次提交只做一件逻辑上的事。
- 提交前运行第 8 节的检查；不要提交 `out/`、`dist/`、`node_modules/`。
- 贡献使用 DCO：`git commit -s`。
- 界面视觉与交互的变更先更新受影响的行为 spec，再改实现；决策变更走 `docs/adr/`。

## Agent skills

### Issue tracker

本地 markdown：spec 与 issue 存放在 `docs/<feature>/`（`spec.md` + `issues/N-<slug>.md`），不使用 GitHub Issues。See `docs/agents/issue-tracker.md`.

### Triage labels

默认五角色标签，以 issue 文件内字段记录。See `docs/agents/triage-labels.md`.

### Domain docs

single-context：根目录 `GLOSSARY.md` + `docs/adr/`。See `docs/agents/domain.md`.
