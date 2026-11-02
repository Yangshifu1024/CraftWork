# 右侧面板多标签页（panel tabs）方案提案

状态：提案（待评审）。本文档整理 ZCode 右侧栏（Side Pane）的实现与功能调研结论，并给出适合 CraftWork 的落地方案。方案确认后再拆分 `spec.md` 与 `issues/`。

调研对象：ZCode 仓库 `/Users/yangzhenbiao/Code/ZCode`（版本 3.17.1，前端在 `packages/ui/src/`）。
CraftWork 现状基线：`frontend/src/features/panel/`、`frontend/src/stores/panel.ts`、`frontend/src/app/AppShell.tsx`（2026-11 当前 main）。

---

## 一、ZCode Side Pane 功能全景

ZCode 的右侧栏叫 **Side Pane**，是对话区右边的多标签页工作面板。核心形态：

- **多 tab 模型**：面板顶部是 tab 条，24 种 tab 类型并存（浏览器、终端、Git、代码查看器、文件管理器、后台任务输出、子代理会话、侧边聊天、计划详情、工作流系列、插件面板、插件市场、开发者工具、白板、新标签页等）。
- **归属与显隐**：tab 按打开时所在的「工作区 + 会话」归属。切换会话时，不属于当前会话的 tab 隐藏但**不销毁**；切回时恢复。工作区公共 tab（Git、开发者工具等）所有会话共享。
- **tab 操作**：关闭当前/其他/全部、最近关闭栈（8 条，可恢复）、标签总览（按标题/路径/类型搜索已开与最近关闭的标签）、拖拽排序、中键关闭、宽度自适应收缩（156px → 60px，装不下才横向滚动）。
- **资源保活**：终端切走后进程不杀（模块级 xterm 注册表，DOM 节点在 stash 容器与面板间移动，scrollback 不丢）；浏览器页面由主进程管理驻留状态（可见/后台/挂起）；切换 tab 不卸载任何面板实例（`forceMount` + 隐藏）。
- **开合策略**：面板开合按「工作区级 + 每会话」双层记忆；窗口变窄时先收面板（对话列 < 480px）、再收左侧栏（< 360px），等 resize 结束 300ms 后才判定；`Cmd/Ctrl+W` 优先关闭当前面板 tab，无 tab 才关窗口。
- **完整视图（Full View）**：`⇧⌘F` 让面板临时占满对话区，对话变成 tab 条里的第一个 tab，并提供一个可吸附到 7 个位置、可最小化成圆形按钮的浮动输入窗口（收起时只露输入框，聚焦滑出操作栏，展开显示窄消息流，回答结束时弹纯文本气泡）。对话组件全程不重新挂载（DOM 不动，只改定位方式）。
- **形态差异**：完整能力仅桌面端；手机远控下面板变成右侧覆盖抽屉；普通 Web 无完整视图。

## 二、ZCode 实现拆解

### 2.1 分层架构

```
lib/workspaceSidePane.ts   纯函数状态库（24 种 tab 类型 + 全部转移函数，2462 行）
lib/taskSidePaneMemory.ts  内存记忆（按 workspaceKey 分桶的 Map，LRU 上限 50，仅内存不落盘）
hooks/useAppPanels.ts      React 编排层（所有打开/关闭入口的唯一写出口，1715 行）
app-shell/AnimatedSidePanePanel.tsx   渲染分发（tab 条 + 各类型组件分发 + 拖拽排序，1578 行）
```

数据流向：组件层**从不直接改 tabs**，一切经 `useAppPanels` 的 `commitSidePaneState` / `commitOpenedSidePaneState`；先写 canonical ref（保证同批次内交互按序合并），再写内存记忆，再 setState 触发渲染。

### 2.2 tab 模型的四个核心设计

