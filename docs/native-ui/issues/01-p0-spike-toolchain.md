# 01. P0 尖刺：工具链与四件套验证

Status: resolved

## 背景

迁移第一步（[spec.md](../spec.md) P0）。在动 CraftWork 之前，用独立 spike 工程验证 mygo 原生 UI 在本机的四个关键假设，任何一条不成立都需要回到 ADR 重新评估。

## 改动

- 安装 mygo CLI 并锁定版本：`go install github.com/egoist/mygo/cmd/mygo@v0.7.4`，`mygo doctor` 检查环境。
- 在仓库外（如 `/tmp/cw-native-spike`）建 native 模板工程（`go.mod` require mygo v0.7.4，CLI 走 `go tool mygo`），单窗口用 `Segmented` 切四个视图：
  1. **窗口壳**：`TitleBarHidden` + `c.TitleBar()` 红绿灯占位 + `.DragWindow()` 拖拽区 + `StateKey` 记忆 + 最小尺寸。
  2. **分栏 + 中文流式 markdown**：`ui.Split` 左右分栏；右侧 `markdown.Stream` 持续追加带代码块/表格/行内代码的中文长文，验证 CJK 换行、深浅色、reveal 淡入。
  3. **编辑器**：`ui/editor` 打开真实 Go 文件，验证高亮/行号/查找/中文注释。
  4. **终端**：`plugins/terminal` 起 zsh，验证真彩/scrollback/选区复制/退出行为（注意首次运行 libghostty 原生库的获取方式）。
- 截图留档（`screencapture`），结论写回本票 Comments。

## 验收

- 四视图均能编译运行、无崩溃；中文渲染与深浅色无明显缺陷。
- `go tool mygo build` 能产出可启动的 .app。
- spike 不进 CraftWork 仓库。

## 依赖

无。所有后续票依赖本票。

## Comments

- 2026-11-03 完成（spike 工程在 `/tmp/cw-native-spike`，不进仓库）。`mygo` CLI v0.7.4 安装成功，`mygo doctor` 通过（Go 1.27.3、macOS 27.2 ✓；bun/makensis/codesign 可选项缺失不影响）。
- **窗口壳 ✓**：`TitleBarHiddenInset` + `c.TitleBar()` 避让 + `.DragWindow()` + `Segmented` 换页全部正常；`StateKey` 与显式 `X/Y` 并存时显式坐标被记忆值覆盖（转正工程二选一即可）。
- **分栏 + 中文流式 Markdown ✓**：`ui.Split`（初始宽度必须非零）+ `markdown.Stream` 词级 reveal、GFM 表格、代码块高亮 + 语言标签 + Copy 按钮、行内代码、链接、中文注释高亮全部正确；流式 goroutine 经 `win.Update` 回主线程符合预期。
- **编辑器 ✓**：`ui/editor` 打开真实 Go 文件，高亮/行号/光标状态条/中文注释正常。
- **终端 ⚠️ 已知风险**：`plugins/terminal` 管线全通（libghostty 自动下载到 `~/Library/Caches/mygo/natives/`，zsh + powerlevel10k 渲染、光标活动），但 **PTY 网格尺寸未跟随视图**（提示符挤在约半尺寸区域；官方裸渲染复现，排除包装层因素）。文档约定「New 后 0.5s 内由视图尺寸协商，否则 80×24」，本机（Retina 双屏）协商失效。结论：不阻断迁移；09 票实施时优先验证升级版 mygo、布局稳定后显式 `Terminal.Resize`、或终端 tab 常驻布局三个方案。
- 截图档：`/tmp/spike-page{0,1,2}.png`（深色主题；浅色验证留待 P1 主题翻译票）。
