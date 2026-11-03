# 01. P0 尖刺：工具链与四件套验证

Status: ready-for-agent

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
