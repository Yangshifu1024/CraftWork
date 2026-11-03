# 14. 聊天区观感：正文居中限宽、composer 外框高亮、标题栏复刻

Status: resolved

## 背景

2026-11-03 用户对照旧版走查发现的复刻差异：

- 聊天正文全宽铺满（`internal/ui/chat/view.go:246` 为防 Markdown 塌零宽显式 `FillWidth`），超宽窗口一行 100+ 字符，行长过长。
- composer 聚焦高亮只套内部输入区，旧版高亮整个 composer 外框（2026-11-03 截图佐证）。
- 自绘标题栏形态与旧版不符（差异定性见下方 Comments）。

## 改动

- 聊天正文列居中限宽：两侧 margin 10%（用户给定参数）。实现注意 `view.go:245` 注释的坑——Markdown 在收缩布局会塌成零宽，限宽后仍要保底宽度；用户气泡的 78% 上限改为相对正文列而非窗口宽。
- composer 聚焦态高亮整个外框而非仅输入区。
- 标题栏复刻：**完全参考 ZCode 实现**（用户 2026-11-03 指定；无旧版截图可比，`interface.md` 的旧版描述仅作背景不作为依据）：
  - 现状：`internal/ui/shell.go:230-247` 的 `topbar` 画了一条独立 48px 标题栏条（`BgPanel` 底 + 底部分隔线 + 居中品牌名），把整个内容区压下去。
  - 目标形态（ZCode `packages/desktop/src/main/desktopWindowChrome.ts` + `DesktopTopOverlay.tsx`）：**无独立标题条**，内容直接从窗口顶开始，红绿灯叠加在内容上。
  - 窗口选项对齐：`TitleBarStyle` 从 `hiddenInset` 改为 `hidden`（ZCode 同款），红绿灯位置完全由 `mygo.WindowOptions.TrafficLightPosition` 控制为 ZCode 基准 `{x: 22, y: 23}`（`MACOS_TRAFFIC_LIGHT_BASE_POSITION`）；内容左上避让约 96px（`BASE_LEFT_PADDING_PX`）。
  - 拖拽语义对齐：顶部内容行的空白区域即拖拽区（保留 `DragWindow` 与双击缩放），交互控件不注册拖拽。
  - 毛玻璃材质对齐：mygo v0.7.4 原生支持（`vibrancy.go`），`WindowOptions{Transparent: true, Vibrancy: mygo.VibrancyUnderWindow}` 与 ZCode 的 `vibrancy: "under-window"` 同名材质；启用后视图每帧铺的底色要相应调整让材质透出（侧栏 / 顶栏半透明，对话区是否保留实底对照 ZCode renderer 的透明度处理），`c.Vibrancy()` 可判断视图是否在材质上。
  - ZCode 的 `visualEffectState: "active"`（非前台仍按激活画）对应 mygo 的 `SetActiveAppearance`；若 under-window 材质失焦观感差异明显再启用，实现时对照 ZCode 观感定。
  - ZCode 左上角工具组（侧栏开关 / 前进后退 / 新建 / 更新）不在本票范围：CraftWork 没有对应功能的不新增，本票只做形态机制。
  - 顺带处理 `topbar` 右段注释（「运行状态与设置入口由 05 / 07 号票接入」——两票已 resolved 而未接入，落实或移除该注释，不留误导）。

## 验收

- 1440×900、1100×800 与超宽（约 2000px）三档窗口下正文列居中、行长合理、无零宽塌陷。
- composer 聚焦态外框高亮，失焦恢复。
- 无独立标题条：内容行顶到窗口顶，红绿灯不遮内容，顶部空白可拖动窗口、双击缩放；设置等浮层打开时拖拽让位语义不变。
- 与 ZCode 并排运行，窗口顶部形态对照一致（红绿灯位置、顶部行起点、拖拽行为）。
- 观感取值先更新 `docs/native-ui/spec.md` 再实现（AGENTS.md 第 11 节）。

## 依赖

无。

## Comments

