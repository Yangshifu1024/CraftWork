//go:build windows

package mcp

import (
	"os/exec"
	"syscall"
)

func processGroupAttr() *syscall.SysProcAttr {
	return nil
}

func killProcessGroup(command *exec.Cmd) error {
	if command.Process == nil {
		return nil
	}
	return command.Process.Kill()
}
