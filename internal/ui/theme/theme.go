// Package ui 是 CraftWork 的原生界面（mygo 立即模式 view）。
//
// 本文件把 frontend/src/styles/base.css 的 token 体系机械翻译为 Go 常量与
// 结构：色板（bg 8 级 / border 3 / text 4 / 状态 5 色 / R0–R4 / 双模式强调色）、
// 字号 7 档派生（15px 基准）、间距 / 圆角 / 控件高度档、阴影 4 档与缓动常量。
// 深浅两套 mygo 主题由色板派生，经 Apply 在每帧套到窗口上，跟随系统实时切换。
//
// 值的唯一来源是 base.css 与 docs/specs/interface.md；改动观感先改那边。
package theme

import (
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/policy"
)

// ---------------------------------------------------------------------------
// 布局与尺寸档（base.css :root 与 docs/specs/interface.md）
// ---------------------------------------------------------------------------

// 顶行、侧栏与面板的布局尺寸（interface.md：侧栏 268 固定、聊天自适应、
// 面板 320–760、顶行 48px、状态条 27px）。
const (
	TopbarHeight = float32(48)
	SidebarWidth = float32(264)
	StatusHeight = float32(27)

	PanelMinWidth  = float32(320)
	PanelMaxWidth  = float32(760)
	PanelDefWidth  = PanelMinWidth // 面板默认取最小宽度，把空间留给对话区
	DividerWidth   = float32(1)    // ui.Split 分隔线 1 DIP
	DefWindowWidth = float32(1440)
)

// NarrowWindowWidth 是窄窗判定阈值：窗口内容宽度低于它时面板停靠栏自动收起
// （对照规格的窄视口 1100×800）。浮层形态由 08 号票（sidePane 移植）接管。
const NarrowWindowWidth = float32(1100)

// 设置中心全屏覆盖层的布局尺寸（overlay.css 全屏形态：导航列 240px，
// 内容列居中限宽 880px，避免设置行在宽窗口里被拉散）。
const (
	SettingsNavWidth        = float32(240)
	SettingsContentMaxWidth = float32(880)
)

// 输入区浮层（11 号票：模型选择 / 审批档位 / 上下文详情 / R0–R4 说明）
// 的布局尺寸。行高取显式值的原因见 drawComposerModels / drawComposerApproval
// 的注释（Tester 800 高窗口里行必须全部落在 placePanel 的可视区内）。
const (
	ComposerApprovalWidth  = float32(360) // 审批档位浮层宽（四浮层的默认宽）
	ComposerModelWidth     = float32(420) // 模型选择浮层宽
	ComposerRiskWidth      = float32(400) // 风险等级说明浮层宽
	ComposerContextWidth   = float32(320) // 上下文详情浮层宽
	ComposerListHeight     = float32(300) // 模型清单高度（固定，限制浮层尺寸）
	ComposerModelRowHeight = float32(44)  // 模型行高
	ComposerLevelRowHeight = float32(48)  // 档位行高

	ComposerPanelPadT = float32(14) // 浮层面板上内边距
	ComposerPanelPadX = float32(16) // 浮层面板左右内边距
	ComposerPanelPadB = float32(12) // 浮层面板下内边距

	// 锚定落位（15 号票：四浮层贴触发控件弹出，观感约定见
	// docs/native-ui/spec.md 的 composer 浮层小节）。
	ComposerFlyoutGap = float32(8) // 面板与触发控件上缘的间距
	// ComposerFlyoutEdgeMargin 是落位钳制的窗口边缘留白：面板不会贴到
	// 窗口边框上，上下左右统一留它。
	ComposerFlyoutEdgeMargin = float32(8)
	// ComposerFlyoutEstH 是浮层面板高度的估算值：打开第一帧面板还没有
	// 实测高（Bounds 是上一帧的），翻转与钳制决策先用它；落位本身用
	// 「底边距钉锚点」机制表达，估算只影响极端小窗下的上下翻转判断，
	// 误差不产生面板位移。值取四浮层实测高的中间量（模型浮层最高约
	// 400，上下文浮层最矮约 200）。
	ComposerFlyoutEstH = float32(300)
)

