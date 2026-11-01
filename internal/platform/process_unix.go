//go:build !windows

// 本文件负责非 Windows 平台的进程组管理：启动时让子进程进入独立进程组，
// 取消或超时时按进程组整体终止。

package platform

import (
	"os/exec"
	"syscall"
)

// ProcessGroupAttr 让子进程拥有独立进程组。
//
// Setpgid 会把子进程设为新进程组的组长，其进程组 ID 等于自身 PID，
// 这样后面可以用负 PID 一次性终止整组进程。
func ProcessGroupAttr() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{Setpgid: true}
}

// KillProcessGroup 杀掉整个进程组，避免取消后留下子进程。
//
// 负 PID 表示向进程组发送信号；使用 SIGKILL 是因为这里只用于取消或
// 超时的强制收尾，不再给子进程优雅退出的机会。
func KillProcessGroup(command *exec.Cmd) error {
	if command.Process == nil {
		return nil
	}
	return syscall.Kill(-command.Process.Pid, syscall.SIGKILL)
}
