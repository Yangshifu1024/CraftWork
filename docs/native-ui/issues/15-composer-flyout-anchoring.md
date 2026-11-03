# 15. composer 浮层锚定化与输入区观感修复（11 / 14 号票实机走查返工）

Status: ready-for-agent

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