// 聊天正文列（14 号票：消息流与 composer 外框共用同一条居中限宽的
// 正文列；观感约定见 docs/native-ui/spec.md）。
const (
	// ChatColumnMarginPercent 是正文列两侧留白占所在列宽的百分比（用户
	// 给定参数 10）。
	ChatColumnMarginPercent = float32(10)
	// ChatColumnPercent 是正文列宽度占所在列的百分比，由两侧 margin 推导。
	ChatColumnPercent = 100 - 2*ChatColumnMarginPercent
	// ChatColumnMinWidth 是正文列的最小保底宽度：mygo 的 Markdown 在收缩
	// 布局里会塌成零宽（AGENTS.md 第 7 节），正文列必须以显式宽度构建
	//（WidthPercent + MinWidth），极窄列下由它保底可读；超宽的部分被
	// 消息流 wrapper 裁剪（用户极端拖拽面板才会到达）。
	ChatColumnMinWidth = float32(320)
)

// 窗口标题栏与材质（14 号票：复刻 ZCode 的无标题条形态，观感约定见
// docs/native-ui/spec.md）。
const (
	// SidebarTopbarHeight 是侧栏顶部拖拽行的高度：红绿灯落在
	// {x:22, y:23}（ZCode 基准），按钮下缘约 37，48px 行容纳它们且不遮
	// 下方内容；整行 DragWindow，双击按系统设置缩放。
	SidebarTopbarHeight = TopbarHeight
	// TrafficLightX / TrafficLightY 是 macOS 红绿灯的窗口选项位置
	//（ZCode MACOS_TRAFFIC_LIGHT_BASE_POSITION），app.go 建窗时取用。
	TrafficLightX = 22
	TrafficLightY = 23
)

// 圆角档：按 base.css 里的用途命名（控件默认 8，与 ui.Theme.Radius 一致）。
const (
	RadiusBadge = float32(4) // 徽章、kbd、风险徽章、计数
	RadiusChip  = float32(5) // 徽章、迷你按钮、组头加号
	RadiusRow   = float32(6) // 侧栏链接、会话行内元素
	RadiusCtl   = float32(7) // 紧凑按钮、输入框、会话项
	RadiusBtn   = float32(8) // 常规按钮与 icon 按钮（ui.Theme.Radius 同值）
	RadiusCard  = float32(9) // 工作区卡片
	RadiusPill  = float32(10)
	RadiusRound = float32(999) // 圆形（头像、圆点）
)

// 控件高度档（base.css 各控件声明的高度）。
const (
	HeightKbd        = float32(19)
	HeightBadge      = float32(20)
	HeightMiniBtn    = float32(22)
	HeightRunPill    = float32(24)
	HeightBtnSmall   = float32(26)
	HeightSideLink   = float32(28)
	HeightBtn        = float32(30)
	HeightInput      = float32(32)
	HeightModeSwitch = float32(40)
)

// 间距档：base.css 中出现的水平 / 垂直间距值。
const (
	Space1 float32 = 2
	Space2 float32 = 4
	Space3 float32 = 6
	Space4 float32 = 8
	Space5 float32 = 10
	Space6 float32 = 12
	Space7 float32 = 16
	Space8 float32 = 24
)

// MonoFont 是等宽字体栈（base.css 的 --font-mono）：工具名、目标路径、
// 差异预览、计数等界面里的代码形文本使用。
const MonoFont = "SF Mono, JetBrains Mono, Cascadia Code, Menlo, Consolas, Liberation Mono, monospace"

// 动效常量：--dur 与 --ease（cubic-bezier(0.22, 0.61, 0.36, 1)）。
var (
	Duration = 160 * time.Millisecond
	Ease     = mygo.CubicBezier(0.22, 0.61, 0.36, 1)
)

// ReduceMotion 报告系统是否要求减少动效（macOS「减弱动态效果」、Windows
// 动画效果关闭、GNOME 关闭动画）。读当前 view 帧的 Context（ui.Context.
// Preferences，Tester 下取测试注入值），供 view 代码在帧内判断。
//
// mygo 的 Animate / Transition 已自动跟随该偏好（值与位移直接到达目标），
// Loop 不停（与系统指示器的行为一致）；装饰性的循环动效（扫光、脉冲）
// 没有信息量，由调用方读本函数自行停用。
func ReduceMotion(c *mygo.Context) bool {
	if c == nil {
		return false
	}
	return c.Preferences().ReduceMotion
}

