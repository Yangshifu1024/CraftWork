# AGENTS.md

本文件是本仓库的协作与实现约定，面向在本仓库内写代码的 Agent 与开发者。

`README.md` 面向使用者与贡献者，介绍功能、架构与贡献流程；本文件回答「在这个仓库里应该怎么改、怎么验证、哪些东西不能碰」。

## 1. 项目现状

CraftWork 是一个本地优先、可审计的桌面 Agent，采用 **Tauri 2 + Rust** 形态：

- Tauri 应用壳（`src-tauri/`）承载入口、窗口与 IPC 接线；全部领域逻辑在 cargo workspace 的领域 crate（`crates/craft-*`），不依赖 Tauri、可独立测试。
- 领域逻辑为 Rust：Agent 循环（craft-agent）、工具（craft-tools）、策略（craft-policy）、SQLite（craft-store）、模型客户端（craft-model）、文档处理（craft-doc）、基础能力（craft-core）。
- 渲染层 React 19 + zustand + Tailwind CSS v4；经 `tauri-bridge` 提供的 `window.craftwork` 调用面与统一 invoke 入口（craftwork_invoke）通信，渲染层不持有任何 Node 能力。
- SQLite 使用 rusqlite（bundled 静态编译，零系统依赖）；交互式终端已移除（决策 D1，摘要见 `docs/arch.md`），Agent 的命令执行走 `shell_exec` 工具。
- 提供商与模型完全由用户添加；目录来自 models.dev 的构建期快照（`include_bytes` 编译期嵌入），应用不内置任何提供商。

## 1.1 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面外壳 | Tauri 2.12（wry + tao；macOS 用系统 WKWebView） |
| 后端 | Rust ≥ 1.90（workspace：`src-tauri` + `crates/craft-*`，edition 2024）、tokio |
| 存储 | rusqlite（bundled/json/backup）+ 单连接 Mutex 串行 + WAL |
| 终端执行 | `shell_exec`：独立进程组、环境白名单清洗、超时杀树（tokio::process + command-group 预留 Windows） |
| 界面 | React 19、zustand、Tailwind CSS v4（`@theme inline` 语义色 token）、radix-ui、cmdk、lucide-react |
| 对话渲染 | streamdown（流式 Markdown，不渲染原始 HTML）+ @streamdown/code（shiki 语法高亮，纯 JS 引擎、不加载 wasm） |
| 打包 | Tauri bundler：`pnpm dist:dir`（.app 目录）/ `pnpm dist`（dmg）；`tauri.conf.json` 为版本事实源 |

规则：

- **不要擅自安装或升级工具**；需要新工具或新的大版本依赖时先向用户提出请求。
- 包管理只用 pnpm；构建脚本许可在 `pnpm-workspace.yaml`。
- Rust 侧 crate 分层纪律：`crates/*` 一律不依赖 Tauri；只有 `src-tauri` 允许接触 Tauri API。
- `docs/arch.md` 第 7 节记录已定决策摘要（D1~D7：移除终端、Agent 侧自研、craft- 前缀等），改动与决策冲突时先显式提出。

### 数据目录

- 默认 `~/.craftwork`：`craftwork.db`、`keys.json`（0600）、`backups/`、`attachments/`、`workspace/default`（内置临时会话目录）、`logs/`（运行日志：`<thread-id>.log` 会话 run 时间线 + `app.log`/`model.log` 共享日志；JSONL 追加写，等级在设置 → 通用调整，用于观测模型流截断/界面停滞等问题）。
- `CRAFTWORK_HOME=/tmp/...` 可隔离数据目录，测试与冒烟必须使用隔离目录。
- SQLite schema 与事件表格式与 Electron 版（历史版本）双向兼容：老库原位可读，迁移链逐版本追加。

### 版本号

`src-tauri/tauri.conf.json` 的 `version` 是发布产物（.app）的版本事实源；发版随打包自动生效。

### 不能提交的生成物

`out/`、`dist/`、`target/`、`node_modules/`、`.DS_Store`、`src-tauri/gen/`、`src-tauri/target/`。

