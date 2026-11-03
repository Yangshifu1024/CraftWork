# ADR-0014：桌面外壳切换 mygo 原生 UI，彻底弃用 WebView

- 状态：Accepted（取代 ADR-0001 的「Wails3 + WebView + React」部分与 ADR-0009 全部；调整 ADR-0004/0007 的机制层，原则保留）
- 日期：2026-11

## 背景

Wails3 v3.1.0 升级完成后，对迁移路线做了两轮调研：

1. **换壳保 React**（Wails3 → mygo webview）：Go 侧耦合仅 3 个文件，唯一缺口是 server 模式无等价物。
2. **切换 mygo 原生 UI**（`package ui`，彻底弃用 WebView/React）：调研发现 CraftWork 前端最重的四个部件——流式 Markdown、代码编辑器、终端、diff——恰好是 mygo `ui` 打磨最深的内置件（`markdown.Stream`、`ui/editor`、`plugins/terminal`、`editor.DiffView`），且该库的同类 workload 就是 AI agent 桌面应用。

用户拍板：**直接走原生 UI，不再使用任何 WebView 相关技术**，并接受随之而来的代价（放弃 server 模式/浏览器验证、放弃 Playwright e2e、编辑器/终端按内置件裁剪验收）。

## 决策

- **桌面外壳与界面全部使用 mygo v0.7.4（精确锁定）**：原生 UI（`package ui`）GPU 自绘，窗口仍用系统 WKWebView 之外的路径——应用内没有任何 WebView。CLI 经 `go tool mygo` 按模块锁定版本。
- **Go 内核不变**：Agent 循环、工具、策略、审批、SQLite、密钥、模型层（约 80% 后端代码）原样保留。
- **IPC 层消失**：native UI 的 view 直接调用 Go 函数；后台事件经订阅回调 + `win.Update` 投递主线程；`internal/record` 的 33ms 增量合并（IPC 优化）移除。
- **前端整体重写为 Go**：`frontend/`（约 2.6 万行）最终删除；`threadReducer`/`sidePane` 两个纯函数库近乎逐行移植；base.css token 体系机械翻译为 Go 主题常量；lucide SVG 直接由 `ui.Icon` 渲染。
- **编辑器/终端/diff/Markdown 采用内置件**，不追求 CodeMirror/xterm 的全部功能，按迁移 spec 的裁剪清单验收。
- **测试体系切换**：Playwright + 截图基线作废，改用 `ui.Tester` headless 测试；浏览器验证改走 `mygo dev`（重编译重启）+ 内置 Inspector。

## 被否决的备选

- **继续 Wails3**：WebView 内存与包体基线、三套运行时（Go/JS/CSS）的维护成本、绑定代码生成链路；且 wails 路线无法获得内置更新器/终端/编辑器。
- **换壳保 React 前端**：保留 2.6 万行 TS 与 CSS，但 WebView 相关约束（CSP、origin、事件批量通道）全部保留，长期仍背两套栈。
- **Fyne / Gio 自绘**：组件生态远弱于 mygo ui（无流式 Markdown、无 Zed 级编辑器、无 libghostty 终端），自绘成本回到数万行。

## 后果

- **server 模式（`-tags server`）、Docker、浏览器验证彻底移除**：WASM 目标只能带 UI 不能带后端（exec/PTY/SQLite 不可用），不做保留。
- 终端插件要求 **macOS 13+**（纯 UI 需 12+），比 Wails 路线略收紧。
- mygo 处于 v0.7.x、单人维护、有 breaking 先例：以精确锁版本 + 不写包装层应对；升级时读其 `docs/ui/migration.md`。
- 失去 CSS 的表达力，视觉全部代码化：主题常量 + setter，深浅色实时切换由 `ui.Theme` 提供。
- **收益**：单进程无 WebView（内存/冷启动基线下降）、无绑定生成与类型漂移问题、内置 Ed25519 自动更新与打包签名公证（M4 留桩可直接接线）、libghostty 终端与 Zed 级编辑器开箱即用。