// Now 返回当前毫秒时间戳：界面层所有「现在」的统一口径（会话列表时间
// 列、消息 createdAt、保存 / 应用反馈的时间戳），对齐 React 版的
// Date.now()。此前五个视图包各自捏了 panelClock / nowMillis / storeClock
// 一类的本地别名，语义相同、名字各异，收敛后一律走这里。
func Now() int64 { return time.Now().UnixMilli() }

// ---------------------------------------------------------------------------
// 字号 7 档（派生公式语义保留：设置改基准字号后全档跟随）
// ---------------------------------------------------------------------------

// BaseFontSize 是界面基准字号（base.css 的 --ui-font-size 默认值）。
const BaseFontSize = float32(15)

// FontLadder 按派生公式从基准字号算出 7 档 UI 字号；
// 档位与 ZCode 对齐（正文 15、次要 13、最小 11）。
type FontLadder struct{ Size float32 }

// Title 是设置中心页标题（+10 → 25px）。
func (l FontLadder) Title() float32 { return l.Size + 10 }

// XL 是设置分区标题与 md h1（+4 → 19px）。
func (l FontLadder) XL() float32 { return l.Size + 4 }

// LG 是 md h2 与空态标题（+2 → 17px）。
func (l FontLadder) LG() float32 { return l.Size + 2 }

// Base 是正文、按钮、输入、菜单与 tab（±0 → 15px）。
func (l FontLadder) Base() float32 { return l.Size }

// Caption 是紧凑辅助文案与空态建议按钮（-1 → 14px）。
func (l FontLadder) Caption() float32 { return l.Size - 1 }

// SM 是次要信息、时间戳与帮助（-2 → 13px）。
func (l FontLadder) SM() float32 { return l.Size - 2 }

// XS 是徽章、kbd 与弱元数据（-4 → 11px）。
func (l FontLadder) XS() float32 { return l.Size - 4 }

// Fonts 是当前字号档，按 base.css 默认基准派生。
var Fonts = FontLadder{Size: BaseFontSize}

// ---------------------------------------------------------------------------
// 色板：base.css 的深浅两套 token
// ---------------------------------------------------------------------------

// ShadowSpec 是一条 box-shadow（mygo Shadow 的参数化，CSS 值原样保留）。
type ShadowSpec struct {
	X, Y, Blur, Spread float32
	Color              mygo.Color
}

// AccentSet 是一套模式强调色：主色、软底、描边与其上的文字色。
type AccentSet struct {
	Accent, Soft, Line, On mygo.Color
}

// Mode 是界面模式（base.css 的 data-mode）。
type Mode int

const (
	// ModeCode 是 Code 模式，强调色随主题 accent。
	ModeCode Mode = iota
	// ModeWork 是 Work 模式，强调色为琥珀（与 Code 蓝形成区分）。
	ModeWork
)

