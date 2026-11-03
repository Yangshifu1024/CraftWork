// toggle_click_test 锁定区头折叠按钮的点击链路（用户报「点不动」的回归：
// 空区点击此前走 ToggleAllProjects 零反馈；按钮可访问名此前只有 Tooltip
// 不进元素树，Tester 无法定位——Label 补齐后可定位可点击）。
package sidebar

import (
	"testing"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

func TestProjectSectionToggleClick(t *testing.T) {
	s := NewSidebar()
	s.ReplaceWorkspaces([]Workspace{
		{ID: "conv", Name: "临时会话", Purpose: "conversation"},
		{ID: "ws-a", Name: "测试项目", Purpose: "project"},
	})
	s.ReplaceThreads(ModeCode, []Thread{
		{ID: "p1", Title: "会话一", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()

	if !tt.HasText("会话一") {
		t.Fatalf("初始项目组应展开: %v", tt.Texts())
	}
	// 点击区头折叠按钮：项目组收起。
	if err := tt.Click("折叠所有项目"); err != nil {
		t.Fatalf("点击折叠按钮失败: %v", err)
	}
	tt.Frame()
	if tt.HasText("会话一") {
		t.Fatal("点击折叠按钮后项目组应收起（会话一应消失）")
	}
	// 再点展开按钮：项目组恢复。
	if err := tt.Click("展开所有项目"); err != nil {
		t.Fatalf("点击展开按钮失败: %v", err)
	}
	tt.Frame()
	if !tt.HasText("会话一") {
		t.Fatal("再次点击应展开项目组")
	}
	_ = mygo.Text
	_ = theme.Fonts
}

// TestProjectSectionEmptyToggleClick 空区（无项目也无孤儿组）点击折叠
// 按钮：折叠/展开空态文案本身（此前点击走 ToggleAllProjects 零反馈，
// 用户感知「点不动」）。
func TestProjectSectionEmptyToggleClick(t *testing.T) {
	s := NewSidebar()
	s.ReplaceWorkspaces([]Workspace{{ID: "conv", Name: "临时会话", Purpose: "conversation"}})
	s.ReplaceThreads(ModeCode, nil)
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()

	if !tt.HasText("还没有项目") {
		t.Fatalf("空区空态应可见: %v", tt.Texts())
	}
	if err := tt.Click("折叠项目会话"); err != nil {
		t.Fatalf("点击折叠按钮失败: %v", err)
	}
	tt.Frame()
	if tt.HasText("还没有项目") {
		t.Fatal("空区点击折叠后空态文案应收起")
	}
	if err := tt.Click("展开项目会话"); err != nil {
		t.Fatalf("点击展开按钮失败: %v", err)
	}
	tt.Frame()
	if !tt.HasText("还没有项目") {
		t.Fatal("再次点击应展开空态文案")
	}
}
