# 04. 侧栏：会话分组列表与模式切换

Status: ready-for-agent

## 背景

对齐 `features/sidebar/Sidebar.tsx`（746 行）+ `stores/threads.ts` 的列表语义（[spec.md](../spec.md) P1）。

## 改动

- `internal/ui/sidebar/`：Code/Work 模式切换（轨道+滑块，`Animate`）、会话分组（临时会话组置顶可折叠、项目分组、孤儿「其他」组）、组内分页（5 条 + 显示更多）、条目两行（标题；时间+状态点：运行脉冲/等待/错误/完成）、选中态模式色竖条。
- 右键菜单：复制会话 Id / 重命名 / 删除（危险确认走全局确认对话框）。
- 底部两行：今日用量、审计入口、命令面板 ⌘K 与设置 ⌘, 入口。
- 状态机移植 `stores/threads.ts` 列表部分（分组/分页/折叠持久化），纯函数 + 单测。

## 验收

- 与 Wails 版行为对照：分组、分页、折叠记忆、右键菜单、模式切换联动占位文案。
- 状态机单测全绿。

## 依赖

03。

## Comments

- 2026-11-03 实现者留档（对兄弟票关键的坑）：mygo 剪贴板/原生对话框在 view 帧内直调会死锁（`onMain` 等主循环而主循环正在跑帧），必须经 `win.Update` 或 goroutine 排到帧后；绝对定位子元素会盖住普通兄弟，垫底用 `track.Draw`；`ui.Tester` 连点需 `Advance` 动效时长。
