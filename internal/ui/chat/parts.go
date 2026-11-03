// parts.go 是消息流里的非文本部件完整卡片（frontend/src/features/chat/Parts.tsx
// 的移植）：审批卡（风险条 + R0–R4 徽章 + 三形态预览 + 四档决定）、工具卡
//（状态竖条 + 参数与输出展开）、思考块（三形态）、待办卡、错误卡。
//
// 共同约定（React 版一致）：卡片只读部件数据、不在卡片里保存业务状态；
// 展开态等界面状态记在 Chat 上（按部件 id 键控）；需要写操作的按钮走
// Actions（审批决定、重试、复制），由 client 编排到服务层。
//
// 审批语义（AGENTS.md 6.2 / docs/specs/features.md F-SEC）：按钮组的显隐
// 条件与后端校验一致——永久授权（本会话 / 本工作区）只在 allowPermanent、
// 风险 R1/R2、档位「分级」三条件同时满足时出现；默认焦点在「拒绝」；
// Esc 与回车都按拒绝处理（Esc 在窗口级退出链，见 state.go）。

package chat

import (
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/editor"
	cwui "github.com/yangshifu1024/craftwork/internal/ui"
)

// ---------------------------------------------------------------------------
// 部件分派
// ---------------------------------------------------------------------------

// partCard 画一张非文本部件卡；未知种类不画（与 React 注册表未登记时
// 返回 null 一致）。streaming 表示该部件正在流式输出（思考块用）。
func (s *Chat) partCard(c *mygo.Context, v *viewSide, part Part, streaming bool) {
	switch part.Kind {
	case PartTool:
		s.toolCard(c, v, part)
	case PartApproval:
		s.approvalCard(c, v, part)
	case PartReasoning:
		s.reasoningBlock(c, v, part, streaming)
	case PartTodo:
		todoCard(c, v, part)
	case PartError:
		s.errorCard(c, v, part)
	default:
		partPlaceholder(c, v, part)
	}
}

// partPlaceholder 画一张最小占位卡（图标 + 标题 + 状态色点）。完整卡片
// 未覆盖的种类（草稿、提示）沿用 05 号票的占位形态。
func partPlaceholder(c *mygo.Context, v *viewSide, part Part) {
	icon, title, status := describePart(v, part)
	if title == "" {
		return
	}
	mygo.Row(c).Gap(cwui.Space3).AlignItems(mygo.Center).
		Height(cwui.HeightBtnSmall+2*cwui.Space3).
		Padding(0, cwui.Space4).Radius(cwui.RadiusRow).
		Background(v.pal.BgHover).
		Children(func() {
			cwui.Icon(c, icon).FontSize(13).TextColor(v.pal.Text3)
			mygo.Text(c, title).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2).MaxLines(1).Grow(1)
			mygo.Box(c).Size(6, 6).Radius(cwui.RadiusRound).Background(status)
		})
}

// describePart 把一个部件换算成占位卡的图标、标题与状态色。
func describePart(v *viewSide, part Part) (icon, title string, status mygo.Color) {
	switch part.Kind {
	case PartTool:
		statusColor := v.pal.Text3
		switch strOf(part.Data, "status") {
		case "running":
			statusColor = v.pal.Accent
		case "done":
			statusColor = v.pal.OK
		case "error":
			statusColor = v.pal.Danger
		}
		title = strOr(strOf(part.Data, "summary"), strOf(part.Data, "name"))
		if title == "" {
			title = "工具调用"
		}
		return "terminal", title, statusColor
	case PartApproval:
		statusColor := v.pal.Warn
		switch strOf(part.Data, "status") {
		case "approved":
			statusColor = v.pal.OK
		case "denied", "canceled":
			statusColor = v.pal.Text3
		}
		title = strOr(strOf(part.Data, "title"), "需要你的批准")
		return "shield", title, statusColor
	case PartTodo:
		done, total := todoStats(part)
		title = "待办清单"
		if total > 0 {
			title = "待办清单 " + strconv.Itoa(done) + " / " + strconv.Itoa(total)
		}
		statusColor := v.pal.Accent
		if total > 0 && done == total {
			statusColor = v.pal.OK
		}
		return "list-todo", title, statusColor
	case PartError:
		code := strOf(part.Data, "code")
		title = strOf(part.Data, "message")
		if title == "" {
			title = "出错了"
		}
		if code != "" {
			title = code + " · " + title
		}
		return "alert", title, v.pal.Danger
	case PartReasoning:
		return "sparkles", "思考过程", v.pal.Purple
	case PartDraft:
		title = strOr(strOf(part.Data, "subject"), "邮件草稿")
		return "mail", title, v.pal.Purple
	default:
		return "circle", "", v.pal.Text3
	}
}

// ---------------------------------------------------------------------------
// 卡片共享小件
// ---------------------------------------------------------------------------

// riskBadge 画一枚 R0–R4 风险徽章（.risk-badge）：等宽小字、风险色文字
// 与软底。空等级不画（工具卡的可选徽章）。
func riskBadge(c *mygo.Context, v *viewSide, level string) {
	if level == "" {
		return
	}
	color := riskColor(v.pal, level)
	mygo.Box(c).Height(19).Padding(0, 5).Radius(4).
		Background(color.Alpha(0.13)).Center().
		Children(func() {
			mygo.Text(c, level).Font(cwui.MonoFont).
				FontSize(cwui.Fonts.XS()).TextColor(color).FontWeight(500)
		})
}

