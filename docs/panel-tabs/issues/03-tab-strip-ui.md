# 03. tab 条 UI 与入口切换

Status: ready-for-agent

## 背景

面板顶部的模式按钮组替换为 tab 条，所有打开入口一步切到新模型（[spec.md](../spec.md) 第 2、4、5 节）。本票交付后旧单槽位模型退役。

## 改动

- `frontend/src/features/panel/Panel.tsx`：
  - 模式按钮组 → tab 条：图标 + 标题 + 关闭按钮（选中/hover/focus-within 显示，隐藏时不响应点击）；复用 `Icon` 组件与新登记图标名；class 与 token 遵循 `styles/panel.css` 既有分区，新增样式放对应分区。
  - 宽度自适应：默认 156px、下限 60px 等比收缩，全到底仍装不下才横向滚动；标题超长淡出遮罩（不用省略号）；关闭按钮绝对定位不占标题空间。
  - 中键关闭：`click` 拦 `button === 1` 防先激活，`auxclick` `preventDefault` 防自动滚动；触屏走先选中再关闭。
  - 右键菜单：关闭、关闭其他、关闭全部、重新打开关闭的标签页（子菜单列当前作用域的最近关闭）；复用 `components/Popover` 或既有菜单组件。
  - 无可见 tab 时的空态：保留现有 `.panel-empty` 提示（本特性不做 new-tab 启动页）。
- 入口切换（`stores/workspace.ts` 与调用方）：
  - `openFileAt` / 文件树点击 → `openFileTab`；`openDiff` → `openDiffTab`（singleton 聚焦 + 刷新）；`openDocument` → `openDocumentTab`；聊天头部按钮 / 命令面板 toggle → `toggle()`（不隐式建 tab）。
  - Code/Work 模式与 tab 类型的映射保留现行为（Code→file、Work→document）。
- 终端语义：关闭终端 tab 仅隐藏视图，不终止 PTY 会话（spec 第 4 节裁决）；终端内部 term-tabs 会话管理不动。
- 本票先做**条件渲染**切换 tab（沿用现状），保活由 05 票交付。
- e2e：tab 条交互补 `data-cw-*` 用例（打开多个 → 切换 → 关闭路径 → 最近关闭恢复）。

## 验收

- 文件树连开 3 个文件出现 3 个 tab；重复打开同文件只激活；diff/terminal singleton 只聚焦。
- 三种关闭路径行为一致；关末 tab 面板收起。
- `pnpm run lint`、`pnpm exec tsc --noEmit`、`pnpm run test`、`pnpm run test:e2e` 通过。
- 文档同步：`docs/specs/interface.md` 更新面板 tab 条视觉与交互；`docs/specs/features.md` 更新功能与验收要点。

## 依赖

01、02。

## Comments

## Comments

- 已实现于 `0132a7d`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
- 实现说明：标题超长的「淡出遮罩」用常驻右缘 6px 渐隐 mask 实现（CSS-only，不用省略号）；「打开终端 / 差异」的 singleton 快捷入口放在 tab 条右侧 head-actions（无对应 tab 时显示），替代旧模式按钮组的常驻入口。