1. **结构化 id = 幂等打开**。singleton 类型的 id 是固定字面量（`"git"`、`"developer-tools"`），重复打开天然只聚焦不新建；多实例类型用 uuid 或复合 id（如 `plan-detail:<工作区>:<会话>:<工具调用id>`、`code-viewer:<sourceKey>`），open 函数先查已有再决定复用还是新建。不存在单独的「是否 singleton」标记。
2. **一份 tabs 数组 + 可见性过滤**。状态是 `{ tabs: Tab[], activeTabId }`，所有会话共享同一数组；`isSidePaneTabVisibleInScope` 按「工作区 + owner」过滤出当前可见子集。owner 维度按 tab 来源分：浏览器用 sessionId、子代理用 rootSessionId、侧聊/计划/插件用 parentSessionId、其余用冻结的 `ownerTaskId`（草稿统一 `__draft__`）。
3. **归属在提交时冻结**。每次打开 tab 时把当时的 owner/工作区盖到未认领的 tab 上，之后不随上下文漂移；跨工作区的后台事件写回原工作区的记忆桶。
4. **关闭末 tab 收敛为收起面板**。所有写入口统一检查：可见 tab 清空 → 面板强制收起。面板开关按钮只切显隐，展开时无可见 tab 才补建一个「新标签页」。

### 2.3 资源保活的三种手段

| 资源 | 手段 | 关键点 |
| --- | --- | --- |
| 终端 | 模块级注册表持有 xterm 实例 + PTY id + 常驻宿主节点 | 切走 = 把 DOM 节点 `appendChild` 到 `display:none` 的 stash 容器；真回收只有显式关 tab 和工作区关闭 |
| 浏览器 | Electron main 进程管驻留（live/suspended），渲染层管壳 | 关闭必须先拿 main 的 authority（删恢复快照）再删 UI 壳，否则重启后 tab 复活 |
| 一切 tab 内容 | `forceMount` + `data-[state=inactive]:hidden` | 切 tab / 面板收起只隐藏不卸载，是 webview 保活的基础；面板首次打开后永久保持挂载 |

### 2.4 布局与持久化

- 左侧栏不用布局库：普通 div + CSS 变量宽度 + 自实现拖拽（拖拽中只写 CSS 变量，松手才 setState 并持久化 `zcode:workspace-shell:sidebar-width-px`）。
- 对话列与 Side Pane 用 react-resizable-panels 的 `useDefaultLayout`（`layoutId="workspace-body-layout"`）自动持久化占比；开合动画只在显式开合时短暂启用过渡，`rememberExpandedSize` 记住收起前的宽度。
- **持久化哲学**：宽度/占比进 localStorage；**tabs 列表、每会话偏好、浏览器 URL 只进内存**（重启即丢，浏览器例外由 main 的恢复快照冷恢复）。避免持久化失效路径（会话删除、文件移动）带来的恢复垃圾。

## 三、CraftWork 现状与差距

### 3.1 已有的底子（保留）

- 面板容器 `Panel.tsx`：拖拽调宽（pointer 事件 + `--panel-w`）、收起动画（`is-closed` 时 DOM 保留）、窄窗口浮层（≤1179px `is-overlay`）、最大化（`is-maximized`）、全局 Esc 逐层退出（浮层 → 最大化 → 运行中会话 → 待审批）。
- 终端已是多会话（`stores/terminal.ts` 的 sessions，输出按会话缓冲 20 万字符，boot 时拉回还活着的 PTY）。
- 入口齐备：聊天头部按钮、命令面板、正文文件引用 `openFileAt`、审批卡片「查看完整差异」、文件树点击。
- 四个面板组件（文件树+编辑器、差异、终端、文档）与后端服务（WorkspaceService / DocumentService / TerminalService）已闭环。

### 3.2 差距清单（以代码为准）

