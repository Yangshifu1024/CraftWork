package panel

// 面板多标签页状态库的单元测试。
//
// 用例逐条对齐 frontend/src/lib/sidePane.test.ts（27 个 it），覆盖 spec
// 第 1–4 节的核心规则：幂等打开、归属盖戳不可覆盖、可见性矩阵、关闭与
// 邻居激活、关闭其他/全部、最近关闭栈、线程清理、排序与 file 上限。

import (
	"reflect"
	"testing"
)

// 工作区 W1 的线程 T1 / T2 上下文，供大多数用例复用。
var (
	w1t1 = Scope{WorkspaceID: "w1", ThreadID: "t1"}
	w1t2 = Scope{WorkspaceID: "w1", ThreadID: "t2"}
	w2t1 = Scope{WorkspaceID: "w2", ThreadID: "t1"}
	w2t2 = Scope{WorkspaceID: "w2", ThreadID: "t2"}
)

// fileTab 造一个 file tab，路径即标题，时间戳固定便于断言。
func fileTab(workspaceID string, threadID string, path string, openedAt int64) Tab {
	return MakeFileTab(Input{WorkspaceID: workspaceID, ThreadID: threadID, Path: path, Title: path, OpenedAt: openedAt})
}

// stateWith 在初始状态上依次打开一组 tab。
func stateWith(t *testing.T, tabs []Tab, scope Scope) State {
	t.Helper()
	state := EmptySidePaneState()
	for _, tab := range tabs {
		state = OpenTab(state, tab, scope)
	}
	return state
}

// tabIDs 取一组 tab 的 id 列表，便于断言顺序。
func tabIDs(tabs []Tab) []string {
	out := make([]string, 0, len(tabs))
	for _, tab := range tabs {
		out = append(out, tab.ID)
	}
	return out
}

// tabTitles 取一组 tab 的标题列表，便于断言顺序。
func tabTitles(tabs []Tab) []string {
	out := make([]string, 0, len(tabs))
	for _, tab := range tabs {
		out = append(out, tab.Title)
	}
	return out
}

func TestFileTabIDComposition(t *testing.T) {
	if got := FileTabID("w1", "", "a.go"); got != "file:w1::a.go" {
		t.Errorf("FileTabID = %q", got)
	}
	if got := DocumentTabID("w1", "f1", "报告.docx"); got != "document:w1:f1:报告.docx" {
		t.Errorf("DocumentTabID = %q", got)
	}
}

// --- describe "openTab 幂等打开" -------------------------------------------------

func TestOpenTabAppendsAndActivates(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
	}, w1t1)
	if len(state.Tabs) != 2 {
		t.Fatalf("tabs = %d, want 2", len(state.Tabs))
	}
	if state.ActiveTabID != FileTabID("w1", "", "b.go") {
		t.Errorf("activeTabId = %q", state.ActiveTabID)
	}
}

func TestOpenTabIdempotentFocus(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
	}, w1t1)
	reopened := OpenTab(state, fileTab("w1", "t1", "a.go", 2000), w1t1)
	if len(reopened.Tabs) != 2 {
		t.Fatalf("重复打开产生了重复 tab: %d", len(reopened.Tabs))
	}
	if reopened.ActiveTabID != FileTabID("w1", "", "a.go") {
		t.Errorf("activeTabId = %q", reopened.ActiveTabID)
	}
	// 打开时间保留首开值，激活时间被刷新。
	if reopened.Tabs[0].OpenedAt != 1000 {
		t.Errorf("openedAt = %d, want 1000", reopened.Tabs[0].OpenedAt)
	}
	if reopened.Tabs[0].ActivatedAt <= 1000 {
		t.Errorf("activatedAt = %d, want > 1000", reopened.Tabs[0].ActivatedAt)
	}
}

func TestOpenTabRefreshesTitle(t *testing.T) {
	state := stateWith(t, []Tab{fileTab("w1", "t1", "a.go", 1000)}, w1t1)
	renamed := fileTab("w1", "t1", "a.go", 1000)
	renamed.Title = "新标题"
	after := OpenTab(state, renamed, w1t1)
	if after.Tabs[0].Title != "新标题" {
		t.Errorf("title = %q", after.Tabs[0].Title)
	}
}

