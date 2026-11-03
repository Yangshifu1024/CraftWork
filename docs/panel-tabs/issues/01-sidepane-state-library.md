# 01. `lib/sidePane` 纯函数状态库

Status: ready-for-agent

## 背景

面板多标签页的地基（[spec.md](../spec.md) 第 1、2、3、4 节）。对齐 ZCode `packages/ui/src/lib/workspaceSidePane.ts` 的分层：类型 + 纯函数转移函数 + 可见性函数，全部可单测、不依赖 React。store 与组件层后续只做薄封装。

## 改动

- 新建 `frontend/src/lib/sidePane.ts`：
  - 类型：`SidePaneTabBase` 与四个类型（`file` / `diff` / `terminal` / `document`）、`SidePaneState`、`SidePaneVisibilityScope`。结构化 id 与盖戳字段见 spec 第 1 节；为派生类型预留扩展规则的注释（`rootThreadId` / `parentThreadId` 各自声明，不混入 `threadId`）。
  - 转移函数（全部纯函数、no-op 返回原引用）：
    - `openTab(state, tab)`：幂等合并——按 id 查已有即激活并合并可刷新字段；不存在则追加并激活。
    - `activateTab(state, tabId)`、`closeTab(state, tabId)`（关 active 时激活同位置邻居）、`closeOtherTabs`（保留并激活目标）、`closeAllTabs`（返回可见集清空的状态）。
    - `pushRecentClosed`（上限 8）、`reopenRecentClosed`（校验当前作用域可见）。
    - `markTabsOwnership`（打开时对未认领 tab 盖戳 `workspaceId + threadId`；已带归属绝不覆盖）。
  - 可见性：`isTabVisibleInScope(tab, { workspaceId, threadId })`——工作区匹配且（公共类型 `terminal` 或 `threadId` 匹配，未认领视为 `"draft"`）。
  - 常量：`RECENT_CLOSED_LIMIT = 8`、`FILE_TAB_LIMIT = 8`（阶段二保活用，先定义）、公共类型集合。
- 单测（Vitest，对齐既有 reducer 测试风格）：幂等打开、盖戳不可覆盖、可见性矩阵（工作区 × 线程 × 公共类型 × 草稿）、关闭邻居选择、关空收敛、最近关闭上限与作用域校验、singleton 语义。

## 验收

- `cd frontend && pnpm exec vitest run src/lib/sidePane.test.ts` 全绿。
- 不改动任何现有文件（本票只新增），`pnpm run typecheck` 通过。

## 依赖

无。后续所有票依赖本票。

## Comments

## Comments

- 已实现于 `c3615ea`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
