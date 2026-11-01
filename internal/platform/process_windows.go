//go:build windows

// 本文件负责 Windows 平台的进程管理占位实现：目前只终止直接子进程，
// 使用作业对象管理整棵进程树的能力留到 M4。

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
//
// 这意味着子进程再派生的进程可能继续存活，属于当前已知限制。
func KillProcessGroup(command *exec.Cmd) error {
	if command.Process == nil {
		return nil
	}
	return command.Process.Kill()
}