func TestOpenTabSingletonFocus(t *testing.T) {
	state := stateWith(t, []Tab{
		MakeDiffTab(Input{WorkspaceID: "w1", ThreadID: "t1", OpenedAt: 1000}),
		MakeTerminalTab(Input{ThreadID: "t1", OpenedAt: 1000}),
	}, w1t1)
	again := OpenTab(state, MakeDiffTab(Input{WorkspaceID: "w1", ThreadID: "t1", OpenedAt: 2000}), w1t1)
	if n := countID(again.Tabs, DiffTabID); n != 1 {
		t.Errorf("diff tab 数量 = %d, want 1", n)
	}
	if again.ActiveTabID != DiffTabID {
		t.Errorf("activeTabId = %q", again.ActiveTabID)
	}
	if n := countID(again.Tabs, TerminalTabID); n != 1 {
		t.Errorf("terminal tab 数量 = %d, want 1", n)
	}
}

func countID(tabs []Tab, id string) int {
	n := 0
	for _, tab := range tabs {
		if tab.ID == id {
			n++
		}
	}
	return n
}

// --- describe "归属盖戳" ---------------------------------------------------------

func TestOpenTabStampsUnclaimed(t *testing.T) {
	state := OpenTab(EmptySidePaneState(), fileTab("w1", "", "a.go", 1000), w1t1)
	if state.Tabs[0].ThreadID != "t1" {
		t.Errorf("threadId = %q, want t1", state.Tabs[0].ThreadID)
	}
}

func TestOpenTabKeepsClaimedOwnership(t *testing.T) {
	state := stateWith(t, []Tab{fileTab("w1", "t1", "a.go", 1000)}, w1t1)
	reopened := OpenTab(state, fileTab("w1", "t1", "a.go", 1000), w1t2)
	if reopened.Tabs[0].ThreadID != "t1" {
		t.Errorf("已带归属被覆盖: %q", reopened.Tabs[0].ThreadID)
	}
}

func TestMarkTabsOwnershipOnlyUnclaimed(t *testing.T) {
	state := State{Tabs: []Tab{
		fileTab("w1", "", "a.go", 1000),
		fileTab("w1", "t2", "b.go", 1000),
	}}
	stamped := MarkTabsOwnership(state, w1t1)
	if stamped.Tabs[0].ThreadID != "t1" {
		t.Errorf("未认领 tab 未盖戳: %q", stamped.Tabs[0].ThreadID)
	}
	if stamped.Tabs[1].ThreadID != "t2" {
		t.Errorf("已认领 tab 被覆盖: %q", stamped.Tabs[1].ThreadID)
	}
	// 无未认领 tab 时返回原状态（no-op 约定）。
	again := MarkTabsOwnership(stamped, w1t1)
	if !reflect.DeepEqual(again, stamped) {
		t.Errorf("no-op 盖戳改变了状态")
	}
}

// --- describe "可见性" -----------------------------------------------------------

func TestVisibilityWorkspaceMismatch(t *testing.T) {
	tab := fileTab("w1", "t1", "a.go", 1000)
	if !IsTabVisibleInScope(tab, w1t1) {
		t.Errorf("同工作区同线程应可见")
	}
	if IsTabVisibleInScope(tab, w2t1) {
		t.Errorf("工作区不匹配应不可见")
	}
}

func TestVisibilityThreadMismatchAndDraft(t *testing.T) {
	tab := fileTab("w1", "t1", "a.go", 1000)
	if IsTabVisibleInScope(tab, w1t2) {
		t.Errorf("线程不匹配应不可见")
	}
	draftTab := fileTab("w1", "", "a.go", 1000)
	if !IsTabVisibleInScope(draftTab, Scope{WorkspaceID: "w1", ThreadID: "draft"}) {
		t.Errorf("草稿与草稿应匹配")
	}
}

func TestVisibilityTerminalIsPublic(t *testing.T) {
	terminal := MakeTerminalTab(Input{ThreadID: "t1", OpenedAt: 1000})
	if terminal.WorkspaceID != "" {
		t.Errorf("终端 workspaceId = %q, want 空（全局公共）", terminal.WorkspaceID)
	}
	if !IsTabVisibleInScope(terminal, w1t1) {
		t.Errorf("终端应跨线程可见")
	}
	if !IsTabVisibleInScope(terminal, w2t2) {
		t.Errorf("终端应跨工作区可见")
	}
}

