# 06. 过程卡片与审批闭环

Status: resolved

## 背景

对齐消息流内的过程部件与审批（[spec.md](../spec.md) P1；审批原则不变，机制从跨 IPC 变为跨 UI 线程）。

## 改动

- 思考块：三形态（流式收起扫光+最新行摘要、流式展开吸底+秒数、结束定格耗时）、手动展开不自动收起。
- 工具卡片：状态色竖条、图标、风险徽章、参数前 6 项、耗时、展开输出/复制、失败默认展开。
- 待办卡片：清单 + 进度条 + `n / m`。
- 错误卡片：9 种错误码→中文建议映射、重试。
- 审批卡片：风险条 + R0–R4 徽章、事实列表、三形态预览（fs_edit/doc_edit 文本 diff 前 6+6 行 / shell 命令 / patch 前 60 行）、「查看完整差异」、四档决定按钮（永久允许仅 R1/R2+分级档）、默认焦点拒绝、Esc=拒绝、决定后结果条+审计提示。
- 「已工作 N」折叠：相邻过程部件分组、运行中平铺、终态收拢+耗时。
- 上下文圆环与实时速度估算（字符/4÷秒，后端实测接管）。

## 验收

- 审批全链路：请求 → 卡片 → 允许/拒绝/永久允许 → 执行/跳过 → 审计落库；取消、关窗按拒绝落审计。
- 受限工作区 R3 直接拒绝、无永久授权项。
- 各卡片在 `ui.Tester` 下的状态机单测。

## 依赖

05。

## Comments

- 2026-11-03 完成于 wt/native-06，四个实现提交（审批卡+卡片+折叠+Esc 链、上下文圆环与速度、附件缩略图与放大预览、接 09 号票差异槽）。`go build ./...`、`go test ./...`（24 包）、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build` 产出 .app+DMG。
- **审批语义对齐**：`internal/ui/chat/parts.go` 逐项移植 ApprovalCard——永久授权按钮的显隐三条件与 `approvals.Resolve` 的服务端校验一致（`allowPermanent` && R1/R2 && 分级档；受限工作区由后端把 allowPermanent 压成 false，界面不另行判断）；结果条 scope 文案、档位徽章（graded 不显示）与档位说明同 React 文案；允许Permanent 事件载荷是 Go 原生 bool（快照 JSON 同样解成 bool），reducer 补 `boolOf` 兼容旧的字符串形态。
- **Esc 链**（AppShell 同序，落在 `Chat.HandleWindowEscape`，经 `Shell.EscapeKey` 由 RootView 在浮层之后接线）：运行中双击 Esc 停止（武装窗口 800ms）；非运行但有待审批 → Esc 拒绝最后一条（范围固定 once）。等待审批时会话仍是运行态（React reducer 同语义），所以审批等待中 Esc 先走双击停止——后端取消 run 会把审批按 canceled 落审计与审批行；拒绝路径覆盖「空闲但残留待审批」的可恢复状态。对话框打开时 Esc 归浮层（07 体系），走不到窗口链；面板关 tab 层级归 08/10 号票。
- **差异预览**：fs_edit/doc_edit 用 `internal/editor.Diff` 计算行级差异后取前 6 删 + 6 增（票面 6+6）；无行级差异或超大截断时退回 React previewLines 的「旧 6 + 新 6」原始形态。「查看完整差异」接 09 号票 `Pane.OpenDiff(title, old, new)` 差异槽（90ed826），装配按主要目录相对路径解析标题。
- **折叠分组**：`fold.go` 按 partRegistry 的 work 单表语义切段（reasoning/tool/todo/approval/非最终正文为 work；error/draft/callout/notice/最终正文留在外面）；组内仍有运行中工具或等待审批时平铺（GroupedParts 的 running 同语义）。发现并保持两处 React 原样行为：reasoning.completed 事件载荷不带 partId，实时路径不回填思考时长（React partIdOf 同样为空），“持续了 N 秒”在快照加载后显示；取消的审批 resolved 事件只落库不广播，卡片实时停留在等待、切会话回来后显示「已取消 · 未执行任何修改」（与 React 一致）。
- **上下文圆环**：SVG dasharray 语义用 mygo Draw 弧线实现（`Path.Circle` + `Trim` + `Transform(Rotation(-90))`），三档色与 Composer 百分比一致；速度生成中按字符/4÷秒实时估算、空闲回退后端实测。
- **ui.Tester 单测**（`parts_test.go`）：审批按钮显隐矩阵（R0–R4 × allowPermanent × 档位 9 组合）、默认焦点拒绝（回车即拒绝）、Esc 分流（空闲拒绝 / 运行中武装取消）、结果条两态、三形态预览（差异/命令/补丁 60 行截断）、思考三形态与不自动收起、工具失败默认展开与复制、待办/错误卡、折叠平铺/收拢与 formatWorkDuration。修复既有 view 测试对折叠后不可见工具卡的断言。
- **冒烟**（真实链路跑在 ui.Tester，截图 8 张 /tmp/cw-06-shots）：/tmp 假 OpenAI 兼容端点（按轮次回放思考→fs_write→fs_edit+todo_write→正文）驱动真实 store→runner→适配器→审批→审计；点击「允许一次」「本会话允许」走真实 DecideApproval；截图覆盖思考流式、审批等待（四按钮）、差异预览审批、待办/工具卡、完成折叠、展开折叠、取消轮（已停止 + 已取消结果条 + 失败工具卡展开）。**审计落库验证**（sqlite 查 audit_log）：`fs_write|R1|allow|once|用户审批` + `fs_write|R1|allow|once|notes.txt`（执行）、`fs_edit|R1|allow|session|用户审批` + 执行、`todo_write|R0|auto`、取消轮 `fs_write|R1|deny|auto|notes.txt`（取消按拒绝落审计）；approvals 表三行 allow once / allow session / canceled；runs 表 done + canceled。
- **未竟事项**：真窗口 `screencapture` 实拍未做成——本会话屏幕处于锁定状态（osascript 无辅助功能授权是已知限制，锁屏连截屏内容都不可得）；ui.Tester 渲染与真窗口同一 mygo scene 管线且驱动的是真实服务链，视觉证据以 harness 截图为准（harness 留档 /tmp/cw-06-shots/harness-main.go，stub 在 /tmp/cw-06-stub）。解锁后如需窗口实拍，可按 09 号票的 `-tags cwsmoke` 模式给聊天列补一个种子模式。受限工作区 R3 直接拒绝是策略层行为（runner 既有单测覆盖），未在界面冒烟中单独驱动。
- 2026-11-03 合并留档：06 提供的窗口级 Esc 链入口为 `Chat.HandleWindowEscape`（`internal/ui/chat/state.go`，返回是否消费）与 `Shell.EscapeKey` 字段（`internal/ui/shell.go`，RootView 在浮层 Shortcut 之后调用）；**10 号票快捷键全表接线时直接挂这两个入口，不要再另起第二份 Esc 分流逻辑**。
