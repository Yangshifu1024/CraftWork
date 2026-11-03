// 本文件是 13 号票的会话标题跑马灯：相位纯函数（marqueePhase /
// marqueeOffset / marqueePeriod，独立单测）与 view 接线（threadTitle）。
//
// 旧版 frontend 没有跑马灯实现（全历史检索确认），本实现按票面要求用
// mygo 自绘能力搭建：悬停且超宽时用 Element.Loop 驱动横向偏移，ClipX
// 裁剪出滚动的可视窗口；减少动效偏好下不启动（AGENTS.md 6.4 的装饰性
// Loop 门控）。

package sidebar

import (
	"math"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// 跑马灯相位段：头 20% 停在起点、30% 滚出、20% 停在尽头、尾 20% 滚回。
const (
	phaseHoldStart = float32(0.2)
	phaseScrollOut = float32(0.3)
	phaseHoldEnd   = float32(0.2)

	// marqueeBasePeriod 是循环周期的底数（两段停顿的时间预算）。
	marqueeBasePeriod = 4 * time.Second
	// marqueeSpeed 是滚动速度（DIP/s）：周期按溢出宽度线性加长。
	marqueeSpeed = float32(40)
)

// marqueePhase 把跑马灯的循环进度（0..1）映射为行程比例（0..1..0）：
// 停顿段是 [0.5, 0.7)，滚回段 [0.7, 1.0)。
// cycle 越界按取模处理（Loop 的返回值理论在 0..1，这里防御负值与 ≥1）。
func marqueePhase(cycle float32) float32 {
	cycle = cycle - float32(math.Floor(float64(cycle)))
	back := 1 - phaseHoldStart - phaseScrollOut - phaseHoldEnd
	switch {
	case cycle < phaseHoldStart:
		return 0
	case cycle < phaseHoldStart+phaseScrollOut:
		return mygo.EaseInOut((cycle - phaseHoldStart) / phaseScrollOut)
	case cycle < phaseHoldStart+phaseScrollOut+phaseHoldEnd:
		return 1
	default:
		return 1 - mygo.EaseInOut((cycle-phaseHoldStart-phaseScrollOut-phaseHoldEnd)/back)
	}
}

// marqueeOffset 由循环进度与溢出量给出标题的横向偏移（≤ 0，向左）。
// 溢出 ≤ 0（标题未超宽）恒为 0。
func marqueeOffset(cycle, overflow float32) float32 {
	if overflow <= 0 {
		return 0
	}
	return -overflow * marqueePhase(cycle)
}

// marqueePeriod 给出溢出量对应的循环周期：4s 底数（两段停顿）+ 每 40px
// 溢出加 2s（滚出与滚回各半），长标题滚得慢一些、短溢出滚得快一些。
func marqueePeriod(overflow float32) time.Duration {
	scroll := time.Duration(overflow / marqueeSpeed * 2 * float32(time.Second))
	return marqueeBasePeriod + scroll
}

// threadTitleWidth 是会话行标题列的可用宽度：侧栏 268 - 列表左右 12×2 -
// 行左右 5×2 - 行描边 2。标题列占满行内容宽（13 号票放宽截断：此前标题
// 与时间行被 ButtonBase 的 Row 语义排成一行，只能拿时间行之外的剩余宽）。
func threadTitleWidth() float32 {
	return theme.SidebarWidth - 2*(theme.Space4+4) - 2*(theme.Space1+3) - 2
}

// threadTitle 画会话行标题（.thread-title）：单行截断；悬停且超宽时以
// 跑马灯滚动展示完整标题。Loop 只在悬停帧被调用，移开即停；ReduceMotion
// 下不启动。测量只在悬停帧发生（MeasureText 逐帧 shaping 的成本不出现在
// 静止列表上）。
func threadTitle(c *mygo.Context, v *viewSide, title string, hovered bool) {
	box := mygo.Box(c).Fill().ClipX()
	x := float32(0)
	if hovered && !theme.ReduceMotion(c) {
		if tw, _ := c.MeasureText(0, mygo.Span{Text: title, Size: theme.Fonts.Base()}); tw > threadTitleWidth() {
			overflow := tw - threadTitleWidth()
			cycle := box.Loop("marquee", marqueePeriod(overflow), mygo.Linear)
			x = marqueeOffset(cycle, overflow)
		}
	}
	box.Children(func() {
		mygo.Text(c, title).FontSize(theme.Fonts.Base()).
			TextColor(v.pal.Text).SingleLine().Translate(x, 0)
	})
}
