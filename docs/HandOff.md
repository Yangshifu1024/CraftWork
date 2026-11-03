# HandOff：CraftWork ⇄ ZCode 观感一比一对齐（2026-11-04）

> 交接文档：写给接手本仓库的下一个 Agent/开发者。目标读者已具备仓库基础认知（先读根目录 `AGENTS.md`）。本文只记「不在其他工件里的会话级上下文」；规格与历史见文内引用，不重复。

## 一句话现状

ZCode 观感一比一对齐进行中（分支 `feature/ui-fixups`，19 个提交未推送）：**亮色主题四大区域（composer / 侧栏 / 消息流 / 空态）已收敛并实机截图验证**；未完成的三件事按优先级——① 模型选择浮层的**真机交互复验**（代码已改完未实机确认，见下）；② **暗色主题**精调（ZCode 默认 Zai Dark）；③ 小项「侧栏审计计数不随档位切换刷新」。

## 用户已拍板的观感决策（不要再改回去）

1. **聊天内容区域恒定 80% 居中、两侧各 10%**（消息列与 composer 同参数）——ZCode 源码的三档断点（672/896/1152，conversationLayout.ts）**只属于 ZCode 自己**，用户明确推翻过一次：我曾按 ZCode 断点改，被要求恢复 80%。别再「对齐源码」改回去。
2. **侧栏行的高亮/选中用模式色**（Soft 底 + Line 描边，Code 蓝 / Work 橙，参考空态建议按钮）——不是 zai 的中性 5% 色块。
3. **空会话引导区域（「模式已就绪」+ 建议句）整体移除**——聊天列空态直接留白。
4. 浮层底部的「所选模型将设为默认…」「档位与设置双向同步」类备注是废话，删（风险等级说明保留，它是内容）。
5. 工具条图标与文字**垂直居中**对齐（zai 的 items-end 在按钮高度不一时中心线错位）。

## 在途：模型选择浮层（最后一个用户报障，代码已改完未实机复验）

用户实机报四项：选择无效 / 弹窗丑 / 搜索无效 / 底部废话。已改（`a90eb69`）：

- **选择语义双写**：点选模型 = 写默认模型（原保存路径）+ `Actions.SetThreadModel` 切换当前会话模型（`service.ThreadService.SetModel` 广播快照）。此前只写默认、当前会话不变 = 「选择无效」的感知来源。
- 浮层 zai 菜单形态：删大标题与搜索 Field 标题、去非激活灰点、删底部备注、菜单 180→200、菜单项文字 SingleLine。
- mygo 主题 `Accent` 中性化（亮 #52525b / 暗 #a1a1aa）：内置控件聚焦描边/勾选不再用品牌蓝。

**未复验**：真机（`go tool mygo dev`，正式数据有 GLM 清单）上——搜索打字过滤、点模型后 composer 按钮变模型名、双写落库。Tester 链路已绿（`toggle_click_test.go` 同期文件与 `composer_test.go`），但**此前 Tester 过而真机挂过**（布局/输入事件路径差异），必须实机复验。用户特别点过「搜索无效」——若仍无效，头号嫌疑是中文输入法组合串未写回 `st.query`（mygo TextInput 的 IME 边界），需实机确认。

## 实机验证的方式（本会话踩实了的）

- **computer-use 技能可用但一次性**：调过 `agent.computerUse.stop()` 后状态持久、本会话无法复活（报 "Computer control was stopped"）——**新会话直接可用**，用户点名优先用它。
- **System Events 辅助功能权限已授予**（备援）：`click at {x,y}` / `set frontmost of process` / AX 元素定位点击（`entire contents of window 1` 过滤 `button whose name is ...`）都实测可用。坑：① **ZCode 窗口会覆盖并抢前台**（用户的 ZCode 活跃会话），坐标点击会落到 ZCode 上——先 `set frontmost of process "CraftWork Dev" to true`，用 AX 元素点击而非坐标；② 权限偶发 -25211 拒绝，隔几秒重试即过；③ mygo 折叠按钮此前只有 Tooltip 没有 Label，不进 AX/元素树（既「点不动」又不可定位）——已补 Label，同类控件记得加。
- **别用临时目录冒烟验证交互**（用户明确要求 `go tool mygo dev` 正式数据）；`screencapture -x` 全屏 + sips 裁剪做静态截图，注意窗口 StateKey 记忆的尺寸可能超屏（工具条截不到时先查窗口 bounds）。

## 规格与工具（对齐的依据）

- `cw-parity-notes/`（仓库外）：ZCode 源码提取的规格 `00-design-tokens.md`~`07-topbar-titlebar.md`（每条带 ZCode 文件:行号）、`SUMMARY.md`、`WORKFLOW.md`（验收口径与截图流程）、`tools/pixdiff`（像素 diff，已自测）。**关键事实**：ZCode 默认主题是 Zai Dark（纯黑/纯白反色、聚焦只加深不变色、深色无 composer 阴影）；字号 text-ui-* 七档基准 14px；圆角三族 8/12/16。
- 本会话新增 theme token：`Brand/OnBrand/ComposerBorder/Focus/Shadow`、`UserBubble`、`AccentMuted`、`ComposerMaxWidth` 已删（80% 方案不需要）。

## 本分支提交与验证基线

`feature/ui-fixups`（未推送）自 `origin/main` 起 19+ 提交：11-14 号票实现与 review 修复 → 15 号票浮层返工（DialogBase 锚定 → 原生 PopoverBase）→ 亮色四区域对齐（上表）→ 模型浮层双写。验证基线：`go test ./...`、`go vet`、`golangci-lint run` 0 issues、`go tool mygo build`。**全部未推送**，用户要求本地验证后再推。

## 已知未尽

- 暗色精调（theme 暗色值是过渡版；Zai Dark 全量表在 `cw-parity-notes/00` 末段与 `styles.css:689-780`）。
- 侧栏「审计」计数在档位切换写审计后不刷新（侧栏计数疑似装配时快照，未查）。
- Zai 语义的「孤儿组不被批量折叠波及」是既有测试锁定的 React 版语义（`model_test.go:183`）——空区点击已单独处理（折叠空态文案），纯孤儿组场景点击仍无反馈，属已知边界。
- mygo `WidthPercent` 首帧求解对构建时序敏感（曾致 composer 与消息列基准不一致）——列宽一律显式计算（上一帧容器实测宽，首帧窗口宽近似）。

## Suggested skills（下一个 Agent 按需调用）

- 实机交互验证：`computer-use:computer-use`（新会话直接可用；备援 System Events）
- 继续票流程 / 开暗色票：`implement-spec`（issue tracker 在 `docs/native-ui/issues/`，15 号票已 resolved）
- 全量再审一轮：`code-review`（本次对齐改动量大，推送前建议再审）
- 界面观感变更：先改 `docs/native-ui/spec.md` 再动手（AGENTS.md 第 11 节）
