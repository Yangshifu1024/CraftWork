package sidebar

// 列表状态机的纯函数单测：分组归属与顺序、组内分页、折叠记忆、
// 批量折叠、状态映射与格式化。

import (
	"reflect"
	"testing"
	"time"
)

func mkThread(id, ws, status string, updated int64) Thread {
	return Thread{ID: id, Title: "会话 " + id, WorkspaceID: ws, Status: status, UpdatedAt: updated}
}

func testWorkspaces() []Workspace {
	return []Workspace{
		{ID: "conv", Name: "临时会话", Purpose: "conversation"},
		{ID: "ws-a", Name: "项目 A", Purpose: "project"},
		{ID: "ws-b", Name: "项目 B", Purpose: "project"},
	}
}

func TestNormalizeMode(t *testing.T) {
	for in, want := range map[string]Mode{
		"code": ModeCode, "work": ModeWork, "": ModeCode, "other": ModeCode,
	} {
		if got := NormalizeMode(in); got != want {
			t.Errorf("NormalizeMode(%q) = %v, want %v", in, got, want)
		}
	}
}

func TestGroupThreads(t *testing.T) {
	threads := []Thread{
		mkThread("t1", "conv", "idle", 1),
		mkThread("p1", "ws-a", "running", 2),
		mkThread("p2", "ws-b", "idle", 3),
		mkThread("o1", "gone", "error", 4), // 工作区已删除
		mkThread("o2", "", "idle", 5),      // 无绑定（历史语义）
		mkThread("t2", "conv", "awaiting", 6),
	}
	groups := GroupThreads(threads, testWorkspaces())
	if len(groups) != 4 {
		t.Fatalf("分组数 = %d, want 4（临时/两项目/其他）: %+v", len(groups), groups)
	}
	// 临时组置顶，包含两个临时会话。
	temp := groups[0]
	if temp.Kind != GroupTemp || temp.Key != "temp" || temp.Name != "临时会话" {
		t.Errorf("临时组头错误: %+v", temp)
	}
	if ids := idsOf(temp.Items); !reflect.DeepEqual(ids, []string{"t1", "t2"}) {
		t.Errorf("临时组成员 = %v, want [t1 t2]", ids)
	}
	// 项目组按工作区顺序，空组也保留组头（项目 B 若无会话仍存在）。
	if groups[1].Kind != GroupProject || groups[1].WorkspaceID != "ws-a" {
		t.Errorf("第一项目组错误: %+v", groups[1])
	}
	if ids := idsOf(groups[1].Items); !reflect.DeepEqual(ids, []string{"p1"}) {
		t.Errorf("项目 A 成员 = %v, want [p1]", ids)
	}
	if ids := idsOf(groups[2].Items); !reflect.DeepEqual(ids, []string{"p2"}) {
		t.Errorf("项目 B 成员 = %v, want [p2]", ids)
	}
	// 孤儿组垫底：绑定缺失与已删除工作区的会话都归入，不凭空消失。
	orphan := groups[3]
	if orphan.Kind != GroupOrphan || orphan.Name != "其他" {
		t.Errorf("孤儿组头错误: %+v", orphan)
	}
	if ids := idsOf(orphan.Items); !reflect.DeepEqual(ids, []string{"o1", "o2"}) {
		t.Errorf("孤儿组成员 = %v, want [o1 o2]", ids)
	}
}

func TestGroupThreadsWithoutWorkspaces(t *testing.T) {
	// 没有任何工作区时：全部会话进孤儿组，临时组仍在但为空。
	groups := GroupThreads([]Thread{mkThread("o1", "", "idle", 1)}, nil)
	if len(groups) != 2 {
		t.Fatalf("分组数 = %d, want 2: %+v", len(groups), groups)
	}
	if len(groups[0].Items) != 0 {
		t.Errorf("临时组应为空: %+v", groups[0])
	}
	if ids := idsOf(groups[1].Items); !reflect.DeepEqual(ids, []string{"o1"}) {
		t.Errorf("孤儿组成员 = %v, want [o1]", ids)
	}
}

func idsOf(items []Thread) []string {
	ids := make([]string, 0, len(items))
	for _, it := range items {
		ids = append(ids, it.ID)
	}
	return ids
}

