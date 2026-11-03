package notify

import "testing"

func TestSetClickHandler(t *testing.T) {
	got := ""
	SetClickHandler(func(threadID string) { got = threadID })
	t.Cleanup(func() { SetClickHandler(nil) })

	if fn := loadClick(); fn == nil {
		t.Fatalf("注入后应能读到点击回调")
	}
	clickMu.RLock()
	fn := clickHandler
	clickMu.RUnlock()
	fn("th-1")
	if got != "th-1" {
		t.Fatalf("回调收到的会话 Id = %q, want th-1", got)
	}

	SetClickHandler(nil)
	if fn := loadClick(); fn != nil {
		t.Fatalf("清除后回调应为 nil")
	}
}