// riskColor 把 R0–R4 字符串映射到色板；越界回退 R0 中性色。
func riskColor(p *cwui.Palette, level string) mygo.Color {
	switch level {
	case "R1":
		return p.R1
	case "R2":
		return p.R2
	case "R3":
		return p.R3
	case "R4":
		return p.R4
	default:
		return p.R0
	}
}

// levelBadge 按当前档位画头部的档位徽章（分级不显示；严格用警示色）。
func levelBadge(c *mygo.Context, v *viewSide, level string) {
	if level == "" || level == "graded" {
		return
	}
	fg, bg := v.pal.Text3, v.pal.BgHover
	if level == "strict" {
		fg, bg = v.pal.Warn, v.pal.WarnSoft
	}
	label := map[string]string{"strict": "严格档位", "readonly": "只读档位", "free": "自由档位"}[level]
	if label == "" {
		label = level + "档位"
	}
	mygo.Box(c).Height(16).Padding(0, cwui.Space3).Radius(cwui.RadiusBadge).
		Background(bg).Center().
		Children(func() {
			mygo.Text(c, label).FontSize(cwui.Fonts.XS()).TextColor(fg)
		})
}

// approvalLevelNote 返回按钮组下方的档位说明（ApprovalCard note 同文案）。
func approvalLevelNote(level string) string {
	switch level {
	case "strict":
		return "严格档位：所有写入与执行逐次询问，不提供永久允许"
	case "readonly":
		return "只读档位：写入与执行会被阻止"
	case "free":
		return "自由档位：所有操作自动放行，不再弹出审批"
	default:
		return "默认选中“拒绝”；Esc 拒绝；R3 / R4 不提供永久允许"
	}
}

// cardBtn 画卡片按钮行里的一枚 27px 按钮（.approval-actions .btn），返回
// 是否被点击。变体：plain（面板底描边）/ ghost（无底）/ danger（危险描边）。
func cardBtn(c *mygo.Context, v *viewSide, label, variant string, autoFocus bool) bool {
	btn := mygo.ButtonBase(c).Height(27).Padding(0, 12).Radius(cwui.RadiusBtn)
	switch variant {
	case "ghost":
		btn.Children(func() { mygo.Text(c, label).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2) })
	case "danger":
		btn.BorderWidth(1).BorderColor(v.pal.Danger)
		btn.Children(func() { mygo.Text(c, label).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Danger) })
	default:
		btn.Background(v.pal.BgElevated).BorderWidth(1).BorderColor(v.pal.BorderStrong)
		btn.Children(func() { mygo.Text(c, label).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text) })
	}
	if autoFocus {
		btn = btn.AutoFocus()
	}
	clicked := btn.Clicked()
	if btn.Hovered() && variant != "danger" {
		btn.Background(v.pal.BgHover)
	}
	return clicked
}

// ---------------------------------------------------------------------------
// 审批卡片（核心：语义与 React ApprovalCard 逐项对齐）
// ---------------------------------------------------------------------------