## 2. 环境与工具链

| 工具 | 说明 |
| --- | --- |
| Rust ≥ 1.90（rust-toolchain.toml 锁 1.99.1） | cargo workspace 构建；MSRV 由 Tauri 2.12 决定 |
| pnpm | 渲染层包管理器；`pnpm-workspace.yaml` 承载构建脚本许可 |
| Vite 7 | 渲染层构建（`vite.config.ts`，产物 `out/web`）；Tauri devUrl 指向 5173 端口 |
| Tauri CLI | `pnpm tauri …`；开发 `pnpm dev`（= tauri dev），打包 `pnpm dist` |

## 3. 常用命令

| 目的 | 命令 |
| --- | --- |
| 桌面开发 | `pnpm dev`（= tauri dev，自动起 Vite） |
| Rust 检查/测试/lint | `pnpm check:rust` / `pnpm test:rust` / `pnpm lint:rust` |
| 类型检查 | `pnpm typecheck`（web 侧 tsconfig） |
| 打包 .app | `pnpm dist:dir`（debug 包）→ `target/debug/bundle/macos/CraftWork.app`；分发 dmg 用 `pnpm dist` |
| 同步模型目录 | `node scripts/modelsync.mjs`（检查漂移加 `--check`；生成快照后 Rust 侧经 include_bytes 生效需重编译） |
| GUI 冒烟（隔离目录） | `CRAFTWORK_HOME=/tmp/cw-smoke pnpm dev` |

## 4. 目录与职责

```text
src/shared/              跨进程共享：事件契约（events.ts）、类型（types.ts）、
                         通道表（channels.ts）、ID 生成（ids.ts）——与 Rust serde
                         类型（craft-core::contract）经契约序列化测试与
                         dispatch 接线对拍测试防漂移
src/renderer/            React 界面：主题（styles.css）、侧栏、对话流、右侧面板、
                         浮层（命令面板/设置/数据/对话框/toast）、zustand 状态壳；
                         lib/tauri-bridge 在 Tauri 环境提供 window.craftwork 调用面
src-tauri/               Tauri 应用壳：入口、窗口、capabilities、IPC 接线
  src/lib.rs             启动装配 + craftwork_invoke 统一分发入口
  src/services.rs        服务层：参数校验、编排与错误转换（对应 Electron 版 services/*）
  src/system.rs          系统能力：对话框、外链唯一出口、剪贴板、窗口控制
crates/
  craft-core/            基础设施：事件总线、契约类型、ID、平台能力（原子写/哈希/
                         环境清洗/子进程）、密钥、行级 diff
  craft-store/           SQLite：打开/迁移、threads、workspaces、events（快照重建）、
                         runs/approvals、审计/用量/设置/备份、Recorder
  craft-policy/          风险分级与审批档位、命令分类、路径安全解析、敏感附件名
  craft-model/           统一模型客户端：错误归一化、目录（内置快照 + 用户提供商）、
                         SSE、OpenAI/Anthropic 适配器、重试、工厂
  craft-tools/           工具框架（Registry）与内置工具：fs_*、apply_patch、shell_exec、
                         git_*、doc_*、xlsx_set_cell、todo_write
  craft-agent/           Run 循环、审批状态机、授权缓存、历史重建、上下文压缩、
                         自动命名、系统提示词组装、附件加载、环境上下文、会话日志
                         （session_log：JSONL 运行时间线，观测流截断/任务异常退出）
  craft-doc/             文档读写：docx / xlsx / pdf 文本级处理（低层 zip + XML）
tests/fixtures/          golden 数据库样本（Node 版生成，Rust 侧兼容测试用）
scripts/                 modelsync.mjs（拉取 models.dev 目录）
assets/catalog/          models.dev 快照（catalog.json.gz + manifest + rules；Rust include_bytes）
docs/                    arch.md（架构文档：分层、主流程图、安全模型、决策摘要）、
                         handoff.md（交接与问题记录）、agents/（协作约定）、
                         <feature>/（spec.md + issues/ 本地票据）
```

