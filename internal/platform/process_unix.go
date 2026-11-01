//go:build !windows

package platform

import (
	"os/exec"
	"syscall"
)

// ProcessGroupAttr 让子进程拥有独立进程组。
func ProcessGroupAttr() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{Setpgid: true}
}

// KillProcessGroup 杀掉整个进程组，避免取消后留下子进程。
func KillProcessGroup(command *exec.Cmd) error {
	if command.Process == nil {
		return nil
	}
	return syscall.Kill(-command.Process.Pid, syscall.SIGKILL)
}
