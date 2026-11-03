# 15. composer 浮层锚定化与输入区观感修复（11 / 14 号票实机走查返工）

Status: resolved

## 背景

2026-11-04 用户本机走查（隔离目录 /tmp/cw-verify）发现的实现缺陷，11 号票两处、14 号票一处：

1. **浮层形态错误（最重）**：11 号票把模型 / 档位 / 风险 / 上下文四浮层做成了 `DialogBase` 中央模态（`placePanel` fraction 0.18）+ 全屏压暗遮罩。旧版 React 是锚定 Popover：贴触发控件弹出、无遮罩。实机效果「点底部的控件、弹窗出现在窗口中上部、整窗变灰」，空间关联与观感全错。形态替换发生在 11 号票接手时保留的半成品（实现者注释自认「Promise 时代是锚定下拉，原生侧统一走 DialogBase 模态」），code-review 两轴均未拦截。
2. **composer 双蓝框**：14 号票的外框聚焦高亮叠上了 mygo `TextArea` 组件自带焦点环（`Element.FocusRing`），聚焦态双重描边。
3. **composerBar 右侧空态渲染不干净**：无模型时画「蓝点 + 空文本」成孤点；从未产生速度时画「—」占位，出现「○ 0% — ●」式无意义组合。

## 改动

- **浮层锚定化**：扩展 `placePanel` 支持锚定矩形落位（触发控件 `Bounds()` 传入，面板贴 composer 上方 / 触发点就近弹出，屏幕边缘翻转沿用 DialogBase 遮罩层定位机制）；composer 四浮层的 backdrop 改为透明（不压暗窗口，点击 backdrop 关闭语义保留）。先更新 `docs/native-ui/spec.md` 的浮层观感小节再实现（AGENTS.md 第 11 节）。
- **去双框**：`internal/ui/chat/composer.go` 的 `TextArea` 加 `.FocusRing(false)`，聚焦表达完全交给外框描边。
- **空态收口**：`modelLabel()` 为空时不渲染模型名的蓝点行（或以灰字占位，对照 React 版空态文案）；`speedLabel` 在从未有速度时省略「—」。口径以 React 版 `Composer.tsx` 为准（git 历史 `9ab0d16^:frontend/src/features/chat/Composer.tsx`）。

## 验收

- 四浮层弹出位置贴触发控件（上缘距 composer 顶一段 gap），不压暗窗口，Esc / 点外关闭不回退。
- 聚焦 composer 只有外框一层描边。
- 无模型 / 无速度时不出现孤点与「—」。
- `ui.Tester` 既有用例全绿；锚定落位计算抽纯函数单测（多窗口下的实机观感留用户走查）。

## 依赖

11 / 12 / 13 / 14（均已合入 feature/ui-fixups）。

## Comments

- 2026-11-03 开票留档：Flyout（mygo 独立锚定窗口，`Anchor` + `Placement` 自动翻转）是最贴近 React Popover 的形态，但其独立窗口的跨窗状态同步（选模型 / 切档后主窗口刷新、审计 toast）与 Tester 多窗口支持不确定，本轮先做「锚定落位 + 透明遮罩」（单窗口、观感接近、改动稳）；Flyout 形态作为后续可选升级留档。
- 2026-11-03 定性：mygo `Element.FocusRing(show bool)` 存在（api_gen.go:4050），TextArea 双框可直接关。
- 2026-11-04 实现留档（三处全落地，验证全绿）：
  - **锚定落位的机制选型**：面板高度在打开首帧未知（`Bounds()` 是上一帧的，首帧为空），若直接用「计算原点 + 上边距」落位，估算误差会变成面板位移（第二帧实测高收敛时跳一下）。改用「底边距钉锚点」表达贴上方分支——`backdrop.Justify(End)` + `panel.Margin(bottom = 窗高 - 锚点上缘 + gap)`，面板底边由布局系统钉在锚点上方，**高度自适应不参与计算，首帧即精确贴附**；估算高（`theme.ComposerFlyoutEstH = 300`）只用于极端小窗的上下翻转决策与钳制，正常窗口触发控件贴底、上方恒充足，误差不产生位移。第二帧起用实测高（`panelH` 逐帧收敛）修正翻转决策。
  - **纯函数**：`placeAnchored(anchor, panelW, panelH, winW, winH, gap, margin) → anchoredPlace{X, Y, Flipped, Clamped}`（`internal/ui/overlays/anchor.go`），决策树全分支表驱动单测（默认贴上、水平钳制、翻转下方、两侧不足钳边、面板超窗）；落位表达在 `placePanelAnchored`（widgets.go），两者以 `anchoredPlace` 分界。
  - **透明 backdrop**：composer 分支 `backdrop.Background(mygo.Transparent)`，`styleBackdrop` 不动（其他浮层不受影响）；点击 backdrop 关闭由 DialogBase 的 backdrop `Clicked()` 承接，Esc 语义不变，无需第二套退出链。
  - **空态口径对照 React 版的取舍**：模型名按钮无模型时显示灰字「选择模型」+ 不画圆点——React 版按可用模型数区分「未配置模型 / 选择模型」，原生侧 chat 包拿不到用户可用模型计数（清单在 overlays 的服务快照里），统一用动作语义占位，完全未配置的指引由 composerFoot 提示行与浮层空态文案承接；上下文占用整块在无会话模型时不渲染（`currentModel ? <ctx-hint> : null` 同口径）；工具条速度位 shown ≤ 0 时整行不画「—」（对 React 的有意收窄），详情浮层内的键值占位「—」保留（键值对齐行里是有意义的空值表达）。
  - **iconTool 签名**：`iconTool` 改返回 `(clicked bool, bounds mygo.Rect)`，info 图标按钮的锚点由此取得；paperclip / image 两处弃收 bounds。
  - **坑**：`ui.Tester.Find` 是**精确匹配**（`n.text == s`），`HasText` 才是包含语义——按子串 Find 文本会扑空，断言要用完整文本（风险浮层备注全文匹配才找到）。
- 2026-11-04 验收对照与未尽事项：
  - 四浮层贴触发控件上方弹出（Tester 集成断言：浮层文本矩形整体不越过锚点上缘、第二帧位置稳定）、透明 backdrop、Esc / 点外关闭不回退——已验证；聚焦单描边（FocusRing(false) 是绘制层，Tester 断言不到）与多窗口实机观感**留用户走查**。
  - **透明 backdrop 下的可发现性**：无遮罩后浮层与界面其余部分无视觉隔离，可发现性靠面板自身的强阴影（`ShadowLg`）与触发控件的就近弹出关系；若实机走查觉得弱，后续可给 backdrop 加极淡的着色（如 alpha 0.08）或给触发控件加打开态高亮，改动都收敛在 `drawComposer` 一处。
  - **Flyout 升级路径**：锚定的决策（`placeAnchored`）与表达（`placePanelAnchored`）已分离，升级 mygo Flyout（独立窗口 `Anchor` + `Placement` 自动翻转）时替换面只在 `drawComposer` 的承载层，纯函数与 Open 快照语义可整体复用；前置条件仍是 Flyout 跨窗状态同步与 Tester 多窗口支持的确认。
