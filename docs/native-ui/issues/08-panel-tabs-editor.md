# 08. 面板：多 tab 体系、文件树与编辑器

Status: ready-for-agent

## 背景

对齐 `features/panel/` + `lib/sidePane.ts`（[spec.md](../spec.md) P2；编辑器按裁剪清单用 `ui/editor`）。

## 改动

- `internal/ui/panel/tabs.go`：移植 `sidePane.ts`（结构化幂等 id、singleton、工作区分桶、归属过滤、最近关闭栈 8、file tab 上限 8、中键关闭、右键菜单、末 tab 收起、宽 156→60 收缩），纯函数 + 单测对齐既有 318 行用例。
- tab 条 UI：5 类 tab、激活态、关闭按钮、`Transition` 收缩动画。
- 文件树：懒加载、过滤平铺（≤200）、多源分段 + 「主要」徽标、扩展名着色图标、Git 状态字母、点击开 tab。
- 编辑器：`ui/editor` 接入——打开/保存（备份 + `expectedHash` 冲突确认）、只读切换、按 tab 隔离草稿、`path:line` 跳转居中、面包屑与状态条（裁剪清单见 spec）。

## 验收

- tabs 状态机单测全绿（对齐 sidePane.test.ts 用例）。
- 文件树打开 → 编辑 → 保存 → 备份出现在数据目录；外部修改后保存触发冲突确认。
- 审批卡「查看完整差异」/聊天 `path:line` 能打开正确 tab 并定位行。

## 依赖

03；保存链路依赖 02。