新增能力优先落到对应领域 crate；`src-tauri/src/services.rs` 只做参数校验、编排与错误转换，渲染层不得绕过通道表直接假设服务实现。

## 5. 改动流程

1. **先读文档**：仓库根 `GLOSSARY.md`（术语）、`docs/<feature>/spec.md`（行为规格，本地票据见 `docs/agents/issue-tracker.md`）、`docs/arch.md`（架构与决策摘要）；改动与既有决策冲突时先显式提出，不要静默推翻。
2. **改后端**：领域 crate 保持普通 Rust 模块，不 import Tauri（全部 `crates/*` 可 cargo test；只有 `src-tauri` 允许接触 Tauri API）。
3. **改界面**：状态壳（zustand）只做事件应用与动作；观感取值一律走 `src/renderer/src/styles.css` 的语义色 token，不写死色值。
4. **补测试**：核心逻辑（策略、路径、diff、存储、Agent、工具、模型层）必须有 cargo test。测试函数名一律用英文 snake_case，按下划线分段描述被测行为（如 `single_root_symlink_escape_rejected`）；不用中文或中英混合标识符——触发 non_snake_case 编译警告，也不利于测试输出与 CI 日志检索。
5. **跑验证**：见第 8 节。
6. **同步文档**：见第 9 节。

### 新增一个界面/能力

1. 数据与行为先落在领域 crate（或更深的模块），写成普通函数。
2. 在 `src/shared/channels.ts` 的 `CraftworkApi` 增加方法签名；`src-tauri/src/services.rs` 的 dispatch 与服务函数同步接线。
3. 界面组件放 `src/renderer/src/components/` 或 `overlays/`；状态进 zustand store，事件经 `applyBatch` 应用。
4. 颜色、字号、间距取自语义 token（`bg-background`、`text-foreground-subtle` 等），不写死值。
5. 图标用 lucide-react，按需导入。

### 新增一个工具

1. 在 `crates/craft-tools/` 实现 `Tool`（`spec` + `execute`，同步执行；调用方负责放入阻塞线程）。
2. 在 `spec` 里声明风险等级、路径参数、命令参数、Work 模式可用性与标题/摘要。
3. 在 `src-tauri/src/lib.rs` 的 `register_builtin_tools` 注册进 Registry。
4. 路径解析、输出截断与写前备份由 Registry 统一处理；写前备份调用 `ctx.env.backup`。
5. 补工具测试（`Registry::assess` + `Registry::execute` 接缝）。

### 模型目录与提供商

1. 提供商与模型清单只来自 `assets/catalog/catalog.json.gz`（由 `scripts/modelsync.mjs` 生成，Rust 经 `include_bytes!` 编译期嵌入）；**禁止在代码里新增内置提供商或模型清单**。
2. 同步规则在 `assets/catalog/rules.json`，只允许 `openai`、`anthropic`、`openai-compatible` 三种协议；Google/Gemini 原生协议、Azure、Bedrock、Vertex 一律排除。
3. 用户提供商保存在 settings 的 `model.custom_providers`；会话与设置保存 `providerID/modelID` 引用，不要只存原始模型 ID。
4. 首次启动没有默认模型；发送消息前必须由用户添加提供商并选择模型。
5. 更新目录后运行 `node scripts/modelsync.mjs`，并提交快照、manifest 与 rules；Rust 侧重编译生效。

## 6. 实现约束（必须遵守）

### 6.1 界面与数据流

- 渲染层状态壳只做「事件应用 + 动作」；会话消息从快照装载、由事件流更新（`applyBatch` 是唯一事件入口）。
- 所有系统操作经 `window.craftwork`（通道表）；渲染层不使用 Node API。
- 大对象（文件内容、差异、目录树）按需调用方法获取，不常驻全局状态。
- 列表渲染给稳定 `key`；消息流用 `use-stick-to-bottom` 贴底。

### 6.2 审批

