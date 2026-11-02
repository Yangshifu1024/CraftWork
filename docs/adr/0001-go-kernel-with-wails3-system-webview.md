# ADR-0001：Go 内核 + Wails3 系统WebView + React 界面

- 状态：Accepted
- 日期：2026-10（v3.0 文档定稿；「不用任何 WebView」的旧约束已撤销）

## 背景

CraftWork 是本地优先的桌面 Agent，需要：真实文件系统与 Shell 能力、长任务与流式输出的并发模型、跨 macOS 与 Windows 的窗口外壳、可复用原型 CSS 的界面实现。候选路线包括 Electron、Tauri、Go 自绘（Fyne / Gio）与 Wails3。

## 决策

- **Go 1.27 承担内核与全部系统能力**：Agent 循环、工具、策略、存储、PTY、文件系统、密钥都在 Go 进程内。
- **桌面外壳使用 Wails3 v3.0.0（精确锁定）**：由 Go 创建窗口、注册服务、嵌入前端资源，界面运行在系统 WebView（macOS WKWebView、Windows WebView2）。
- **前端使用 React 19 + TypeScript 5 + Vite 8**，对话界面用 assistant-ui headless primitives，编辑器 CodeMirror 6，终端 xterm.js 6，状态管理 Zustand 5，图标 lucide-react。
- **不打包 Chromium**：不使用 Electron、Tauri 或任何自带浏览器内核的运行时装包。
- **样式使用原型 CSS 分区迁移**（保留 class 名与 CSS token），不使用 Tailwind。

## 被否决的备选

- **Electron**：自带 Chromium，包体与内存不可接受，Go 只能作为 sidecar。
- **Tauri**：Rust 主控与前端能力边界同 Go 内核不匹配，工具链引入 Rust。
- **Fyne / Gio**：自绘组件工作量大，无法直接复用原型 CSS；中文输入与复杂排版风险高。
- **Tailwind + shadcn**：与原型 13px 密度、手写 token 体系冲突，会带来大量覆盖样式。

## 后果

- 单二进制分发，WebView 不占包体（macOS < 120MB 目标）。
- 应用自身界面运行在 WebView 中，但安全边界是「不加载任意第三方网页、不执行不可信 HTML、密钥不进前端」（见 ADR-0009），而不是「不用 WebView」。
- Wails3 v3.0.0 较新，平台差异与 API 变动风险通过精确锁版本、不依赖未公开 API 缓解。
- WebView 常驻内存使空闲内存基线高于自绘方案（目标 < 400MB）。
- 同一份代码可编译为 server 模式（`-tags server`）：无窗口，浏览器里运行完整应用。
