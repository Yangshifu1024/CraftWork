# Tauri 2 迁移计划

状态：**已实施完成（2026-11-05，P0~P9 全部落地，待用户完整验收；updater 除外——链路未接线，见 D6 与 migration-parity 票 19）**。是否迁移的 ADR 决策与三项产品/架构决策见下节「已定决策」，`docs/adr/0002` 随 issue 01 落盘。

## 已定决策

| # | 决策 | 影响 |
| --- | --- | --- |
| D1 | **移除终端功能**（交互式 PTY 终端面板：node-pty + xterm.js + 终端 IPC 面） | 不再引入 portable-pty；`shell_exec` 工具（Agent 执行命令的能力）保留；WKWebView 中文 IME 风险整体消除；node-pty 原生模块移除后，三平台打包不再有交叉编译障碍 |
| D2 | **Agent 侧完全自研，主要参考 openai/codex**（codex-rs，Apache-2.0） | 不依赖 rig / genai / goose 等 Agent 框架或其组件；Run 循环、审批、上下文压缩、模型客户端（SSE + 双协议适配器）全部自研平移，架构与工程拆分参考 codex-rs。边界说明：「自研」指 Agent/模型层；基础设施 crate（tokio、rusqlite、reqwest、zip、quick-xml、similar 等）仍使用成熟第三方库 |
| D3 | **crate 命名采用 `craft-` 前缀**（原决策为「无前缀，有冲突则换前缀」，实测后触发） | `crates/craft-{core,store,policy,model,tools,agent,doc}`。触发依据：2026-11-05 最小 workspace 实测，本地包名 `core` 会遮蔽 Rust 内置 `core` crate，且**连 `::core::fmt` 绝对路径也被遮蔽**（E0432）——serde / thiserror 等 derive 宏展开代码普遍使用 `::core::` 绝对路径，任何依赖名为 `core` 的包的 crate 都会编译失败，属硬冲突，无规避价值 |
| D4 | 右侧面板移除终端标签后保留「文件 / 文档」两个标签，默认标签改为「文件」 | 面板结构不动，改动局部（issue 02） |
| D5 | MCP（stdio 客户端）继续留桩 | 与现状一致：设置页可存服务器配置，连接能力不实现；rmcp 连接建议迁移完成后单独立项 |
| D6 | 先出未签名包；签名、公证待凭据；**updater 链路未接线**（计划曾称「备好」，实况 tauri-plugin-updater 未引入、无签名公钥，仅设置字段留存） | P9 产出可运行 .app / dmg（unsigned）；凭据到位后签名/公证填配置即生效，updater 另立项接线（docs/migration-parity/issues/19） |
| D7 | 本计划仅保障 macOS 实测 | Win / Linux 实机走查后续单独立项（D1 移除终端后打包障碍已消失） |

## 0. 调研结论摘要（计划依据）

**收益**：

- 渲染层（React 19 + zustand + Tailwind v4 + radix-ui + cmdk + streamdown）约 90% 代码原样保留，仅构建目标与标题栏拖拽区模型需要调整。
- 安装包体积与内存占用显著下降（Electron 自带 Chromium → 系统 WebView）。
- 安全模型净增强：模型密钥与 SQLite 天然隔离在 Rust 进程；capabilities 权限清单把「每个 IPC 面显式授权」变成框架强制；审批与路径校验的进程边界更硬。
- 官方插件覆盖全部现有系统能力（剪贴板、通知、对话框、外链、单实例、自动更新），2026 年 10 月仍在活跃发版。
- 自动更新从留桩变为开箱可用（强制 Ed25519 签名校验）。
- 移除终端后无任何原生 Node 模块依赖（node:sqlite → rusqlite bundled 静态编译），跨平台打包可行性大增。

**代价与退步**：

