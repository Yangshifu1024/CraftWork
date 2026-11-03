// 本文件是 package ui 对 theme / icons 两个叶子包的转出口。
//
// 主题 token 与图标登记表下沉为叶子包（internal/ui/theme、internal/ui/icons），
// 让设置中心、浮层（internal/ui/overlays）以及侧栏 / 聊天 / 面板等平级视图包
// 可以直接引用同一套 token，而不经过本包（本包的外壳要 import 这些视图包，
// 反向依赖会成环）。这里用类型别名与常量转发保持既有引用（shell、并行票的
// 视图包）不受迁移影响；新代码建议直接 import 叶子包。
package ui

import (
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/icons"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// 布局尺寸（interface.md：顶行 48、状态条 27、侧栏 268 固定、面板 320–760）。
const (
	TopbarHeight = theme.TopbarHeight
	SidebarWidth = theme.SidebarWidth
	StatusHeight = theme.StatusHeight

	PanelMinWidth  = theme.PanelMinWidth
	PanelMaxWidth  = theme.PanelMaxWidth
	PanelDefWidth  = theme.PanelDefWidth
	DividerWidth   = theme.DividerWidth
	DefWindowWidth = theme.DefWindowWidth

	NarrowWindowWidth = theme.NarrowWindowWidth
)

// 圆角档：按 base.css 里的用途命名。
const (
	RadiusBadge = theme.RadiusBadge
	RadiusChip  = theme.RadiusChip
	RadiusRow   = theme.RadiusRow
	RadiusCtl   = theme.RadiusCtl
	RadiusBtn   = theme.RadiusBtn
	RadiusCard  = theme.RadiusCard
	RadiusPill  = theme.RadiusPill
	RadiusRound = theme.RadiusRound
)

// 控件高度档（base.css 各控件声明的高度）。
const (
	HeightKbd        = theme.HeightKbd
	HeightBadge      = theme.HeightBadge
	HeightMiniBtn    = theme.HeightMiniBtn
	HeightRunPill    = theme.HeightRunPill
	HeightBtnSmall   = theme.HeightBtnSmall
	HeightSideLink   = theme.HeightSideLink
	HeightBtn        = theme.HeightBtn
	HeightInput      = theme.HeightInput
	HeightModeSwitch = theme.HeightModeSwitch
)

// 间距档：base.css 中出现的水平 / 垂直间距值。
const (
	Space1 = theme.Space1
	Space2 = theme.Space2
	Space3 = theme.Space3
	Space4 = theme.Space4
	Space5 = theme.Space5
	Space6 = theme.Space6
	Space7 = theme.Space7
	Space8 = theme.Space8
)

// 动效常量：--dur 与 --ease。
var (
	Duration = theme.Duration
	Ease     = theme.Ease
)

// ReduceMotion 报告系统是否要求减少动效（theme.ReduceMotion 同函数）。
var ReduceMotion = theme.ReduceMotion

// 类型别名：Palette 的方法（Risk、ModeAccent）随别名一起可用。
type (
	Palette    = theme.Palette
	ShadowSpec = theme.ShadowSpec
	AccentSet  = theme.AccentSet
	Mode       = theme.Mode
	FontLadder = theme.FontLadder
)

// 模式与基准字号常量。
const (
	ModeCode     = theme.ModeCode
	ModeWork     = theme.ModeWork
	BaseFontSize = theme.BaseFontSize

	// MonoFont 是等宽字体栈（base.css --font-mono）。
	MonoFont = theme.MonoFont
)

// Fonts 是全局字号档（设置改基准字号后全档跟随；指针共享 theme 包的
// 全局值，SetAppearance 更新后这里同步可见）。
var Fonts = &theme.Fonts

// Apply 把窗口主题切换为与当前外观（系统偏好或用户显式设置）匹配的
// CraftWork 主题，并返回其色板。
func Apply(c *mygo.Context) *Palette { return theme.Apply(c) }

// SetAppearance 应用设置里的外观（system/dark/light）与界面基准字号，
// 下一帧生效。设置保存路径在主线程调用。
func SetAppearance(themeSetting string, fontSize int) { theme.SetAppearance(themeSetting, fontSize) }

// WindowBackgroundHex 返回窗口创建参数用的底色（首帧前的窗口表面）。
func WindowBackgroundHex(themeSetting string) string { return theme.WindowBackgroundHex(themeSetting) }

// Icon 按登记名画一个 lucide 图标（详见 icons 包）。
func Icon(c *mygo.Context, name string) mygo.Element { return icons.Icon(c, name) }
