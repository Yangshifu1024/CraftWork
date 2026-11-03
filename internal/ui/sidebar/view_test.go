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

// TestSidebarViewGroupHeadSpacing 校验全折叠时的组头垂直节奏（13 号票）：
// 相邻两个组头（「项目会话」区头 → 项目组头）之间的文本 Y 差应等于
// 区头底边距 + 组头顶边距 + 一行行高的居中偏移，即 25 + groupHeadPad.t。
// 组头顶边距在 base.css 是 12px，此前误算成 22px，全折叠时组间距因此多出 10。
func TestSidebarViewGroupHeadSpacing(t *testing.T) {
	s, _, _, _, tt := newSidebarTester(t)
	// 折叠临时组与全部项目组：列表只剩三个组头。
	s.ToggleTemp()
	s.ToggleAllProjects()
	tt.Frame()
	head, ok := tt.Find("项目会话")
	if !ok {
		t.Fatalf("找不到「项目会话」区头: %v", tt.Texts())
	}
	group, ok := tt.Find("项目 A")
	if !ok {
		t.Fatalf("折叠后应仍见「项目 A」组头: %v", tt.Texts())
	}
	// 区头与组头都 AlignItems 居中，两行文本的 Y 差 = 一行内容高（20，
	// 组头最高子元素是 20×20 的 + 按钮）拆开后的 25 + 组头顶边距。
	delta := group.Y - head.Y
	want := float32(25) + theme.Space6 // 12：base.css .thread-group-head 顶边距
	if delta < want-2 || delta > want+2 {
		t.Errorf("全折叠时区头到项目组的文本间距 = %.1f, want %.0f±2（组头顶边距应为 12）", delta, want)
	}
}

// TestSidebarViewTempEmptyRowHeight 校验空分组空态行的行高（13 号票）：
// 空态文案顶部与组头文本的距离应等于组头剩余高度 + 空态顶边距 26
// （base.css .thread-empty 的 padding 26px 16px）。此前顶边距误算成 36、
// 组头顶边距误算成 22，空态行整体下坠。
func TestSidebarViewTempEmptyRowHeight(t *testing.T) {
	s := NewSidebar()
	s.ReplaceWorkspaces([]Workspace{{ID: "conv", Name: "临时会话", Purpose: "conversation"}})
	s.ReplaceThreads(ModeCode, nil)
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()
	head, ok := tt.Find("临时会话")
	if !ok {
		t.Fatalf("找不到临时组头: %v", tt.Texts())
	}
	empty, ok := tt.Find("还没有临时会话，点上方 + 新建")
	if !ok {
		t.Fatalf("找不到临时组空态文案: %v", tt.Texts())
	}
	// 组头行高拆分：组头底边到组名顶 ≈ 25 - 组名半行居中偏移 ≈ 22.65，
	// 再接空态顶边距 26（base.css .thread-empty 的 26px）。
	delta := empty.Y - head.Y
	want := 22.65 + theme.Space7 + theme.Space4 + theme.Space1
	if delta < want-3 || delta > want+3 {
		t.Errorf("临时组头到空态文案的间距 = %.1f, want %.1f±3（空态顶边距应为 26）", delta, want)
	}
}

