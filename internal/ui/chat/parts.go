// parts.go 是非文本部件的最小占位卡与附件 chip。
//
// 占位卡约定（05 号票范围）：每个非文本部件在消息流里的位置画
// 「图标 + 标题 + 状态色点」，保证消息流结构完整；工具 / 审批 / 待办 /
// 错误的完整卡片（风险徽章、参数展开、允许/拒绝按钮、进度条、差异预览）
// 归 06 号票实现，届时整卡替换这里的占位。

package chat

import (
	"strconv"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui"
)

// partPlaceholder 在部件位置画一张最小占位卡（图标 + 标题 + 状态色点）。
func partPlaceholder(c *mygo.Context, v *viewSide, part Part) {
	icon, title, status := describePart(v, part)
	if title == "" {
		return
	}
	mygo.Row(c).Gap(ui.Space3).AlignItems(mygo.Center).
		Height(ui.HeightBtnSmall+2*ui.Space3).
		Padding(0, ui.Space4).Radius(ui.RadiusRow).
		Background(v.pal.BgHover).
		Children(func() {
			ui.Icon(c, icon).FontSize(13).TextColor(v.pal.Text3)
			mygo.Text(c, title).FontSize(ui.Fonts.SM()).TextColor(v.pal.Text2).MaxLines(1).Grow(1)
			// 状态色点：占位卡的运行状态指示（完整卡片的状态竖条归 06）。
			mygo.Box(c).Size(6, 6).Radius(ui.RadiusRound).Background(status)
		})
}

// describePart 把一个部件换算成占位卡的图标、标题与状态色。
// 取值尽量短：模型名 / 审批标题 / 错误码，超出由 MaxLines 截断。
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
		// 待办：标题带完成计数（items[].status == "done"）。
		done, total := 0, 0
		if items, ok := part.Data["items"].([]any); ok {
			total = len(items)
			for _, item := range items {
				if m, ok := item.(map[string]any); ok && strOf(m, "status") == "done" {
					done++
				}
			}
		}
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

// attachChip 画一枚附件 chip（.attach-chip：28 高、描边、名称省略、
// 可选尺寸尾巴）。可移除形态由外层包按钮实现（输入区条内），
// 消息内的展示 chip 不可移除。
func attachChip(c *mygo.Context, v *viewSide, name, size string) {
	mygo.Row(c).Gap(ui.Space3).AlignItems(mygo.Center).
		Height(28).Padding(0, ui.Space4).Radius(ui.RadiusRow).
		MaxWidth(240).
		Background(v.pal.BgPanel).
		BorderWidth(1).BorderColor(v.pal.Border).
		Children(func() {
			ui.Icon(c, "file").FontSize(12).TextColor(v.pal.Text3)
			mygo.Text(c, name).FontSize(ui.Fonts.SM()).TextColor(v.pal.Text2).MaxLines(1).Grow(1)
			if size != "" {
				mygo.Text(c, size).FontSize(ui.Fonts.XS()).TextColor(v.pal.Text4)
			}
		})
}