- 主进程全部领域逻辑（约 6 个领域包 + 服务层）需用 Rust 重写，无自动迁移路径——这是工程量绝对主体。
- 失去与 Node/npm 生态共享主进程代码的能力（未来需要 npm 生态时只能走 sidecar 子进程）。
- 更新无差量下载（Electron 有 blockmap 差量），每次全量下载。
- 三平台 WebView 引擎不再一致（macOS WKWebView / Windows WebView2 / Linux WebKitGTK），macOS 端渲染引擎从 Chromium 变为 WebKit，需全量视觉回归。
- 本计划范围内仅保障 macOS（与现状一致）；Windows / Linux 需实机另行走查。

**生态关键事实**（2026-11 时点）：

- Tauri 稳定线 2.12.x（MSRV Rust 1.90），2.12 起放弃 Windows 7；Tauri 3 刚进 alpha，迁移周期内无大版本风险。
- Rust Agent 框架（rig、Goose、swiftide）无一与本项目「审批一等公民 + 事件流驱动 + 本地可审计 + 运行时自定义提供商」对位；所有知名 Rust 编码 Agent（Codex CLI、Zed、Goose、Forge）均自研循环——**与 D2 决策相互印证：循环自研是此类产品的普遍选择**。
- codex-rs 是「自研循环」最完整的开源参考实现（Apache-2.0，130+ crate：protocol / tools / execpolicy / sandboxing / rollout / memories 等），官方博客《Unrolling the Codex agent loop》阐述其循环设计。

## 1. 目标架构

### 1.1 技术基线

| 项 | 选型 |
| --- | --- |
| 桌面外壳 | Tauri 2.12.x（wry + tao），`src-tauri/` 承载应用壳与 IPC 接线 |
| 语言/运行时 | Rust ≥ 1.90，tokio 异步运行时 |
| 渲染层 | 不变：React 19、zustand、Tailwind CSS v4、radix-ui、cmdk、streamdown（**移除** @xterm/xterm、@xterm/addon-fit） |
| 前端构建 | Vite 7（沿用），tauri.conf.json 四件套接线（beforeDevCommand / devUrl / frontendDist / beforeBuildCommand） |
| 存储 | rusqlite 0.41（bundled + json + backup），单写线程 + WAL（实际落地 0.37 bundled；json feature 经 bundled SQLite 内建 JSON 函数替代） |
| 进程执行（shell_exec 工具） | tokio::process（process_group）+ command-group（Windows Job Object 路线预留） |
| 模型客户端 | 自研：reqwest 0.13（stream feature）+ SSE 状态机直译现有 JS 逻辑（D2） |
| 打包 | Tauri bundler（dmg / NSIS / AppImage / deb），CI 用 tauri-action 三平台矩阵 |
| 测试 | 主进程侧 cargo test（vitest 用例逐包翻译）｜实况注记：前端 vitest 随 Electron 形态一并移除，现状测试 = cargo test + dispatch 接线对拍 + 契约金样 |

### 1.2 目录结构（cargo workspace，与现有领域包一一对应）

```text
src/                      # 渲染层：原样保留（React 19，终端 UI 移除）
src/shared/               # 事件契约、类型、通道表：迁为 TS+Rust 双侧类型（见 §3）
src-tauri/                # Tauri 应用壳：入口、窗口、托盘、IPC 接线、capabilities/
crates/
  craft-core/             # ← core/：事件总线、Recorder、平台能力（原子写/哈希/环境清洗/子进程）、密钥、行级 diff
  craft-store/            # ← store/：SQLite 打开/迁移、threads、workspaces、events、runs/approvals、审计/用量/设置/备份
  craft-policy/           # ← policy/：风险分级、审批档位、命令分类、路径安全解析、敏感附件名
  craft-model/            # ← model/：模型客户端（目录、SSE、适配器、重试、错误归一化）
  craft-tools/            # ← tools/：Registry 与内置工具
  craft-agent/            # ← agent/：Run 循环、审批状态机、授权缓存、历史重建、压缩、命名、提示词组装
  craft-doc/              # ← doc/：docx / xlsx / pdf 文本级处理
```

分层纪律不变：`crates/*` 一律不依赖 Tauri（保持可 cargo test）；只有 `src-tauri` 允许接触 Tauri API，等价于现在「只有 index.ts / ipc.ts / services/app.ts / services/terminal.ts 允许接触 Electron」。