- 审批必须显式决定：请求 → 卡片 → 用户决定 → 执行/跳过，**不允许后端侧静默放行**。
- `once / session / workspace` 三种范围由服务端校验；`allowPermanent` 为 false 时 session/workspace 一律按 once 处理。
- 受限工作区：R3 直接拒绝、不提供永久授权、已有授权不生效。
- 取消、窗口关闭、应用退出统一按拒绝或取消落审计。
- 审批决定与工具执行必须写 `audit_log`。

### 6.3 安全

- **外链唯一出口**：http/https 经 `src-tauri/src/system.rs` 的 `open_external` 校验后交系统浏览器；渲染层不加载任何远程页面。
- Markdown 不渲染原始 HTML（streamdown 默认按文本处理）。
- 所有路径校验在后端完成：`craft-policy::resolve` 先词法校验，再解析符号链接真实路径；不信任渲染层传来的路径。
- 密钥只存在 Rust 进程与 `~/.craftwork/keys.json`（0600）；不进入日志、数据库、渲染层。
- `shell_exec` 必须清洗环境变量（白名单 + 敏感词剔除）、独立进程组、超时杀树、输出截断。
- CSP、capabilities 权限清单保持开启；`on_navigation` 拦截就地导航，`window.open` 默认不可用。

### 6.4 界面约定

- 深浅两套主题（craft-light / craft-dark）都定义在 `styles.css`；语义色 token 经 `@theme inline` 映射，**新增颜色必须两套同改**。
- 设置项必须立即生效；未接线的控件要禁用并标注里程碑，不要做「看起来能改但无效果」的控件。
- 动效遵循 `prefers-reduced-motion`；运行态统一用文字扫光（`animated-gradient-text`），不旋转图标。
- 窗口级快捷键集中在 `src/renderer/src/hooks/shortcuts.ts`；Esc 退出链为「浮层一层 → 输入框不拦截」。
- 窗口拖拽区：标题条元素加 `data-tauri-drag-region`（逐元素命中模型；Electron 时代的 `-webkit-app-region` 已随迁移移除）。

### 6.5 数据

- SQLite 是事实源；事件表按会话 `seq` 单调递增，写入在事务内完成（seq 分配与插入同事务）。
- 事件写入与广播统一走 `craft-store::recorder::Recorder`，不要手写 append + emit 配对。
- 修改表结构时在 `crates/craft-store/src/db.rs` 追加迁移版本（`SCHEMA_VERSION`），不要直接改已有迁移；**已发布版本号语义不得改动**。
- 写入文件前备份；外部修改时用 `expectedHash` 阻止覆盖。

### 6.6 错误处理

- 后端返回可读中文错误；渲染层 toast 展示（tauri-bridge 的 invoke 拒绝即错误字符串）。
- 模型调用错误统一使用 `craft-model::errors` 的错误码（`auth`、`rate_limit`、`timeout`、`context_overflow` 等）；新增提供商必须映射到既有错误码，不要向界面抛原始 HTTP 错误。
- 后台异步失败要写日志或投递 `error` 事件，不能静默退出。

## 7. 已知坑

Tauri 与工具链：

- 渲染层构建产物经 `tauri://` 加载，`vite.config.ts` 的 `base` 必须是 `"./"`，否则白屏。
- Tailwind v4 的语义色必须经 `@theme inline` 映射（token 值用 `var(--craft-*)`），直接在普通选择器里定义 `--color-*` 不会生成工具类。
- Tauri 的窗口拖拽是逐元素命中：`data-tauri-drag-region` 只对直接挂载的元素生效，子元素天然不可拖（与 Electron 的矩形减法合成完全不同，不要套用旧心智）。
- `craftwork_invoke` 的同步领域操作在 `spawn_blocking` 里跑；`Runner::run` 在无 tokio reactor 的线程上会自动起 current-thread 运行时（见 runner.rs 的 spawn 双路径）。
- 包名不能与 Rust 内置 crate 同名：`core` 会遮蔽内置 crate 且 `::core::` 绝对路径连带 derive 宏一起失效（实测 E0432，决策 D3 用 craft- 前缀的原因）。
- rusqlite 0.37 无 `json` feature（bundled SQLite 3.5x 默认内建 JSON 函数）；`Connection` 非 Sync，共享走 Mutex。
- macOS 打包签名与公证未启用（决策 D6：凭据到位后填配置即生效）；跨平台打包无原生模块障碍（无 node-pty），Win/Linux 实机走查待排期。

