package ui

// 三栏外壳：顶部 48px 拖拽区、侧栏 268 固定、聊天列自适应、面板 Split
// （320–760 可拖拽、双击恢复默认宽）。布局尺寸对照 docs/specs/interface.md；
// 侧栏 / 聊天的具体内容由 04 / 05 号票接入，面板槽位已由 08 号票接上
// （tab 条 + 内容区，见 panelview.go）。

import (
	"time"

	cwmygo "github.com/egoist/mygo"
	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/service"
)

// panelGripWidth 是面板左缘双击恢复条的宽度（DIP）；ui.Split 的拖拽把手
// 盖住分隔线两侧各约 3 DIP，双击条落在它外侧。
const panelGripWidth = float32(8)

// wsCacheTTL 是活动工作区缓存的存活时间：面板每帧都要读当前工作区做
// 可见性过滤，而 service.Active() 是两次 SQLite 查询，用短 TTL 缓存挡住
// 帧率级的重复查询（工作区切换最迟 1 秒后被面板看到）。
const wsCacheTTL = time.Second

// Shell 持有三栏外壳的界面状态。立即模式约定：只在主线程（view 或
// win.Update 回调）里读写，不放锁。
type Shell struct {
	// chatWidth 是 ui.Split 第一栏（对话列）的宽度指针。初值必须非零，
	// 否则左栏首帧不可见（mygo 已知坑），按默认窗口宽度预排。
	chatWidth float32
	// panelWidth 是面板的逻辑宽度，恒在 [PanelMinWidth, PanelMaxWidth]。
	// Split 的指针是对话列宽度，拖拽结果经 panelWidthFromChat 折算回来。
	panelWidth float32
	// panelOpen 是用户对面板展开与否的偏好；窄窗自动收起不改动它，
	// 窗口变宽后面板恢复。
	panelOpen bool
	// panelReset 标记「双击恢复默认宽」待生效：下一帧直接按默认宽重排
	// 对话列，而不是从 chatWidth 反推（否则恢复值会被拖拽残值冲掉）。
	panelReset bool
	// splitW 是上一帧 split 区域的总宽：与当前一致说明宽度变化只能来自
	// 拖拽；不一致说明窗口变了，面板保持记忆宽、对话列吸收差值。
	splitW float32

	// SidebarView 是侧栏槽位的内容（04 号票的 sidebar.View）；nil 时画
	// 占位文本。委托注入而非直接引用 internal/ui/sidebar，避免包环。
	SidebarView func(c *mygo.Context, pal *Palette)
	// ChatView 是聊天列槽位的内容（05 号票的 chat.View）；nil 时画
	// 占位文本。
	ChatView func(c *mygo.Context, pal *Palette)
	// ThreadID 返回当前会话 Id（05 号票注入）：面板 tab 按「工作区 +
	// 线程」过滤显隐；未注入时全部归属草稿上下文。
	ThreadID func() string

	// pane 是面板（08 号票）：多 tab 状态 + 文件树 + 编辑器。依赖由
	// SetPanelServices / BindWindow 注入，未注入时面板渲染空态。
	pane *Pane
	// workspaceSvc 提供活动工作区（面板可见性 scope 与文件树数据源）。
	workspaceSvc *service.WorkspaceService
	// documentSvc 提供文档预览。
	documentSvc *service.DocumentService
	// settingsSvc 提供设置快照（终端字号等）。
	settingsSvc *service.SettingsService
	// win 是主窗口：面板后台 IO 的结果经 win.Update 回主线程。
	win *cwmygo.Window
	// wsCache / wsCacheAt 是活动工作区的短 TTL 缓存。
	wsCache   *service.WorkspaceView
	wsCacheAt time.Time
}