// Palette 是一套外观（深或浅）下的全部 base.css token。
// 字段名与 CSS 变量一一对应，便于对照检查。
type Palette struct {
	Bg         mygo.Color // --bg
	BgSunken   mygo.Color // --bg-sunken
	BgPanel    mygo.Color // --bg-panel
	BgElevated mygo.Color // --bg-elevated
	BgFloat    mygo.Color // --bg-float
	BgHover    mygo.Color // --bg-hover
	BgActive   mygo.Color // --bg-active
	BgInput    mygo.Color // --bg-input

	Border       mygo.Color // --border
	BorderSoft   mygo.Color // --border-soft
	BorderStrong mygo.Color // --border-strong

	Text  mygo.Color // --text
	Text2 mygo.Color // --text-2
	Text3 mygo.Color // --text-3
	Text4 mygo.Color // --text-4

	Accent       mygo.Color // --accent
	AccentStrong mygo.Color // --accent-strong
	AccentSoft   mygo.Color // --accent-soft
	AccentLine   mygo.Color // --accent-line
	OnAccent     mygo.Color // --on-accent

	OK         mygo.Color // --ok
	OKSoft     mygo.Color // --ok-soft
	Warn       mygo.Color // --warn
	WarnSoft   mygo.Color // --warn-soft
	Danger     mygo.Color // --danger
	DangerSoft mygo.Color // --danger-soft
	Purple     mygo.Color // --purple
	PurpleSoft mygo.Color // --purple-soft
	Cyan       mygo.Color // --cyan

	// 品牌与输入壳（对齐 ZCode zai 主题，00-design-tokens.md）：brand 亮色
	// 纯黑 / 深色纯白，发送键反色；聚焦描边 zai 是灰加深，非彩色。
	AccentMuted         mygo.Color // mygo 内置控件聚焦/勾选的中性加深灰（zai 聚焦不变色）
	Brand               mygo.Color // --brand
	OnBrand             mygo.Color // 品牌底上的前景
	ComposerBorder      mygo.Color // composer 输入壳描边（0.5px）
	ComposerBorderFocus mygo.Color // composer 输入壳聚焦描边
	ComposerShadow      mygo.Color // composer 输入壳软阴影（深色为透明=无）
	UserBubble          mygo.Color // 用户消息气泡底（ConversationUserInputBubble bg-surface）

	R0 mygo.Color // --r0 只读
	R1 mygo.Color // --r1 写工作区
	R2 mygo.Color // --r2 执行
	R3 mygo.Color // --r3 网络或越界
	R4 mygo.Color // --r4 破坏性

	CodeBg      mygo.Color // --code-bg
	ScrollThumb mygo.Color // --scroll-thumb

	SidebarBg    mygo.Color // --sidebar-bg
	SidebarHover mygo.Color // --sidebar-hover
	ChatBg       mygo.Color // --chat-bg

	// 以下不是 base.css token，是视图包曾经就地写 hex 的派生视觉色；
	// 收进色板是为了让主题 hex 在 theme 一个出口管理（AGENTS.md 6.4）。

	// DiffAddText / DiffDelText 是差异预览增删行的文本色（审批卡 diffLine，
	// React 版 .diff-line.add / .del 的文字色），深浅两套各取对应值。
	DiffAddText mygo.Color
	DiffDelText mygo.Color

	// FileDirIcon 是文件树目录图标的着色；FileCodeIcon / FileDocIcon /
	// FileSheetIcon / FilePdfIcon 是按扩展名的文件图标着色（panel.css 的
	// .f-ico.go/.doc/.sheet/.pdf；md 与未识别类型仍用 Text3）。
	FileDirIcon   mygo.Color
	FileCodeIcon  mygo.Color
	FileDocIcon   mygo.Color
	FileSheetIcon mygo.Color
	FilePdfIcon   mygo.Color

	// OnDanger 是填充式危险按钮上的文字色（.btn.danger 的填充档）。
	OnDanger mygo.Color

	// Dark 标记这套色板是否为深色外观：个别视觉细节（如文件树目录图标的
	// 着色）在深浅两模式下取不同值。
	Dark bool

	// WorkMode 是 Work 模式的琥珀强调色组（Code 模式用 Accent 系字段）。
	WorkMode AccentSet

	ShadowSm  ShadowSpec // --shadow-sm
	ShadowMd  ShadowSpec // --shadow-md
	ShadowLg  ShadowSpec // --shadow-lg
	ShadowPop ShadowSpec // --shadow-pop 的主投影；pop 还带一圈 1px 描边，见 PopRing
	PopRing   ShadowSpec // --shadow-pop 的 0 0 0 1px 描边圈
}

// Risk 把工具风险等级映射到风险徽章色（.risk-R0..R4）；越界回退 R0 中性色。
func (p *Palette) Risk(level policy.Risk) mygo.Color {
	switch level {
	case policy.R1:
		return p.R1
	case policy.R2:
		return p.R2
	case policy.R3:
		return p.R3
	case policy.R4:
		return p.R4
	default:
		return p.R0
	}
}

