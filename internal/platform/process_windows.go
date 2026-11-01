//go:build windows

package platform

import (
	"os/exec"
	"syscall"
)

// ProcessGroupAttr 在 Windows 上不设置额外属性。
func ProcessGroupAttr() *syscall.SysProcAttr {
	return nil
}

// KillProcessGroup 在 Windows 上只终止直接子进程；作业对象管理留到 M4。
func KillProcessGroup(command *exec.Cmd) error {
	if command.Process == nil {
		return nil
	}
	return command.Process.Kill()
}
