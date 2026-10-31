// Package notify 发送尽力而为的系统通知。
//
// 通知失败不影响业务流程；Windows 暂未接入，调用会静默返回。
package notify

import (
	"context"
	"os/exec"
	"runtime"
	"strings"
	"time"
)

// Send 异步发送一条系统通知。
func Send(title, message string) {
	title = strings.TrimSpace(title)
	message = strings.TrimSpace(message)
	if title == "" && message == "" {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		switch runtime.GOOS {
		case "darwin":
			script := "display notification " + appleScriptString(message) +
				" with title " + appleScriptString(title)
			_ = exec.CommandContext(ctx, "osascript", "-e", script).Run()
		case "linux":
			if _, err := exec.LookPath("notify-send"); err != nil {
				return
			}
			_ = exec.CommandContext(ctx, "notify-send", title, message).Run()
		}
	}()
}

func appleScriptString(value string) string {
	value = strings.ReplaceAll(value, `\`, `\\`)
	value = strings.ReplaceAll(value, `"`, `\"`)
	return `"` + value + `"`
}
