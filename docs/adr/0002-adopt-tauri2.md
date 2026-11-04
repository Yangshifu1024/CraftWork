# 采用 Tauri 2 重构桌面外壳

桌面外壳从 Electron 45 迁往 Tauri 2：主进程领域逻辑以 cargo workspace 形态用 Rust 重写（`crates/craft-*` 与既有领域包一一对应），渲染层（React 19 + Tailwind v4）原样保留。调研依据与完整计划见 `docs/tauri-migration/plan.md`（含 2026-11 生态调研：Tauri 2.12.5 稳定线、官方插件覆盖、Rust Agent 框架无一与本项目「审批一等公民 + 事件流驱动 + 可审计」对位、头部编码 Agent 均自研循环）。随决策一并定案：移除终端功能（连带消除 node-pty 原生依赖与 WKWebView 中文 IME 风险）；Agent 侧完全自研（不引入 rig / genai / goose，基础设施 crate 仍用成熟第三方库，循环与协议设计参考 openai/codex）；crate 命名采用 `craft-` 前缀（实测无前缀的 `core` 会遮蔽内置 crate 且 `::core::` 绝对路径连带 derive 宏一起失效）。

## Considered Options

- **保持 Electron**：无迁移成本，但继续背负 Chromium 打包体积、内存与双运行时维护；Node 生态共享是唯一不可替代收益，本项目主进程不依赖 npm 领域生态，实际损失有限。
- **采用 Tauri 2（选定）**：体积与内存显著下降；capabilities 权限模型与本项目「最小权限、显式审批」理念契合；密钥与数据库天然隔离在 Rust 进程。代价：主进程全量重写（33~53 人日）、macOS 渲染引擎 Chromium → WebKit 需全量视觉回归、更新无差量下载。
- **等待 Tauri 3**：3.0 仅 alpha，无路线图；2.12.x 稳定且活跃，等待无收益。
- **迁移版保留终端**：被否——node-pty 是唯一原生 Node 模块与跨平台打包障碍，WKWebView 下 xterm.js IME 有实锤缺陷；`shell_exec` 保留工具能力，交互式终端面板移除。

## Consequences

- 分层纪律平移：`crates/*` 不依赖 Tauri（可 cargo test），仅 `src-tauri` 接触 Tauri API，等价于既有「只有 index.ts / ipc.ts / services 部分文件接触 Electron」。
- 数据层双向兼容：SQLite schema 与 `~/.craftwork` 目录不变，Electron 版与 Tauri 版互读，回滚不丢数据。
- 全部工作在 `tauri-migration` 分支按 `docs/tauri-migration/issues/01~10` 顺序执行，P9 演练回滚后合回 `main`；决策细节与未决项见 plan.md D1~D7。