- 2026-11-03 开票留档：`docs/native-ui/spec.md` 无正文列宽约定，本次限宽是新增观感约定，动手前先写 spec。
- 2026-11-03 标题栏定性（用户指定参照 `/Users/yangzhenbiao/Code/ZCode`）：截图里的「CraftWork」不是系统标题栏——mygo `TitleBarHiddenInset` 已生效（`internal/darwin/window.go:201-203` 设 `NSWindowTitleHidden`），这行字是 `shell.go:241` 应用自绘；差异在应用自绘了一条独立标题栏条，而旧版与 ZCode 都是顶部融入内容的无条形态。ZCode 关键参照：`titleBarStyle: "hidden"` + `trafficLightPosition {x:22, y:23}`、顶部左上为工具按钮组（no-drag）、其余顶部区域拖拽（`DesktopTopOverlay.tsx`，h-14、红绿灯区 padding-left 96px）。
- 2026-11-03 勘误（用户指正）：初稿误写「mygo 若无 vibrancy 能力则跳过」——实际 mygo v0.7.4 原生支持（`mygo/vibrancy.go` 完整材质清单 + `SetVibrancy` 运行时切换 + `c.Vibrancy()` 视图查询），`VibrancyUnderWindow` 与 ZCode 的 `"under-window"` 同名同源，已改为对齐实现。文档：mygo.egoist.dev/docs/windows#options。
- 2026-11-03 实现留档（Status → resolved）：
  - **实现决策**：
    - 正文列限宽用 `WidthPercent(80) + MinWidth(320)` 显式宽度构建（`chatColumnWidth` 纯函数为同一公式），天然不塌零宽；用户气泡 78% 改为相对正文列（max-width 百分比基于父级，移进正文列即相对列宽）。composer 外框与正文列同参数同宽（两段上下对齐），composer-wrap 原左右 18px 内边距由正文列自身 margin 取代。
    - composer 聚焦态用外框 `FocusWithin` 判定（帧内记录 `ComposerFocused` 供走查断言），聚焦描边切模式强调色、失焦回 `BorderStrong`。
    - 「内容左上避让 96px」未按字面引入：96px 是 ZCode 左上工具组（侧栏开关/前进后退/新建）与侧栏隐藏态 header 的参数，CraftWork 无对应功能（票内已声明不新增），侧栏固定可见。机制目的「红绿灯不遮内容」由「拖拽行高 48 > 红绿灯下缘 37」达成，红绿灯 `{x:22,y:23}` 完全落在侧栏顶部空白拖拽行内。
    - 侧栏透材质的实现路径：`theme.MaterialPalette` 返回半透明侧栏色板**副本**（底 72%、悬停 60%），外壳在 `c.Vibrancy()` 为真时以副本下发给侧栏视图——侧栏视图自身零改动（13 号票槽位不受影响），无材质环境自动回退实底。对话区与面板保持实底（对照 ZCode workspace 的 `bg-background`）。
    - `SetActiveAppearance`（ZCode `visualEffectState:"active"`）未启用：无头环境无法对照失焦观感，留给实机走查后决定（票内既定决策）。
  - **框架坑**：
    - mygo 布局是 CSS flexbox：交叉轴默认 stretch，但显式宽度（`WidthPercent`）的子元素不拉伸、默认对齐 start——居中必须显式 `Justify(Center)`（Row 主轴）或 `AlignItems(Center)`（Column 交叉轴）；`MinWidth` 在 `clampW` 里后于 `MaxWidth` 生效，保底语义与 CSS 一致。
    - `DragWindow` 只对「自身非交互」的元素生效（`input.go`：命中链上第一个 interactive 元素才算 target），因此头部行内放普通 Text/徽章时点文字同样可拖，行内将来加按钮则天然 no-drag，无需手工划分。
    - `WindowOptions.Transparent` 与 `BackgroundColor` 互斥（darwin 分支 Transparent 优先），改用透明窗口后 `theme.WindowBackgroundHex` 已删除。
    - Tester 侧 `SetVibrancy(bool)` / `SetTitleBar(TitleBar)` 可模拟材质与红绿灯占位，材质底色分派用帧图像像素透明度断言（侧栏 <255、对话区 =255）。
  - **未尽事项**：
    - 「与 ZCode 并排对照」的视觉验收留给用户实机走查（无头环境无法截图对比）：红绿灯位置、顶部行起点、拖拽行为、失焦材质观感。
    - 侧栏悬停行在材质上是半透明色块，与 ZCode 的 color-mix 淡色 hover 有细微差异；如走查不满意，可在 MaterialPalette 里继续调 alpha 或推动侧栏侧的半透明 token 化（涉及 13 号票槽位）。
    - 窗口级 Esc 退出链、浮层让位语义未改动（RootView 与 DialogBase 的既有机制不受标题栏形态影响）；`go tool mygo build` 产物可启动（.app + DMG 已产出），真实窗口的拖拽/双击缩放/红绿灯位置需实机确认。