func TestVisibilityDiffByThread(t *testing.T) {
	diff := MakeDiffTab(Input{WorkspaceID: "w1", ThreadID: "t1", OpenedAt: 1000})
	if !IsTabVisibleInScope(diff, w1t1) {
		t.Errorf("同线程差异 tab 应可见")
	}
	if IsTabVisibleInScope(diff, w1t2) {
		t.Errorf("差异 tab 换线程应不可见")
	}
}

func TestVisibleTabsAndActiveFallback(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t2", "b.go", 1000),
	}, w1t1)
	t1View := VisibleTabs(state, w1t1)
	if got := tabTitles(t1View); !reflect.DeepEqual(got, []string{"a.go"}) {
		t.Errorf("t1 可见 tab = %v", got)
	}
	// activeTabId 指向 t2 的 tab（切走前的遗留），兜底到 t1 的最后一个可见 tab。
	if got := VisibleActiveTabID(state, w1t1); got != FileTabID("w1", "", "a.go") {
		t.Errorf("visibleActiveTabId = %q", got)
	}
	if got := VisibleActiveTabID(EmptySidePaneState(), w1t1); got != "" {
		t.Errorf("空状态兜底 = %q, want 空", got)
	}
}

// --- describe "关闭与邻居激活" ---------------------------------------------------

func TestCloseTabActivatesRightNeighbor(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
		fileTab("w1", "t1", "c.go", 1000),
	}, w1t1)
	next, removed := CloseTab(state, FileTabID("w1", "", "b.go"))
	if len(removed) != 1 {
		t.Fatalf("removed = %d, want 1", len(removed))
	}
	if next.ActiveTabID != FileTabID("w1", "", "c.go") {
		t.Errorf("activeTabId = %q, want c.go", next.ActiveTabID)
	}
}

func TestCloseTabFallsBackToPrevious(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
	}, w1t1)
	next, _ := CloseTab(state, FileTabID("w1", "", "b.go"))
	if next.ActiveTabID != FileTabID("w1", "", "a.go") {
		t.Errorf("activeTabId = %q, want a.go", next.ActiveTabID)
	}
}

func TestCloseLastTabCollapses(t *testing.T) {
	state := stateWith(t, []Tab{fileTab("w1", "t1", "a.go", 1000)}, w1t1)
	next, _ := CloseTab(state, FileTabID("w1", "", "a.go"))
	if len(next.Tabs) != 0 {
		t.Fatalf("tabs = %d, want 0", len(next.Tabs))
	}
	if next.ActiveTabID != "" {
		t.Errorf("activeTabId = %q, want 空（面板收敛为收起）", next.ActiveTabID)
	}
}

func TestCloseInactiveTabKeepsActive(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
	}, w1t1)
	next, _ := CloseTab(state, FileTabID("w1", "", "a.go"))
	if next.ActiveTabID != FileTabID("w1", "", "b.go") {
		t.Errorf("activeTabId = %q, want b.go", next.ActiveTabID)
	}
}

func TestCloseOtherTabs(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
		fileTab("w1", "t2", "hidden.go", 1000),
	}, w1t1)
	next, removed := CloseOtherTabs(state, FileTabID("w1", "", "b.go"), w1t1)
	if got := tabTitles(removed); !reflect.DeepEqual(got, []string{"a.go"}) {
		t.Errorf("removed = %v", got)
	}
	if got := tabTitles(next.Tabs); !reflect.DeepEqual(got, []string{"b.go", "hidden.go"}) {
		t.Errorf("tabs = %v", got)
	}
	if next.ActiveTabID != FileTabID("w1", "", "b.go") {
		t.Errorf("activeTabId = %q", next.ActiveTabID)
	}
}

func TestCloseOtherTabsInvisibleTargetNoop(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t2", "b.go", 1000),
	}, w1t1)
	next, removed := CloseOtherTabs(state, FileTabID("w1", "", "b.go"), w1t1)
	if len(removed) != 0 {
		t.Errorf("removed = %d, want 0", len(removed))
	}
	if !reflect.DeepEqual(next, state) {
		t.Errorf("no-op 改变了状态")
	}
}

