// placeAnchored 纯函数走查（15 号票）：composer 四浮层贴触发控件落位的
// 翻转与钳制。纯计算不依赖 view 帧，表驱动覆盖决策树全部分支。

package overlays

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// anchorCase 是一例落位输入与期望输出。
type anchorCase struct {
	name string
	// 输入：锚点矩形、面板尺寸、窗口尺寸。
	anchor         mygo.Rect
	panelW, panelH float32
	winW, winH     float32
	// 期望：面板原点与决策标记。
	wantX, wantY float32
	wantFlipped  bool
	wantClamped  bool
}

// runAnchorCases 跑表驱动用例，逐字段对照。
func runAnchorCases(t *testing.T, cases []anchorCase) {
	t.Helper()
	const gap = theme.ComposerFlyoutGap           // 8
	const margin = theme.ComposerFlyoutEdgeMargin // 8
	for _, tc := range cases {
		got := placeAnchored(tc.anchor, tc.panelW, tc.panelH, tc.winW, tc.winH, gap, margin)
		if got.X != tc.wantX || got.Y != tc.wantY {
			t.Errorf("%s: origin = (%v, %v), want (%v, %v)", tc.name, got.X, got.Y, tc.wantX, tc.wantY)
		}
		if got.Flipped != tc.wantFlipped {
			t.Errorf("%s: Flipped = %v, want %v", tc.name, got.Flipped, tc.wantFlipped)
		}
		if got.Clamped != tc.wantClamped {
			t.Errorf("%s: Clamped = %v, want %v", tc.name, got.Clamped, tc.wantClamped)
		}
	}
}

// TestPlaceAnchoredAboveDefault 默认贴锚点上方：面板底边 = 锚点上缘 - gap，
// 水平与锚点中心对齐。触发控件在输入区工具条（窗口底部），这是常态分支。
func TestPlaceAnchoredAboveDefault(t *testing.T) {
	runAnchorCases(t, []anchorCase{{
		name:   "窗口中部触发，上方空间充足",
		anchor: mygo.Rect{X: 600, Y: 700, W: 100, H: 28},
		panelW: 420, panelH: 320,
		winW: 1280, winH: 800,
		wantX: 600 + 50 - 210, // 锚点中心 650 - 面板半宽
		wantY: 700 - 8 - 320,  // 锚点上缘 - gap - 面板高
	}})
}

// TestPlaceAnchoredHorizontalClamp 水平钳制：锚点贴窗口左右边缘时面板
// 收进 [margin, winW-margin-panelW]，不再以锚点为中心。
func TestPlaceAnchoredHorizontalClamp(t *testing.T) {
	runAnchorCases(t, []anchorCase{{
		name:   "锚点偏左，面板左缘越过左边距",
		anchor: mygo.Rect{X: 100, Y: 400, W: 80, H: 28},
		panelW: 420, panelH: 320,
		winW: 1280, winH: 800,
		wantX: 8, // 140-210=-70 → 钳到 margin
		wantY: 400 - 8 - 320,
	}, {
		name:   "锚点偏右，面板右缘越过右边距",
		anchor: mygo.Rect{X: 1100, Y: 400, W: 80, H: 28},
		panelW: 420, panelH: 320,
		winW: 1280, winH: 800,
		wantX: 1280 - 8 - 420, // 852
		wantY: 400 - 8 - 320,
	}})
}

// TestPlaceAnchoredFlipBelow 上方空间不足、下方充足时翻到锚点下方：
// 面板顶边 = 锚点下缘 + gap。
func TestPlaceAnchoredFlipBelow(t *testing.T) {
	runAnchorCases(t, []anchorCase{{
		name:   "触发控件在窗口顶部，面板翻到下方",
		anchor: mygo.Rect{X: 600, Y: 40, W: 100, H: 28},
		panelW: 420, panelH: 320,
		winW: 1280, winH: 800,
		wantX:       600 + 50 - 210,
		wantY:       40 + 28 + 8, // 锚点下缘 + gap
		wantFlipped: true,
	}})
}

// TestPlaceAnchoredClampEdges 两侧都放不下时钳制到窗口边缘（留 margin）；
// 锚点越过窗口边缘（越界值）同样被拉回。
func TestPlaceAnchoredClampEdges(t *testing.T) {
	runAnchorCases(t, []anchorCase{{
		name:   "窗口过矮，上方优先但整体钳到上边距",
		anchor: mygo.Rect{X: 600, Y: 100, W: 100, H: 28},
		panelW: 420, panelH: 700,
		winW: 1280, winH: 760,
		// 上方可用 100-8-8=84 < 700 → 下方可用 760-8-136=616 > 84，翻到
		// 下方（yBelow=136）；下方也放不下（maxY=760-8-700=52），钳到 52。
		wantX:       600 + 50 - 210,
		wantY:       52,
		wantFlipped: true,
		wantClamped: true,
	}, {
		name:   "锚点越过下缘，钳到下边距",
		anchor: mygo.Rect{X: 600, Y: 900, W: 100, H: 28},
		panelW: 420, panelH: 320,
		winW: 1280, winH: 800,
		// 上方可用 900-8-8=884 足够（锚点越界视为贴下方），yAbove=572 在
		// [8, 800-8-320=472] 之外 → 钳到 472。
		wantX:       600 + 50 - 210,
		wantY:       472,
		wantClamped: true,
	}})
}

// TestPlaceAnchoredPanelWiderThanWindow 面板比可用窗口还宽：水平钳回
// 左边距（宽度上限另由 placePanelAnchored 的 MaxWidth 收紧，这里只保
// 纯函数不越界）。
func TestPlaceAnchoredPanelWiderThanWindow(t *testing.T) {
	runAnchorCases(t, []anchorCase{{
		name:   "面板宽超过窗口",
		anchor: mygo.Rect{X: 300, Y: 400, W: 80, H: 28},
		panelW: 2000, panelH: 200,
		winW: 1280, winH: 800,
		wantX: 8, // maxX = 1280-8-2000 < margin → 取 margin
		wantY: 400 - 8 - 200,
	}})
}
