package ui

// 面板编排层（Pane）的单测：分桶记忆、终端全局开关、末 tab 收起、file
// tab 上限淘汰，以及接入真实 WorkspaceService 的读取 / 保存链路（备份
// 落盘、定位行跳转）。tab 状态机的纯函数用例在 internal/ui/panel 包，
// 逐条对齐 sidePane.test.ts。

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/egoist/mygo/ui/editor"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/store"
	cwpanel "github.com/yangshifu1024/craftwork/internal/ui/panel"
)

// paneHarness 是编排层测试的脚手架：Workspace 提供器可切换工作区，
// Collapse 记录收起次数，updateFeed 把后台 goroutine 的回主线程调用按
// 顺序在测试侧消费（channel 同步建立 happens-before，读状态无竞争）。
type paneHarness struct {
	pane *Pane
	// root 是服务脚手架的工作区目录（仅真实服务用例使用）。
	root       string
	collapseN  int
	workspace  func() *service.WorkspaceView
	updateFeed chan func()
	pumpDone   chan struct{}
}

func newPaneHarness(t *testing.T, wsID string) *paneHarness {
	t.Helper()
	h := &paneHarness{
		updateFeed: make(chan func(), 64),
		pumpDone:   make(chan struct{}),
	}
	h.workspace = func() *service.WorkspaceView {
		return &service.WorkspaceView{ID: wsID, Name: wsID}
	}
	h.pane = NewPane(PaneDeps{
		Workspace: func() *service.WorkspaceView { return h.workspace() },
		Thread:    func() string { return "" },
		Update: func(fn func()) {
			h.updateFeed <- fn
		},
		Collapse: func() { h.collapseN++ },
	})
	// 后台泵：把 Update 回调在测试 goroutine 顺序执行，直到测试结束。
	go func() {
		for {
			select {
			case fn := <-h.updateFeed:
				fn()
			case <-h.pumpDone:
				return
			}
		}
	}()
	t.Cleanup(func() { close(h.pumpDone) })
	return h
}

// flush 等待所有已排队的 Update 回调执行完。连锁加载（OpenFileAt 解析
// 后再触发一次读取）会分两拍入队，所以这里要求泵连续两轮空闲才算排空。
func (h *paneHarness) flush(t *testing.T) {
	t.Helper()
	idleRounds := 0
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		select {
		case fn := <-h.updateFeed:
			fn()
			idleRounds = 0
		case <-time.After(20 * time.Millisecond):
			idleRounds++
			if idleRounds >= 2 {
				return
			}
		}
	}
	t.Fatal("flush 超时：后台回调未完成")
}

func TestPaneCloseLastTabCollapsesAndKeepsRecentClosed(t *testing.T) {
	h := newPaneHarness(t, "w1")
	h.pane.OpenFilesTab()
	h.pane.OpenFileTab("", "a.go", "a.go")
	if got := h.pane.currentActiveID(); got != cwpanel.FileTabID("w1", "", "a.go") {
		t.Fatalf("activeTabId = %q", got)
	}
	if h.collapseN != 0 {
		t.Fatalf("还有可见 tab 时不应收起，collapseN = %d", h.collapseN)
	}
	h.pane.CloseTab(cwpanel.FileTabID("w1", "", "a.go"))
	if h.collapseN != 0 {
		t.Fatalf("files tab 仍可见，collapseN = %d", h.collapseN)
	}
	// 关闭后入最近关闭栈。
	state := h.pane.buckets["w1"].state
	if len(state.RecentClosed) != 1 || state.RecentClosed[0].ID != cwpanel.FileTabID("w1", "", "a.go") {
		t.Fatalf("recentClosed = %+v", state.RecentClosed)
	}
	// 关掉最后一个可见 tab（files），面板收起。
	h.pane.CloseTab(cwpanel.FilesTabID)
	if h.collapseN != 1 {
		t.Fatalf("末 tab 关闭应收起面板，collapseN = %d", h.collapseN)
	}
	// 从最近关闭栈重开。
	h.pane.ReopenRecentClosed(cwpanel.FileTabID("w1", "", "a.go"))
	if h.pane.currentActiveID() != cwpanel.FileTabID("w1", "", "a.go") {
		t.Fatalf("重开后激活项 = %q", h.pane.currentActiveID())
	}
}