// approvalCard 画一次审批请求。状态全部由 data.status 驱动：
// waiting 显示按钮组；approved / denied / canceled 显示结果条。
func (s *Chat) approvalCard(c *mygo.Context, v *viewSide, part Part) {
	d := part.Data
	status := strOf(d, "status")
	waiting := status == "waiting" || status == ""
	approved := status == "approved"
	denied := status == "denied" || status == "canceled"

	color := riskColor(v.pal, strOf(d, "risk"))

	// 卡片外壳：三态描边（.approval-card 的 is-waiting / is-approved /
	// is-denied）；决定后的卡片整体降低不透明度。
	card := mygo.Column(c).FillWidth().Radius(9).
		Background(v.pal.BgElevated).
		Clip()
	switch {
	case waiting:
		card.BorderWidth(1).BorderColor(v.pal.Warn.Alpha(0.42))
	case approved:
		card.BorderWidth(1).BorderColor(v.pal.OK.Alpha(0.36))
	default:
		card.BorderWidth(1).BorderColor(v.pal.Border)
		card.Opacity(0.82)
	}
	card.Children(func() {
		// 顶部 2px 风险条：风险色 → 透明的横向渐变（.approval-strip）。
		mygo.Box(c).Height(2).FillWidth().
			Gradient(color, color.Alpha(0), 90)
		// 头部：盾牌 + 标题 + 风险徽章 + 工具名 + 档位徽章 + 右侧提示。
		mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(cwui.Space3).
			Padding(10, 12, 8, 12).Children(func() {
			cwui.Icon(c, "shield").FontSize(14).TextColor(v.pal.Warn)
			mygo.Text(c, "需要你的批准").FontSize(cwui.Fonts.Base()).FontWeight(500).TextColor(v.pal.Text)
			riskBadge(c, v, strOf(d, "risk"))
			if tool := strOf(d, "tool"); tool != "" {
				mygo.Text(c, tool).Font(cwui.MonoFont).
					FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text)
			}
			levelBadge(c, v, s.approvalLevelRaw)
			mygo.Spacer(c)
			hint := "已拒绝"
			if waiting {
				hint = "等待决定"
			} else if approved {
				hint = "已批准"
			}
			mygo.Text(c, hint).FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text3)
		})
		// 主体：摘要、目标、事实列表、预览。
		mygo.Column(c).FillWidth().Padding(0, 12, 10, 12).Children(func() {
			summary := strOr(strOf(d, "detail"), strOf(d, "title"))
			if summary != "" {
				mygo.Text(c, summary).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2).FillWidth()
				mygo.Box(c).Height(cwui.Space3)
			}
			if target := strOf(d, "target"); target != "" {
				mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(6).
					Padding(6, 8).Radius(5).
					Background(v.pal.BgSunken).BorderWidth(1).BorderColor(v.pal.BorderSoft).
					Children(func() {
						cwui.Icon(c, "file").FontSize(12).TextColor(v.pal.Text3)
						mygo.Text(c, target).Font(cwui.MonoFont).
							FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2).MaxLines(1).Grow(1)
					})
				mygo.Box(c).Height(cwui.Space3)
			}
			if facts := approvalFacts(d); len(facts) > 0 {
				mygo.Row(c).FillWidth().AlignItems(mygo.Start).Gap(5).Children(func() {
					for _, f := range facts {
						mygo.Row(c).AlignItems(mygo.Start).Gap(4).
							Padding(2, 6).Radius(4).
							Background(v.pal.BgPanel).BorderWidth(1).BorderColor(v.pal.Border).
							Children(func() {
								cwui.Icon(c, "info").FontSize(11).TextColor(v.pal.Text3)
								mygo.Text(c, f.label+"："+f.value).
									FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text3)
							})
					}
				})
				mygo.Box(c).Height(9)
			}
			s.approvalPreview(c, v, d)
		})
		// 等待决定：按钮组 + 档位说明。拒绝默认聚焦（回车即拒绝）；
		// Esc 的拒绝路径在窗口级退出链（state.go HandleWindowEscape）。
		if waiting {
			permanent := s.permanentAllowed(strOf(d, "risk"), boolOf(d, "allowPermanent"))
			approvalID := strOf(d, "approvalId")
			decide := func(allow bool, scope string) {
				if s.acts != nil && s.acts.Decide != nil {
					s.acts.Decide(approvalID, allow, scope)
				}
			}
			mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(6).
				Padding(9, 12).Background(v.pal.BgSunken.Alpha(0.6)).
				Children(func() {
					if cardBtn(c, v, "拒绝", "danger", true) {
						decide(false, "once")
					}
					mygo.Spacer(c)
					if cardBtn(c, v, "允许一次", "plain", false) {
						decide(true, "once")
					}
					if permanent {
						if cardBtn(c, v, "本会话允许", "ghost", false) {
							decide(true, "session")
						}
						if cardBtn(c, v, "本工作区始终允许", "ghost", false) {
							decide(true, "workspace")
						}
					}
				})
			mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(4).
				Padding(0, 12, 8, 12).Children(func() {
				cwui.Icon(c, "info").FontSize(10).TextColor(v.pal.Text4)
				mygo.Text(c, approvalLevelNote(s.approvalLevelRaw)).
					FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text4)
			})
		}
		// 决定后的结果条：授权范围与审计提示（approved）或未执行提示。
		if approved {
			mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(6).
				Padding(8, 12).Background(v.pal.OKSoft).
				Children(func() {
					cwui.Icon(c, "check-circle").FontSize(13).TextColor(v.pal.OK)
					mygo.Text(c, "已允许"+scopeLabel(strOf(d, "scope"))).
						FontSize(cwui.Fonts.SM()).FontWeight(500).TextColor(v.pal.OK)
					mygo.Text(c, "· 已写入审计").FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text3)
				})
		} else if denied {
			mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(6).
				Padding(8, 12).Background(v.pal.BgHover).
				Children(func() {
					cwui.Icon(c, "x-circle").FontSize(13).TextColor(v.pal.Text3)
					label := "已拒绝"
					if status == "canceled" {
						label = "已取消"
					}
					mygo.Text(c, label).FontSize(cwui.Fonts.SM()).FontWeight(500).TextColor(v.pal.Text3)
					mygo.Text(c, "· 未执行任何修改").FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text3)
				})
		}
	})
}

// scopeLabel 把授权范围换算成结果条文案（React ApprovalCard 同表）。
func scopeLabel(scope string) string {
	switch scope {
	case "session":
		return "（本会话）"
	case "workspace":
		return "（本工作区）"
	default:
		return "一次"
	}
}

// permanentAllowed 报告是否显示「本会话 / 本工作区」按钮。三个条件与
// 服务端校验一致：allowPermanent 为真、风险不超过 R2、当前分级档位
// （受限工作区由后端把 allowPermanent 压成 false，界面无需另行判断）。
// 档位未知（单测未同步）按分级处理，与服务端默认档一致。
func (s *Chat) permanentAllowed(risk string, allowPermanent bool) bool {
	if !allowPermanent {
		return false
	}
	if risk != "R1" && risk != "R2" {
		return false
	}
	return s.approvalLevelRaw == "graded" || s.approvalLevelRaw == ""
}

// approvalFact 是审批卡上的一条事实。
type approvalFact struct{ label, value string }