### 1.3 数据目录与兼容性（重要）

- 继续沿用 `~/.craftwork`（Rust 侧显式指定，不迁到系统 App Data 目录）：`craftwork.db`、`keys.json`、`backups/`、`attachments/`、`workspace/default` 全部原位复用。
- SQLite schema 与事件表格式不变：**Electron 版写入的数据库，Rust 版直接可读**，老用户零迁移。这是分阶段并行开发的前提。
- 迁移链（`store/db.ts` 的版本序列）在 `craft-store` crate 逐版本翻译，已发布的旧版本号语义不得改动。

## 2. 技术选型清单（模块 → 方案）

| 现有实现 | Tauri/Rust 方案 | 版本基线 | 备注 |
| --- | --- | --- | --- |
| node:sqlite | rusqlite（bundled, json, backup, functions） | 0.41.x（实际落地 0.37，仅 bundled） | 单写线程经 mpsc 串行提交，「seq 分配与插入同事务」语义不变 |
| 自写 SSE + OpenAI/Anthropic 适配器 | **自研平移**：reqwest 0.13（stream feature）+ SSE 状态机直译；循环层设计参考 codex-rs 的 provider 抽象（D2） | reqwest 0.13.x | 错误码归一化必须映射回 `auth / rate_limit / timeout / context_overflow` 等 |
| 模型流式推送到前端 | tauri::ipc::Channel（有序、高吞吐），Rust 侧 50~100ms 攒批 | tauri 2.12 | 低频状态广播才用 emit |
| 密钥 keys.json（0600） | 平移现有方案（std::fs + PermissionsExt + 原子写） | — | keyring 4.x 仅作未来可选增强，不做硬依赖 |
| docx（jszip + 手写 XML） | zip + quick-xml 低层路线（直译现有 JS 逻辑） | zip 9.x / quick-xml 0.42 | Rust 无 python-docx 等价物，此为社区公认做法 |
| xlsx（xlsx_set_cell） | umya-spreadsheet 3.x（保真度不达标则退低层路线） | 3.1.x | 需真实文件保真度测试定案 |
| pdf（只读文本层） | pdf-extract（不达标再评估 pdfium-render + 分发动态库） | 0.12.x | mupdf-rs 因 AGPL 排除 |
| 行级 diff | similar crate 或直译现有算法 | — | — |
| shell_exec（环境清洗/进程组/杀树） | tokio::process + `process_group(0)` + 两段式超时杀进程组；command-group（Windows） | tokio 1.x / command-group 5.x | Windows 白名单必须保留 `SystemRoot`、`PATH` |
| 剪贴板/通知/对话框/外链/单实例 | tauri-plugin-clipboard-manager / notification / dialog / opener / single-instance | 2.x 稳定线 | 全部走 capabilities 授权 |
| 自动更新（现留桩） | tauri-plugin-updater（**未接入**，待凭据后立项） | 2.13.x | 强制签名；无差量更新 |
| contextBridge 通道表 | #[tauri::command] + capabilities 权限清单 | — | 自定义命令默认对窗口全开，须在 build.rs 显式收紧并审计 |
| will-navigate 拦截 / setWindowOpenHandler | on_navigation 返回 false；window.open 默认不可用 | — | 外链唯一出口 = opener 插件 |
| -webkit-app-region 拖拽区 | data-tauri-drag-region（逐元素命中模型） | — | 现有「矩形减法合成」坑不再适用，标题栏需重做结构 |

## 3. 类型与事件契约双维护

`src/shared/types.ts`、`events.ts`、`channels.ts` 是跨进程契约，迁移后两侧分别是 TypeScript 与 Rust serde 结构体。策略：

1. 以现有 TS 类型为准，手写 Rust serde 对应类型（独立 crate 或放 `craft-core`，落地时定）。
2. 建**黄金文件对拍测试**：固定一组代表性事件 JSON 样本（含全部 Part 类型、审批事件、错误事件），Rust 侧 serde 解析 + 重新序列化后与 TS 侧 `JSON.parse/stringify` 结果做结构等价断言，防两侧漂移。
3. 通道映射：

