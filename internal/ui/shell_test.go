package ui

// 外壳布局的纯函数单测 + ui.Tester 走查（三栏、窄窗收起、拖拽钳制与双击恢复）。

import (
	"testing"
	"time"

	mygo "github.com/egoist/mygo/ui"
)

func TestClampPanel(t *testing.T) {
	cases := []struct{ in, want float32 }{
		{100, PanelMinWidth},
		{PanelMinWidth, PanelMinWidth},
		{500, 500},
		{PanelMaxWidth, PanelMaxWidth},
		{1200, PanelMaxWidth},
	}
	for _, c := range cases {
		if got := clampPanel(c.in); got != c.want {
			t.Errorf("clampPanel(%v) = %v, want %v", c.in, got, c.want)
		}
	}
}

func TestChatPanelWidthRoundTrip(t *testing.T) {
	const splitW = 1440 - SidebarWidth
	// 对话列宽度与面板宽度互为反函数（分隔线 1 DIP 计入）。
	chat := chatWidthFor(splitW, 480)
	if chat != splitW-1-480 {
		t.Fatalf("chatWidthFor = %v", chat)
	}
	if got := panelWidthFromChat(splitW, chat); got != 480 {
		t.Errorf("panelWidthFromChat = %v, want 480", got)
	}
	// 拖过头时钳制生效：对话列被压得只剩 100，面板顶到上限 760。
	if got := panelWidthFromChat(splitW, 100); got != PanelMaxWidth {
		t.Errorf("上限钳制 = %v, want %v", got, PanelMaxWidth)
	}
	// 反向拖到最右：面板只剩 100，回到下限 320。
	if got := panelWidthFromChat(splitW, splitW-1-100); got != PanelMinWidth {
		t.Errorf("下限钳制 = %v, want %v", got, PanelMinWidth)
	}
}

func TestPanelShown(t *testing.T) {
	if !panelShown(1440, true) {
		t.Errorf("宽窗且展开时应显示")
	}
	if panelShown(NarrowWindowWidth-1, true) {
		t.Errorf("窄窗应自动收起")
	}
	if !panelShown(NarrowWindowWidth, true) {
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
	if s.chatWidth != chatWidthFor(DefWindowWidth-SidebarWidth, PanelDefWidth) {
		t.Errorf("chatWidth 初值与默认面板宽不自洽: %v", s.chatWidth)
	}
	if s.panelWidth != PanelDefWidth || !s.panelOpen || s.panelReset {
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
	textPanel   = "面板 · 08 号票接入"
)

func TestShellViewShowsThreePanes(t *testing.T) {
	_, tt := newShellTester(t)
	for _, text := range []string{"CraftWork", textSidebar, textChat, textPanel} {
		if !tt.HasText(text) {
			t.Errorf("外壳缺文本 %q，实有 %v", text, tt.Texts())
		}
	}
}

func TestShellViewCollapsesPanelWhenNarrow(t *testing.T) {
	_, tt := newShellTester(t)
	// 窄于阈值：面板整体不建，对话列占满。
	tt.SetSize(int(NarrowWindowWidth-100), 800)
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
	if s.panelWidth != PanelMaxWidth {
		t.Fatalf("上限钳制后 = %v, want %v", s.panelWidth, PanelMaxWidth)
	}

	// 面板左缘（分隔线右侧 4 DIP，落在双击条上、拖拽把手之外）双击：
	// 恢复默认 320。
	gripX := (1440 - PanelMaxWidth) + 4
	tt.ClickAt(gripX, dividerY)
	tt.Advance(80 * time.Millisecond)
	tt.ClickAt(gripX, dividerY)
	tt.Frame()
	if s.panelWidth != PanelDefWidth || s.panelReset {
		t.Fatalf("双击恢复后面板宽 = %v (reset=%v), want %v", s.panelWidth, s.panelReset, PanelDefWidth)
	}
	// 几何确认：占位文本回到初始位置（view 真的重排了）。
	restored := panelBox(t)
	if diff := restored.X - at320.X; diff < -1 || diff > 1 {
		t.Errorf("双击恢复后占位文本 X = %v, want 约 %v", restored.X, at320.X)
	}
}