func TestProjectThreadCount(t *testing.T) {
	threads := []Thread{
		mkThread("t1", "conv", "idle", 1),
		mkThread("p1", "ws-a", "idle", 2),
		mkThread("p2", "ws-a", "idle", 3),
		mkThread("o1", "gone", "idle", 4),
	}
	count := ProjectThreadCount(GroupThreads(threads, testWorkspaces()))
	if count != 2 {
		t.Errorf("项目会话总数 = %d, want 2（只数项目组，孤儿单独展示）", count)
	}
}

func TestVisibleItemsPagination(t *testing.T) {
	items := make([]Thread, 12)
	for i := range items {
		items[i] = mkThread("th"+itoaTest(i), "", "idle", int64(i))
	}
	// 首页 5 条，还有剩余。
	visible, hasMore := VisibleItems(items, 0)
	if len(visible) != PageSize || !hasMore {
		t.Errorf("首页 = %d 条 (hasMore=%v), want 5 条且还有剩余", len(visible), hasMore)
	}
	// 追加一页到 10 条。
	if got := NextLimit(PageSize); got != 10 {
		t.Errorf("NextLimit = %d, want 10", got)
	}
	visible, hasMore = VisibleItems(items, 10)
	if len(visible) != 10 || !hasMore {
		t.Errorf("第二页 = %d 条 (hasMore=%v), want 10 条且还有剩余", len(visible), hasMore)
	}
	// 越过全长时按全长截断且无更多。
	visible, hasMore = VisibleItems(items, 99)
	if len(visible) != 12 || hasMore {
		t.Errorf("越界 limit = %d 条 (hasMore=%v), want 全部 12 条且无剩余", len(visible), hasMore)
	}
	// 空组。
	visible, hasMore = VisibleItems(nil, 0)
	if len(visible) != 0 || hasMore {
		t.Errorf("空组应无可见条目")
	}
}

// itoaTest 是测试专用的下标转字符串。
func itoaTest(i int) string { return string(rune('a' + i)) }

func TestCollapseToggle(t *testing.T) {
	c := NewCollapses()
	if c.IsCollapsed("ws-a") {
		t.Errorf("初始应全展开")
	}
	c.Collapsed = c.Toggle("ws-a")
	if !c.IsCollapsed("ws-a") || c.IsCollapsed("ws-b") {
		t.Errorf("翻转后 ws-a 应折叠、ws-b 不受影响")
	}
	c.Collapsed = c.Toggle("ws-a")
	if c.IsCollapsed("ws-a") {
		t.Errorf("再次翻转应恢复展开")
	}
}

func TestCollapseAllProjects(t *testing.T) {
	groups := []Group{
		{Kind: GroupTemp, Key: "temp"},
		{Kind: GroupProject, Key: "ws-a"},
		{Kind: GroupProject, Key: "ws-b"},
		{Kind: GroupOrphan, Key: "orphan"},
	}
	c := NewCollapses()
	if c.AllProjectsCollapsed(groups) {
		t.Errorf("初始不应视为全部折叠")
	}
	// 只折一组不算全部折叠。
	c.Collapsed = c.Toggle("ws-a")
	if c.AllProjectsCollapsed(groups) {
		t.Errorf("仅折叠一组不应视为全部折叠")
	}
	// 批量折叠：所有项目组折叠，临时组与孤儿组不受影响。
	c.Collapsed = c.ToggleAllProjects(groups)
	if !c.AllProjectsCollapsed(groups) {
		t.Errorf("批量折叠后应视为全部折叠")
	}
	if !c.IsCollapsed("ws-a") || !c.IsCollapsed("ws-b") {
		t.Errorf("批量折叠应覆盖全部项目组")
	}
	if c.IsCollapsed("temp") || c.IsCollapsed("orphan") {
		t.Errorf("临时组与孤儿组不应被批量折叠波及")
	}
	// 再点一次（全部折叠态）：清空项目组折叠表。
	c.Collapsed = c.ToggleAllProjects(groups)
	if c.AllProjectsCollapsed(groups) {
		t.Errorf("全部展开后不应视为全部折叠")
	}
	if c.IsCollapsed("ws-a") || c.IsCollapsed("ws-b") {
		t.Errorf("批量展开应清空项目组折叠登记")
	}
}