| 现有模式 | Tauri 2 对应 |
| --- | --- |
| `window.craftwork.*` 方法（通道表） | `#[tauri::command]` + preload 等价的 TS invoke 封装（保持渲染层调用面不变；终端相关方法随 D1 删除） |
| 事件广播（applyBatch 入口） | emit（低频状态），载荷仍是事件批次 JSON |
| 工具输出流 / 模型 token 流 | tauri::ipc::Channel，Rust 侧攒批推送 |
| 大对象按需获取（diff、目录树、附件） | command 返回（超大走 ipc::Response 原始字节） |

## 4. 阶段总览

依赖关系：P0 → P1 → P2 →（P3、P4 可并行）→ P5 →（P6、P7、P8 可并行）→ P9。每个阶段独立可验证，全程 main 分支保持 Electron 版可构建可发布，迁移在仓库内并行目录推进，最终一次性切换。

| 阶段 | issue | 目标 | 关键产出 | 验收要点 |
| --- | --- | --- | --- | --- |
| P0 决策与脚手架 | 01 | ADR 落盘、搭 workspace | ADR 0002、cargo workspace、CI 骨架 | cargo check/test 全绿；Electron 版不受影响 |
| P1 终端移除 + 应用壳与渲染层 | 02 | 先移除终端功能，再平移渲染层 | 无终端的干净渲染层 + 可启动的 Tauri 壳 | 空状态界面、深浅主题、标题栏拖拽、字体/流式渲染走查 |
| P2 存储层与密钥 | 03 | craft-core + craft-store | 数据原位复用、迁移链翻译 | 用 Electron 版生成的 db 通过快照重建与迁移测试 |
| P3 模型客户端 | 04 | craft-model | 自研 SSE 客户端（参考 codex provider 设计） | 错误码映射单测、流式解析单测、目录加载 |
| P4 策略/工具/审批 | 05 | craft-policy + craft-tools | 风险分级、路径安全、Registry、审批状态机 | 现有 policy/tests 用例逐条翻译通过 |
| P5 Run 循环与 Agent | 06 | craft-agent | 端到端对话闭环（参考 codex-rs 循环设计） | 发消息 → 流式回复 → 工具 → 审批 → 落库 → 前端渲染全链路 |
| P6 进程执行 | 07 | shell_exec 完整执行体 | Agent 命令执行能力 | 超时杀树（含孙进程）、环境清洗、输出截断 |
| P7 文档工具 | 08 | craft-doc | docx/xlsx/pdf 三工具 | 真实样本文件与 Electron 版输出对拍 |
| P8 系统能力接线 | 09 | 服务层 + capabilities | 剪贴板/通知/对话框/外链/单实例/备份 | 按 AGENTS.md §8 走查清单（去终端项）全量过一遍 |
| P9 打包分发与切换 | 10 | bundler + updater + 清理 | 签名公证的 .app、CI 矩阵、删除 Electron 主进程与终端残留 | 实况：产出未签名 .app/dmg，签名/公证/updater 余项待凭据（票 19）；dist:dir 等价产物可启动；文档三处同步 |

## 5. 测试与验收策略

- **单测平移**：`tests/` 下 vitest 用例按包翻译为 cargo test（policy / editor / store / model / tools / title / context），翻译过程本身就是行为对照。（实况注记：前端 vitest 已随 Electron 形态移除，等效保障收敛为 cargo test。）
- **数据兼容测试**（P2 起，作为常驻测试资产）：提交一份由 Electron 版生成的小型 `craftwork.db` 样本（脱敏），Rust 侧断言「打开 → 迁移 → 快照重建」结果与 Electron 版一致。
- **对拍测试**：文档工具（P7）用固定真实样本，断言 Rust 输出与 Electron 版输出的文本层等价。
- **必测清单**（调研得出的高风险项，进验收）：
  1. streamdown 流式渲染 + KaTeX 字体资产随包输出（WebKit 引擎基线）；
  2. umya-spreadsheet 重写复杂 xlsx 的保真度；
  3. shell_exec 超时杀树（含孙进程）、Windows 环境白名单保留 SystemRoot（跨平台时）；
  4. Electron 版老库 → Rust 版迁移与重建；
  5. 高频 token 流经 Channel 的有序性与攒批延迟体感。
