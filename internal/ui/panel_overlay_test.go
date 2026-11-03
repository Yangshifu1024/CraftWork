package ui

// 面板与浮层体系的联动走查（ui.Tester，07 号票清偿后的回归锚点）：
// 「应用全部」在应用内确认浮层里走完取消链路，错误提示经通知层可见。

import (
	"path/filepath"
	"testing"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// newOverlayTester 起一个带真实工作区服务的 RootView 测试窗：面板的
// applyDiff 守卫（活动工作区 + 服务）全部就位，浮层画在 RootView 里。
func newOverlayTester(t *testing.T) (*Shell, *mygo.Tester, string) {
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
	s := NewShell()
	s.SetPanelServices(ws, nil)
	tt := mygo.NewTester(RootView(s), 1440, 900)
	tt.Frame()
	return s, tt, view.ID
}

func TestPaneApplyDiffShowsOverlayConfirm(t *testing.T) {
	s, tt, _ := newOverlayTester(t)
	// 打开差异并点「应用全部」：确认文案应出现在应用内浮层里
	//（不再是原生对话框），浮层标题出现在可视文本中。
	s.pane.OpenDiff("note.txt", "v1\n", "v2\n")
	tt.Frame()
	if err := tt.Click("应用全部"); err != nil {
		t.Fatalf("点击应用全部失败: %v（可见 %v）", err, tt.Texts())
	}
	tt.Frame()
	if !tt.HasText("应用全部修改？") {
		t.Fatalf("确认浮层未出现: %v", tt.Texts())
	}
	if !s.pane.diff.applying {
		t.Fatalf("确认期间应处于应用中")
	}
	// 取消：浮层关闭，applying 复位，不写任何文件（此用例没有真实文件，
	// 写路径一旦被触发会在 update 回调里报错通知）。
	if err := tt.Click("取消"); err != nil {
		t.Fatalf("点击取消失败: %v", err)
	}
	tt.Frame()
	if s.pane.diff.applying {
		t.Fatalf("取消后应复位 applying")
	}
	if tt.HasText("应用全部修改？") {
		t.Errorf("取消后浮层应关闭")
	}
}

func TestPaneConfirmReloadShowsDangerOverlay(t *testing.T) {
	_, tt, _ := newOverlayTester(t)
	// paneConfirmReload 的生产实现直接经 overlays.Confirm 发起（不走面板
	// 替身）：浮层文案出现，Esc 按取消收尾（安全默认保留草稿）。
	decided := false
	paneConfirmReload("note.txt", func(ok bool) { decided = ok })
	tt.Frame()
	if !tt.HasText("文件已被外部修改") {
		t.Fatalf("冲突确认浮层未出现: %v", tt.Texts())
	}
	// 危险样式：确认侧是「重新加载」（破坏性动作），取消走 Esc。
	if err := tt.Click("保留草稿"); err != nil {
		t.Fatalf("点击保留草稿失败: %v", err)
	}
	tt.Frame()
	if decided {
		t.Fatalf("Esc 应按取消收尾（保留草稿）")
	}
	if tt.HasText("文件已被外部修改") {
		t.Errorf("取消后浮层应关闭")
	}
}

func TestPaneReportErrorShowsToast(t *testing.T) {
	s, tt, _ := newOverlayTester(t)
	// reportError 走 overlays.Notify（07 号票起不再弹原生对话框）：
	// 通知层在下一帧消费队列，错误标题应出现在界面上。
	s.pane.reportError("面板出错了", "读取失败")
	tt.Frame()
	if !tt.HasText("面板出错了") {
		t.Fatalf("错误 toast 未出现: %v", tt.Texts())
	}
}