// approvalFacts 取事实列表（事件是 []agent.Fact 结构体或 JSON map 两种形状）。
func approvalFacts(d map[string]any) []approvalFact {
	list, ok := d["facts"].([]any)
	if !ok {
		return nil
	}
	out := make([]approvalFact, 0, len(list))
	for _, item := range list {
		m, ok := item.(map[string]any)
		if !ok {
			continue
		}
		label, value := strOf(m, "label"), strOf(m, "value")
		if label != "" || value != "" {
			out = append(out, approvalFact{label: label, value: value})
		}
	}
	return out
}

// diffPreviewLine 是预览区的一行：种类与文本。
type diffPreviewLine struct {
	kind string // add / del / ctx
	text string
}

// approvalPreview 画三形态预览：fs_edit / doc_edit 的文本差异（internal/
// editor 计算，删 6 行 + 增 6 行）、shell_exec 的命令原文、apply_patch 的
// 补丁前 60 行；fs_edit / doc_edit 带查看完整差异按钮。
func (s *Chat) approvalPreview(c *mygo.Context, v *viewSide, d map[string]any) {
	tool := strOf(d, "tool")
	args, _ := d["args"].(map[string]any)
	if args == nil {
		args = map[string]any{}
	}
	oldText := strOr(strOf(args, "old_string"), strOf(args, "old_text"))
	newText := strOr(strOf(args, "new_string"), strOf(args, "new_text"))
	patchText := strOf(args, "patch")

	var lines []diffPreviewLine
	var head string
	var showFullDiff bool
	switch {
	case (tool == "fs_edit" || tool == "doc_edit") && oldText != "":
		lines, head, showFullDiff = textDiffPreview(oldText, newText), "文本级差异预览", true
	case tool == "apply_patch" && patchText != "":
		lines, head = patchPreview(patchText), "补丁预览"
	case tool == "shell_exec":
		lines = []diffPreviewLine{{kind: "ctx", text: strOf(d, "target")}}
		head = "命令预览"
	default:
		return
	}

	mygo.Column(c).FillWidth().Radius(6).Background(v.pal.BgSunken).
		BorderWidth(1).BorderColor(v.pal.Border).Clip().
		Children(func() {
			mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(6).
				Padding(4, 8).Children(func() {
				cwui.Icon(c, "diff").FontSize(12).TextColor(v.pal.Text3)
				mygo.Text(c, head).FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text3)
				mygo.Spacer(c)
				if showFullDiff {
					if cardBtn(c, v, "查看完整差异", "ghost", false) {
						if s.acts != nil && s.acts.OpenDiff != nil {
							s.acts.OpenDiff(strOf(d, "target"), oldText, newText)
						}
					}
				}
			})
			mygo.Box(c).FillWidth().Height(1).Background(v.pal.BorderSoft)
			mygo.Column(c).FillWidth().Padding(2, 0).Children(func() {
				for _, line := range lines {
					diffLine(c, v, line)
				}
			})
		})
	mygo.Box(c).Height(9)
}

// textDiffPreview 用 internal/editor 计算文本差异，摘取前 6 条删除行与
// 前 6 条新增行（票面「前 6+6 行」）。没有差异或被截断时退回「旧文本前
// 6 行 + 新文本前 6 行」的原始形态，保证预览始终有内容。
func textDiffPreview(oldText, newText string) []diffPreviewLine {
	result := editor.Diff(oldText, newText)
	var dels, adds []diffPreviewLine
	for _, hunk := range result.Hunks {
		for _, line := range hunk.Lines {
			switch line.Kind {
			case "del":
				if len(dels) < 6 {
					dels = append(dels, diffPreviewLine{kind: "del", text: line.Text})
				}
			case "add":
				if len(adds) < 6 {
					adds = append(adds, diffPreviewLine{kind: "add", text: line.Text})
				}
			}
			if len(dels) >= 6 && len(adds) >= 6 {
				return append(dels, adds...)
			}
		}
	}
	if len(dels)+len(adds) > 0 {
		return append(dels, adds...)
	}
	// 退化：两段文本没有行级差异（或超大截断），按原始形态展示。
	return rawPreview(oldText, newText)
}

// rawPreview 是无差异时的退化预览：旧文本前 6 行标删除、新文本前 6 行
// 标新增（React previewLines 同形态）。
func rawPreview(oldText, newText string) []diffPreviewLine {
	var out []diffPreviewLine
	for _, line := range strings.Split(oldText, "\n") {
		if len(out) == 6 {
			break
		}
		out = append(out, diffPreviewLine{kind: "del", text: line})
	}
	for _, line := range strings.Split(newText, "\n") {
		if len(out) >= 12 {
			break
		}
		out = append(out, diffPreviewLine{kind: "add", text: line})
	}
	return out
}

// patchPreview 取补丁前 60 行并按行首字符分级（@@ 上下文、+ 新增、
// - 删除、其余上下文）。
func patchPreview(patch string) []diffPreviewLine {
	raw := strings.Split(patch, "\n")
	if len(raw) > 60 {
		raw = raw[:60]
	}
	out := make([]diffPreviewLine, 0, len(raw))
	for _, line := range raw {
		kind := "ctx"
		switch {
		case strings.HasPrefix(line, "+"):
			kind = "add"
		case strings.HasPrefix(line, "-"):
			kind = "del"
		}
		out = append(out, diffPreviewLine{kind: kind, text: line})
	}
	return out
}

