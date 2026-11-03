# 09. 面板：Diff 视图与终端

Status: ready-for-agent

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