// NewShell 创建外壳状态：面板默认展开、取最小宽度，把空间留给对话区。
func NewShell() *Shell {
	s := &Shell{
		chatWidth:  chatWidthFor(DefWindowWidth-SidebarWidth, PanelDefWidth),
		panelWidth: PanelDefWidth,
		panelOpen:  true,
	}
	s.pane = NewPane(PaneDeps{
		Workspace: s.activeWorkspace,
		// Thread 由 05 号票的聊天列经 ThreadID 注入；未注入时所有 tab
		// 归属草稿上下文，与「未发送首条消息」的可见性一致。
		Thread: func() string {
			if s.ThreadID != nil {
				return s.ThreadID()
			}
			return ""
		},
		WorkspacesSvc: func() *service.WorkspaceService { return s.workspaceSvc },
		DocumentsSvc:  func() *service.DocumentService { return s.documentSvc },
		Settings: func() service.Settings {
			if s.settingsSvc == nil {
				return service.DefaultSettings()
			}
			return s.settingsSvc.Get()
		},
		Update: func(fn func()) {
			if s.win != nil {
				s.win.Update(fn)
			}
			// 窗口尚未创建时丢弃：面板交互只存在于窗口建立之后，与
			// emitter「应用未创建前事件丢弃」的语义一致。
		},
		Collapse: func() { s.panelOpen = false },
	})
	return s
}

// SetPanelServices 注入面板依赖的后端服务（native UI 约定：view 直调）。
// 在装配期、窗口创建前调用。
func (s *Shell) SetPanelServices(workspace *service.WorkspaceService, document *service.DocumentService) {
	s.workspaceSvc = workspace
	s.documentSvc = document
}

// SetSettingsService 注入设置服务（终端字号等设置项的面板数据源）。
func (s *Shell) SetSettingsService(settings *service.SettingsService) {
	s.settingsSvc = settings
}

// Shutdown 回收面板持有的长生命周期资源（终端进程等）。窗口关闭 /
// 应用退出时由装配层调用。
func (s *Shell) Shutdown() {
	s.pane.Shutdown()
}

// smokeSeed 是 GUI 冒烟的装配钩子（cwsmoke 构建标签下由 smoke_cwsmoke.go
// 注册，正常构建恒为 nil）：窗口绑定后把界面状态预置到截图走查需要的
// 形态（打开终端 / 差异 tab、固定窗口位置），免键鼠驱动。
var smokeSeed func(s *Shell)

// BindWindow 在窗口创建后记录实例，面板后台 IO 的结果经它回主线程。
func (s *Shell) BindWindow(win *cwmygo.Window) {
	s.win = win
	if smokeSeed != nil {
		win.Update(func() { smokeSeed(s) })
	}
}

// OpenFileAt 打开文件并定位到行（path:line 引用与命令面板文件搜索的
// 正式入口，转面板 Pane.OpenFileAt；短路径经 LocateFile 唯一匹配）。
func (s *Shell) OpenFileAt(path string, line int) {
	s.pane.OpenFileAt(path, line)
}

// activeWorkspace 返回当前活动工作区（短 TTL 缓存）；没有时返回 nil。
func (s *Shell) activeWorkspace() *service.WorkspaceView {
	if s.workspaceSvc == nil {
		return nil
	}
	if s.wsCache != nil && time.Since(s.wsCacheAt) < wsCacheTTL {
		return s.wsCache
	}
	ws, err := s.workspaceSvc.Active()
	if err != nil || ws == nil {
		// 没有活动工作区：缓存空值同样吃 TTL，避免每帧重查。
		s.wsCache, s.wsCacheAt = nil, time.Now()
		return nil
	}
	s.wsCache, s.wsCacheAt = ws, time.Now()
	return ws
}

// clampPanel 把面板宽度钳制进 [PanelMinWidth, PanelMaxWidth]。
func clampPanel(v float32) float32 {
	if v < PanelMinWidth {
		return PanelMinWidth
	}
	if v > PanelMaxWidth {
		return PanelMaxWidth
	}
	return v
}