// ModeAccent 返回模式的强调色组：Work 用琥珀，Code（含未知值）用主题 accent。
func (p *Palette) ModeAccent(mode Mode) AccentSet {
	if mode == ModeWork {
		return p.WorkMode
	}
	return AccentSet{Accent: p.Accent, Soft: p.AccentSoft, Line: p.AccentLine, On: p.OnAccent}
}

// darkPalette 返回深色色板（base.css :root 默认值）。
func darkPalette() Palette {
	return Palette{
		Dark:       true,
		Bg:         mygo.Hex("#0b0d11"),
		BgSunken:   mygo.Hex("#08090c"),
		BgPanel:    mygo.Hex("#101318"),
		BgElevated: mygo.Hex("#151920"),
		BgFloat:    mygo.Hex("#171b23"),
		BgHover:    mygo.Hex("#1a1f28"),
		BgActive:   mygo.Hex("#222834"),
		BgInput:    mygo.Hex("#0e1116"),

		AccentMuted:         mygo.Hex("#a1a1aa"),
		Brand:               mygo.Hex("#ffffff"),
		OnBrand:             mygo.Hex("#000000"),
		ComposerBorder:      mygo.RGBA(255, 255, 255, 0.10),
		ComposerBorderFocus: mygo.RGBA(255, 255, 255, 0.15),
		ComposerShadow:      mygo.RGBA(0, 0, 0, 0), // 深色无 composer 阴影（styles.css:77）
		UserBubble:          mygo.RGBA(255, 255, 255, 0.08),

		Border:       mygo.Hex("#1d222b"),
		BorderSoft:   mygo.Hex("#171b22"),
		BorderStrong: mygo.Hex("#2a313d"),

		Text:  mygo.Hex("#e8ebf1"),
		Text2: mygo.Hex("#a2abba"),
		Text3: mygo.Hex("#6d7686"),
		Text4: mygo.Hex("#4d5563"),

		Accent:       mygo.Hex("#5b9dff"),
		AccentStrong: mygo.Hex("#7cb2ff"),
		AccentSoft:   mygo.RGBA(91, 157, 255, 0.13),
		AccentLine:   mygo.RGBA(91, 157, 255, 0.32),
		OnAccent:     mygo.Hex("#06101f"),

		OK:         mygo.Hex("#43c08d"),
		OKSoft:     mygo.RGBA(67, 192, 141, 0.13),
		Warn:       mygo.Hex("#e8b04a"),
		WarnSoft:   mygo.RGBA(232, 176, 74, 0.14),
		Danger:     mygo.Hex("#ef7070"),
		DangerSoft: mygo.RGBA(239, 112, 112, 0.13),
		Purple:     mygo.Hex("#a78bfa"),
		PurpleSoft: mygo.RGBA(167, 139, 250, 0.13),
		Cyan:       mygo.Hex("#4ec9d4"),

		R0: mygo.Hex("#7c8798"),
		R1: mygo.Hex("#e8b04a"),
		R2: mygo.Hex("#ec9350"),
		R3: mygo.Hex("#ee7070"),
		R4: mygo.Hex("#e0475f"),

		CodeBg:      mygo.Hex("#0c0f14"),
		ScrollThumb: mygo.Hex("#2b323e"),

		SidebarBg:    mygo.Hex("#101318"),
		SidebarHover: mygo.RGBA(255, 255, 255, 0.05),
		ChatBg:       mygo.Hex("#0b0d11"),

		DiffAddText: mygo.Hex("#a6e3c7"),
		DiffDelText: mygo.Hex("#f0b3b3"),

		FileDirIcon:   mygo.Hex("#d6a45c"),
		FileCodeIcon:  mygo.Hex("#59b7d6"),
		FileDocIcon:   mygo.Hex("#5b9dff"),
		FileSheetIcon: mygo.Hex("#43c08d"),
		FilePdfIcon:   mygo.Hex("#e07070"),

		OnDanger: mygo.Hex("#ffffff"),

		WorkMode: AccentSet{
			Accent: mygo.Hex("#f0b354"),
			Soft:   mygo.RGBA(240, 179, 84, 0.14),
			Line:   mygo.RGBA(240, 179, 84, 0.34),
			On:     mygo.Hex("#1d1405"),
		},

		ShadowSm:  ShadowSpec{Y: 1, Blur: 2, Color: mygo.RGBA(0, 0, 0, 0.35)},
		ShadowMd:  ShadowSpec{Y: 10, Blur: 30, Color: mygo.RGBA(0, 0, 0, 0.4)},
		ShadowLg:  ShadowSpec{Y: 24, Blur: 70, Color: mygo.RGBA(0, 0, 0, 0.55)},
		ShadowPop: ShadowSpec{Y: 16, Blur: 44, Color: mygo.RGBA(0, 0, 0, 0.5)},
		PopRing:   ShadowSpec{Blur: 0, Spread: 1, Color: mygo.Hex("#2a313d")},
	}
}

