package platform

import (
	"context"
	"errors"
	"os/exec"
	"time"
)

// RunSpec 描述一次子进程执行。
type RunSpec struct {
	Path        string
	Args        []string
	Dir         string
	Timeout     time.Duration
	OutputLimit int
}

// RunResult 是一次子进程执行的结果。
type RunResult struct {
	Output   string
	ExitCode int
	TimedOut bool
	Err      error
}

// Run 执行子进程，统一处理超时、进程组清理与环境清洗。
func Run(parent context.Context, spec RunSpec) RunResult {
	ctx, cancel := context.WithTimeout(parent, spec.Timeout)
	defer cancel()

	command := exec.CommandContext(ctx, spec.Path, spec.Args...)
	command.Dir = spec.Dir
	command.Env = SanitizedEnv()
	command.SysProcAttr = ProcessGroupAttr()
	command.Cancel = func() error {
		return KillProcessGroup(command)
	}
	command.WaitDelay = 2 * time.Second

	limit := spec.OutputLimit
	if limit <= 0 {
		limit = 32 * 1024
	}
	buffer := NewLimitedBuffer(limit)
	command.Stdout = buffer
	command.Stderr = buffer

	err := command.Run()
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return RunResult{Output: buffer.String(), ExitCode: -1, TimedOut: true, Err: ctx.Err()}
	}
	exitCode := 0
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			exitCode = exitErr.ExitCode()
			err = nil
		}
	}
	return RunResult{Output: buffer.String(), ExitCode: exitCode, Err: err}
}
