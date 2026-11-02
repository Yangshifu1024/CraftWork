# 02. panel store 改造与 workspaceKey 分桶

Status: ready-for-agent

## 背景

把 `stores/panel.ts` 的单槽位 `tabs: Record<Mode, PanelTab>` 换成多 tab 状态，并落地按工作区分桶的记忆层（[spec.md](../spec.md) 第 1、3 节）。对齐 ZCode `taskSidePaneMemory.ts`（模块级 Map + LRU）与 `useAppPanels` 的「唯一写出口」模式，但按 CraftWork 惯例用 Zustand 实现。CraftWork 数据层已是多工作区模型（`threads.workspace_id`、`app.activeWorkspace`），分桶数据基础已就绪。

## 改动

- `frontend/src/stores/panel.ts` 重构：
  - 状态：`tabs` / `activeTabId` / `recentClosed`（对应当前工作区桶的投影）；保留 `open` / `maximized` / `width` / `narrow` 既有语义不变。
  - 分桶记忆层：模块级 `Map<workspaceId, PaneBucket>`，桶含 `{ tabs, activeTabId, recentClosed, collapsedByThread: Record<threadId, boolean> }`；LRU 上限 50 桶，读写 touch 顺序。
  - 唯一写出口：`openFileTab / openDiffTab / openDocumentTab / openTerminalTab / activateTab / closeTab / closeOtherTabs / closeAllTabs / reopenRecentClosed / reorderTabs`（reorder 为阶段三 tab 拖拽预留，先落转移函数）。内部统一：调 `lib/sidePane` 转移函数 → 写回桶 → setState。删除 `setTab(mode, tab)`。
  - 收敛规则：**当前线程可见 tab 清空 → `open: false` + `maximized: false`**；所有写出口复用同一段收敛检查。
  - 激活项记忆：`activeTabByThread: Map<"workspaceId::threadId", tabId>`，切线程恢复。
- 切换接线（本票或与 `stores/app.ts` / `stores/threads.ts` 协作）：
  - `app.activeWorkspace` 变化 → 旧桶写回、新桶恢复、过滤渲染。
  - 活动线程变化 → 按 `<workspaceId>::<threadId>` 恢复激活项与开合偏好。
  - 线程删除 → 清理其名下 tab（不入最近关闭栈）。
  - 跨工作区打开类调用（后台事件场景）：写回原工作区的桶，不污染当前视图。
- 单测：切桶写回/恢复、LRU 淘汰、切线程激活项恢复、线程删除清理、可见 tab 清空收起、关末收敛与最大化联动。

## 验收

- store 单测全绿；`pnpm run typecheck` 通过。
- 手动冒烟（Mock 或 server 模式）：项目工作区开文件 → 切临时会话工作区 → 面板收起且无串台 → 切回恢复。

## 依赖

01（状态库）。

## Comments

## Comments

- 已实现于 `204bcf6`（分支 feature/panel-tabs）。验证：前端 lint / tsc / vitest / build 全绿。
- 实现边界说明：入口迁移（workspace 的 openFile/openDiff/openDocument 改调新打开方法）随本提交一并落地（03 票的 tab 条骨架也在此提交），两票合并交付后旧单槽位模型退役。
