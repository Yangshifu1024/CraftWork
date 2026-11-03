# 09. 面板：Diff 视图与终端

Status: resolved

## 背景

对齐 DiffPanel 与 TerminalPanel（[spec.md](../spec.md) P2；两者都换 mygo 内置件，终端 PTY 由插件自管）。

## 改动

- **Diff**：`editor.DiffView`（统一/并排、词级高亮），接 ChatService 的 diff 数据源；「应用全部」走既有应用链路（确认 + 备份）；singleton tab。
- **终端**：`plugins/terminal` 替代 TerminalService——按工作区根起默认 shell、标题跟随、真彩、scrollback、选区复制粘贴、Ctrl+L 清屏、退出提示；字号设置应用；singleton tab；工作区切换时重起会话（行为对齐现实现）。
- 删除 `internal/service` 终端事件类型依赖（06 票保留的类型此时清理）。
- 主题：终端配色跟随应用深浅色（对应现 xterm 双套主题语义）。

## 验收

- 编辑器改动 → 差异 tab 展示正确 → 应用全部落备份。
- 终端跑 `vim`/`htop` 级程序无渲染错乱；中文与 emoji 宽度正确；重启后工作区恢复。
- 终端进程随窗口关闭正确回收（无孤儿进程）。

## 依赖

08（tab 体系）。

## Comments

- 2026-11-03 完成。差异 tab（`internal/ui/panel_diff.go`）：全局单份 `DiffSlot`（标题 + 新旧文本 + 打开时冻结的工作区身份），渲染交 `editor.DiffView`（统一/并排 segmented 记忆在 Pane、`DiffInlineWordAlt` 词级高亮、上下文折叠由内置件自算，不经过后端 ComputeDiff）；`Pane.OpenDiff(title, old, new)` 是审批卡「查看完整差异」的正式入口（06 号票直接调用）。「应用全部」：确认框在后台 goroutine 弹（`paneConfirmApplyAll`，可替换变量，07 浮层合入后替换）→ `WorkspaceService.WriteFile`（备份 + 原子替换 + 审计，不带 hash 允许覆盖）→ 编辑器打开磁盘最新状态。
- 终端 tab（`internal/ui/panel_terminal.go`）：tab 首帧惰性 `terminal.New`；标题跟随（OnTitle 经 win.Update 回主线程，代次号丢弃迟到回调）；深浅两套 `terminal.Theme`（对齐 React terminalTheme，构造时同时给 Theme/DarkTheme，插件按窗口外观每帧自选）；设置字号 `SetFont`（设置读取 1s TTL）；退出提示 foot 行「进程已退出 · 退出码 N」+ 重起按钮（对齐 React 把退出行写进缓冲区 + badge 翻转的语义，插件本身会写 "[Process exited]"）；启动失败空态带重试；工作区切换重起会话（`termWsID` 比对）；窗口关闭 `OnClosed → Shell.Shutdown → Pane.Shutdown → term.Close`（SIGHUP，插件 2 秒后补杀）。
- **P0 尖刺终端网格尺寸问题的结论（01 号票遗留）**：读 `plugins/terminal` 源码确认协商机制是「每次 paint 都 `layout`（按元素矩形与设备像素 cell 尺寸算 cols/rows）→ `viewResize` → 未启动则按该尺寸起程序（撤销 500ms 的 80×24 兜底）/ 已启动则 `pty.Resize`」，即协商不依赖 New 后 0.5s 的窗口期。在锁定 v0.7.4 的独立探针上以三种形态复测（eager New、尖刺几何 1080×720@(60,60)+AlwaysOnTop+TitleBarHiddenInset、懒 New 经 win.Update；主屏 Retina 与副屏各测）：PTY 全部正确协商（1080×720 得 125×45；CraftWork 面板 320 宽得 50×40，`stty size` 落盘为证）。**「卡半尺寸」在 v0.7.4 无法复现**，无需显式 Resize / 常驻布局补丁；01 票观察到的现象推测为早期帧（80×24 兜底先起、协商后重排）或首次运行库下载期的暂态被误读为终态。已选方案即「tab 首帧惰性创建」+ 视图 `Fill()`，协商全程交给插件。GUI 证据：vim（行号/语法高亮/状态条/光标，`stty size`=50 40）、macOS `top` 全屏刷新（本机未装 htop，按仓库约定不擅自安装）、`echo 你好👍` 与 ANSI 色（CJK 双宽与 emoji 占位正确）、浅色主题终端截图。
- **清理与合并**：`internal/service/terminal.go`（creack/pty 会话流）删除，`go mod tidy` 移除 creack/pty。**终端事件类型（terminal.output/exit、TerminalDelta）恢复保留**：与 feature/native-ui 合并后发现 05 号票 chat reducer 的事件开关与单测按契约引用它们（忠实移植 React 版 threadReducer），「无引用随编译清理」前提不成立；TerminalService 已删，终端不再产生这两类事件，类型仅为契约保留（09c5242）。
- 冒烟基建：本机 osascript 无辅助功能授权（点击/按键送不进窗口），新增 `-tags cwsmoke` 的 `internal/ui/smoke_cwsmoke.go`（正常构建不参与编译；`smokeSeed` 在 shell.go 为 nil 变量）：环境变量预置 tab/差异槽/固定窗口位置/延时应用/延时关窗。剪贴板链路以 OSC 52 验证（程序写 → pbpaste 读回含 CJK/emoji 的内容）；鼠标拖拽选区无法合成，未走查（与 OSC 52 共用同一 `WriteClipboard` 出口）。工作区切换重起无 UI 入口未 GUI 走查（逻辑单测覆盖 terminalTarget/状态复位）。
