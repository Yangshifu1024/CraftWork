# 面板多标签页（panel tabs）规格

状态：已评审定稿（调研与方案背景见同目录 [proposal.md](./proposal.md)）。本文是行为规格，按实现阶段标注归属；带「阶段二」「阶段三」标记的条目在对应里程碑交付。

## 范围

- 右侧面板从「模式内互斥按钮」升级为**多 tab 模型**，完整搬 ZCode Side Pane 的状态模型：结构化幂等 id、workspaceKey 分桶、线程归属过滤、唯一写出口、forceMount 保活。
- **纯前端特性**：不新增 Go 绑定方法、不改事件协议；终端 PTY 生命周期仍由 TerminalService 与终端面板内部管理。
- 路线图项（子代理会话、侧边聊天、后台任务输出、浏览器、插件面板）只预留类型体系的扩展规则，不在本规格范围（见 proposal.md 4.6）。

## 术语

- **tab**：面板里的一次内容呈现单元，含类型、结构化 id、冻结归属。
- **分桶（bucketing）**：按工作区把 tab 状态分成互不串台的内存桶，切换工作区整桶写回/恢复。
- **公共 tab**：跨线程可见的 tab 类型（当前集合 = `terminal`）。
- **owner（归属）**：tab 冻结的线程 id；草稿线程归一为 `"draft"`。
- **singleton tab**：id 为固定字面量、重复打开只聚焦不新建的 tab。
- **可见视图**：当前 `<工作区, 线程>` 作用域过滤后的 tabs 子集；隐藏的 tab 不销毁。

## 1. 数据模型（阶段一）

```ts
type SidePaneTabType = "file" | "diff" | "terminal" | "document";

interface SidePaneTabBase {
  id: string;               // 结构化 id，幂等键；singleton 为固定字面量
  type: SidePaneTabType;
  title: string;
  workspaceId: string;      // 打开时冻结的所属工作区（分桶键）
  threadId?: string | null; // 打开时冻结的归属线程；null = 未认领
  openedAt: number;
}
// file:     { id: `file:<workspaceId>:<folderID>:<path>`, folderID, path, language? }
// diff:     singleton，id 固定 "diff"
// terminal: singleton，id 固定 "terminal"
// document: { id: `document:<workspaceId>:<folderID>:<path>`, folderID, path, kind }

interface SidePaneState {
  tabs: SidePaneTab[];         // 一份共享数组；各线程看过滤视图
  activeTabId: string | null;  // 可指向当前线程不可见的 tab（保留他人激活项）
  recentClosed: SidePaneTab[]; // 上限 8，按当前作用域过滤展示
}
```

- 新增派生类型（子代理等）时各自声明过滤维度字段（如 `rootThreadId` / `parentThreadId`），可见性函数按类型分派；不得把多种派生维度塞进同一个 `threadId` 字段（ZCode 已踩过混用互伤的坑）。
- 实现分层对齐：`frontend/src/lib/sidePane.ts` 纯函数转移函数（可单测）→ `stores/panel.ts` 薄封装（分桶记忆 + 唯一写出口）→ 组件层只读状态，**从不直接改 tabs**。

## 2. 打开与幂等（阶段一）

- `openFileTab` / `openDocumentTab`：按 id 查已有 → 激活并刷新内容；不存在 → 追加并激活。
- `openDiffTab` / `openTerminalTab`：singleton，已有 → 仅聚焦；diff 聚焦同时刷新差异内容（沿用现有 `openDiff` 语义）。
- 打开动作总是：盖戳归属（`workspaceId + threadId` 取当前值；**已带归属的 tab 绝不覆盖**）→ 确保面板展开（现有 `setTab` 语义）→ 激活。
- 所有打开/关闭入口统一经 store 的写出口提交；组件层（面板、聊天、文件树）只调用入口函数。

验收：同文件重复打开只有一个 tab；已打开的 diff 重复点「查看完整差异」只聚焦并更新内容；无副作用重复调用状态不变（幂等）。

## 3. 归属与可见性（阶段一）

- 可见性 = `tab.workspaceId === 当前工作区` **且**（tab 是公共类型 **或** `tab.threadId === 当前线程`，未认领 tab 视为草稿归属）。
- 切线程：不属于新线程的 tab **隐藏不销毁**；切回恢复，激活项按 `<workspaceId>::<threadId>` 记忆。
- 切工作区（`app.activeWorkspace` 变化）：旧桶写回内存 → 恢复新桶 → 按新作用域过滤；两桶互不串台。
- 跨工作区后台事件（其他线程的任务产出打开类动作）：写回**原工作区的桶**，不污染当前视图。
- 草稿线程（未发送首条消息）owner 为 `"draft"`；线程删除 → 联动清理其名下 tab，**不入**最近关闭栈。
- 「关闭动作导致当前线程可见 tab 清空」→ 面板自动收起（含退出最大化）。切换线程 / 工作区**不因无可见 tab 改变开合**（有文件树兜底视图；无显式偏好时保持用户当前开合，避免覆盖持久化偏好与首屏文件树）——2026-11-03 验收修订：原「切作用域无可见 tab 即收起」会在初始启动时收起面板并覆盖 localStorage 恢复的开合，已改。