通用：

- 应用没有任何内置提供商；不要写死 `openai`、`gemini` 等提供商分支，也不要提供「推荐模型」。
- 同一模型 ID 可以出现在多个提供商下，解析与选择必须使用 `provider/model` 引用；只有兼容旧数据时才用原始 ID 回退。
- `assets/catalog/catalog.json.gz` 是生成物，不要手改；更新目录用 `node scripts/modelsync.mjs`。
- 交互式终端已移除（D1）：不要重新引入 PTY/xterm 相关代码；Agent 的命令执行走 `shell_exec` 工具。

## 8. 验证清单

提交前至少执行：

```bash
pnpm typecheck
pnpm test:rust        # cargo test --workspace
pnpm lint:rust        # cargo clippy --workspace -- -D warnings
pnpm build            # tauri build
```

涉及界面时额外：

- `CRAFTWORK_HOME=/tmp/cw-<主题> pnpm dev` 隔离目录冒烟。
- 走查一遍：首次启动空状态 → 从模型目录添加提供商 → 配置密钥 → 发送消息 → 工具调用 → 审批允许/拒绝 → 数据面板 → 深浅主题切换。

## 9. 文档同步

| 改动 | 需要同步 |
| --- | --- |
| 术语或概念定义 | 仓库根 `GLOSSARY.md` |
| 使用方式、功能、架构、贡献流程 | `README.md`、`CONTRIBUTING.md` |
| 架构结构、主流程、决策记录 | `docs/arch.md` |
| 开发约定、命令、陷阱 | 本文件 |

## 10. 当前留桩

- 自动更新：updater 链路未接线（tauri-plugin-updater 未引入、无签名公钥，仅设置字段留存），签名密钥与更新服务器待凭据后另立项接线（决策 D6）。
- 沙箱与 MCP（stdio 客户端）：设置页可存储服务器配置，连接能力留桩（决策 D5）。
- Work 模式限制：Word 只支持同一格式片段内的文本替换；Excel 不计算公式；PDF 只读文本层，不支持 OCR 与编辑。
- Python 批量操作：环境 UTF-8 补丁、`python` 系只读识别、运行时探测与提示词引导已接线（决策 D10）。
- 技能（Skills）：目录扫描、管理页、Agent 调用（`skill` 工具）与 composer 斜杠显式调用已接线（决策 D11，规格见 `docs/skills/spec.md`）；附加目录扫描、插件技能源、创建脚手架与 frontmatter 扩展字段留桩。
- 真实密钥验收：OpenAI、Anthropic 或任意 OpenAI 兼容网关需要本地配置密钥后各跑一次工具调用闭环。
- 跨平台：NSIS/AppImage 配置已备；无原生模块障碍（无 node-pty），Windows / Linux 实机执行与走查待排期；仅 macOS 实测。

## 11. 提交规范

- 提交信息使用中文，一次提交只做一件逻辑上的事。
- 提交前运行第 8 节的检查；不要提交 `out/`、`dist/`、`target/`、`node_modules/`。
- 贡献使用 DCO：`git commit -s`。
- 界面视觉与交互的变更先更新受影响的行为 spec（`docs/<feature>/spec.md` 或对应 issue 的行为约定），再改实现；决策变更同步 `docs/arch.md` 的决策摘要。

## Agent skills

### Issue tracker

本地 markdown：spec 与 issue 存放在 `docs/<feature>/`（`spec.md` + `issues/N-<slug>.md`），不使用 GitHub Issues。See `docs/agents/issue-tracker.md`.

### Triage labels

默认五角色标签，以 issue 文件内字段记录。See `docs/agents/triage-labels.md`.

### Domain docs

single-context：根目录 `GLOSSARY.md` + `docs/arch.md`。See `docs/agents/domain.md`.
