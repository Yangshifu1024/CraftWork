// anchor.go 是 composer 浮层锚定落位的纯计算（15 号票）：四浮层对齐
// React 版的 Popover 形态，面板贴触发控件就近弹出。这里只做几何决策，
// 不触碰 view；落位如何表达成 backdrop 对齐与面板边距见 composer.go
// 的 placePanelAnchored。观感约定见 docs/native-ui/spec.md 的 composer
// 浮层小节。

package overlays

import (
	mygo "github.com/egoist/mygo/ui"
)

// anchoredPlace 是一次锚定落位的计算结果（窗口 DIP 坐标）。
type anchoredPlace struct {
	// X / Y 是面板原点，已含边缘钳制：Flipped 为假时 Y 依赖估算面板高，
	// 实际落位由 placePanelAnchored 用「底边距钉锚点」机制表达（面板高
	// 度自适应，不产生位移）；其余分支 Y 直接用作上边距。
	X, Y float32
	// Flipped 表示面板翻到了锚点下方（上方空间不足且下方更宽裕）。
	Flipped bool
	// Clamped 表示 Y 落在了边缘钳制位（锚点越界或上下都放不下）。
	Clamped bool
}

// placeAnchored 计算面板贴锚点的落位：默认面板贴锚点上方（面板底边 =
// 锚点上缘 - gap）；上方空间不足且下方更宽裕时翻到下方（面板顶边 =
// 锚点下缘 + gap）；仍放不下或锚点越界时钳制在 [margin, win-margin-尺寸]
// 内。水平方向与锚点中心对齐，同样钳制。面板尺寸大于窗口可用空间时贴
// 左 / 上边距（宽度另有 MaxWidth 收紧，这里是几何兜底）。
//
// 面板高度在打开第一帧未知（Bounds 是上一帧的），调用方用估算值参与
// 翻转与钳制决策；正常窗口下触发控件在输入区工具条（窗口底部），上方
// 空间总是充足，估算误差不会改变决策。
func placeAnchored(anchor mygo.Rect, panelW, panelH, winW, winH, gap, margin float32) anchoredPlace {
	// 水平：锚点中心对齐，钳进窗口边缘。
	x := anchor.X + anchor.W/2 - panelW/2
	minX, maxX := margin, winW-margin-panelW
	if maxX < minX {
		maxX = minX // 面板比可用窗口还宽：贴左边距
	}
	if x < minX {
		x = minX
	}
	if x > maxX {
		x = maxX
	}

	// 垂直：优先上方，上方放不下且下方更宽裕才翻。可用空间都从 margin
	// 起算（贴边的面板保持与窗口边缘的呼吸距离）。
	yAbove := anchor.Y - gap - panelH
	yBelow := anchor.Y + anchor.H + gap
	availAbove := anchor.Y - gap - margin
	availBelow := winH - margin - yBelow
	y, flipped := yAbove, false
	if availAbove < panelH && availBelow > availAbove {
		y, flipped = yBelow, true
	}
	// 钳制：锚点越界或两侧都放不下时拉回窗口边缘。
	minY, maxY := margin, winH-margin-panelH
	if maxY < minY {
		maxY = minY // 面板比可用窗口还高：贴上边距
	}
	clamped := false
	if y < minY {
		y, clamped = minY, true
	}
	if y > maxY {
		y, clamped = maxY, true
	}
	return anchoredPlace{X: x, Y: y, Flipped: flipped, Clamped: clamped}
}
