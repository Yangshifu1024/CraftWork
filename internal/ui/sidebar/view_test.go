package sidebar

// 侧栏 view 的 ui.Tester 走查：分组渲染、模式切换、组内分页、折叠、
// 右键菜单与底部两行。Actions 注入本地记录函数（不接服务），
// 同路径也覆盖单测回退语义。

import (
	"testing"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// settleClick 是点击后的统一等待：推进一帧再加一个动效时长，让
// 滑块/竖条动画播完、点击状态稳定（只 Frame 会与连击检测打架）。
func settleClick(tt *mygo.Tester) {
	tt.Frame()
	tt.Advance(theme.Duration + 20*time.Millisecond)
	tt.Frame()
}

// renamedRecord 收集重命名动作的目标与标题（右键菜单 → Prompt 提交）。
type renamedRecord struct {
	id    string
	title string
}

// newSidebarTester 在 1440×900 下起一个注入了数据与本地动作的侧栏 view。
// 返回值里的 copied / deleted / renamed 收集右键菜单动作的目标会话 Id
// 与重命名结果（注入的本地记录函数）。
func newSidebarTester(t *testing.T) (*Sidebar, *[]string, *[]string, *[]renamedRecord, *mygo.Tester) {
	t.Helper()
	s := NewSidebar()
	s.ReplaceWorkspaces(testWorkspaces())
	s.ReplaceThreads(ModeCode, []Thread{
		{ID: "t1", Title: "临时会话一", WorkspaceID: "conv", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "p1", Title: "项目会话一", WorkspaceID: "ws-a", Status: "running", UpdatedAt: theme.Now()},
		{ID: "p2", Title: "项目会话二", WorkspaceID: "ws-a", Status: "awaiting", UpdatedAt: theme.Now()},
		{ID: "p3", Title: "项目会话三", WorkspaceID: "ws-a", Status: "error", UpdatedAt: theme.Now()},
		{ID: "p4", Title: "项目会话四", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "p5", Title: "项目会话五", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "p6", Title: "项目会话六", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "o1", Title: "孤儿会话", WorkspaceID: "gone", Status: "idle", UpdatedAt: theme.Now()},
	})
	s.ReplaceThreads(ModeWork, []Thread{
		{ID: "w1", Title: "工作会话", WorkspaceID: "conv", Status: "idle", UpdatedAt: theme.Now()},
	})
	// 本地动作记录：断言右键菜单到动作的接线。
	var copied, deleted []string
	var renamed []renamedRecord
	s.SetActions(&Actions{
		CopyThreadID: func(id string) { copied = append(copied, id) },
		RenameThread: func(id, title string) { renamed = append(renamed, renamedRecord{id, title}) },
		DeleteThread: func(id string) { deleted = append(deleted, id) },
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
		// 重命名 Prompt 由右键菜单直接发起，浮层画在这里（生产走 RootView）。
		overlays.View(c)
	}
	tt := mygo.NewTester(view, 1440, 900)
	return s, &copied, &deleted, &renamed, tt
}

// mustClick 点一个有文字的元素，失败直接终止用例。
func mustClick(t *testing.T, tt *mygo.Tester, label string) {
	t.Helper()
	if err := tt.Click(label); err != nil {
		t.Fatalf("点击 %q 失败: %v", label, err)
	}
}

// mustChoose 在右键菜单里选一项，失败直接终止用例。
func mustChoose(t *testing.T, tt *mygo.Tester, item string) {
	t.Helper()
	if err := tt.ChooseMenuItem(item); err != nil {
		t.Fatalf("右键菜单选 %q 失败: %v（菜单 %v）", item, err, tt.Menu())
	}
}

func TestSidebarViewShowsGroupsAndItems(t *testing.T) {
	_, _, _, _, tt := newSidebarTester(t)
	for _, text := range []string{
		"Code", "Work",
		"临时会话", "临时会话一",
		"项目会话", "项目 A",
		"其他", "孤儿会话",
		"用量", "审计", "命令面板", "设置", "⌘K", "⌘,",
	} {
		if !tt.HasText(text) {
			t.Errorf("侧栏缺文本 %q，实有 %v", text, tt.Texts())
		}
	}
	// 组内分页：项目 A 有 6 条，默认只显示 5 条，有「显示更多」。
	if !tt.HasText("项目会话五") {
		t.Errorf("第 5 条应在首页可见: %v", tt.Texts())
	}
	if tt.HasText("项目会话六") {
		t.Errorf("第 6 条不应在首页可见")
	}
	if !tt.HasText("显示更多（还有 1 条）") {
		t.Errorf("缺「显示更多」按钮: %v", tt.Texts())
	}
}

func TestSidebarViewShowMoreAppends(t *testing.T) {
	_, _, _, _, tt := newSidebarTester(t)
	mustClick(t, tt, "显示更多（还有 1 条）")
	settleClick(tt)
	if !tt.HasText("项目会话六") {
		t.Errorf("点击后第 6 条应可见: %v", tt.Texts())
	}
	if tt.HasText("显示更多") {
		t.Errorf("全部展开后「显示更多」应消失")
	}
}

func TestSidebarViewSelectThread(t *testing.T) {
	s, _, _, _, tt := newSidebarTester(t)
	mustClick(t, tt, "临时会话一")
	if s.ActiveID() != "t1" {
		t.Errorf("点击后活动会话 = %q, want t1", s.ActiveID())
	}
}

func TestSidebarViewSwitchMode(t *testing.T) {
	s, _, _, _, tt := newSidebarTester(t)
	mustClick(t, tt, "Work")
	settleClick(tt)
	if s.Mode != ModeWork {
		t.Fatalf("点击后模式 = %v, want work", s.Mode)
	}
	// Work 模式列表替换：临时组只剩 w1，项目组为空（空项目组保留组头，
	// 对齐 React 版：组头常驻，+ 号仍是创建入口）。
	if !tt.HasText("工作会话") {
		t.Errorf("Work 列表应显示工作会话: %v", tt.Texts())
	}
	if tt.HasText("项目会话一") {
		t.Errorf("Work 列表不应显示 Code 会话")
	}
	if !tt.HasText("项目 A") || !tt.HasText("项目 B") {
		t.Errorf("空项目组的组头应保留: %v", tt.Texts())
	}
	// 切回 Code：列表恢复。
	mustClick(t, tt, "Code")
	settleClick(tt)
	if s.Mode != ModeCode {
		t.Fatalf("切回后模式 = %v, want code", s.Mode)
	}
	if !tt.HasText("项目会话五") {
		t.Errorf("切回 Code 应恢复列表: %v", tt.Texts())
	}
}

func TestSidebarViewEmptyProjectState(t *testing.T) {
	// 没有任何项目工作区与孤儿会话时：项目区显示空态文案（React 版语义）。
	s := NewSidebar()
	s.ReplaceWorkspaces([]Workspace{{ID: "conv", Name: "临时会话", Purpose: "conversation"}})
	s.ReplaceThreads(ModeCode, nil)
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()
	if !tt.HasText("还没有项目\n点上方 + 创建项目") {
		t.Errorf("缺项目区空态文案: %v", tt.Texts())
	}
}

func TestSidebarViewCollapseTempGroup(t *testing.T) {
	_, _, _, _, tt := newSidebarTester(t)
	head, ok := tt.Find("临时会话") // 组名文本（与「临时会话一」是不同元素）
	if !ok {
		t.Fatalf("找不到临时组头文本")
	}
	// 折叠开关是组名左侧的 18×18 小按钮：中心在文本左间距 - 9 处，行内居中。
	toggleX := head.X - theme.Space3 - 9
	toggleY := head.Y + head.H/2
	tt.ClickAt(toggleX, toggleY)
	settleClick(tt)
	if tt.HasText("临时会话一") {
		t.Errorf("折叠后条目应隐藏: %v", tt.Texts())
	}
	// 再点一次展开。
	tt.ClickAt(toggleX, toggleY)
	settleClick(tt)
	if !tt.HasText("临时会话一") {
		t.Errorf("再次点击应展开: %v", tt.Texts())
	}
}

func TestSidebarViewContextMenuItems(t *testing.T) {
	_, copied, deleted, renamed, tt := newSidebarTester(t)
	rect, ok := tt.Find("临时会话一")
	if !ok {
		t.Fatalf("找不到会话条目")
	}
	// 复制会话 Id：动作记录到本地注入的函数（生产实现经 win.Update 排队，
	// 帧内直调剪贴板会死锁，见 client.go CopyThreadID）。
	tt.RightClickAt(rect.X+10, rect.Y+5)
	tt.Frame()
	mustChoose(t, tt, "复制会话 Id")
	settleClick(tt)
	if len(*copied) != 1 || (*copied)[0] != "t1" {
		t.Errorf("复制动作应记录 t1, got %v", *copied)
	}

	// 重命名菜单发起应用内 Prompt（07 号票浮层体系）：标题预填当前值。
	tt.RightClickAt(rect.X+10, rect.Y+5)
	tt.Frame()
	mustChoose(t, tt, "重命名")
	tt.Frame()
	if !tt.HasText("重命名会话") {
		t.Fatalf("Prompt 浮层应出现: %v", tt.Texts())
	}
	// 焦点默认在输入框：清掉预填标题，输入新标题后回车提交。
	for range "临时会话一" {
		tt.Key(0, mygo.KeyBackspace)
	}
	tt.Type("新名字")
	tt.Key(0, mygo.KeyEnter)
	tt.Frame()
	if len(*renamed) != 1 || (*renamed)[0].id != "t1" || (*renamed)[0].title != "新名字" {
		t.Fatalf("提交应重命名 t1 为新名字, got %+v", *renamed)
	}
	if tt.HasText("重命名会话") {
		t.Errorf("提交后 Prompt 应关闭")
	}

	// 删除菜单项走注入动作记录。
	rect, _ = tt.Find("临时会话一")
	tt.RightClickAt(rect.X+10, rect.Y+5)
	tt.Frame()
	mustChoose(t, tt, "删除")
	settleClick(tt)
	if len(*deleted) != 1 || (*deleted)[0] != "t1" {
		t.Errorf("删除动作应记录 t1, got %v", *deleted)
	}
}

func TestSidebarViewToastQueue(t *testing.T) {
	s := NewSidebar()
	if s.TakeToast() != "" {
		t.Errorf("初始不应有待弹提示")
	}
	s.QueueToast("会话已删除")
	if got := s.TakeToast(); got != "会话已删除" {
		t.Errorf("TakeToast = %q, want 会话已删除", got)
	}
	if s.TakeToast() != "" {
		t.Errorf("取走后应清空")
	}
}