| # | 差距 | 现状 |
| --- | --- | --- |
| 1 | 单槽位 | `tabs: Record<Mode, PanelTab>`，每种模式只记一个互斥按钮，无 tab 条、无关闭概念 |
| 2 | 编辑器不是一等 tab | 是 FilesPanel 的子状态（`selected` 非空整树被替换），单文件打开 |
| 3 | 切换即卸载 | 条件渲染，CodeMirror/xterm 实例销毁重建，光标/撤销/滚动位置丢失 |
| 4 | 零持久化 | 宽度、开合、tab 全部不落盘（全前端仅主题 `cw-theme` 持久化） |
| 5 | 无最近关闭 | `setTab` 直接覆盖，无栈 |
| 6 | diff 单槽位 | `workspace.diff` 单值（`.diff-files` 多文件样式已备未接线） |
| 7 | 无对话列宽度自动收起 | 只有固定 1179px 浮层切换 |
| 8 | 无面板快捷键 | 开合只能按钮/命令面板；`Cmd+W` 未接 |

## 四、CraftWork 落地方案

### 4.1 设计原则

1. **完整搬状态模型**：ZCode 的分层（纯函数状态库 → 内存记忆 → 唯一写出口 → forceMount 渲染）、幂等 id 设计、**workspaceKey 分桶、按线程归属过滤**全部搬。CraftWork 数据层本就是多工作区模型（`workspaces` 表有 `project` / `conversation` 两种用途，`threads.workspace_id` 关联，前端有 `app.activeWorkspace`），分桶的数据基础已就绪，跨项目串台是真实风险，必须现在就做对。
2. **面板能力按依赖排期**：会话派生类面板（子代理会话、侧边聊天、后台任务输出）与浏览器面板纳入路线图（见 4.6），各自标注前置依赖；其中状态模型的归属维度（rootThreadId 等复合 id 维度）在阶段一的类型设计里一并预留，避免将来重构。
3. **搬语义不搬实现**：ZCode 依赖 Electron main 进程的机制（浏览器关闭 authority、驻留管理、webview 停靠层）不复制其实现，但在方案里给出 CraftWork 下的等价语义或替代手段（见 4.6、4.7）。
4. **渐进迁移**：现有 `Mode`（code/work）与四个入口的行为语义不变，逐步把「模式内互斥按钮」替换为「统一 tab 列表」。

### 4.2 目标数据模型（阶段一）

```ts
// frontend/src/lib/sidePane.ts（新建，纯函数，可单测）

type SidePaneTabType = "file" | "diff" | "terminal" | "document";
// 会话派生类型在路线图里（见 4.6）：subagent-thread / side-chat / background-task / browser / plugin。
// 复合 id 的派生维度（rootThreadId 等）现在就定下命名规则，新类型按规则扩展，不动基类。

interface SidePaneTabBase {
  id: string;              // 结构化 id，幂等键；singleton 类型为固定字面量
  type: SidePaneTabType;
  title: string;
  workspaceId: string;     // 打开时冻结的所属工作区（分桶键），提交时盖戳、不随上下文漂移
  threadId?: string | null; // 打开时冻结的归属线程；null = 未认领（草稿语义 "draft"）
  openedAt: number;
}
// file:     { id: `file:<workspaceId>:<folderID>:<path>`, folderID, path, language? }
// diff:     singleton，id 固定 "diff"，打开即刷新内容（沿用现有 openDiff 语义）
// terminal: singleton，id 固定 "terminal"，内部保留现有多会话 term-tabs
// document: { id: `document:<workspaceId>:<folderID>:<path>`, folderID, path, kind }

interface SidePaneState {
  tabs: SidePaneTab[];         // 一份共享数组；各线程看到的是可见性过滤视图，不是各存一份
  activeTabId: string | null;  // 允许指向当前线程不可见的 tab（切走时保留别人的激活项）
  recentClosed: SidePaneTab[]; // 上限 8；按当前 workspace + thread 过滤后展示
}
```

幂等规则：`file`/`document` 按 id 查已有即激活并刷新（同文件不重复开）；`diff`/`terminal` singleton 重复打开只聚焦。**不做「+」新标签页空态**：CraftWork 的 tab 都有明确入口（文件树/聊天引用/审批卡片），类型少，启动页无价值。