// diffLine 画预览区的一行：符号 + 行文本（等宽 10.5px，行背景按种类）。
func diffLine(c *mygo.Context, v *viewSide, line diffPreviewLine) {
	row := mygo.Row(c).FillWidth().AlignItems(mygo.Start).Gap(8).Padding(0, 8)
	switch line.kind {
	case "add":
		row.Background(v.pal.OK.Alpha(0.09))
	case "del":
		row.Background(v.pal.Danger.Alpha(0.09))
	}
	textColor := v.pal.Text3
	switch line.kind {
	case "add":
		textColor = diffAddText(v.pal)
	case "del":
		textColor = diffDelText(v.pal)
	}
	sign := " "
	switch line.kind {
	case "add":
		sign = "+"
	case "del":
		sign = "−"
	}
	row.Children(func() {
		mygo.Text(c, sign).Font(cwui.MonoFont).Width(8).
			FontSize(10.5).TextColor(signColor(v, line.kind))
		mygo.Text(c, line.text).Font(cwui.MonoFont).
			FontSize(10.5).TextColor(textColor).Grow(1)
	})
}

// signColor 返回行首符号的颜色：增行用 ok、删行用 danger、上下文弱化。
func signColor(v *viewSide, kind string) mygo.Color {
	switch kind {
	case "add":
		return v.pal.OK
	case "del":
		return v.pal.Danger
	default:
		return v.pal.Text4
	}
}

// diffAddText / diffDelText 是增删行文本色（深浅主题各取 base.css 的值）。
func diffAddText(p *cwui.Palette) mygo.Color {
	if p.Dark {
		return mygo.Hex("#a6e3c7")
	}
	return mygo.Hex("#146c43")
}

func diffDelText(p *cwui.Palette) mygo.Color {
	if p.Dark {
		return mygo.Hex("#f0b3b3")
	}
	return mygo.Hex("#a12a2a")
}

// ---------------------------------------------------------------------------
// 工具卡片
// ---------------------------------------------------------------------------

// toolIcon 是后端工具名到图标的映射（Parts.tsx toolIcon 同表）。
func toolIcon(name string) string {
	switch name {
	case "fs_read":
		return "file-text"
	case "fs_list":
		return "folder-open"
	case "fs_glob":
		return "file"
	case "fs_grep":
		return "search"
	case "fs_write":
		return "save"
	case "fs_edit":
		return "edit"
	case "shell_exec":
		return "terminal"
	case "git_status":
		return "git-branch"
	case "todo_write":
		return "list-todo"
	default:
		return "zap"
	}
}

// toolCard 画工具调用卡：状态色竖条、图标、风险徽章、参数前 6 项、耗时、
// 可展开的输出与复制按钮；失败默认展开（展开态记在 Chat 上）。
func (s *Chat) toolCard(c *mygo.Context, v *viewSide, part Part) {
	d := part.Data
	name := strOf(d, "name")
	status := strOf(d, "status")
	open := s.toolOpen(part.ID, status == "error")

	// 状态竖条与状态标签的颜色（.tool-card.status-*）。
	barColor := v.pal.Text4
	statusColor := v.pal.Text4
	statusLabel, statusIcon := "完成", "check"
	switch status {
	case "running":
		barColor, statusColor, statusLabel, statusIcon = v.pal.Accent, v.pal.Accent, "运行中", "loader"
	case "error":
		barColor, statusColor, statusLabel, statusIcon = v.pal.Danger, v.pal.Danger, "失败", "x"
	}

	card := mygo.Column(c).FillWidth().Radius(8).
		Background(v.pal.BgPanel).BorderWidth(1).BorderColor(v.pal.Border).Clip()
	card.Children(func() {
		// 卡片左缘 2px 状态竖条（.tool-card::before 的绝对定位近似）。
		mygo.Box(c).Absolute().Left(0).Top(0).Bottom(0).Width(2).Background(barColor)
		// 头部行（点击切换展开）。
		head := mygo.ButtonBase(c).FillWidth().AlignItems(mygo.Center).Gap(cwui.Space3).
			Padding(8, 10, 8, 12)
		clicked := head.Clicked()
		if head.Hovered() {
			head.Background(v.pal.BgHover)
		}
		head.Children(func() {
			// 22px 圆角方块内的工具图标。
			mygo.Box(c).Size(22, 22).Radius(6).Background(v.pal.BgActive).Center().
				Children(func() {
					cwui.Icon(c, toolIcon(name)).FontSize(12).TextColor(v.pal.Text2)
				})
			mygo.Text(c, name).Font(cwui.MonoFont).
				FontSize(cwui.Fonts.SM()).FontWeight(500).TextColor(v.pal.Text)
			riskBadge(c, v, strOf(d, "risk"))
			if summary := strOf(d, "summary"); summary != "" {
				mygo.Text(c, summary).FontSize(cwui.Fonts.SM()).
					TextColor(v.pal.Text3).MaxLines(1).Grow(1)
			} else {
				mygo.Spacer(c)
			}
			if ms := int64(numOf(d, "durationMs")); ms > 0 {
				mygo.Text(c, strconv.FormatInt(ms, 10)+" ms").
					FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text4)
			}
			mygo.Row(c).Gap(4).AlignItems(mygo.Center).Children(func() {
				icon := cwui.Icon(c, statusIcon).FontSize(12).TextColor(statusColor)
				if status == "running" {
					// 运行中：图标旋转（.tool-status .spin）。
					icon.Rotate(icon.Loop("tool-spin", 900*time.Millisecond, mygo.Linear) * 360)
				}
				mygo.Text(c, statusLabel).FontSize(cwui.Fonts.XS()).
					FontWeight(500).TextColor(statusColor)
			})
			chev := cwui.Icon(c, "chevron-right").FontSize(12).TextColor(v.pal.Text4)
			if open {
				chev.Rotate(90)
			}
		})
		if clicked {
			s.setToolOpen(part.ID, !open)
		}
		// 展开体：参数（前 6 项）、输出、脚注。
		if open {
			mygo.Column(c).FillWidth().Padding(8, 12, 10, 12).
				Background(v.pal.BgSunken).Children(func() {
				if args := toolArgs(d); len(args) > 0 {
					mygo.Row(c).FillWidth().AlignItems(mygo.Start).GapX(14).GapY(4).Children(func() {
						for _, arg := range args {
							mygo.Row(c).AlignItems(mygo.Start).Gap(4).Children(func() {
								mygo.Text(c, arg.key).FontSize(cwui.Fonts.SM()).FontWeight(500).TextColor(v.pal.Text2)
								mygo.Text(c, arg.value).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2)
							})
						}
					})
					mygo.Box(c).Height(7)
				}
				output := strOf(d, "output")
				if output != "" {
					mygo.Box(c).FillWidth().MaxHeight(240).Radius(5).
						Background(v.pal.Bg).BorderWidth(1).BorderColor(v.pal.BorderSoft).
						Padding(7, 9).Clip().Children(func() {
						mygo.Text(c, output).Font(cwui.MonoFont).
							FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2).FillWidth()
					})
					mygo.Box(c).Height(6)
				}
				foot := "执行完成"
				if b, ok := d["isError"].(bool); ok && b {
					foot = "执行失败"
				} else if status == "running" {
					foot = "正在执行"
				}
				mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(cwui.Space3).Children(func() {
					mygo.Text(c, foot).FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text4)
					mygo.Spacer(c)
					btn := mygo.ButtonBase(c).Height(18).Padding(0, 5).Radius(4).
						AlignItems(mygo.Center).Gap(3)
					copyOut := btn.Clicked()
					if btn.Hovered() {
						btn.Background(v.pal.BgHover)
					}
					btn.Children(func() {
						cwui.Icon(c, "copy").FontSize(11).TextColor(v.pal.Text3)
						mygo.Text(c, "复制输出").FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text3)
					})
					if copyOut && s.acts != nil && s.acts.Copy != nil {
						// 剪贴板要经主循环，走 client 的 Copy 动作排队执行。
						s.acts.Copy(output)
					}
				})
			})
		}
	})
}

