// 本文件负责子进程的统一执行：设置超时、清洗环境变量、限制输出长度，
// 并在取消或超时时清理整个进程组，避免留下孤儿进程。

package platform

import (
	"context"
	"errors"
	"os/exec"
	"time"
)

// RunSpec 描述一次子进程执行。
type RunSpec struct {
	Path        string        // 可执行文件的绝对路径或 PATH 中的名称
	Args        []string      // 传给子进程的参数，不含可执行文件本身
	Dir         string        // 工作目录；为空时继承当前进程
	Timeout     time.Duration // 整体超时时间，从启动开始计算
	OutputLimit int           // 标准输出与标准错误合计保留的字节数；小于等于 0 时使用默认值
}

// RunResult 是一次子进程执行的结果。
type RunResult struct {
	Output   string // 合并后的标准输出与标准错误，可能已截断
	ExitCode int    // 退出码；超时或被信号终止时为 -1
	TimedOut bool   // 是否因超时而结束
	Err      error  // 启动失败等执行错误；非零退出码不算错误
}

// Run 执行子进程，统一处理超时、进程组清理与环境清洗。
//
// 标准输出与标准错误共用同一个有界缓冲，因此输出顺序与实际打印顺序
// 一致。非零退出码会被转换成 ExitCode 并清空 Err；只有启动失败、
// 超时等无法正常执行的情况才返回 Err。
func Run(parent context.Context, spec RunSpec) RunResult {
	ctx, cancel := context.WithTimeout(parent, spec.Timeout)
	defer cancel()

	command := exec.CommandContext(ctx, spec.Path, spec.Args...)
	command.Dir = spec.Dir
	command.Env = SanitizedEnv()
	// 让子进程进入独立进程组，取消时可以整组杀掉。
	command.SysProcAttr = ProcessGroupAttr()
	command.Cancel = func() error {
		return KillProcessGroup(command)
	}
	// 取消后最多再等 2 秒，防止子进程继承的管道让 Wait 永久阻塞。
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
		// 超时优先于其它错误上报，调用方据此区分“跑太久”和“启动失败”。
		return RunResult{Output: buffer.String(), ExitCode: -1, TimedOut: true, Err: ctx.Err()}
	}
	exitCode := 0
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			// 进程正常启动但以非零码退出，属于可预期的结果而不是执行错误。
			exitCode = exitErr.ExitCode()
			err = nil
		}
	}
	return RunResult{Output: buffer.String(), ExitCode: exitCode, Err: err}
}