**Mode（code/work）的处置**：保留 Mode 作为视觉与默认值的上下文——Code 模式激活色不变、打开文件走编辑器 tab、Work 模式打开文档 tab（现行为）；tab 列表本身统一一份，`tabs: Record<Mode, PanelTab>` 删除，`setTab(mode, tab)` 迁移为 `openFileTab / openDiffTab / openDocumentTab / activateTab`。现有的 `workspace store`（tree/content/draft/diff/doc 数据槽）不动，只把「当前显示哪个」从单值升级为 tab 数组。

**workspaceKey 分桶（对齐 ZCode，数据已就绪）**：

- 分桶键直接用 `workspaceId`（CraftWork 有稳定 ID，比 ZCode 用 path/identity 拼接更干净）；临时会话线程落在 `conversation` 工作区的桶里。
- 存储结构与 ZCode 同构：模块级 `Map<workspaceId, WorkspacePaneMemory>`，每桶持有 `{ tabs, activeTabId, recentClosed, 每线程开合偏好 }`，LRU 上限 50 桶（防项目数增长后内存无限涨）。
- 切换工作区（`app.activeWorkspace` 变化）→ 当前桶写回内存 → 恢复目标桶 → 按目标桶的可见性过滤渲染。跨工作区的后台事件（如 Agent 在别的线程改了文件）写回**原工作区的桶**，不污染当前视图。
- localStorage 持久化仍只存宽度与开合偏好；tab 列表只进内存（ZCode 同款哲学：避免恢复失效路径）。重启后每个工作区的桶为空，首次打开时按入口重建。

**归属与可见性（对齐 ZCode `isSidePaneTabVisibleInScope`）**：

- 打开 tab 时冻结 `workspaceId + threadId`（未认领的盖戳为当前值；已带归属的绝不覆盖）。
- 可见性 = 工作区匹配 **且** 归属匹配。工作区公共 tab（当前集合 = `terminal`）对所有线程可见；其余类型（file/diff/document）按 `threadId` 过滤，切线程隐藏不销毁，切回恢复，激活项记忆按 `<workspaceId>::<threadId>` 保存。
- 草稿（未发送首条消息的线程）归一为 `"draft"` owner；删除线程时联动清理该线程的 tab（不入最近关闭栈）。
- 若实测「切线程文件 tab 全隐藏」碍事，降级开关是一行常量：把 `file`/`document` 移入公共集合即可——但默认对齐 ZCode（过滤），因为它保证了「回到某个线程时看到的就是离开时的上下文」。

**会话派生维度的预留**：未来 tab 类型（子代理会话、侧边聊天）的可见性维度不是简单 `threadId`——ZCode 的教训是子代理按 `rootSessionId`、侧聊按 `parentSessionId`，混用一个维度会互相误伤（见其 `workspaceSidePane.ts:335-351` 注释）。基类因此保留 `threadId` 为「打开时所在线程」，新增派生类型时各自声明过滤维度字段（如 `rootThreadId` / `parentThreadId`），可见性函数按类型分派。

### 4.3 阶段一：多 tab 模型（核心，先行）

改动面：

