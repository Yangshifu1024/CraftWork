// 浮层共享的小部件与格式化助手：按钮、徽章、设置行、对话框外壳等。
// 观感对齐 frontend/src/styles/base.css 与 overlay.css；值取自 theme 包的
// token 与 mygo 语义主题，不另设第二套色值。

package overlays

import (
	"fmt"
	"strings"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// pal 返回当前帧的 CraftWork 色板（根 view 每帧已 Apply，这里幂等取回）。
func pal(c *mygo.Context) *theme.Palette { return theme.Apply(c) }

// ---------------------------------------------------------------------------
// 对话框外壳
// ---------------------------------------------------------------------------

// styleBackdrop 是模态遮罩：半透明黑；点击由 DialogBase 处理为取消。
func styleBackdrop(back mygo.Element) {
	back.Background(mygo.RGBA(0, 0, 0, 0.45))
}

// stylePanel 给对话框面板上观感：bg-float 底、强描边与大阴影（overlay.css
// 的 .confirm-dialog / .command-palette / .data-modal 共用形）。
func stylePanel(c *mygo.Context, panel mygo.Element) {
	p := pal(c)
	panel.Radius(12).
		Background(p.BgFloat).
		Border(1, p.BorderStrong).
		Shadow(p.ShadowLg.X, p.ShadowLg.Y, p.ShadowLg.Blur, p.ShadowLg.Spread, p.ShadowLg.Color)
}

// placePanel 把面板定位到窗口上方 fraction 处（水平居中）；fraction 为 0
// 时交给 DialogBase 默认的垂直居中。宽度在小窗口下按窗宽收紧。注意
// AlignSelf 覆盖的是交叉轴（水平）对齐，垂直偏移要改 backdrop 的主轴
// Justify 再用上边距落位。
func placePanel(c *mygo.Context, backdrop, panel mygo.Element, width float32, topFraction float32) {
	w, h := c.Size()
	panel.Width(width).MaxWidth(w - 60)
	if topFraction > 0 {
		backdrop.Justify(mygo.Start).AlignItems(mygo.Center)
		panel.Margin(h*topFraction, 0, 0, 0)
	}
}

// ---------------------------------------------------------------------------
// 按钮（base.css .btn 系）：构建后返回元素，调用方用 .Clicked() / .Disabled()。
// ---------------------------------------------------------------------------

// btnBase 是按钮的公共壳：30 高、8 圆角、内容居中（.btn）。
func btnBase(c *mygo.Context) mygo.Element {
	b := mygo.ButtonBase(c)
	return b.Height(theme.HeightBtn).Padding(0, 12).Radius(theme.RadiusBtn).
		AlignItems(mygo.Center).Justify(mygo.Center).Gap(theme.Space3)
}

// btn 普通按钮：面板底 + 描边。
func btn(c *mygo.Context, label string) mygo.Element {
	b := btnBase(c)
	b.Background(pal(c).BgElevated).Border(1, pal(c).BorderStrong).Children(func() {
		mygo.Text(c, label)
	})
	return b
}

// smallBtn 紧凑按钮（.btn.sm：26 高）。
func smallBtn(c *mygo.Context, label string) mygo.Element {
	b := btn(c, label)
	return b.Height(theme.HeightBtnSmall).Padding(0, 10)
}

// primaryBtn 主按钮：模式强调色填充（.btn.primary）。
func primaryBtn(c *mygo.Context, label string) mygo.Element {
	b := btnBase(c)
	b.Background(pal(c).Accent).Children(func() {
		mygo.Text(c, label).TextColor(pal(c).OnAccent)
	})
	return b
}

// ghostBtn 幽灵按钮：无底无描边（.btn.ghost）。
func ghostBtn(c *mygo.Context, label string) mygo.Element {
	b := btnBase(c)
	b.Children(func() {
		mygo.Text(c, label).TextColor(pal(c).Text2)
	})
	return b
}

// smallGhostBtn 紧凑幽灵按钮。
func smallGhostBtn(c *mygo.Context, label string) mygo.Element {
	b := ghostBtn(c, label)
	return b.Height(theme.HeightBtnSmall).Padding(0, 10)
}

// dangerBtn 危险按钮：危险色文字 + 危险描边（.btn.danger）；filled 为真时
// 用填充式危险色（自由档位确认门专用，对齐 .free-dialog 的确认按钮）。
func dangerBtn(c *mygo.Context, label string, filled bool) mygo.Element {
	b := btnBase(c)
	if filled {
		b.Background(pal(c).Danger).Children(func() {
			mygo.Text(c, label).TextColor(mygo.Hex("#ffffff"))
		})
		return b
	}
	b.Children(func() {
		mygo.Text(c, label).TextColor(pal(c).Danger)
	})
	return b
}

// iconBtn 图标按钮（.icon-btn.sm）：24×24 圆角方块，label 供读屏与单测定位。
func iconBtn(c *mygo.Context, name string, label string) mygo.Element {
	b := mygo.ButtonBase(c)
	return b.Width(24).Height(24).Radius(theme.RadiusCtl).Center().
		Label(label).
		Children(func() {
			icons.Icon(c, name).Size(14, 14).TextColor(pal(c).Text2)
		})
}

// actionsRow 是对话框底部右对齐的按钮行（.confirm-actions）。
func actionsRow(c *mygo.Context, build func()) {
	mygo.Row(c).Fill().Justify(mygo.End).Gap(7).Margin(15, 0, 0, 0).Children(build)
}

// ---------------------------------------------------------------------------
// 设置中心小部件
// ---------------------------------------------------------------------------

// settingsSection 是设置分组的通用外壳（.settings-section）：标题 + 说明 + 内容。
func settingsSection(c *mygo.Context, title, desc string, build func()) {
	mygo.Column(c).Fill().Gap(theme.Space4).Children(func() {
		mygo.Text(c, title).FontSize(theme.Fonts.XL()).Bold()
		if desc != "" {
			mygo.Text(c, desc).FontSize(theme.Fonts.SM()).TextColor(pal(c).Text3)
		}
		build()
	})
}

// setRow 是单条设置项（.set-row）：左标签 + 说明，右侧控件。
func setRow(c *mygo.Context, label, desc string, control func()) {
	mygo.Row(c).Fill().AlignItems(mygo.Center).Gap(theme.Space7).
		Padding(theme.Space4, 0).BorderWidth(0, 0, 1, 0).BorderColor(pal(c).BorderSoft).
		Children(func() {
			mygo.Column(c).Grow(1).Gap(theme.Space1).Children(func() {
				mygo.Text(c, label)
				if desc != "" {
					mygo.Text(c, desc).FontSize(theme.Fonts.SM()).TextColor(pal(c).Text3)
				}
			})
			mygo.Box(c).Children(control)
		})
}

// badge 语气徽章（.badge）：soft 底 + 同色文字。
func badge(c *mygo.Context, text string, color mygo.Color, soft mygo.Color) mygo.Element {
	return mygo.Box(c).
		Padding(1, 7).Radius(theme.RadiusBadge).
		Background(soft).Children(func() {
		mygo.Text(c, text).FontSize(theme.Fonts.XS()).TextColor(color)
	})
}

// kbd 键位提示（.kbd）。
func kbd(c *mygo.Context, text string) mygo.Element {
	return mygo.Box(c).
		Padding(0, 5).Height(theme.HeightKbd).Radius(theme.RadiusBadge).
		Background(pal(c).BgHover).BorderWidth(1).BorderColor(pal(c).Border).Children(func() {
		mygo.Text(c, text).FontSize(theme.Fonts.XS()).TextColor(pal(c).Text3)
	})
}

// mutedText 弱化说明文字（.sec-desc）。
func mutedText(c *mygo.Context, text string) mygo.Element {
	return mygo.Text(c, text).FontSize(theme.Fonts.SM()).TextColor(pal(c).Text3)
}

// emptyState 是空态占位（.panel-empty）：图标 + 主句 + 提示。
func emptyState(c *mygo.Context, icon, title, hint string) {
	mygo.Column(c).Fill().Center().Gap(theme.Space2).Padding(theme.Space8, 0).Children(func() {
		icons.Icon(c, icon).Size(20, 20).TextColor(pal(c).Text4)
		mygo.Text(c, title).TextColor(pal(c).Text2)
		if hint != "" {
			mygo.Text(c, hint).FontSize(theme.Fonts.SM()).TextColor(pal(c).Text4)
		}
	})
}

// enumSelect 是带展示文案的下拉（ui.SelectBase 泛型）：options 里 value 与
// label 分离，选中值写回 *selected；Changed 报告新选择。
func enumSelect[T comparable](c *mygo.Context, selected *T, width float32, options []selectOption[T], onChanged func()) mygo.Element {
	p := pal(c)
	sel := mygo.SelectBase(c, selected)
	sel.Trigger.Width(width).Height(theme.HeightInput).Padding(0, 10).
		Justify(mygo.SpaceBetween).AlignItems(mygo.Center).
		Background(p.BgInput).Border(1, p.Border).Radius(theme.RadiusBtn).
		Children(func() {
			mygo.Text(c, labelOf(selected, options)).SingleLine()
			icons.Icon(c, "chevron-down").Size(13, 13).TextColor(p.Text3)
		})
	sel.Popup(func(panel mygo.Element) {
		panel.MinWidth(width).Padding(4).Radius(theme.RadiusBtn).
			Background(p.BgFloat).Border(1, p.BorderStrong).
			Shadow(p.ShadowPop.X, p.ShadowPop.Y, p.ShadowPop.Blur, p.ShadowPop.Spread, p.ShadowPop.Color)
		for _, opt := range options {
			item := sel.Item(opt.Value).Fill().Padding(5, 10).Radius(theme.RadiusRow)
			if item.Highlighted() {
				item.Background(p.Accent)
			}
			item.Children(func() {
				mygo.Text(c, opt.Label).SingleLine()
			})
		}
	})
	if sel.Trigger.Changed() && onChanged != nil {
		onChanged()
	}
	return sel.Trigger
}

// selectOption 是下拉的一个选项：value 写回状态，label 用于展示。
type selectOption[T comparable] struct {
	Value T
	Label string
}

// labelOf 返回当前选中值的展示文案；值不在选项里时显示原值。
func labelOf[T comparable](selected *T, options []selectOption[T]) string {
	if selected == nil {
		return ""
	}
	for _, opt := range options {
		if opt.Value == *selected {
			return opt.Label
		}
	}
	return fmt.Sprint(*selected)
}

// ---------------------------------------------------------------------------
// 格式化助手（移植 frontend/src/lib/utils.ts，语义逐行对齐）
// ---------------------------------------------------------------------------

// formatTime 把毫秒时间戳格式化为「HH:MM」或「MM/DD HH:MM」（跨天带日期）。
func formatTime(ts int64) string {
	if ts == 0 {
		return ""
	}
	t := time.UnixMilli(ts)
	now := time.Now()
	sameDay := t.Year() == now.Year() && t.Month() == now.Month() && t.Day() == now.Day()
	clock := fmt.Sprintf("%02d:%02d", t.Hour(), t.Minute())
	if sameDay {
		return clock
	}
	return fmt.Sprintf("%02d/%02d %s", int(t.Month()), t.Day(), clock)
}

// formatTokens 紧凑展示 token 数：<1000 原样、<10000 一位小数 k、更大取整 k。
func formatTokens(v int64) string {
	switch {
	case v == 0:
		return "0"
	case v < 1000:
		return fmt.Sprint(v)
	case v < 10000:
		return fmt.Sprintf("%.1fk", float64(v)/1000)
	default:
		return fmt.Sprintf("%dk", (v+500)/1000)
	}
}

// formatCost 把微美元费用格式化为美元；小于 1 美分保留四位小数。
func formatCost(micros int64) string {
	if micros == 0 {
		return "$0.00"
	}
	v := float64(micros) / 1e6
	if v < 0.01 {
		return fmt.Sprintf("$%.4f", v)
	}
	return fmt.Sprintf("$%.2f", v)
}

// formatBytes 按 1024 进制换算为 B / KB / MB / GB。
func formatBytes(v int64) string {
	if v == 0 {
		return "0 B"
	}
	units := []string{"B", "KB", "MB", "GB"}
	size := float64(v)
	i := 0
	for size >= 1024 && i < len(units)-1 {
		size /= 1024
		i++
	}
	if i == 0 {
		return fmt.Sprintf("%.0f %s", size, units[i])
	}
	return fmt.Sprintf("%.1f %s", size, units[i])
}

// pathLeaf 取路径最后一段作为展示名（去掉结尾斜杠）。
func pathLeaf(path string) string {
	trimmed := strings.TrimRight(path, "/")
	if i := strings.LastIndex(trimmed, "/"); i >= 0 {
		trimmed = trimmed[i+1:]
	}
	return trimmed
}