// chatWidthFor 由面板宽度算对话列宽度（split 总宽扣除 1 DIP 分隔线）。
func chatWidthFor(splitW, panelW float32) float32 {
	return splitW - DividerWidth - panelW
}

// panelWidthFromChat 由对话列宽度（用户拖拽的结果）反推面板宽度并钳制。
func panelWidthFromChat(splitW, chatW float32) float32 {
	return clampPanel(splitW - DividerWidth - chatW)
}

// panelShown 判断面板停靠栏是否显示：用户偏好展开，且窗口不窄于
// NarrowWindowWidth（窄窗收起是浮层形态的骨架替代，浮层归 08 号票）。
func panelShown(windowW float32, open bool) bool {
	return open && windowW >= NarrowWindowWidth
}

// View 画整窗外壳。root view 每帧从这里开始：先套主题（跟随系统），
// 再按当前窗口宽度分派三栏。
func (s *Shell) View(c *mygo.Context) {
	pal := Apply(c)
	windowW, _ := c.Size()

	mygo.Column(c).Fill().Children(func() {
		s.topbar(c, pal)
		s.body(c, pal, windowW)
	})
}

// topbar 画顶部 48px 拖拽区：左段品牌名与侧栏同宽居中，右段留给运行状态
// 与设置入口（05 / 07 号票）。c.TitleBar() 给出红绿灯占位，左右各留出避让；
// DragWindow 让空白区域可拖动窗口，双击等价标题栏缩放。
func (s *Shell) topbar(c *mygo.Context, pal *Palette) {
	bar := c.TitleBar()
	mygo.Row(c).Height(TopbarHeight).
		Padding(0, bar.Right+Space6, 0, bar.Left+Space6).
		DragWindow().
		Background(pal.BgPanel).
		BorderWidth(0, 0, 1, 0).
		BorderColor(pal.Border).
		Children(func() {
			// 左段：品牌名，与侧栏同宽并居中（frontend .topbar-left）。
			mygo.Box(c).Width(SidebarWidth).Center().Children(func() {
				mygo.Text(c, "CraftWork").Bold()
			})
			mygo.Spacer(c)
			// 右段：占位留空。运行 pill 与模型 / 设置入口由 05 / 07 号票接入，
			// 不放「看起来能点」的控件。
		})
}

// body 画三栏主体：侧栏 268 固定；右侧用 ui.Split 分为对话列与面板，
// 对话列宽度由面板宽度折算并钳制；窄窗时面板整体不建，对话列占满。
func (s *Shell) body(c *mygo.Context, pal *Palette, windowW float32) {
	splitW := windowW - SidebarWidth
	shown := panelShown(windowW, s.panelOpen)
	switch {
	case !shown:
		// 面板收起时对话列占满；同步 chatWidth，面板的宽度记忆保持
		// 原值，不随窄窗变化。
		s.chatWidth = chatWidthFor(splitW, s.panelWidth)
	case s.panelReset:
		// 双击恢复：直接按默认宽重排，清掉标记。
		s.panelWidth = PanelDefWidth
		s.chatWidth = chatWidthFor(splitW, s.panelWidth)
		s.panelReset = false
	case s.splitW == splitW:
		// 窗口没变而对话列宽度变了：只能来自用户拖拽。反推面板宽并
		// 钳制回写，拖拽手感即为 320–760 的边界。
		s.panelWidth = panelWidthFromChat(splitW, s.chatWidth)
		s.chatWidth = chatWidthFor(splitW, s.panelWidth)
	default:
		// 窗口宽度变了（含窄窗收起后变宽）：面板保持记忆宽，对话列
		// 吸收差值。
		s.chatWidth = chatWidthFor(splitW, s.panelWidth)
	}
	s.splitW = splitW

	mygo.Row(c).Grow(1).AlignItems(mygo.Stretch).Children(func() {
		s.sidebar(c, pal)
		if shown {
			mygo.Split(c, &s.chatWidth, func() { s.chatPane(c, pal) }, func() { s.panelPane(c, pal) }).Grow(1)
		} else {
			s.chatPane(c, pal).Grow(1)
		}
	})
}