// lightPalette 返回浅色色板（base.css html[data-theme="light"]）。
func lightPalette() Palette {
	return Palette{
		Dark:       false,
		Bg:         mygo.Hex("#eef0f4"),
		BgSunken:   mygo.Hex("#e7eaf0"),
		BgPanel:    mygo.Hex("#ffffff"),
		BgElevated: mygo.Hex("#ffffff"),
		BgFloat:    mygo.Hex("#ffffff"),
		BgHover:    mygo.Hex("#f1f4f8"),
		BgActive:   mygo.Hex("#e7ecf4"),
		BgInput:    mygo.Hex("#ffffff"),

		AccentMuted:         mygo.Hex("#52525b"),
		Brand:               mygo.Hex("#000000"),
		OnBrand:             mygo.Hex("#ffffff"),
		ComposerBorder:      mygo.RGBA(13, 13, 13, 0.10),
		ComposerBorderFocus: mygo.RGBA(13, 13, 13, 0.15),
		ComposerShadow:      mygo.RGBA(0, 0, 0, 0.04),
		UserBubble:          mygo.RGBA(13, 13, 13, 0.05),

		Border:       mygo.Hex("#e0e5ee"),
		BorderSoft:   mygo.Hex("#ebeff5"),
		BorderStrong: mygo.Hex("#c9d2e0"),

		Text:  mygo.Hex("#10141b"),
		Text2: mygo.Hex("#525d6d"),
		Text3: mygo.Hex("#7d8798"),
		Text4: mygo.Hex("#a2abba"),

		Accent:       mygo.Hex("#2563eb"),
		AccentStrong: mygo.Hex("#1d4ed8"),
		AccentSoft:   mygo.RGBA(37, 99, 235, 0.09),
		AccentLine:   mygo.RGBA(37, 99, 235, 0.28),
		OnAccent:     mygo.Hex("#ffffff"),

		OK:         mygo.Hex("#0f9d6e"),
		OKSoft:     mygo.RGBA(15, 157, 110, 0.1),
		Warn:       mygo.Hex("#b7791f"),
		WarnSoft:   mygo.RGBA(183, 121, 31, 0.11),
		Danger:     mygo.Hex("#d64545"),
		DangerSoft: mygo.RGBA(214, 69, 69, 0.09),
		Purple:     mygo.Hex("#7c5cf0"),
		PurpleSoft: mygo.RGBA(124, 92, 240, 0.09),
		Cyan:       mygo.Hex("#0d8f9c"),

		R0: mygo.Hex("#69748a"),
		R1: mygo.Hex("#b7791f"),
		R2: mygo.Hex("#c2610f"),
		R3: mygo.Hex("#c53c3c"),
		R4: mygo.Hex("#b3122f"),

		CodeBg:      mygo.Hex("#f7f9fc"),
		ScrollThumb: mygo.Hex("#c3ccdb"),

		SidebarBg:    mygo.Hex("#f3f3f2"),
		SidebarHover: mygo.RGBA(13, 13, 13, 0.05),
		ChatBg:       mygo.Hex("#f8f8f8"),

		DiffAddText: mygo.Hex("#146c43"),
		DiffDelText: mygo.Hex("#a12a2a"),

		FileDirIcon:   mygo.Hex("#b07a24"),
		FileCodeIcon:  mygo.Hex("#59b7d6"),
		FileDocIcon:   mygo.Hex("#5b9dff"),
		FileSheetIcon: mygo.Hex("#43c08d"),
		FilePdfIcon:   mygo.Hex("#e07070"),

		OnDanger: mygo.Hex("#ffffff"),

		WorkMode: AccentSet{
			Accent: mygo.Hex("#b06f0c"),
			Soft:   mygo.RGBA(176, 111, 12, 0.1),
			Line:   mygo.RGBA(176, 111, 12, 0.28),
			On:     mygo.Hex("#ffffff"),
		},

		ShadowSm:  ShadowSpec{Y: 1, Blur: 2, Color: mygo.RGBA(16, 24, 40, 0.06)},
		ShadowMd:  ShadowSpec{Y: 10, Blur: 28, Color: mygo.RGBA(16, 24, 40, 0.1)},
		ShadowLg:  ShadowSpec{Y: 24, Blur: 64, Color: mygo.RGBA(16, 24, 40, 0.18)},
		ShadowPop: ShadowSpec{Y: 14, Blur: 40, Color: mygo.RGBA(16, 24, 40, 0.14)},
		PopRing:   ShadowSpec{Blur: 0, Spread: 1, Color: mygo.Hex("#e0e5ee")},
	}
}