// newBadgeTester 起一个 badge 数字互不相同的侧栏（临时组 2 条、项目 A
// 3 条、项目 B 1 条，区头总数 4），供计数徽章位置断言用。
func newBadgeTester(t *testing.T) (*Sidebar, *mygo.Tester) {
	t.Helper()
	s := NewSidebar()
	s.ReplaceWorkspaces(testWorkspaces())
	s.ReplaceThreads(ModeCode, []Thread{
		{ID: "t1", Title: "临时会话一", WorkspaceID: "conv", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "t2", Title: "临时会话二", WorkspaceID: "conv", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "p1", Title: "项目会话一", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "p2", Title: "项目会话二", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "p3", Title: "项目会话三", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
		{ID: "q1", Title: "项目 B 会话", WorkspaceID: "ws-b", Status: "idle", UpdatedAt: theme.Now()},
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	return s, mygo.NewTester(view, 1440, 900)
}

// TestSidebarViewCountBadgeFollowsName 校验组头计数徽章紧跟分组文字
// （13 号票）：徽章文本与组名的间距应等于组头 6px 间距 + 徽章自身的
// 8px 水平内边距，而不是被组名的 Grow 推到最右贴着 + 按钮。
func TestSidebarViewCountBadgeFollowsName(t *testing.T) {
	_, tt := newBadgeTester(t)
	tt.Frame()
	name, ok := tt.Find("项目 A")
	if !ok {
		t.Fatalf("找不到项目组名: %v", tt.Texts())
	}
	badge, ok := tt.Find("3")
	if !ok {
		t.Fatalf("找不到项目 A 的计数徽章: %v", tt.Texts())
	}
	delta := badge.X - (name.X + name.W)
	want := theme.Space3 + theme.Space4 // 组头 gap 6 + 徽章内边距 8
	if delta < want-2 || delta > want+2 {
		t.Errorf("计数徽章与组名的间距 = %.1f, want %.0f±2（徽章应紧跟组名）", delta, want)
	}
}

// TestSidebarViewLongGroupNameClamps 校验超长项目名在去掉 Grow 后仍被
// 单行截断（.thread-group-name 的 min-width:0 + ellipsis），不把计数
// 徽章与 + 按钮推出侧栏可视区。
func TestSidebarViewLongGroupNameClamps(t *testing.T) {
	s := NewSidebar()
	longName := "一个特别特别长的项目名称用来验证组名截断行为是否正确"
	s.ReplaceWorkspaces([]Workspace{
		{ID: "conv", Name: "临时会话", Purpose: "conversation"},
		{ID: "ws-a", Name: longName, Purpose: "project"},
	})
	s.ReplaceThreads(ModeCode, []Thread{
		{ID: "p1", Title: "项目会话一", WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()
	name, ok := tt.Find(longName)
	if !ok {
		t.Fatalf("找不到长组名: %v", tt.Texts())
	}
	badge, ok := tt.Find("1")
	if !ok {
		t.Fatalf("找不到计数徽章: %v", tt.Texts())
	}
	// 组名应被截断到行内（远短于自然宽），徽章右缘留在侧栏可视区内。
	if name.W > threadTitleWidth() {
		t.Errorf("长组名应被截断, 宽 %.1f 超过行可用宽 %.1f", name.W, threadTitleWidth())
	}
	if right := badge.X + badge.W; right > theme.SidebarWidth-theme.Space4-4 {
		t.Errorf("徽章右缘 %.1f 应留在侧栏内（列表右内边距 %.0f 之前）", right, theme.Space4+4)
	}
}

// longTitle 是超过一行可用宽度的会话标题（15px 字号下约 340px 宽）。
const longTitle = "解释这个仓库的整体架构与模块划分，从入口开始讲"

// TestSidebarViewItemTitleTwoRows 校验会话行的两行布局（13 号票）：
// 标题在上、时间与状态在下（.thread-item 的 flex-direction: column），
// 且标题列占满行内容宽。此前标题与时间行被 ButtonBase 的 Row 语义排成
// 一行，标题只能拿时间行之外的剩余宽度，截断远早于旧版。
func TestSidebarViewItemTitleTwoRows(t *testing.T) {
	s := NewSidebar()
	s.ReplaceWorkspaces(testWorkspaces())
	s.ReplaceThreads(ModeCode, []Thread{
		{ID: "p1", Title: longTitle, WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()
	title, ok := tt.Find(longTitle)
	if !ok {
		t.Fatalf("找不到长标题: %v", tt.Texts())
	}
	idle, ok := tt.Find("已完成")
	if !ok {
		t.Fatalf("找不到状态文本: %v", tt.Texts())
	}
	if title.Y+title.H >= idle.Y {
		t.Errorf("标题底 %.1f 应在状态行顶 %.1f 之上（两行布局）", title.Y+title.H, idle.Y)
	}
	// 标题列占满行内容宽：268 - 列表左右 12×2 - 行左右 5×2 - 描边 2。
	wantW := threadTitleWidth()
	if title.W < wantW-40 {
		t.Errorf("标题宽 %.1f 应接近可用宽 %.1f（截断放宽）", title.W, wantW)
	}
}

// TestSidebarViewMarqueeHover 冒烟校验标题跑马灯（13 号票）：悬停长标题
// 推进超过两个滚动周期，view 不崩、标题仍渲染；ReduceMotion 下同样悬停
// 不滚动也不崩（装饰性循环动效在减少动效偏好下停用，AGENTS.md 6.4）。
func TestSidebarViewMarqueeHover(t *testing.T) {
	s := NewSidebar()
	s.ReplaceWorkspaces(testWorkspaces())
	s.ReplaceThreads(ModeCode, []Thread{
		{ID: "p1", Title: longTitle, WorkspaceID: "ws-a", Status: "idle", UpdatedAt: theme.Now()},
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1440, 900)
	tt.Frame()
	title, ok := tt.Find(longTitle)
	if !ok {
		t.Fatalf("找不到长标题: %v", tt.Texts())
	}
	tt.Move(title.X+10, title.Y+title.H/2)
	tt.Frame()
	// 直接推进 30s，覆盖任何溢出量对应的滚动周期两轮以上。
	tt.Advance(30 * time.Second)
	tt.Frame()
	if !tt.HasText(longTitle) {
		t.Errorf("跑马灯滚动后标题应仍渲染")
	}

	// ReduceMotion：悬停同样推进，不应崩（Loop 不启动）。
	tt.SetPreferences(mygo.Preferences{ReduceMotion: true})
	tt.Move(title.X+10, title.Y+title.H/2)
	tt.Frame()
	tt.Advance(30 * time.Second)
	tt.Frame()
	if !tt.HasText(longTitle) {
		t.Errorf("ReduceMotion 下标题应仍渲染")
	}
}
