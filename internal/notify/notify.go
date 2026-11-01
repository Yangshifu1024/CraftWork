// Package notify 发送尽力而为的系统通知。
//
// 通知失败不影响业务流程；Windows 暂未接入，调用会静默返回。
//
// 本文件负责按操作系统选择通知方式：macOS 通过 osascript 调用系统通知中心，
// Linux 使用 notify-send；命令在后台执行并带 5 秒超时，通知只用于提醒，
// 点击通知不会回调应用内逻辑。
package notify

import (
	"context"
	"os/exec"
	"runtime"
	"strings"
	"time"
)

// Send 异步发送一条系统通知。
//
// 标题与正文都会去掉首尾空白；两者都为空时不发送。命令在独立协程中执行，
// 调用方不会等待，也不会收到错误：通知是尽力而为的附属功能，失败只影响
// 提醒本身。macOS 使用 AppleScript 的 display notification，Linux 使用
// notify-send；未安装对应命令或系统不支持时静默跳过。
func Send(title, message string) {
	title = strings.TrimSpace(title)
	message = strings.TrimSpace(message)
	if title == "" && message == "" {
		return
	}
	go func() {
		// 限制命令执行时间，避免外部命令卡住后台协程。
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		switch runtime.GOOS {
		case "darwin":
			// 标题与正文都需要转义，防止内容里的引号破坏脚本结构。
			script := "display notification " + appleScriptString(message) +
				" with title " + appleScriptString(title)
			_ = exec.CommandContext(ctx, "osascript", "-e", script).Run()
		case "linux":
			// 没有 notify-send 的桌面环境直接跳过。
			if _, err := exec.LookPath("notify-send"); err != nil {
				return
			}
			_ = exec.CommandContext(ctx, "notify-send", title, message).Run()
		}
	}()
}

// appleScriptString 把文本转成 AppleScript 字符串字面量：先转义反斜杠，
// 再转义双引号，最后用双引号包起来。顺序不能颠倒，否则会重复转义。
func appleScriptString(value string) string {
	value = strings.ReplaceAll(value, `\`, `\\`)
	value = strings.ReplaceAll(value, `"`, `\"`)
	return `"` + value + `"`
}