// toolArg 是参数展示的一项。
type toolArg struct{ key, value string }

// toolArgs 取参数前 6 项。JSON map 无序，按键排序保证展示稳定
// （React 版按 JSON 插入序展示，Go 侧拿不到插入序，取确定序）。
func toolArgs(d map[string]any) []toolArg {
	args, _ := d["args"].(map[string]any)
	if len(args) == 0 {
		return nil
	}
	keys := make([]string, 0, len(args))
	for k := range args {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	if len(keys) > 6 {
		keys = keys[:6]
	}
	out := make([]toolArg, 0, len(keys))
	for _, k := range keys {
		out = append(out, toolArg{key: k, value: formatArgValue(args[k])})
	}
	return out
}

// formatArgValue 把参数值转成可读字符串（Parts.tsx formatArg 同语义）。
func formatArgValue(value any) string {
	if s, ok := value.(string); ok {
		return s
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return fmt.Sprint(value)
	}
	return string(raw)
}

// ---------------------------------------------------------------------------
// 思考块（三形态）
// ---------------------------------------------------------------------------

// reasoningBlock 画思考块。三种形态对齐 ReasoningBlock：
//   - 流式中收起：「正在思考」扫光 + 最新一行摘要；
//   - 流式中展开：「正在思考 · N 秒」，正文跟随最新思考；
//   - 结束：定格「思考过程 · 持续了 N 秒」，点击展开回顾。
//
// 手动展开 / 收起过后（interacted），流式结束不再自动收起。
func (s *Chat) reasoningBlock(c *mygo.Context, v *viewSide, part Part, streaming bool) {
	text := strOf(part.Data, "text")
	if streaming {
		// 流式中的部件记一个本地起点，供展开态的秒数跳动（后端落库的
		// durationMs 要到流式结束才有）。
		s.rememberReasoningStart(part.ID)
	} else {
		// 流式结束：清掉起点。
		s.forgetReasoningStart(part.ID)
	}
	touched := s.reasoningTouch[part.ID]
	open := s.reasoningOpen[part.ID]
	if !touched && !streaming {
		// 流式结束且用户没操作过：回到收起态（React 的 prevStreaming
		// effect 同语义）；操作过则尊重用户选择。
		open = false
	}
	// 展开且流式中：秒数取本地起点（至少 1 秒）；结束取后端落库时长。
	durationMs := int64(numOf(part.Data, "durationMs"))
	headLabel := "思考过程"
	switch {
	case streaming && open:
		headLabel = "正在思考 · " + strconv.Itoa(s.reasoningElapsed(part.ID)) + " 秒"
	case streaming:
		headLabel = "正在思考"
	case durationMs <= 0:
		headLabel = "思考过程"
	default:
		if seconds := (durationMs + 999) / 1000; seconds >= 1 {
			headLabel = "思考过程 · 持续了 " + strconv.FormatInt(seconds, 10) + " 秒"
		} else {
			headLabel = "思考过程 · 持续了几秒"
		}
	}

	block := mygo.Column(c).FillWidth().Radius(8).
		Background(v.pal.BgPanel.Alpha(0.55)).
		BorderWidth(1).BorderColor(v.pal.BorderStrong).Clip()
	// React 用 dashed 描边；mygo 边框无虚线样式，这里保持 1px 实线近似。
	block.Children(func() {
		head := mygo.ButtonBase(c).FillWidth().AlignItems(mygo.Center).Gap(7).
			Padding(7, 10)
		clicked := head.Clicked()
		if head.Hovered() {
			head.Background(v.pal.BgHover)
		}
		head.Children(func() {
			chev := cwui.Icon(c, "chevron-right").FontSize(12).TextColor(v.pal.Text4)
			if open {
				chev.Rotate(90)
			}
			cwui.Icon(c, "sparkles").FontSize(12).TextColor(v.pal.Text3)
			if streaming && !open {
				// 「正在思考」扫光：文字色在弱化与强调色间往复
				//（.r-thinking 的渐变扫光近似）。
				sweep := head.Loop("think-sweep", 2200*time.Millisecond, mygo.Bounce(mygo.EaseInOut))
				mygo.Text(c, headLabel).FontSize(cwui.Fonts.SM()).
					TextColor(v.pal.Text3.Mix(v.pal.Accent, sweep))
			} else {
				mygo.Text(c, headLabel).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text3)
			}
			// 流式收起态：右侧浮现思考文本的最后一个非空行。
			if streaming && !open {
				if summary := lastNonEmptyLine(text); summary != "" {
					mygo.Text(c, "·").FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text4)
					mygo.Text(c, summary).FontSize(cwui.Fonts.XS()).
						TextColor(v.pal.Text4).MaxLines(1).Grow(1)
				}
			}
		})
		if clicked {
			s.setReasoningOpen(part.ID, !open)
		}
		if open {
			mygo.Text(c, text).FontSize(cwui.Fonts.SM()).
				TextColor(v.pal.Text2).FillWidth().
				Padding(2, 12, 10, 12)
		}
	})
}

