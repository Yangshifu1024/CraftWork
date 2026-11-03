package ui

// 终端 tab 的单测：不起新的终端进程（PTY 会话由 GUI 冒烟覆盖），只测
// 会话状态机的纯逻辑——字号折算、会话目标解析、重起与回收的状态复位、
// 设置缓存的 TTL。

import (
	"testing"
	"time"

	"github.com/yangshifu1024/craftwork/internal/service"
)

func TestTerminalFontSizeOrDefault(t *testing.T) {
	cases := map[int]float32{
		0:  12, // 脏数据回退默认
		9:  12, // 低于下限回退
		10: 10,
		14: 14,
		20: 20,
		21: 12, // 高于上限回退
	}
	for in, want := range cases {
		if got := terminalFontSizeOrDefault(in); got != want {
			t.Fatalf("terminalFontSizeOrDefault(%d) = %v, want %v", in, got, want)
		}
	}
}

func TestPaneTerminalTargetFollowsWorkspace(t *testing.T) {
	h := newPaneHarness(t, "w1")
	h.workspace = func() *service.WorkspaceView {
		return &service.WorkspaceView{ID: "w1", Name: "w1", Path: "/tmp/w1"}
	}
	if id, dir := h.pane.terminalTarget(); id != "w1" || dir != "/tmp/w1" {
		t.Fatalf("有工作区时应取 id 与根目录, got %q %q", id, dir)
	}
	h.workspace = func() *service.WorkspaceView { return nil }
	if id, dir := h.pane.terminalTarget(); id != "" || dir != "" {
		t.Fatalf("无工作区时应返回空（插件回退主目录）, got %q %q", id, dir)
	}
}

func TestPaneTerminalRestartAndShutdownResetState(t *testing.T) {
	h := newPaneHarness(t, "w1")
	h.pane.termRequested = true
	h.pane.termWsID = "w1"
	h.pane.termExited = true
	h.pane.termExitCode = 3
	h.pane.termTitle = "zsh"
	// 重起：状态复位，下一帧的 ensureTerminal 会重新发起创建。
	h.pane.restartTerminal()
	if h.pane.termRequested || h.pane.term != nil || h.pane.termExited || h.pane.termTitle != "" {
		t.Fatalf("重起应复位会话状态: %+v", h.pane)
	}
	// 关闭回收：无会话时是空操作，有会话标记时同样复位。
	h.pane.termRequested = true
	h.pane.Shutdown()
	if h.pane.term != nil {
		t.Fatalf("Shutdown 应清空会话引用")
	}
}

func TestPaneTerminalSettingsTTLCache(t *testing.T) {
	h := newPaneHarness(t, "w1")
	calls := 0
	h.pane.deps.Settings = func() service.Settings {
		calls++
		s := service.DefaultSettings()
		s.TerminalFontSize = 14
		return s
	}
	first := h.pane.terminalSettings()
	if first.TerminalFontSize != 14 || calls != 1 {
		t.Fatalf("首次应读取设置, got %d calls=%d", first.TerminalFontSize, calls)
	}
	// TTL 内不再读取。
	if again := h.pane.terminalSettings(); calls != 1 || again.TerminalFontSize != 14 {
		t.Fatalf("TTL 内应命中缓存, calls=%d", calls)
	}
	// 过期后重新读取。
	h.pane.termSettingsAt = time.Now().Add(-2 * termSettingsTTL)
	if got := h.pane.terminalSettings(); calls != 2 || got.TerminalFontSize != 14 {
		t.Fatalf("过期后应重新读取, calls=%d", calls)
	}
}