1. `frontend/src/lib/sidePane.ts`：类型 + 纯函数转移函数（`openTab`（幂等合并 + 盖戳归属）、`activateTab`、`closeTab`（关 active 时选邻居）、`closeOtherTabs`、`closeAllTabs`、`pushRecentClosed`、`reopenRecentClosed`、`ensureMinTab`）+ 可见性函数 `isTabVisibleInScope(tab, { workspaceId, threadId })`。
2. `frontend/src/stores/panel.ts`：接分桶记忆层（模块级 `Map<workspaceId, …>`，LRU 50），`tabs`/`activeTabId`/`recentClosed` 替换 `tabs: Record<Mode, PanelTab>`；保留 open/maximized/width/narrow 语义；**当前线程可见 tab 清空 → 自动收起**（含退出最大化）。
3. 线程/工作区切换接线：`app.activeWorkspace` 与 `threads.activeThreadId` 变化 → 写回旧桶、恢复新桶、按新 scope 过滤渲染；关闭线程时清理其名下 tab。
4. `frontend/src/features/panel/Panel.tsx`：模式按钮组改 tab 条——关闭按钮（选中/hover 显示）、中键关闭、右键菜单（关闭/关闭其他/关闭全部）、最近关闭入口；tab 宽度自适应（上限 156px、下限 60px，全部到下限仍装不下才横向滚动）；「打开终端」等入口改为 `openXxxTab`。
5. `Editor` 升一等 tab：`FilesPanel` 不再整树替换为编辑器；文件树点文件 → `openFileTab`；编辑器 tab 关闭即回文件树（文件树本身可以保留为一个常驻的「文件」tab 或作为无 tab 时的默认视图，倾向后者：文件树是导航器不是内容 tab）。
6. 差异：保持 singleton + 「应用全部修改」现有流程；`.diff-files` 多文件列表留到后续。
7. 入口迁移：`openFileAt`、审批「查看完整差异」、文档打开（workspace store 里的 `openFile/openDiff/openDocument`）改调新转移函数；命令面板的 toggle-panel / maximize-panel 不变。
8. 单测：`lib/sidePane` 转移函数全覆盖（幂等、盖戳归属、可见性过滤、切桶恢复、关末收起、最近关闭上限与 scope 校验、singleton 语义）。

### 4.4 阶段二：保活与持久化

1. **切 tab 不卸载**：所有 tab 内容 `forceMount`，非激活 `display:none`（`.panel-body` 内按 tab 切换 hidden class，沿用现有 `is-closed` 保 DOM 思路）。收益：CodeMirror 光标/撤销/搜索状态保留、xterm 实例不重建（终端内部多会话本就依赖保活）、文档预览滚动位置保留。
   - 已知技术点：`display:none` 下 CodeMirror/xterm 测量失效，**切换显示时要主动触发一次 relayout**（CodeMirror `requestMeasure`、xterm `fit()`），挂到 tab 激活的 effect 里；两个内核（WKWebView/Chromium）行为一致，无分叉。
   - 内存闸门：file tab 上限 8 个，超出自动关闭最久未激活的（走关闭流程但不进最近关闭栈——与 ZCode 的投影清理一致）。
2. **持久化（localStorage，双宿主通用）**：
   - `cw-panel-width`：面板宽度；
   - `cw-panel-open`：开合状态；
   - **tab 列表不持久化**（与 ZCode 哲学一致）：文件可由文件树重建、终端 PTY 随进程退出消失、恢复失效路径是纯负担。重启后面板打开时显示默认视图。
3. 终端会话跨重启：维持现状（boot 拉活 PTY），PTY 生命周期在 Go 侧不受前端 tab 影响。

### 4.5 阶段三：布局与快捷键

1. **对话列宽度自动收起**：监听 `window.resize`，防抖 300ms 后实测对话列宽度，< 480px 收面板；与现有 1179px 浮层逻辑并存（窄窗口仍走浮层）。只在窗口 resize 时判定，不监听对话列 ResizeObserver，避免手动开面板被关回去。
2. **快捷键**：
   - `Cmd/Ctrl+B`：开合右侧面板（建议，与主流编辑器一致；命令面板同步暴露）。
   - `Cmd/Ctrl+W`：关闭当前 tab，无 tab 时不拦截（**仅桌面模式注册**；server 模式下 Cmd+W 是浏览器自身关标签，绝不拦截——这是双宿主的硬差异）。
   - Esc 退出链在最前插入一级：激活 tab 可关时先关 tab（可与现有「浮层 → 最大化」链合并设计，实现时定稿）。