func TestCloseAllVisibleTabs(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t2", "hidden.go", 1000),
		MakeTerminalTab(Input{ThreadID: "t1", OpenedAt: 1000}),
	}, w1t1)
	next, removed := CloseAllVisibleTabs(state, w1t1)
	wantRemoved := []string{FileTabID("w1", "", "a.go"), TerminalTabID}
	if !reflect.DeepEqual(tabIDs(removed), wantRemoved) {
		t.Errorf("removed = %v, want %v", tabIDs(removed), wantRemoved)
	}
	if got := tabTitles(next.Tabs); !reflect.DeepEqual(got, []string{"hidden.go"}) {
		t.Errorf("tabs = %v", got)
	}
	// 激活项按邻居选择落到存活的隐藏 tab 上；渲染层会因可见性兜底为空态，
	// 切回 t2 时该 tab 自动恢复显示。
	if next.ActiveTabID != FileTabID("w1", "", "hidden.go") {
		t.Errorf("activeTabId = %q", next.ActiveTabID)
	}
	if got := VisibleActiveTabID(next, w1t1); got != "" {
		t.Errorf("t1 的可见激活项 = %q, want 空", got)
	}
}

func TestActivateTabRefreshesAndNoop(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
	}, w1t1)
	activated := ActivateTab(state, FileTabID("w1", "", "a.go"))
	if activated.ActiveTabID != FileTabID("w1", "", "a.go") {
		t.Errorf("activeTabId = %q", activated.ActiveTabID)
	}
	if activated.Tabs[0].ActivatedAt <= 1000 {
		t.Errorf("activatedAt = %d, want > 1000", activated.Tabs[0].ActivatedAt)
	}
	noop := ActivateTab(state, "missing")
	if !reflect.DeepEqual(noop, state) {
		t.Errorf("激活不存在的 tab 改变了状态")
	}
}

// --- describe "最近关闭栈" -------------------------------------------------------

func TestPushRecentClosedLimitAndDedup(t *testing.T) {
	state := EmptySidePaneState()
	for i := 0; i < RecentClosedLimit+2; i++ {
		state = PushRecentClosed(state, []Tab{fileTab("w1", "t1", "f"+string(rune('0'+i))+".go", 1000)})
	}
	if len(state.RecentClosed) != RecentClosedLimit {
		t.Fatalf("recentClosed = %d, want %d", len(state.RecentClosed), RecentClosedLimit)
	}
	if state.RecentClosed[0].Title != "f9.go" {
		t.Errorf("栈顶 = %q, want f9.go", state.RecentClosed[0].Title)
	}
	titles := tabTitles(state.RecentClosed)
	for _, title := range titles {
		if title == "f0.go" {
			t.Errorf("栈尾淘汰失效: %v", titles)
		}
	}
	deduped := PushRecentClosed(state, []Tab{fileTab("w1", "t1", "f9.go", 1000)})
	if n := countTitle(deduped.RecentClosed, "f9.go"); n != 1 {
		t.Errorf("f9.go 出现 %d 次, want 1", n)
	}
}

func countTitle(tabs []Tab, title string) int {
	n := 0
	for _, tab := range tabs {
		if tab.Title == title {
			n++
		}
	}
	return n
}

func TestTakeRecentClosedScopeCheck(t *testing.T) {
	state := PushRecentClosed(EmptySidePaneState(), []Tab{fileTab("w1", "t1", "a.go", 1000)})
	got, tab, ok := TakeRecentClosed(state, FileTabID("w1", "", "a.go"), w1t1)
	if !ok || tab.Title != "a.go" {
		t.Fatalf("取回失败: ok=%v tab=%v", ok, tab)
	}
	if len(got.RecentClosed) != 0 {
		t.Errorf("取回后栈未清空: %d", len(got.RecentClosed))
	}
	_, tab, ok = TakeRecentClosed(state, FileTabID("w1", "", "a.go"), w1t2)
	if ok || tab.ID != "" {
		t.Errorf("跨线程恢复未被拒绝: ok=%v tab=%v", ok, tab)
	}
}

func TestTakeRecentClosedTerminalPublic(t *testing.T) {
	state := PushRecentClosed(EmptySidePaneState(), []Tab{MakeTerminalTab(Input{ThreadID: "t1", OpenedAt: 1000})})
	_, tab, ok := TakeRecentClosed(state, TerminalTabID, w2t2)
	if !ok || tab.ID != TerminalTabID {
		t.Errorf("终端重开受限: ok=%v tab=%v", ok, tab)
	}
}

// --- describe "线程清理与排序、上限" ---------------------------------------------