// ---------------------------------------------------------------------------
// ui.Theme 派生与跟随系统
// ---------------------------------------------------------------------------

// uiTheme 把 CraftWork 色板映射进 mygo 的语义主题字段，让内置控件与自绘元素
// 同源同色。语义映射：Surface=BgElevated（控件面）、TextMuted=Text2（次要文字）、
// AccentHover/Pressed=AccentStrong（深浅两模式里 strong 都是更强调的一档）、
// Selection=AccentSoft（::selection 用 accent-soft）、Focus=Accent
// （:focus-visible 描边 accent）。
//
// 已知取舍：mygo 默认主题会跟随桌面偏好（系统强调色、高对比、文字缩放），
// CraftWork 观感以 base.css 为准，accent 与字号固定，不随桌面偏好变化。
func uiTheme(p Palette, dark bool) *mygo.Theme {
	t := &mygo.Theme{Dark: dark}
	t.Background = p.Bg
	t.Surface = p.BgElevated
	t.SurfaceHover = p.BgHover
	t.SurfacePressed = p.BgActive
	t.Border = p.Border
	t.Text = p.Text
	t.TextMuted = p.Text2
	// Accent 中性化（zai 聚焦语义「只加深不变色」）：mygo 内置控件的聚焦
	// 描边 / 勾选取 Accent——用中性灰而非品牌蓝，彩色仅保留在自绘语义色。
	t.Accent = p.AccentMuted
	t.AccentHover = p.Accent
	t.AccentPressed = p.Accent
	t.AccentText = p.OnAccent
	t.Danger = p.Danger
	t.Warning = p.Warn
	t.Success = p.OK
	t.Selection = p.AccentSoft
	t.Focus = p.Accent
	t.Scrollbar = p.ScrollThumb
	t.ScrollbarWidth = 6 // base.css 细滚动条：10px 槽内 4px 可见拇指，取近似
	t.Radius = RadiusBtn
	t.FontSize = BaseFontSize
	// Inverse/InverseText 留零：工具提示与 toast 由 mygo 自动取主题反色。
	// Font 留空：跟随系统界面字体（--font-ui 的系统栈）。
	return t
}

// 主题单例：登记表色板各构建一次，Apply 以指针身份区分深浅。
var (
	darkPaletteUI  = uiTheme(darkPalette(), true)
	lightPaletteUI = uiTheme(lightPalette(), false)
	darkPal        = darkPalette()
	lightPal       = lightPalette()
)

// lastDark 是最近一帧 Apply 实际选用的深浅；跟随系统时随系统变化。
// 供命令面板等无上下文的调用方读取当前生效主题。
var lastDark = true

// IsDark 返回最近一帧实际生效的深浅主题。
func IsDark() bool { return lastDark }