3. **双击分隔条重置默认宽度**；补 `.panel-resize` 拖拽中的 `is-dragging` 视觉态（样式已有未接线）。
4. 键盘可达性：分隔条补 `role="separator"` + 方向键调宽（对齐 ZCode 侧栏做法）。

### 4.6 面板能力路线图（会话派生 / 浏览器 / 插件）

ZCode 的 24 种 tab 类型里，与 CraftWork 演进方向相关的会话派生类与浏览器类逐项纳入路线。每项的**状态模型归 4.2 的类型体系管**（复合 id 维度、归属过滤规则现在就定好命名与分派规则），**面板本体按前置依赖排期**：

| 面板 | ZCode 对应物 | CraftWork 语义 | 前置依赖 | 建议排期 |
| --- | --- | --- | --- | --- |
| 子代理会话 | `subagent-session`（按 rootThreadId 分组、只读 transcript、child 失效自动回收 tab） | Agent 派生子线程时，在面板打开子线程的只读视图，父线程名下可见 | Agent 循环支持子代理并派生线程 id | Agent 侧立项后 |
| 侧边聊天 | `selection-side-chat`（带 ordinal 多开、关闭即结束派生会话、不入最近关闭） | 在编辑器/消息里选中内容发起辅助对话，面板里单独一个 tab | 派生对话能力（子线程 + 选区上下文注入） | 同上 |
| 后台任务输出 | `bash-output`（按 rootThreadId 归属、可见时才拉增量、「查看完整文件」跳编辑器 tab） | `shell_exec` 支持转后台后，长任务输出在面板跟踪 | 后台任务执行与输出流事件 | 工具侧立项后 |
| 浏览器（人工） | `browser`（webview 多实例、URL 记忆、标题/favicon 事件回写） | 面板内浏览网页 / 预览本地应用 | 技术选型 PoC（见下） | PoC 通过后 |
| 浏览器（Agent 受控） | `browser-use`（CDP 控制、main 管驻留） | Agent 操控内嵌浏览器 | 浏览器 PoC + 浏览器工具链（审批/风险分级） | 远期 |
| 插件面板 | `plugin-ui`（清单声明 surface、常驻停靠层、逻辑键共享页面实例） | 插件在面板里渲染界面 | M4 MCP/沙箱 | M4 |

**浏览器面板的技术选型是 CraftWork 特有的硬问题**——ZCode 依赖 Electron `<webview>` + main 进程 CDP，Wails3 两者都没有。候选手段：

1. **iframe 嵌入**（成本低）：需放开生产 CSP 的 `frame-src`；受目标站点 `X-Frame-Options` / `frame-ancestors` 限制，大量网站拒绝被嵌，适合预览本地应用与允许嵌入的页面；与宿主同进程，崩溃隔离弱。
2. **Wails3 第二窗口按锚点摆位**（成本中，语义最接近 ZCode 的停靠层）：为每个浏览器 tab 开一个无边框子 `WebviewWindow`，面板 tab 作为锚点，宿主窗口移动/缩放时跟随定位——ZCode 插件停靠层（DOM 外常驻层 + 锚点几何追踪）的窗口版。代价：多窗口焦点/遮挡/z 序管理复杂，WKWebView 多实例内存开销，跨平台窗口行为差异要逐个验证。
3. **系统浏览器外部打开**（成本最低，现有 `OpenExternal`）：面板只放 URL 工具卡与浏览记录，不内嵌。

结论：先按选项 1 做 PoC（覆盖「预览本地 dev server + 少数可嵌站点」场景），同时验证选项 2 的可行性；Agent 受控浏览在两条路都未验证前不承诺。

其余远期项：