func TestPruneThreadTabs(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		MakeDiffTab(Input{WorkspaceID: "w1", ThreadID: "t1", OpenedAt: 1000}),
		MakeTerminalTab(Input{ThreadID: "t1", OpenedAt: 1000}),
		fileTab("w1", "t2", "b.go", 1000),
	}, w1t1)
	next, removed := PruneThreadTabs(state, "t1")
	wantRemoved := []string{FileTabID("w1", "", "a.go"), DiffTabID}
	if !reflect.DeepEqual(tabIDs(removed), wantRemoved) {
		t.Errorf("removed = %v, want %v", tabIDs(removed), wantRemoved)
	}
	wantTabs := []string{TerminalTabID, FileTabID("w1", "", "b.go")}
	if !reflect.DeepEqual(tabIDs(next.Tabs), wantTabs) {
		t.Errorf("tabs = %v, want %v", tabIDs(next.Tabs), wantTabs)
	}
}

func TestReorderTabs(t *testing.T) {
	state := stateWith(t, []Tab{
		fileTab("w1", "t1", "a.go", 1000),
		fileTab("w1", "t1", "b.go", 1000),
		fileTab("w1", "t1", "c.go", 1000),
	}, w1t1)
	reordered := ReorderTabs(state, FileTabID("w1", "", "c.go"), FileTabID("w1", "", "a.go"))
	if got := tabTitles(reordered.Tabs); !reflect.DeepEqual(got, []string{"c.go", "a.go", "b.go"}) {
		t.Errorf("reordered = %v", got)
	}
	if reordered.ActiveTabID != state.ActiveTabID {
		t.Errorf("激活项被改变: %q", reordered.ActiveTabID)
	}
	if noop := ReorderTabs(state, "missing", "a.go"); !reflect.DeepEqual(noop, state) {
		t.Errorf("id 不存在时改变了状态")
	}
}

func TestEnforceFileTabLimit(t *testing.T) {
	// 直接拼接构造，避免 OpenTab 刷新 ActivatedAt 干扰淘汰排序；
	// OpenedAt 递增（1000..1008），未显式激活过时 ActivatedAt 等于 OpenedAt。
	state := EmptySidePaneState()
	for i := 0; i < 9; i++ {
		state.Tabs = append(state.Tabs, fileTab("w1", "t1", "f"+string(rune('0'+i))+".go", 1000+int64(i)))
	}
	// 激活 f0 刷新它的激活时间；此后从未激活且 OpenedAt 最小的是 f1，
	// 所以被淘汰的是 f1 而不是 OpenedAt 最大的 f8。
	state = ActivateTab(state, FileTabID("w1", "", "f0.go"))
	next, evicted := EnforceFileTabLimit(state, FileTabLimit)
	if got := tabTitles(evicted); !reflect.DeepEqual(got, []string{"f1.go"}) {
		t.Fatalf("evicted = %v, want [f1.go]", got)
	}
	if countTitle(next.Tabs, "f1.go") != 0 {
		t.Errorf("f1.go 未被删除")
	}
	if countTitle(next.Tabs, "f8.go") != 1 {
		t.Errorf("f8.go 丢失")
	}
	// 未超限时 no-op。
	if _, evicted := EnforceFileTabLimit(next, FileTabLimit); len(evicted) != 0 {
		t.Errorf("未超限仍淘汰: %v", tabTitles(evicted))
	}
}

func TestDocumentTabIdempotent(t *testing.T) {
	state := stateWith(t, []Tab{
		MakeDocumentTab(Input{WorkspaceID: "w1", ThreadID: "t1", Path: "报告.docx", Title: "报告.docx", Kind: "docx", OpenedAt: 1000}),
	}, w1t1)
	again := OpenTab(state, MakeDocumentTab(Input{WorkspaceID: "w1", ThreadID: "t1", Path: "报告.docx", Title: "报告.docx", Kind: "docx", OpenedAt: 2000}), w1t1)
	if len(again.Tabs) != 1 {
		t.Fatalf("tabs = %d, want 1", len(again.Tabs))
	}
	if again.Tabs[0].Type != TabDocument {
		t.Errorf("type = %q", again.Tabs[0].Type)
	}
}

func TestNormalizeOwner(t *testing.T) {
	for _, c := range []struct{ in, want string }{
		{"", DraftOwner},
		{"  ", DraftOwner},
		{"t1", "t1"},
	} {
		if got := NormalizeOwner(c.in); got != c.want {
			t.Errorf("NormalizeOwner(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}