- **走查清单**：沿用 AGENTS.md §8 并移除终端项（首次启动 → 添加提供商 → 密钥 → 发消息 → 工具调用 → 审批允许/拒绝 → 数据面板 → 主题切换）。

## 6. 风险与回滚

| # | 风险 | 等级 | 缓解 |
| --- | --- | --- | --- |
| 1 | 主进程重写工作量超预期 | 高 | 分阶段并行推进，每阶段有独立验收；main 分支始终可发布 Electron 版，切换点之前可随时止损 |
| 2 | umya-spreadsheet 保真度不达标 | 中 | P7 spike 先行，备选低层路线（zip + quick-xml 直译现有 xlsx XML 逻辑） |
| 3 | macOS 渲染引擎差异的视觉回归 | 中 | P1 全量视觉走查先行暴露；token 语义色不涉及引擎差异 |
| 4 | 更新无差量下载 | 中 | 接受（当前更新本就留桩）；updater 链路备好即可 |
| 5 | 参考 codex-rs 的边界失守（照抄代码引入许可义务） | 低 | 参考**设计与工程拆分**而非复制代码；如确需复用片段，遵守 Apache-2.0（保留许可与 NOTICE 声明） |
| 6 | 类型双维护漂移 | 低 | 黄金文件对拍测试进 CI |

回滚策略：切换（P9 完成时）前，Electron 版完整保留；切换提交为一次性大提交（删除主进程 + 依赖 + 构建配置 + 终端残留），出问题时 revert 单个提交即可回到 Electron 形态。数据层双向兼容（Rust 版不改旧 schema 语义），回滚不丢数据。

## 7. 工程量粗估（单人、已熟悉 Rust 为前提）

| 阶段 | 相对量 | 粗估 |
| --- | --- | --- |
| P0 决策与脚手架 | 小 | 1~2 天 |
| P1 终端移除 + 应用壳与渲染层 | 小 | 2~3 天 |
| P2 存储层与密钥 | 中 | 4~6 天 |
| P3 模型客户端 | 中 | 2~4 天（自研直译，无选型 spike） |
| P4 策略/工具/审批 | 中大 | 5~8 天 |
| P5 Run 循环与 Agent | 大 | 6~10 天 |
| P6 进程执行 | 小 | 2~3 天 |
| P7 文档工具 | 中大 | 5~8 天 |
| P8 系统能力接线 | 小中 | 3~4 天 |
| P9 打包分发与切换 | 中 | 3~5 天 |
| **合计** | — | **约 5~9 周（33~53 人日）** |

粗估依据：主进程约 6 个领域包全部重写，其中 P4/P5/P7 与现有行为语义（审批、事件契约、文档格式处理）强耦合，是耗时主体；P2/P3 有 rusqlite、reqwest 等成熟底座。移除终端（原估最大的风险与工作块之一）后总量与风险双降。若团队 Rust 经验不足，上浮 50% 以上并建议先做一周 Rust/tokio 预研。

## 8. 未决问题

产品与架构决策已全部收口（D1~D7），仅剩执行期事项：

1. **xlsx 路线**——umya-spreadsheet 3.x 保真度 spike 后定（P7，issue 08）；备选 zip + quick-xml 低层路线。
2. **版本号事实源**——按惯例执行：迁移后应用版本以 `tauri.conf.json` 为准，`src-tauri` 侧常量与之同步，`package.json` 仅管渲染层依赖；切换时更新 AGENTS.md 对应章节。
3. **真实密钥验收**——迁移期间以 mock / 隔离目录验收为准（与现状一致）；切换完成后由用户配置真实密钥各跑一次工具调用闭环（AGENTS.md §10 留桩延续）。
4. **执行分支策略**——全部工作在 `tauri-migration` 分支按 issue 顺序提交（每阶段至少一次提交，对应验收点），P9 完成并演练回滚后合回 `main`。