| 项 | 触发条件 | 说明 |
| --- | --- | --- |
| 完整视图（面板占满 + 浮动输入窗口） | 单独立项评估 | 依赖「对话区 DOM 不重挂载、只改定位」的改造；ZCode 的单实例原则与 fixed 投影机制值得照搬，但体量大，对当前产品价值待定 |
| diff 多文件列表 | Agent 产出多文件变更批 | `.diff-files` 样式已备；届时把 diff 从 singleton 升级为 `diff:<hash>` 复合 id |
| 标签总览搜索 | tab 类型超过 ~6 种 | 现阶段最近关闭入口已够 |

### 4.7 只搬语义、不搬实现的部分，与明确不搬的东西

**只搬语义**（ZCode 靠 Electron main 进程实现，CraftWork 没有对应层次）：

- 浏览器关闭 authority（先确认资源关闭成功再删 UI 壳，防重启后 tab 复活）：CraftWork 的对应资源是终端 PTY——关闭终端 tab 前必须 `TerminalService.Close` 成功再删 tab 壳，失败时保留 tab 并 toast，语义等价、无需 main 进程。
- 浏览器驻留管理（live/suspended 分级）：iframe 方案下页面始终随 DOM 挂载，无此需求；若走第二窗口方案，等价物是窗口 hide/show，在浏览器 PoC 里一并定。
- webview 停靠层：见 4.6 浏览器 PoC。

**明确不搬**：

- **react-resizable-panels**：CraftWork 自研拖拽 + CSS 变量已满足；ZCode 自己的左侧栏也弃用了该库改 CSS 变量，说明库的收益主要在多 panel 联动，CraftWork 用不上。
- **「+」新标签页与启动页**：入口已齐，类型少；将来插件面板（M4）落地时再评估是否需要统一启动页。
- **dnd-kit 拖拽排序**：tab 数量少收益低；确需时后置（可用原生 DnD 实现）。
- **workflow 系列 5 种 tab、白板、模型轨迹、开发者工具、treemapping**：CraftWork 无对应产品概念，ZCode 也是围绕自家工作流产品长出来的，不具备通用性。

### 4.8 风险与对策

| 风险 | 对策 |
| --- | --- |
| forceMount 后多实例内存开销 | tab 类型与数量有上限（file 8 个）；xterm 多实例已是现状；验证时盯 DevTools 内存 |
| `display:none` 下编辑器/终端测量失效 | tab 激活 effect 里强制 relayout（阶段二的核心技术点，先写单测 + 手测双内核） |
| server 模式快捷键冲突 | `Cmd/Ctrl+W` 仅桌面注册；`isDesktop` 判断沿用 `lib/bus.ts` 既有规则 |
| 迁移期行为回归 | 阶段一保持入口语义不变（打开文件/差异/文档仍唤起面板），e2e 现有用例应全绿；新增 tab 条交互用 `data-cw-*` 补测试 |
| 文档同步 | 实现时按 AGENTS.md 第 9 节同步 `docs/specs/interface.md`（tab 条视觉与交互）与 `docs/specs/features.md` |

## 五、验证基线（实现后）

- 单测：`lib/sidePane` 全部转移函数；可见性过滤（工作区匹配 × 线程归属 × 公共 tab）；分桶记忆（切桶写回/恢复、LRU 淘汰、跨工作区事件写回原桶）；panel store 关末收起/最大化联动。
- 前端：`pnpm run lint`、`pnpm exec tsc --noEmit`、`pnpm run test`。
- 手测清单（1440×900 与 1100×800）：文件树开 3 个文件切 tab（光标/滚动保留）→ 中键关闭 → 右键关闭其他 → 最近关闭恢复 → diff singleton 聚焦 → 终端切走再切回（输出连续）→ 关末 tab 面板收起 → **线程 A 开文件 → 切线程 B（tab 隐藏、面板收起）→ 切回 A（tab 与激活项恢复）** → **项目工作区与临时会话工作区来回切（两桶互不串台）** → 刷新后宽度/开合恢复 → 窄窗口浮层 → 双内核各走一遍。
- server 模式冒烟：`wails3 task run:server` 走一遍上述清单，确认 Cmd+W 未被拦截、localStorage 持久化生效。
