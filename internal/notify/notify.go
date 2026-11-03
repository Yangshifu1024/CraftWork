// Package notify 发送尽力而为的系统通知。
//
// 通知失败不影响业务流程：系统不支持（mygo.NotificationsSupported 为否，
// 如未打包的 go build 运行）或用户拒绝授权时静默跳过，其余失败只记日志。
//
// 发送走 mygo.NewNotification（10 号票：替换 Wails 时期的 osascript /
// notify-send 方案），带 threadID 的通知以 "thread/<id>" 为 ID 在通知中心
// 聚成一组，点击回调进应用聚焦窗口并跳到对应会话（装配层经 SetClickHandler
// 注入跳转动作）。展示与点击回调都不阻塞调用方。
package notify

import (
	"errors"
	"log/slog"
	"strings"
	"sync"

	"github.com/egoist/mygo"
)

// clickHandler 是通知点击回调（装配层注入）：参数是通知携带的会话 Id。
// 回调在 mygo 的主线程触发，实现方自行决定是否再编排。
var (
	clickMu      sync.RWMutex
	clickHandler func(threadID string)
)

// SetClickHandler 注入通知点击回调；装配阶段调用一次。传 nil 清除。
func SetClickHandler(fn func(threadID string)) {
	clickMu.Lock()
	clickHandler = fn
	clickMu.Unlock()
}

// loadClick 读当前点击回调（Send.OnClick 用）。
func loadClick() func(threadID string) {
	clickMu.RLock()
	defer clickMu.RUnlock()
	return clickHandler
}

// Send 异步发送一条系统通知。
//
// 标题与正文都会去掉首尾空白；两者都为空时不发送。threadID 非空时通知
// 以它分组，点击后由点击回调跳会话；为空时点击只走回调（threadID 为空串）。
// macOS 首次发送会向用户请求授权，Show 等待授权结果；拒绝后这里静默
// 放弃（ErrNotificationsDenied），后续调用同样安静失败。
func Send(threadID, title, message string) {
	title = strings.TrimSpace(title)
	message = strings.TrimSpace(message)
	if title == "" && message == "" {
		return
	}
	if !mygo.NotificationsSupported() {
		// 未打包的运行（go run / go test）不接系统通知中心：静默跳过，
		// 与 Wails 时期「无 osascript / notify-send 即跳过」的探测语义一致。
		return
	}
	opts := mygo.NotificationOptions{Title: title, Body: message}
	if threadID != "" {
		// 同一会话的多条通知在通知中心聚成一叠；同 ID 的通知被新的一条
		// 替换（运行中反复审批 / 失败时不堆积）。
		opts.ID = "thread/" + threadID
		opts.Group = threadID
	}
	n := mygo.NewNotification(opts)
	n.OnClick(func() {
		// macOS 的已点通知会留在通知中心，点击后主动移除。
		n.Close()
		if fn := loadClick(); fn != nil {
			fn(threadID)
		}
	})
	go func() {
		if err := n.Show(); err != nil && !errors.Is(err, mygo.ErrNotificationsDenied) {
			slog.Warn("发送系统通知失败", "err", err)
		}
	}()
}
