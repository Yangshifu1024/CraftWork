# 11. composer 功能接线：模型切换 / 审批档位 / 上下文详情三浮层与 R0–R4 提示

Status: resolved

## 背景

迁移时 `internal/ui/chat/composer.go:11` 与 `:176` 注释把三处交互归给 07 号票（「模型切换与审批档位浮层归 07 号票：模型名与档位在这里只读展示」），但 07 号票（resolved）实际范围是设置中心、通用浮层基建、数据面板、命令面板与创建项目，**不含这三处**。结果是掉缝留桩：模型名、档位、上下文占用三个控件可点击但无任何效果，违反 AGENTS.md 6.4「未接线的控件要禁用并标注里程碑」；注释指向的票已关闭，无人接手。

对照旧版形态：React 版最终实现看 git 历史（`git show 9ab0d16^:frontend/src/features/chat/Composer.tsx`，frontend 源码已删但历史在）。

## 改动

- 模型名点击 → 模型选择浮层：用户提供商目录清单、搜索、当前模型勾选态；选择写入默认模型并与设置中心双向同步（`model.Ref` 语义，见 AGENTS.md 第 7 节）。
- 审批档位点击 → 档位切换浮层：分级 / 严格 / 只读 / 自由，与设置双向同步；自由档确认门复用 `overlays` 既有自由确认对话框；切换立即生效并写审计。
- 上下文占用点击 → 详情弹窗：当前会话 token 用量分解（系统提示词 / 历史消息 / 工具结果等，以 React 版最终形态为准）。
- 档位文字旁补 R0–R4 风险说明提示（hover 浮层或 tooltip），内容与 `overlays/settings.go` 的 `riskRows` 同源，不写第二份清单。
- 若任一浮层短期不落地，对应控件必须禁用并标注归属，不留「可点无效果」状态。

## 验收

- 三处点击各弹出对应浮层，Esc / 遮罩关闭，窗口级 Esc 退出链语义不回退（浮层层级走 `DialogBase`/`OverlayShortcut` 既有机制）。
- 档位切换立即生效、设置中心同步反映、`audit_log` 有记录。
- `ui.Tester` 覆盖三浮层的开合与档位同步。

## 依赖

07 的浮层基建（已就位：Store、`Confirm`、`placePanel`）。

## Comments

- 2026-11-03 开票留档（走查定性）：07 号票 Comments 与改动清单均无 composer 三浮层，`composer.go:11` 的归属注释是错误引用。本票是清偿，不是新功能。
- 2026-11-03 实现留档（接手说明）：前两位实现者子代理先后被模型提供商的内容过滤误拦（1301，票内「审批档位 / 风险等级」词汇密集所致，与代码无关），浮层本体与用例的半成品由主会话接手完成。半成品取舍：`overlays/composer.go`（四浮层：模型 / 档位 / 风险 / 上下文）与 `composer_test.go` 结构保留，修复三处——① 测试种子改走 `SaveProvider` 生产写路径（`ProviderService` 构造时读库进内存快照，`List()` 只看快照，`db.SetSetting` 直写会读到空清单）；② 档位行补显式 `Height(48)`（不设高度的面板在 800 高窗口把「自由」行推出可视区，07 号票「显式高度」坑的变体）；③ 用例期望对齐 `formatTokens` 共享口径（≥10k 取整，12.5k→13k）。
- 2026-11-03 实现留档（接线语义）：chat 侧 `composerBar` 三处展示改为可点击（档位「shield+标签」、上下文「圆环+百分比+速度」、模型名），旁附 `info` 图标开 R0–R4 说明；浮层保存走 `Store.save` 与设置中心同一路径（双向同步天然成立）；档位回调 `approvalLevelSynced` 帧内同步输入区展示（`approvalLevelLabelOf` 与 `app.levelLabel` 同表，两处收敛待 review 轮）；上下文详情打开瞬间快照传入（`contextDetail`，与 `contextHint` 同口径：实测优先估算兜底、运行中实时速度）。
- 2026-11-03 未尽事项：设置中心运行中改档位时聊天列展示不同步（`approvalLevel` 只在装配时与本票浮层回调两处更新，既有缺口，归属后续票）；模型选择写的是默认模型（新建会话生效），当前会话模型不突变，与 React 版「设为默认」文案一致。
- 2026-11-03 review 修复留档（R0–R4 入口形态）：票面「hover 浮层或 tooltip」落为点击入口（档位文字旁 `info` 图标），理由：mygo 立即模式下 hover 弹层需要自管悬停计时与移出关闭（进入 / 离开判定、防抖），点击语义稳定，且与模型 / 档位 / 上下文三浮层同样走 DialogBase 模态——Esc / 遮罩关闭由浮层体系统一处理，不另起第二套退出链。
- 2026-11-03 review 修复留档（正文列双份公式）：`chatColumnWidth` 纯函数（测试断言用）与 view 侧 `WidthPercent(theme.ChatColumnPercent)` 布局是同一公式的两处表达，同源性由 `chat/view_test.go` 的几何断言约束——`TestChatColumnWidthFormula` 锁公式取值，`TestChatViewColumnWidthWideAndFloored` 经 `lastColumnW`（messageRow 帧内记录的真实绘制宽）断言 view 真按公式构建，任一侧单独改动都会被另一侧的断言捕获。
- 验证：`go build ./...`、`go test ./...`（26 包）、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app + DMG。