func TestPaneBucketsPerWorkspace(t *testing.T) {
	h := newPaneHarness(t, "w1")
	h.pane.OpenFileTab("", "a.go", "a.go")
	h.workspace = func() *service.WorkspaceView {
		return &service.WorkspaceView{ID: "w2", Name: "w2"}
	}
	// 切到 w2：w1 的 tab 不可见，激活项兜底为空（无可见 tab）。
	if got := h.pane.currentActiveID(); got != "" {
		t.Fatalf("切换工作区后应无可见激活项，got %q", got)
	}
	h.pane.OpenFileTab("", "b.go", "b.go")
	h.workspace = func() *service.WorkspaceView {
		return &service.WorkspaceView{ID: "w1", Name: "w1"}
	}
	// 切回 w1：激活记忆恢复到 a.go。
	if got := h.pane.currentActiveID(); got != cwpanel.FileTabID("w1", "", "a.go") {
		t.Fatalf("切回工作区应恢复激活项，got %q", got)
	}
}

func TestPaneTerminalTabIsGlobal(t *testing.T) {
	h := newPaneHarness(t, "w1")
	h.pane.OpenTerminalTab()
	h.workspace = func() *service.WorkspaceView {
		return &service.WorkspaceView{ID: "w2", Name: "w2"}
	}
	// 新桶按终端全局开关自带终端 tab。
	if got := h.pane.currentActiveID(); got != cwpanel.TerminalTabID {
		t.Fatalf("w2 桶应自带终端 tab，got %q", got)
	}
	// 在 w2 关闭终端：其他桶静默移除，全局开关复位。
	h.pane.CloseTab(cwpanel.TerminalTabID)
	h.workspace = func() *service.WorkspaceView {
		return &service.WorkspaceView{ID: "w1", Name: "w1"}
	}
	for _, tab := range h.pane.buckets["w1"].state.Tabs {
		if tab.ID == cwpanel.TerminalTabID {
			t.Fatalf("关闭终端后 w1 桶不应保留终端 tab")
		}
	}
	if h.pane.terminalOpen {
		t.Fatalf("全局开关应复位")
	}
}

func TestPaneFileTabLimitEvictsLeastRecent(t *testing.T) {
	h := newPaneHarness(t, "w1")
	for i := 0; i < cwpanel.FileTabLimit+1; i++ {
		name := "f" + string(rune('a'+i)) + ".go"
		h.pane.OpenFileTab("", name, name)
	}
	files := 0
	for _, tab := range h.pane.buckets["w1"].state.Tabs {
		if tab.Type == cwpanel.TabFile {
			files++
		}
	}
	if files != cwpanel.FileTabLimit {
		t.Fatalf("file tab 数 = %d, want %d", files, cwpanel.FileTabLimit)
	}
	// 最老的 fa.go 被淘汰（自动清理不入最近关闭栈）。
	for _, tab := range h.pane.buckets["w1"].state.Tabs {
		if tab.Path == "fa.go" {
			t.Fatalf("最久未看的 fa.go 应被淘汰")
		}
	}
	for _, tab := range h.pane.buckets["w1"].state.RecentClosed {
		if tab.Type == cwpanel.TabFile {
			t.Fatalf("上限淘汰不应入最近关闭栈: %+v", h.pane.buckets["w1"].state.RecentClosed)
		}
	}
}