// lastNonEmptyLine 取文本从后往前第一个非空行（流式摘要，React 同语义）。
func lastNonEmptyLine(text string) string {
	lines := strings.Split(text, "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		if line := strings.TrimSpace(lines[i]); line != "" {
			return line
		}
	}
	return ""
}

// ---------------------------------------------------------------------------
// 待办卡片
// ---------------------------------------------------------------------------

// todoStats 统计待办清单的完成数与总数。
func todoStats(part Part) (done, total int) {
	items, _ := part.Data["items"].([]any)
	total = len(items)
	for _, item := range items {
		if m, ok := item.(map[string]any); ok && strOf(m, "status") == "done" {
			done++
		}
	}
	return done, total
}

// todoCard 画任务清单卡：标题行（图标 + 计数 + 进度条）与清单项
// （完成删除线、进行中高亮）。
func todoCard(c *mygo.Context, v *viewSide, part Part) {
	items, _ := part.Data["items"].([]any)
	done, total := todoStats(part)
	pct := float32(0)
	if total > 0 {
		pct = float32(done) / float32(total)
	}

	card := mygo.Column(c).FillWidth().Radius(8).
		Background(v.pal.BgPanel).BorderWidth(1).BorderColor(v.pal.Border).Clip()
	card.Children(func() {
		// 头部：图标 + 「任务清单」+ 计数 + 64px 进度条。
		mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(7).
			Padding(7, 11).Children(func() {
			cwui.Icon(c, "list-todo").FontSize(13).TextColor(v.acc.Accent)
			mygo.Text(c, "任务清单").FontSize(cwui.Fonts.SM()).
				FontWeight(500).TextColor(v.pal.Text2)
			mygo.Spacer(c)
			mygo.Text(c, strconv.Itoa(done)+" / "+strconv.Itoa(total)).
				Font(cwui.MonoFont).FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text3)
			// 进度条：3px 圆角槽 + ok 色填充。
			mygo.Box(c).Width(64).Height(3).Radius(2).Background(v.pal.BgActive).Clip().
				Children(func() {
					if pct > 0 {
						mygo.Box(c).Width(64 * pct).Height(3).Background(v.pal.OK)
					}
				})
		})
		mygo.Box(c).FillWidth().Height(1).Background(v.pal.BorderSoft)
		mygo.Column(c).FillWidth().Padding(5, 0).Children(func() {
			for _, item := range items {
				m, ok := item.(map[string]any)
				if !ok {
					continue
				}
				status := strOf(m, "status")
				text := strOf(m, "text")
				rowColor := v.pal.Text2
				switch status {
				case "done":
					rowColor = v.pal.Text3
				case "active":
					rowColor = v.pal.Text
				}
				mygo.Row(c).FillWidth().AlignItems(mygo.Start).Gap(cwui.Space3).
					Padding(4, 11).Children(func() {
					// 13px 勾选框：完成填 ok 色打勾，进行中描边用强调色。
					boxColor := v.pal.Text4
					if status == "active" {
						boxColor = v.acc.Accent
					}
					tbox := mygo.Box(c).Size(13, 13).Radius(3.5).Center()
					if status == "done" {
						tbox.Background(v.pal.OK)
					} else {
						tbox.BorderWidth(1.5).BorderColor(boxColor)
					}
					tbox.Children(func() {
						if status == "done" {
							cwui.Icon(c, "check").FontSize(9).TextColor(v.pal.Bg)
						}
					})
					label := mygo.Text(c, text).FontSize(cwui.Fonts.SM()).TextColor(rowColor).Grow(1)
					if status == "done" {
						// 完成项删除线（.todo-item.done .tlabel）。
						label = label.Strikethrough()
					}
					_ = label
					if status == "active" {
						mygo.Text(c, "进行中").FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text4)
					}
				})
			}
		})
	})
}