func TestPaginationNotResetByCollapse(t *testing.T) {
	// 折叠再展开不重置已展开条数（React 版 limit 是组维度记忆）。
	s := NewSidebar()
	s.ReplaceWorkspaces(testWorkspaces())
	items := make([]Thread, 8)
	for i := range items {
		items[i] = mkThread("p"+itoaTest(i), "ws-a", "idle", int64(i))
	}
	s.ReplaceThreads(ModeCode, items)
	s.ShowMore("ws-a")
	if s.Limit("ws-a") != 10 {
		t.Fatalf("展开后游标 = %d, want 10", s.Limit("ws-a"))
	}
	s.coll.Collapsed = s.coll.Toggle("ws-a")
	s.coll.Collapsed = s.coll.Toggle("ws-a")
	if s.Limit("ws-a") != 10 {
		t.Errorf("折叠再展开后游标 = %d, want 仍为 10", s.Limit("ws-a"))
	}
}

func TestStatusOf(t *testing.T) {
	cases := map[string]Status{
		"running": StatusRunning, "awaiting": StatusAwaiting,
		"error": StatusError, "idle": StatusIdle, "": StatusIdle, "junk": StatusIdle,
	}
	for in, want := range cases {
		if got := StatusOf(in); got != want {
			t.Errorf("StatusOf(%q) = %v, want %v", in, got, want)
		}
	}
}

// timeFixedZone 构造固定时区，让时间格式化用例不随机器时区漂移。
func timeFixedZone() *time.Location {
	return time.FixedZone("test", 8*3600)
}

// timeInZone 构造指定时区下的本地时刻。
func timeInZone(year int, month time.Month, day, hour, minute int, loc *time.Location) time.Time {
	return time.Date(year, month, day, hour, minute, 0, 0, loc)
}

func TestFormatTime(t *testing.T) {
	loc := timeFixedZone()
	now := timeInZone(2026, 11, 3, 21, 5, loc).UnixMilli()
	sameDay := timeInZone(2026, 11, 3, 9, 41, loc).UnixMilli()
	if got := FormatTime(sameDay, now); got != "09:41" {
		t.Errorf("当天时间 = %q, want 09:41", got)
	}
	otherDay := timeInZone(2026, 10, 26, 8, 7, loc).UnixMilli()
	if got := FormatTime(otherDay, now); got != "10/26 08:07" {
		t.Errorf("跨天时间 = %q, want 10/26 08:07", got)
	}
	if got := FormatTime(0, now); got != "" {
		t.Errorf("零值 = %q, want 空", got)
	}
}

func TestFormatTokens(t *testing.T) {
	cases := map[int64]string{
		0: "0", 999: "999", 1000: "1.0k", 1234: "1.2k", 9999: "9.9k",
		10000: "10k", 128000: "128k", -5: "0",
	}
	for in, want := range cases {
		if got := FormatTokens(in); got != want {
			t.Errorf("FormatTokens(%d) = %q, want %q", in, got, want)
		}
	}
}

func TestSidebarModeIsolation(t *testing.T) {
	s := NewSidebar()
	s.ReplaceWorkspaces(testWorkspaces())
	s.ReplaceThreads(ModeCode, []Thread{mkThread("c1", "conv", "idle", 1)})
	s.ReplaceThreads(ModeWork, []Thread{mkThread("w1", "conv", "idle", 2)})
	s.Select("c1")
	s.SetMode(ModeWork)
	if s.ActiveID() != "" {
		t.Errorf("切换模式后活动会话应互不影响，got %q", s.ActiveID())
	}
	if ids := idsOf(s.Groups()[0].Items); !reflect.DeepEqual(ids, []string{"w1"}) {
		t.Errorf("Work 模式列表 = %v, want [w1]", ids)
	}
	s.SetMode(Mode("junk"))
	if s.Mode != ModeCode {
		t.Errorf("未知模式应收敛为 Code, got %v", s.Mode)
	}
}