// newPaneServiceHarness 在真实 WorkspaceService 上测读取与保存链路。
func newPaneServiceHarness(t *testing.T) *paneHarness {
	t.Helper()
	dir := t.TempDir()
	db, err := store.Open(filepath.Join(dir, "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	bus := events.NewBus(func(events.Batch) {})
	t.Cleanup(bus.Close)
	ws := service.NewWorkspaceService(db, bus, filepath.Join(dir, "backups"))
	view, err := ws.Add(dir, "trusted")
	if err != nil {
		t.Fatal(err)
	}
	h := newPaneHarness(t, view.ID)
	// 注入真实服务。
	h.pane.deps.WorkspacesSvc = func() *service.WorkspaceService { return ws }
	h.root = dir
	return h
}

func TestPaneFileSlotLoadEditSave(t *testing.T) {
	h := newPaneServiceHarness(t)
	if err := os.WriteFile(filepath.Join(h.root, "note.txt"), []byte("hello\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// 打开文件：tab 建立并触发异步读取。
	h.pane.OpenFileTab("", "note.txt", "note.txt")
	h.flush(t)
	tabID := cwpanel.FileTabID(h.pane.scope().WorkspaceID, "", "note.txt")
	slot := h.pane.files[tabID]
	if slot == nil || slot.content == nil {
		t.Fatalf("文件槽未加载: %+v", slot)
	}
	if slot.content.Content != "hello\n" || slot.editorMode != modeRead {
		t.Fatalf("槽内容 = %+v", slot.content)
	}
	// 进入编辑态并写入草稿。
	h.pane.setEditorMode(tabID, modeEdit)
	slot.buffer.Edit(editor.Edit{Start: 5, End: 5, Text: " world"})
	if !slot.buffer.Modified() {
		t.Fatalf("草稿应处于已修改状态")
	}
	// 保存：走 WriteFile 备份链路，成功后回读刷新并回到只读。
	h.pane.saveSlot(tabID)
	h.flush(t)
	data, err := os.ReadFile(filepath.Join(h.root, "note.txt"))
	if err != nil || string(data) != "hello world\n" {
		t.Fatalf("保存后的文件内容 = %q err=%v", data, err)
	}
	slot = h.pane.files[tabID]
	if slot.editorMode != modeRead || slot.buffer.Modified() {
		t.Fatalf("保存后应回到只读且无未保存标记")
	}
	if slot.content.Hash == "" || slot.content.Content != "hello world\n" {
		t.Fatalf("保存后应回读最新内容与哈希: %+v", slot.content)
	}
	// 备份落盘：备份目录里应有 note.txt 的旧内容。
	entries, err := os.ReadDir(filepath.Join(h.root, "backups"))
	if err != nil || len(entries) == 0 {
		t.Fatalf("备份应出现在数据目录: err=%v n=%d", err, len(entries))
	}
	found := false
	for _, entry := range entries {
		data, err := os.ReadFile(filepath.Join(h.root, "backups", entry.Name()))
		if err == nil && strings.Contains(entry.Name(), "note.txt") && string(data) == "hello\n" {
			found = true
		}
	}
	if !found {
		t.Fatalf("备份内容应是保存前的原文")
	}
}

func TestPaneOpenFileAtLocatesShortPath(t *testing.T) {
	h := newPaneServiceHarness(t)
	if err := os.MkdirAll(filepath.Join(h.root, "sub"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(h.root, "sub", "deep.go"), []byte("package sub\n\nfunc F() {}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// 用短路径引用（deep.go）打开并定位第 3 行。
	h.pane.OpenFileAt("deep.go", 3)
	h.flush(t)
	// folderID 由 LocateFile 解析得出，断言按路径找到 tab 再取槽。
	var slot *FileSlot
	for id, candidate := range h.pane.files {
		if candidate.path == "sub/deep.go" {
			slot = h.pane.files[id]
		}
	}
	if slot == nil || slot.content == nil {
		t.Fatalf("短路径应经 LocateFile 命中 sub/deep.go: %+v", h.pane.files)
	}
	if slot.targetLine != 3 {
		t.Fatalf("targetLine = %d, want 3", slot.targetLine)
	}
}

func TestPaneOpenFileAtRejectsUnknown(t *testing.T) {
	h := newPaneServiceHarness(t)
	// 未知路径：LocateFile 无命中，面板保持无 file tab；不弹窗（在
	// reportError 的对话框出口上，GUI 冒烟另行覆盖）。
	h.pane.OpenFileAt("nope.go", 0)
	h.flush(t)
	if got := h.pane.currentVisible(); len(got) != 0 {
		t.Fatalf("未知路径不应产生 tab: %+v", got)
	}
}

func TestPaneSaveConflictReloadsAfterConfirm(t *testing.T) {
	h := newPaneServiceHarness(t)
	if err := os.WriteFile(filepath.Join(h.root, "note.txt"), []byte("v1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	h.pane.OpenFileTab("", "note.txt", "note.txt")
	h.flush(t)
	tabID := cwpanel.FileTabID(h.pane.scope().WorkspaceID, "", "note.txt")
	slot := h.pane.files[tabID]
	// 编辑草稿后，文件被外部程序改写。
	h.pane.setEditorMode(tabID, modeEdit)
	slot.buffer.Edit(editor.Edit{Start: slot.buffer.Len(), End: slot.buffer.Len(), Text: "local draft"})
	if err := os.WriteFile(filepath.Join(h.root, "note.txt"), []byte("external\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// 冲突确认浮层在单测里替换为「选择重新加载」。
	confirmed := false
	previousConfirm := paneConfirmReload
	paneConfirmReload = func(path string, done func(ok bool)) {
		confirmed = true
		done(true)
	}
	defer func() { paneConfirmReload = previousConfirm }()
	h.pane.saveSlot(tabID)
	h.flush(t)
	if !confirmed {
		t.Fatalf("外部修改的保存应触发冲突确认")
	}
	// 重新加载后：草稿被丢弃，内容与磁盘一致，回到只读。
	slot = h.pane.files[tabID]
	if slot.content == nil || slot.content.Content != "external\n" {
		t.Fatalf("确认后应重新加载磁盘内容: %+v", slot.content)
	}
	if slot.editorMode != modeRead || slot.buffer.Modified() {
		t.Fatalf("重载后应回到只读且无未保存标记")
	}
	// 磁盘保持外部内容未被覆盖。
	data, err := os.ReadFile(filepath.Join(h.root, "note.txt"))
	if err != nil || string(data) != "external\n" {
		t.Fatalf("冲突保存不应覆盖磁盘: %q err=%v", data, err)
	}
}
