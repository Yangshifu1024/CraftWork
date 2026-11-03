package ui

// 外壳布局的纯函数单测 + ui.Tester 走查（三栏、窄窗收起、拖拽钳制与双击恢复）。

import (
	"testing"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

func TestClampPanel(t *testing.T) {
	cases := []struct{ in, want float32 }{
		{100, theme.PanelMinWidth},
		{theme.PanelMinWidth, theme.PanelMinWidth},
		{500, 500},
		{theme.PanelMaxWidth, theme.PanelMaxWidth},
		{1200, theme.PanelMaxWidth},
	}
	for _, c := range cases {
		if got := clampPanel(c.in); got != c.want {
			t.Errorf("clampPanel(%v) = %v, want %v", c.in, got, c.want)
		}
	}
}

func TestChatPanelWidthRoundTrip(t *testing.T) {
	const splitW = 1440 - theme.SidebarWidth
	// 对话列宽度与面板宽度互为反函数（分隔线 1 DIP 计入）。
	chat := chatWidthFor(splitW, 480)
	if chat != splitW-1-480 {
		t.Fatalf("chatWidthFor = %v", chat)
	}
	if got := panelWidthFromChat(splitW, chat); got != 480 {
		t.Errorf("panelWidthFromChat = %v, want 480", got)
	}
	// 拖过头时钳制生效：对话列被压得只剩 100，面板顶到上限 760。
	if got := panelWidthFromChat(splitW, 100); got != theme.PanelMaxWidth {
		t.Errorf("上限钳制 = %v, want %v", got, theme.PanelMaxWidth)
	}
	// 反向拖到最右：面板只剩 100，回到下限 320。
	if got := panelWidthFromChat(splitW, splitW-1-100); got != theme.PanelMinWidth {
		t.Errorf("下限钳制 = %v, want %v", got, theme.PanelMinWidth)
	}
}

func TestPanelShown(t *testing.T) {
	if !panelShown(1440, true) {
		t.Errorf("宽窗且展开时应显示")
	}
	if panelShown(theme.NarrowWindowWidth-1, true) {
		t.Errorf("窄窗应自动收起")
	}
	if !panelShown(theme.NarrowWindowWidth, true) {
		t.Errorf("阈值宽度应视为宽窗（>= 判定）")
	}
	if panelShown(1440, false) {
		t.Errorf("用户收起后不应显示")
	}
}

func TestNewShellDefaults(t *testing.T) {
	s := NewShell()
	// ui.Split 宽度指针初值必须非零（mygo 已知坑），且与默认面板宽自洽。
	if s.chatWidth <= 0 {
		t.Fatalf("chatWidth 初值 = %v，必须非零", s.chatWidth)
	}
	if s.chatWidth != chatWidthFor(theme.DefWindowWidth-theme.SidebarWidth, theme.PanelDefWidth) {
		t.Errorf("chatWidth 初值与默认面板宽不自洽: %v", s.chatWidth)
	}
	if s.panelWidth != theme.PanelDefWidth || !s.panelOpen || s.panelReset {
		t.Errorf("默认状态错误: %+v", *s)
	}
}

// newShellTester 在 1440×900 下起一个外壳 view。
func newShellTester(t *testing.T) (*Shell, *mygo.Tester) {
	t.Helper()
	s := NewShell()
	tt := mygo.NewTester(s.View, 1440, 900)
	return s, tt
}

const (
	textSidebar = "侧栏 · 04 号票接入"
	textChat    = "对话 · 05 号票接入"
	// textPanel 是面板空态的标题文案（08 号票接入后面板显示文件树，
	// 测试进程没有装配工作区服务，落在这里的空态上）。
	textPanel = "没有活动工作区"
)

func TestShellViewShowsThreePanes(t *testing.T) {
	_, tt := newShellTester(t)
	// 14 号票：无独立标题条，品牌名不再出现在窗口顶（内容直接从窗口顶
	// 开始，红绿灯叠加在侧栏拖拽行上）。
	for _, text := range []string{textSidebar, textChat, textPanel} {
		if !tt.HasText(text) {
			t.Errorf("外壳缺文本 %q，实有 %v", text, tt.Texts())
		}
	}
	if tt.HasText("CraftWork") {
		t.Errorf("不应再有独立标题条的品牌名 %q", "CraftWork")
	}
}

// TestShellViewSidebarTopDragRow 验证侧栏顶部的拖拽标题行（14 号票）：
// 高度 = SidebarTopbarHeight（容纳 {x:22,y:23} 的红绿灯且不遮下方内容），
// 帧内记录供断言。
func TestShellViewSidebarTopDragRow(t *testing.T) {
	s, tt := newShellTester(t)
	tt.Frame()
	if s.topDragH != theme.SidebarTopbarHeight {
		t.Fatalf("侧栏顶部拖拽行高 = %v, want %v", s.topDragH, theme.SidebarTopbarHeight)
	}
}

func TestShellViewCollapsesPanelWhenNarrow(t *testing.T) {
	_, tt := newShellTester(t)
	// 窄于阈值：面板整体不建，对话列占满。
	tt.SetSize(int(theme.NarrowWindowWidth-100), 800)
	if tt.HasText(textPanel) {
		t.Errorf("窄窗下面板未收起")
	}
	if !tt.HasText(textChat) || !tt.HasText(textSidebar) {
		t.Errorf("窄窗下侧栏或对话列丢失: %v", tt.Texts())
	}
	// 变宽恢复：用户偏好未被窄窗收起改动。
	tt.SetSize(1440, 900)
	if !tt.HasText(textPanel) {
		t.Errorf("窗口变宽后面板未恢复")
	}
}

func TestShellViewPanelDragClampAndDoubleClick(t *testing.T) {
	s, tt := newShellTester(t)
	const dividerY = 450
	tt.Frame() // 先稳一帧，让 Split 以首帧宽度就位

	panelBox := func(t *testing.T) mygo.Rect {
		t.Helper()
		rect, ok := tt.Find(textPanel)
		if !ok {
			t.Fatalf("找不到面板占位文本: %v", tt.Texts())
		}
		return rect
	}
	at320 := panelBox(t)

	// 把分隔线从 1119 拖到 900：面板宽到 539（未越界，不钳制）。
	tt.Press(1119, dividerY)
	tt.Move(900, dividerY)
	tt.Release(900, dividerY)
	tt.Frame()
	if s.panelWidth != 539 {
		t.Fatalf("拖拽后面板宽 = %v, want 539", s.panelWidth)
	}

	// 从 900 拖到 200（越上限）：面板钳制在 760，分隔线停在 680。
	tt.Press(900, dividerY)
	tt.Move(200, dividerY)
	tt.Release(200, dividerY)
	tt.Frame()
	if s.panelWidth != theme.PanelMaxWidth {
		t.Fatalf("上限钳制后 = %v, want %v", s.panelWidth, theme.PanelMaxWidth)
	}

	// 面板左缘（分隔线右侧 4 DIP，落在双击条上、拖拽把手之外）双击：
	// 恢复默认 320。
	gripX := (1440 - theme.PanelMaxWidth) + 4
	tt.ClickAt(gripX, dividerY)
	tt.Advance(80 * time.Millisecond)
	tt.ClickAt(gripX, dividerY)
	tt.Frame()
	if s.panelWidth != theme.PanelDefWidth || s.panelReset {
		t.Fatalf("双击恢复后面板宽 = %v (reset=%v), want %v", s.panelWidth, s.panelReset, theme.PanelDefWidth)
	}
	// 几何确认：占位文本回到初始位置（view 真的重排了）。
	restored := panelBox(t)
	if diff := restored.X - at320.X; diff < -1 || diff > 1 {
		t.Errorf("双击恢复后占位文本 X = %v, want 约 %v", restored.X, at320.X)
	}
}

func TestShellEscapeChainClosesPanelTab(t *testing.T) {
	s, tt := newShellTester(t)
	tt.Frame()
	if s.PanelHasActiveTab() {
		t.Fatalf("空面板不应有激活 tab")
	}
	// 打开一个差异 tab（singleton，无需服务）：Esc 第一层关闭它。
	s.pane.OpenDiffTab("notes.txt")
	tt.Frame()
	if !s.PanelHasActiveTab() {
		t.Fatalf("OpenDiffTab 后应有激活 tab")
	}
	s.HandleWindowEscape()
	tt.Frame()
	if s.PanelHasActiveTab() {
		t.Fatalf("Esc 应关闭面板激活 tab")
	}
}

func TestShellEscapeChainFallsThroughToChat(t *testing.T) {
	s, tt := newShellTester(t)
	tt.Frame()
	calls := 0
	s.EscapeKey = func() bool { calls++; return true }
	// 无 tab 且无焦点守卫：落到聊天列两级。
	s.HandleWindowEscape()
	if calls != 1 {
		t.Fatalf("无 tab 时应落到聊天列两级，调用 %d 次", calls)
	}
	// 焦点在面板内：整链跳过（终端把 Esc 当输入）。
	s.panelFocus = true
	s.HandleWindowEscape()
	if calls != 1 {
		t.Fatalf("焦点守卫应跳过整条链，实际调用 %d 次", calls)
	}
}

func TestShellViewShortcuts(t *testing.T) {
	s := NewShell()
	chatEsc := 0
	s.EscapeKey = func() bool { chatEsc++; return true }
	newThreads := 0
	s.NewThread = func() { newThreads++ }
	tt := mygo.NewTester(RootView(s), 1440, 900)
	tt.Frame()

	// ⌘W 无激活 tab：不消费（无菜单接管，按键落空）。
	tt.Key(mygo.Cmd, mygo.KeyW)
	tt.Frame()
	if s.PanelHasActiveTab() {
		t.Fatalf("⌘W 不应凭空建 tab")
	}
	// ⌘N 走侧栏编排。
	tt.Key(mygo.Cmd, mygo.KeyN)
	tt.Frame()
	if newThreads != 1 {
		t.Fatalf("⌘N 应新建会话一次，实际 %d 次", newThreads)
	}
	// ⌘B 开合面板。
	open := s.PanelOpen()
	tt.Key(mygo.Cmd, mygo.KeyB)
	tt.Frame()
	if s.PanelOpen() == open {
		t.Fatalf("⌘B 应切换面板偏好")
	}
	// 打开 tab 后 Esc（走 RootView 的退出链注册）关闭它，而非聊天列两级。
	s.pane.OpenDiffTab("a.txt")
	tt.Frame()
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if s.PanelHasActiveTab() {
		t.Fatalf("Esc 应经 RootView 关闭激活 tab")
	}
	if chatEsc != 0 {
		t.Fatalf("有 tab 时 Esc 不应落到聊天列两级，实际 %d 次", chatEsc)
	}
	// 再按一次：落到聊天列两级（消费）。
	tt.Key(0, mygo.KeyEscape)
	tt.Frame()
	if chatEsc != 1 {
		t.Fatalf("无层级时 Esc 应落到聊天列两级，实际 %d 次", chatEsc)
	}
}

// ---------------------------------------------------------------------------
// 14 号票：窗口材质（under-window 毛玻璃）下的底色分派（色板派生的纯函数
// 断言在 theme 包 TestMaterialPalette）
// ---------------------------------------------------------------------------

// TestShellViewVibrancy 验证底色随材质开关分派：材质开时侧栏像素半透明
// （透出 under-window 材质）、对话区保持实底；材质关时全部实底（Tester
// 与无材质平台的回退形态）。像素经 Tester 帧图像取样。
func TestShellViewVibrancy(t *testing.T) {
	_, tt := newShellTester(t)
	tt.Frame()

	sidebarPx := func() int {
		_, _, _, a := tt.Image().RGBAAt(100, 500).RGBA()
		return int(a >> 8)
	}
	chatPx := func() int {
		_, _, _, a := tt.Image().RGBAAt(800, 500).RGBA()
		return int(a >> 8)
	}

	// 无材质：侧栏与对话区都实底。
	if a := sidebarPx(); a != 255 {
		t.Fatalf("无材质时侧栏应实底，alpha = %d", a)
	}
	if a := chatPx(); a != 255 {
		t.Fatalf("无材质时对话区应实底，alpha = %d", a)
	}

	// 材质开：侧栏半透明透材质，对话区保持实底。
	tt.SetVibrancy(true)
	tt.Frame()
	if a := sidebarPx(); a >= 255 {
		t.Fatalf("材质开启时侧栏应半透明透材质，alpha = %d", a)
	}
	if a := chatPx(); a != 255 {
		t.Fatalf("材质开启时对话区应保持实底，alpha = %d", a)
	}

	// 材质关（Tester 模拟无材质环境）：回到全部实底。
	tt.SetVibrancy(false)
	tt.Frame()
	if a := sidebarPx(); a != 255 {
		t.Fatalf("材质关闭后侧栏应回到实底，alpha = %d", a)
	}
}
