//go:build windows

package tools

import (
	"os/exec"
	"syscall"
)

func processGroupAttr() *syscall.SysProcAttr {
	return nil
}

// killProcessGroup 在 Windows 上只终止直接子进程；更完整的作业对象管理留到 M4。
func killProcessGroup(cmd *exec.Cmd) error {
	if cmd.Process == nil {
		return nil
	}
	return cmd.Process.Kill()
}