// appearance 是用户在设置里选择的外观与界面基准字号（设置立即生效的落点）。
// 立即模式约定：只在主线程（view 或 win.Update 回调）写；SetAppearance 由
// 设置保存路径调用，Apply 每帧读。
var appearance = struct {
	themeSetting string // system / dark / light
	fontSize     int    // 界面基准字号
}{themeSetting: "system", fontSize: int(BaseFontSize)}

// SetAppearance 应用设置里的外观主题与界面基准字号，下一帧生效。
//
// 显式主题（dark/light）覆盖系统偏好；字号驱动字号档全档派生，并同步到
// 两套 mygo 主题（内置控件的 FontSize 随动）。非法值按设置服务的钳制规则
// 回退默认，保证这里与 service.SettingsService.Save 的边界一致。
func SetAppearance(themeSetting string, fontSize int) {
	switch themeSetting {
	case "dark":
		appearance.themeSetting = themeSetting
		lastDark = true
	case "light":
		appearance.themeSetting = themeSetting
		lastDark = false
	default:
		appearance.themeSetting = "system"
	}
	if fontSize < 11 || fontSize > 18 {
		fontSize = int(BaseFontSize)
	}
	appearance.fontSize = fontSize
	Fonts.Size = float32(fontSize)
	darkPaletteUI.FontSize = Fonts.Size
	lightPaletteUI.FontSize = Fonts.Size
}

// Apply 把窗口主题切换为与当前外观匹配的 CraftWork 主题，并返回其色板。
//
// 在根 view 每帧开头调用：mygo 每帧把 c.Theme() 重置为跟随系统的默认主题，
// 据其 Dark 标志选边；系统切换外观时引擎重画一帧，这里即随之实时切换。
// 用户在设置里显式选择 dark / light 时覆盖系统偏好（外观设置立即生效）。
// SetTheme 同时把主题背景铺到窗口根上（首帧即有着色，不留白屏）。
func Apply(c *mygo.Context) *Palette {
	dark := c.Theme().Dark
	switch appearance.themeSetting {
	case "dark":
		dark = true
	case "light":
		dark = false
	}
	lastDark = dark
	if dark {
		c.SetTheme(darkPaletteUI)
		return &darkPal
	}
	c.SetTheme(lightPaletteUI)
	return &lightPal
}

// ThemePreview 返回外观设置里主题卡预览用的三块颜色（侧栏条、主区、强调色）。
// 跟随系统时以「深侧栏 + 浅主区 + 深强调」示意两套并存。
func ThemePreview(setting string) (sidebar, main, accent mygo.Color) {
	switch setting {
	case "dark":
		return darkPal.SidebarBg, darkPal.ChatBg, darkPal.Accent
	case "light":
		return lightPal.SidebarBg, lightPal.ChatBg, lightPal.Accent
	default:
		return darkPal.SidebarBg, lightPal.ChatBg, darkPal.Accent
	}
}

// withAlpha 返回同色不同透明度的颜色（借 mygo.RGBA 的 alpha 换算）。
func withAlpha(c mygo.Color, a float32) mygo.Color {
	return mygo.RGBA(c.R, c.G, c.B, a)
}

// 材质（under-window 毛玻璃）模式下侧栏各色的不透明度：底 72% 保留一层
// 侧栏色调、悬停行 60% 更透（对照 ZCode 侧栏不画实底、悬停用半透明淡色）。
const (
	materialSidebarAlpha = float32(0.72)
)

// MaterialPalette 返回窗口材质模式下的色板副本：侧栏底与侧栏悬停行改为
// 半透明，让 under-window 材质从其下透出；对话区、面板与全部文字色保持
// 实底（对照 ZCode 的 workspace 主区 bg-background）。外壳在 c.Vibrancy()
// 为真时以副本替换原色板下发给侧栏视图（无材质环境继续用实底，视图代码
// 两态共用）；返回副本，不改写深浅单例。
func MaterialPalette(p *Palette) *Palette {
	q := *p
	q.SidebarBg = withAlpha(p.SidebarBg, materialSidebarAlpha)
	// SidebarHover 在 zai 色板里本就是半透明（亮黑 5% / 深白 5%），材质下
	// 原样透出即可；再叠 withAlpha 会把 alpha 抬高反而更实。
	q.SidebarHover = p.SidebarHover
	return &q
}
