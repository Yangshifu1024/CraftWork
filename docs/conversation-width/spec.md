# spec：对话内容列宽度档位

对话区（`Conversation` 卡片）内的内容列——消息流与沉底输入卡——的宽度行为约定。对标 ZCode 的 `conversationLayout`（容器查询多档渐进），按本仓库结构（右侧面板为独立占位列，非内联浮层）裁剪。

## 行为约定

1. **容器上下文**：对话卡片根节点声明命名容器 `conversation`（Tailwind `@container/conversation`）；宽度档位全部以该容器宽度裁决，与窗口宽度、侧栏宽度解耦——右侧面板开合改变容器宽度时档位自动升降。
2. **会话档位**（有消息的会话）：
   - 容器 < 864px：内容列全宽（`max-w-full`）；
   - 容器 ≥ 864px：上限 896px（`max-w-4xl`）；
   - 容器 ≥ 1280px：上限 1152px（`max-w-6xl`）。
3. **草稿档**：会话尚无消息（新建会话 / 草稿槽）时内容列收窄到 672px（`max-w-2xl`），输入聚焦；发出首条消息后放宽到会话档位。
4. **平滑过渡**：档位间切换（断点跨越、草稿→会话、面板开合）对 `max-width` 做 150ms 过渡；各档默认值均为可插值长度（`max-w-full`），不出现 none↔固定值的硬跳。
5. **宽内容**：表格、代码块等宽内容保持在内容列内横向滚动（streamdown `table-wrapper` 自带 `overflow-x-auto`）；表格单元格设最小列宽（6em），窄容器下列文字不逐字竖断。
6. **不做左移让位**：ZCode 宽档的内容列左移 168px 是为其内联覆盖式状态面板让位；本仓库右侧面板是独立占位列，内容列保持居中。
7. **表格与代码块高度全展开**：表格与代码块不限高、不折叠、无展开按钮。三处来源都要禁掉：
   - streamdown 组件默认 `tableMaxHeight: 300px`、`codeBlockMaxHeight: 400px`（内联 max-height + 垂直滚动）——在 `Markdown` 组件显式置 0；
   - streamdown 代码块默认的 `content-visibility: auto` + 200px 估算高度（视口外占位、临近跳变为真实高度，滚动抖动）——样式层关闭（`content-visibility: visible`）；
   - 自身样式不得引入任何 max-height。
8. **计划批准条必须实底**：计划批准提示条浮在输入区 sticky 渐变层的顶部透明区上，`bg-surface` 是半透明层次色（浅色 3% 黑 / 深色 5% 白），消息文字会穿透显示——必须用实底（`bg-input`，与输入卡同源）+ 边框 + 轻阴影，保证浮层自身可读、不与消息流文字重叠。

## 实现落点

- 宽度档位类：`src/renderer/src/components/conversation.tsx` 顶部共享常量（消息列与输入卡共用，禁止两处各自硬编码）。
- 表格最小列宽：`src/renderer/src/styles.css` 的 streamdown 覆盖区（`.markdown-body`）。
