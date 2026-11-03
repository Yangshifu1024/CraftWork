# 14. 聊天区观感：正文居中限宽、composer 外框高亮、标题栏复刻

Status: ready-for-agent

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
