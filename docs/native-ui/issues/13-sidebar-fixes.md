# 13. 侧栏修正：新建即切换、分组行布局与间距、标题跑马灯

Status: resolved

## 背景

2026-11-03 用户对照旧版逐项走查发现的复刻差异，集中在侧栏（`internal/ui/sidebar/`）。附走查截图佐证：空态行高过大、计数右对齐、全折叠间距过大。

## 改动

- 新建会话后立即切换到该会话：当前点 + 新建后列表出现新会话但选中不跟随；旧版行为是创建即切换。
- Code / Work 模式切换按钮：图标与文字紧贴，补间距（取 theme spacing token）。
- 空分组空状态：「还没有临时会话，点上方 + 新建」所在行的行高或上下间距过大，收紧到与列表行节奏一致。
- 会话标题截断过短（「解释这个仓...」）：放宽可显示宽度；hover 时跑马灯滚动显示完整标题（旧版行为）。
- 分组计数 badge：应紧跟分组文字，当前右对齐贴到 + 操作按钮旁（截图红框标注）。
- 全折叠时各分组之间的间距过大，收紧。

## 验收

- 六项逐一对照走查截图复核。
- `ui.Tester`：新建即切换断言、计数 badge 位置断言；间距与字号取 `internal/ui/theme`，不写死值。
- 1440×900 与 1100×800 两档窗口宽度走查（AGENTS.md 第 8 节清单）。

## 依赖

无。

## Comments

- 2026-11-03 开票留档：走查截图见会话记录（临时会话空态、CraftWork 分组标题截断、计数红框标注、全折叠间距）。
- 2026-11-03 实现留档（13 号票收尾）：

  **六项落地方式**

  1. 新建即切换：根因不在侧栏选中——`createAsync` 一直有 `Select(thread.ID)`，缺的是聊天列联动。收尾回调在侧栏选中之后补发 `Actions.SelectThread`（app.go 已接 `chatClient.SelectThread`），侧栏 + / ⌘N / 命令面板 / 创建项目四条新建路径一并生效，app.go 零改动。
  2. Code / Work 按钮间距：`modeButton` 的 ButtonBase 是无样式 Row（不像 mygo 的 `Button` 自带 Gap），补 `Gap(theme.Space3)` 对齐 `.mode-btn` 的 gap: 6px。
  3. 空态行高：`listEmpty` 的 padding 写的是 36/30，注释目标却是 26/16——token 组合算术错误。修正为 `Space7+Space4+Space1`（26）与 `Space7`（16）。
  4. 标题截断放宽 + 跑马灯：截断过短的根因是**两行被排成了一行**——`threadItem` 的 ButtonBase 是 Row，标题文本与时间状态行水平排，标题只能拿时间行之外的剩余宽度（实测 228 可用宽只拿到 136.5）。行内改纵向 `Column(Gap 3)` 后标题占满行内容宽。跑马灯见下。
  5. 计数徽章：组名不再 `Grow(1)`，徽章紧跟其后，弹性 `mygo.Spacer` 把 + 按钮推到行尾（对齐 `.thread-group-add` 的 margin-left: auto）；孤儿组头无 + 按钮，徽章同样紧跟组名。超长组名单行截断不把徽章推出可视区（有 Tester 断言）。
  6. 全折叠间距：`groupHeadPad.t` 写的是 22（`Space6+Space4+Space1`），CSS 是 12——同样是算术错误，每个组头顶部多 10px，全折叠时逐组累积。修正为 `Space6`；区头顶边距随常量联动 28 → 18（12 + margin-top 6）。

  **跑马灯实现**：旧版 frontend 全历史检索确认从未实现跑马灯（票里的「旧版行为」无对应代码），按票面要求用 mygo 自绘新搭。结构是 `Box.Fill().ClipX()` 裁剪窗口内放 `SingleLine` 标题，悬停且超宽时 `Element.Loop` 驱动横向 `Translate`：相位为「停 20% → 滚出 30% → 停 20% → 滚回 30%」，段内 EaseInOut，周期 = 4s 底数 + 溢出每 40px 加 2s（速度恒定 ~40 DIP/s）。溢出量 = `c.MeasureText(0, Span{...})` 与 `threadTitleWidth()`（token 组合的行可用宽）之差，只在悬停帧测量，静止列表零成本。`theme.ReduceMotion` 为真不启动；Loop 只在悬停帧调用，移开即停。

  **框架坑**

  - mygo 的 `ButtonBase`/`Box` 都是 Row 语义，纵向布局必须显式 `Column`；「CSS 是 column、Go 里忘包」这类移植错不报错、只表现为挤压截断。
  - `Element.Loop` 从首次调用起计时且不调用不推进帧，天然适合「悬停才开始、移开即暂停」的循环动效；但它按墙钟取模，帧间暂停不会重置起点（本场景无影响）。
  - `ui.Tester.Find` 是文本全等匹配：badge 数字、组名等短文本极易撞车（"3" 同时命中区头与项目组徽章），位置断言前先保证数据里的文本互不相同。
  - badge 文本 rect 是文本自身的位置，不含徽章 Box 的 8px 水平内边距，断言间距时要把内边距算进去。

  **未尽事项**

  - `threadList` 的滚动区内边距写的是 12（`Space4+4`），`.thread-list` CSS 是 8——同为算术错误但不在本票六项内，未动，建议下轮走查一并修正。
  - 区头「项目会话」与组头之间仍有 margin-top 6 的区隔（对齐 CSS），全折叠的整体节奏以 1440×900 / 1100×800 两档实测为准（本环境无头，未截屏复核）。
  - 临时组空态与项目区空态共用 `listEmpty`（26/16），若后续想区分两处节奏需拆开。

  验证：`go build`、`go test ./...`、`go vet`、`golangci-lint run` 全绿；`go tool mygo build` 产出 .app 与 DMG。新增/修改用例：全折叠组头间距、空态行高、徽章跟随组名、长组名截断、标题两行布局、跑马灯相位/偏移/周期纯函数、悬停与 ReduceMotion 冒烟、新建即切换（真实 store + service 两条链路）。