验收：线程 A 开文件 → 切线程 B 面板收起 → 切回 A 文件 tab 与激活项恢复；项目工作区与临时会话工作区来回切互不串台；删除线程后其 tab 消失且不出现在最近关闭。

## 4. tab 操作（阶段一）

- **关闭当前 tab**：关闭后激活原位置的邻居 tab；数组空 → 面板收起。
- **关闭其他**：保留并激活被右键的 tab。
- **关闭全部**：清空当前作用域可见 tab，面板收起。
- **最近关闭**：栈上限 8；`reopen` 前校验当前作用域可见（防跨工作区/跨线程泄漏）；恢复时按打开语义重建（file 重新读文件）。singleton 类型的关闭也入栈。
- **终端特殊语义**：关闭终端 tab 仅隐藏视图，**不终止 PTY 会话**（会话管理留在终端面板内部，结束会话失败时 toast）；这与 ZCode「关 tab 即杀」不同，是贴合 CraftWork 现状的裁决。

验收：中键/按钮/菜单三种关闭路径行为一致；关闭末 tab 面板收起；最近关闭在跨线程切换后被过滤；重开后的 tab 归属为当前线程。

## 5. tab 条交互（阶段一）

- tab 条位于面板顶部（替换现有模式按钮组）；每 tab：图标 + 标题 + 关闭按钮。
- 关闭按钮仅选中 / hover / focus-within 显示；隐藏时不响应点击。
- **中键关闭**：点击任意非交互区域直接关闭，不先激活；阻止浏览器中键自动滚动。
- **右键菜单**：关闭、关闭其他、关闭全部、重新打开关闭的标签页（子菜单列最近关闭）。
- **宽度自适应**：tab 默认 156px；空间不足全体等比收缩，下限 60px；全部到底仍装不下才横向滚动；标题超长淡出遮罩，不用省略号。
- Mode（code/work）保留为视觉与默认值上下文：激活色沿用 `--mode-accent`，打开文件在 Code 模式落编辑器 tab、Work 模式落文档 tab（现行为）。

验收：156→60 收缩与横向滚动临界正确；中键不触发激活与自动滚动；触屏（无中键）先选中再关闭可用。

## 6. 保活（阶段二）

- 所有 tab 内容 **forceMount**：切换 tab / 面板收起只 `display:none`，不卸载组件。
- 切换显示时主动触发 relayout：CodeMirror `requestMeasure`、xterm `fit()`；两个内核（WKWebView / Chromium）行为一致。
- 收益验收：切走再切回，编辑器光标/撤销历史/搜索状态保留；终端输出连续无重放闪烁；文档预览滚动位置保留。
- **file tab 上限 8 个**：超出自动关闭最久未激活的（走关闭流程，**不入**最近关闭栈）。
- 面板整体收起仍沿用现有 `is-closed` 保 DOM 机制，二者叠加后任何 tab 任何时刻都不重挂载。

## 7. 持久化（阶段二）

- localStorage：`cw-panel-width`（宽度）、`cw-panel-open`（开合）。启动时读取，损坏值回退默认。
- **tab 列表不持久化**（ZCode 同款哲学）：重启后各工作区桶为空，面板打开显示默认视图；避免恢复失效路径（文件移动、线程删除）。
- 双宿主（桌面 / server 浏览器）行为一致，localStorage 均可用。

验收：刷新后宽度与开合恢复；tab 列表清空属预期行为。

## 8. 布局增强（阶段三）

- **对话列宽度自动收起**：仅监听 `window.resize`，防抖 300ms（等 resize 结束）后实测对话列宽度，< 480px 时收起面板；与现有 ≤1179px 浮层逻辑并存（窄窗口仍走浮层）；不监听对话列 ResizeObserver，避免手动开面板被策略关回去。
- **双击分隔条**重置宽度为默认 320px；补拖拽中 `.panel-resize.is-dragging` 视觉态（样式已备）。
- **键盘可达**：分隔条 `role="separator"` + `aria-*`，左右方向键调宽。

## 9. 快捷键（阶段三）

| 键 | 行为 | 注册范围 |
| --- | --- | --- |
| `Cmd/Ctrl+B` | 开合右侧面板 | 桌面 + server |
| `Cmd/Ctrl+W` | 关闭当前 tab；无可见 tab 时不拦截 | **仅桌面**（server 模式下是浏览器关标签，绝不拦截） |
| `Esc` | 退出链最前插入一级：激活 tab 可关时先关 tab | 沿用现有 Esc 链顺序（面板 tab → 浮层 → 最大化 → 运行中会话 → 待审批） |

- 命令面板同步暴露「开合面板」「关闭当前标签页」命令。
- 快捷键实现遵循 CraftWork 全局快捷键注册惯例；改键不在本特性范围。

## 10. 非目标

见 proposal.md 4.6（路线图：子代理会话、侧边聊天、后台任务输出、浏览器 PoC、插件面板、完整视图、diff 多文件、标签总览搜索）与 4.7（不搬清单：react-resizable-panels、new-tab 启动页、dnd-kit、workflow 系列类型）。

## 文档同步（实现时）

- 布局/交互/视觉定稿 → `docs/specs/interface.md`；功能与验收要点 → `docs/specs/features.md`；新术语 → 根 `GLOSSARY.md`。
