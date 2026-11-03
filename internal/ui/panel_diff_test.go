package ui

// 差异 tab 的单测：数据槽与 singleton tab 的联动、视图方式记忆，以及
// 「应用全部」链路（确认框替身 + 真实 WorkspaceService 的备份落盘与
// 编辑器打开）。

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/egoist/mygo/ui/editor"

	cwpanel "github.com/yangshifu1024/craftwork/internal/ui/panel"
)

func TestPaneOpenDiffStoresSlotAndFocusesTab(t *testing.T) {
	h := newPaneHarness(t, "w1")
	h.pane.OpenDiff("note.txt", "old\n", "new\n")
	h.flush(t)
	if got := h.pane.currentActiveID(); got != cwpanel.DiffTabID {
		t.Fatalf("打开差异后应聚焦差异 tab，got %q", got)
	}
	d := h.pane.diff
	if d == nil || d.title != "note.txt" || d.oldText != "old\n" || d.newText != "new\n" {
		t.Fatalf("数据槽应记录标题与新旧文本: %+v", d)
	}
	if d.workspaceID != "w1" {
		t.Fatalf("数据槽应冻结工作区身份, got %q", d.workspaceID)
	}
	// singleton：再次打开刷新数据槽与标题，tab 不新增。
	h.pane.OpenDiff("other.txt", "a\n", "b\n")
	h.flush(t)
	tabs := 0
	for _, tab := range h.pane.buckets["w1"].state.Tabs {
		if tab.Type == cwpanel.TabDiff {
			tabs++
		}
	}
	if tabs != 1 {
		t.Fatalf("差异是 singleton，tab 数 = %d", tabs)
	}
	if h.pane.diff.title != "other.txt" {
		t.Fatalf("再次打开应刷新数据槽, got %q", h.pane.diff.title)
	}
	// 视图方式记忆：切到并排后再打开新差异，保持并排。
	h.pane.diffViewIdx = 1
	h.pane.OpenDiff("third.txt", "x\n", "y\n")
	h.flush(t)
	if h.pane.diffViewIdx != 1 {
		t.Fatalf("视图方式应跨打开保留")
	}
	// 布局映射。
	d = h.pane.diff
	d.ensureDiffState(false)
	if d.state == nil || d.state.Layout != editor.DiffUnified {
		t.Fatalf("统一视图应映射 DiffUnified: %+v", d.state)
	}
	d.ensureDiffState(true)
	if d.state.Layout != editor.DiffSplit {
		t.Fatalf("并排视图应映射 DiffSplit: %+v", d.state)
	}
	if added, deleted := d.diffStats(); added == 0 || deleted == 0 {
		t.Fatalf("x\\n → y\\n 应各有一行增删, got +%d -%d", added, deleted)
	}
}

func TestPaneApplyDiffWritesWithBackupAfterConfirm(t *testing.T) {
	h := newPaneServiceHarness(t)
	if err := os.WriteFile(filepath.Join(h.root, "note.txt"), []byte("v1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// 确认框替身：选择「应用修改」。
	confirmed := false
	previous := paneConfirmApplyAll
	paneConfirmApplyAll = func(path string) bool {
		confirmed = true
		if path != "note.txt" {
			t.Fatalf("确认框应带文件路径, got %q", path)
		}
		return true
	}
	defer func() { paneConfirmApplyAll = previous }()

	h.pane.OpenDiff("note.txt", "v1\n", "v2\n")
	if h.pane.diff.applying {
		t.Fatalf("初始不应处于应用中")
	}
	h.pane.applyDiff()
	h.flush(t)
	if !confirmed {
		t.Fatalf("应用全部应先弹确认框")
	}
	if h.pane.diff.applying {
		t.Fatalf("应用结束后应复位 applying")
	}
	if h.pane.diff.appliedAt <= 0 {
		t.Fatalf("应用成功应记录时间")
	}
	// 磁盘写入新内容，备份目录保留旧内容（WriteFile 既有链路）。
	data, err := os.ReadFile(filepath.Join(h.root, "note.txt"))
	if err != nil || string(data) != "v2\n" {
		t.Fatalf("应用后文件内容 = %q err=%v", data, err)
	}
	entries, err := os.ReadDir(filepath.Join(h.root, "backups"))
	if err != nil || len(entries) == 0 {
		t.Fatalf("应用应产生备份: err=%v n=%d", err, len(entries))
	}
	found := false
	for _, entry := range entries {
		if old, readErr := os.ReadFile(filepath.Join(h.root, "backups", entry.Name())); readErr == nil &&
			strings.Contains(entry.Name(), "note.txt") && string(old) == "v1\n" {
			found = true
		}
	}
	if !found {
		t.Fatalf("备份应保留应用前的原文")
	}
	// 成功后在编辑器打开磁盘最新状态。
	tabID := cwpanel.FileTabID(h.pane.scope().WorkspaceID, "", "note.txt")
	if slot := h.pane.files[tabID]; slot == nil || slot.content == nil || slot.content.Content != "v2\n" {
		t.Fatalf("应用后应在编辑器打开最新文件: %+v", h.pane.files[tabID])
	}
}

func TestPaneApplyDiffCancelledAndGuards(t *testing.T) {
	h := newPaneServiceHarness(t)
	if err := os.WriteFile(filepath.Join(h.root, "note.txt"), []byte("v1\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// 确认框替身：取消。
	previous := paneConfirmApplyAll
	paneConfirmApplyAll = func(string) bool { return false }
	defer func() { paneConfirmApplyAll = previous }()

	h.pane.OpenDiff("note.txt", "v1\n", "v2\n")
	h.pane.applyDiff()
	h.flush(t)
	if h.pane.diff.applying {
		t.Fatalf("取消后应复位 applying")
	}
	data, err := os.ReadFile(filepath.Join(h.root, "note.txt"))
	if err != nil || string(data) != "v1\n" {
		t.Fatalf("取消不应写盘, got %q err=%v", data, err)
	}
	// 无数据槽时是空操作。
	h.pane.diff = nil
	h.pane.applyDiff()
	if h.pane.diff != nil {
		t.Fatalf("无数据槽时不应新建状态")
	}
}