// ---------------------------------------------------------------------------
// 错误卡片
// ---------------------------------------------------------------------------

// errorHints 是模型错误码到中文建议的映射（Parts.tsx errorHints 同表，
// 9 种错误码）；未登记的码只显示原始信息。
var errorHints = map[string]string{
	"auth":                 "密钥无效或权限不足：请在设置 → 模型 → 提供商中重新保存 API Key，并确认该 Key 允许调用模型。",
	"rate_limit":           "请求被限流或额度不足：请稍后重试，或在设置中切换其他模型。",
	"timeout":              "模型响应超时：可以重试；长对话可先精简历史或新建会话。",
	"context_overflow":     "输入超出模型上下文窗口：请新建会话、精简历史，或切换到上下文更大的模型。",
	"provider_unavailable": "模型服务暂时不可用：请稍后重试，或切换到其他提供商。",
	"invalid_request":      "请求参数不被模型接受：请检查模型名称、端点与工具配置。",
	"interrupted":          "上次运行在应用退出时中断，已保留已产生的消息；可以点击重试继续。",
	"canceled":             "本次生成已取消。",
	"max_iterations":       "模型多轮调用工具达到了本次运行的上限。可以点击重试继续，或在设置 → 工具与权限中调高「单次运行最大往返」。",
}

// errorCard 画错误卡：错误码徽章、原文、对应建议（如有）、重试与复制。
func (s *Chat) errorCard(c *mygo.Context, v *viewSide, part Part) {
	code := strOf(part.Data, "code")
	message := strOf(part.Data, "message")
	hint, hasHint := errorHints[code]

	card := mygo.Column(c).FillWidth().Radius(8).
		Background(v.pal.DangerSoft).
		BorderWidth(1).BorderColor(v.pal.Danger.Alpha(0.36)).Clip()
	card.Children(func() {
		mygo.Row(c).FillWidth().AlignItems(mygo.Center).Gap(7).
			Padding(8, 11).Children(func() {
			cwui.Icon(c, "alert").FontSize(14).TextColor(v.pal.Danger)
			mygo.Text(c, "执行出错").FontSize(cwui.Fonts.SM()).
				FontWeight(500).TextColor(v.pal.Danger)
			mygo.Spacer(c)
			mygo.Box(c).Height(19).Padding(0, 7).Radius(cwui.RadiusBadge).
				Background(v.pal.Danger.Alpha(0.15)).Center().
				Children(func() {
					mygo.Text(c, strOr(code, "error")).
						FontSize(cwui.Fonts.XS()).TextColor(v.pal.Danger)
				})
		})
		if message != "" {
			mygo.Text(c, message).FontSize(cwui.Fonts.SM()).
				TextColor(v.pal.Text2).FillWidth().Padding(0, 11, 9, 11)
		}
		if hasHint {
			mygo.Text(c, hint).FontSize(cwui.Fonts.XS()).
				TextColor(v.pal.Text3).FillWidth().Padding(7, 11, 9, 11)
		}
		mygo.Row(c).FillWidth().Gap(6).Padding(8, 11).Children(func() {
			if cardBtn(c, v, "重试", "plain", false) {
				if s.acts != nil && s.acts.Regenerate != nil {
					s.acts.Regenerate()
				}
			}
			if cardBtn(c, v, "复制错误", "ghost", false) {
				if s.acts != nil && s.acts.Copy != nil {
					s.acts.Copy(message)
				}
			}
		})
	})
}

// attachChip 画一枚附件 chip（.attach-chip：28 高、描边、名称省略、
// 可选尺寸尾巴）。可移除形态由外层包按钮实现（输入区条内），
// 消息内的展示 chip 不可移除。
func attachChip(c *mygo.Context, v *viewSide, name, size string) {
	mygo.Row(c).Gap(cwui.Space3).AlignItems(mygo.Center).
		Height(28).Padding(0, cwui.Space4).Radius(cwui.RadiusRow).
		MaxWidth(240).
		Background(v.pal.BgPanel).
		BorderWidth(1).BorderColor(v.pal.Border).
		Children(func() {
			cwui.Icon(c, "file").FontSize(12).TextColor(v.pal.Text3)
			mygo.Text(c, name).FontSize(cwui.Fonts.SM()).TextColor(v.pal.Text2).MaxLines(1).Grow(1)
			if size != "" {
				mygo.Text(c, size).FontSize(cwui.Fonts.XS()).TextColor(v.pal.Text4)
			}
		})
}