// sidebar 画 268 固定侧栏。内容经 SidebarView 委托给 04 号票的侧栏视图
// （模式切换、会话分组列表与底部两行都在那里面）；未注入时画占位文本。
// 侧栏自身的 48px 标题行由窗口顶栏的左段承担，槽位直接从顶栏线下方开始。
func (s *Shell) sidebar(c *mygo.Context, pal *Palette) {
	if s.SidebarView != nil {
		s.SidebarView(c, pal)
		return
	}
	mygo.Column(c).Width(SidebarWidth).
		Background(pal.SidebarBg).
		BorderWidth(0, 1, 0, 0).
		BorderColor(pal.Border).
		Children(func() {
			mygo.Box(c).Grow(1).Center().Children(func() {
				mygo.Text(c, "侧栏 · 04 号票接入").TextColor(pal.Text3)
			})
		})
}

// chatPane 画自适应对话列。内容经 ChatView 委托给 05 号票的聊天视图
// （会话头部、消息流与输入区）；未注入时画占位文本。
func (s *Shell) chatPane(c *mygo.Context, pal *Palette) mygo.Element {
	if s.ChatView != nil {
		return mygo.Column(c).Fill().Children(func() {
			s.ChatView(c, pal)
		})
	}
	return mygo.Column(c).Fill().
		Background(pal.ChatBg).
		Children(func() {
			mygo.Box(c).Grow(1).Center().Children(func() {
				mygo.Text(c, "对话 · 05 号票接入").TextColor(pal.Text3)
			})
		})
}

// panelPane 画停靠面板：08 号票的 tab 条 + 内容区（文件树 / 编辑器；
// 差异与终端 tab 由 09 号票接入，先渲染空态占位）。面板左缘有一条
// 8 DIP 的双击区：双击把面板宽度恢复为默认 320。
//
// 面板三形态的骨架归属：
//   - 停靠（已接）：占第三栏，Split 拖拽 320–760，双击恢复默认宽。
//   - 窄窗浮层（08 号票）：窄窗时面板覆盖聊天区右侧、不挤压对话列；
//     本票骨架为窄窗直接收起（见 panelShown）。
//   - 最大化（08 号票）：面板覆盖中间区域、左缘压住侧栏边框，留桩。
func (s *Shell) panelPane(c *mygo.Context, pal *Palette) {
	// 面板内容：多 tab 状态机（internal/ui/panel）+ 文件树 / 编辑器；
	// 末 tab 关闭 / 关闭全部会经 Collapse 回调把 panelOpen 置 false，
	// 下一帧这里整体消失。
	mygo.Column(c).Fill().
		Background(pal.SidebarBg).
		BorderWidth(0, 0, 0, 1).
		BorderColor(pal.Border).
		Children(func() {
			s.pane.View(c, pal, s.panelWidth)
			// 双击恢复默认宽度；标记在下一帧 body 里生效。
			grip := mygo.Box(c).Absolute().Top(0).Bottom(0).Left(0).Width(panelGripWidth)
			if grip.DoubleClicked() {
				s.panelWidth = PanelDefWidth
				s.panelReset = true
			}
		})
}

// TogglePanel 切换面板展开 / 收起偏好（命令面板用；窄窗自动收起不改它）。
func (s *Shell) TogglePanel() { s.panelOpen = !s.panelOpen }

// PanelOpen 报告面板是否展开（命令面板据此显示动作方向）。
func (s *Shell) PanelOpen() bool { return s.panelOpen }

// ClosePanelActiveTab 关闭面板当前工作区桶的激活 tab（命令面板 ⌘W 用）。
func (s *Shell) ClosePanelActiveTab() { s.pane.CloseActiveTab() }
