package tools

import (
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"craftwork/internal/platform"
	"craftwork/internal/policy"
)

type shellExecTool struct{}

func (t *shellExecTool) Spec() Spec {
	return Spec{
		Name:        "shell_exec",
		Description: "在工作区内执行 Shell 命令，默认超时 120 秒，输出截断到 32 KB。命令需要经过审批。",
		Risk:        policy.R2,
		PathArg:     "cwd",
		CommandArg:  "command",
		Title:       "执行命令",
		SummaryArg:  "command",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"command"},
			"properties": map[string]any{
				"command": map[string]any{"type": "string", "description": "要执行的命令"},
				"cwd":     map[string]any{"type": "string", "description": "工作目录（相对工作区），默认工作区根"},
				"timeout": map[string]any{"type": "integer", "minimum": 1, "maximum": 600, "description": "超时秒数，默认 120"},
			},
		},
	}
}

func (t *shellExecTool) Execute(ctx context.Context, call Call) Result {
	args := call.Args
	command := stringArg(args, "command")
	if strings.TrimSpace(command) == "" {
		return errorResult("command 不能为空")
	}
	cwd := call.Target
	timeoutSeconds := intArg(args, "timeout", 120)
	if timeoutSeconds < 1 {
		timeoutSeconds = 1
	}
	if timeoutSeconds > 600 {
		timeoutSeconds = 600
	}

	start := time.Now()
	run := platform.Run(ctx, platform.RunSpec{
		Path: shellPath(), Args: []string{"-c", command}, Dir: cwd,
		Timeout: time.Duration(timeoutSeconds) * time.Second, OutputLimit: maxOutputBytes,
	})
	duration := time.Since(start).Round(time.Millisecond)
	if run.TimedOut {
		return errorResult("命令超时（%s）：%s", duration, run.Output)
	}
	if run.Err != nil && run.ExitCode < 0 {
		return errorResult("执行命令失败: %v", run.Err)
	}
	output := strings.TrimRight(run.Output, "\n")
	if output == "" {
		output = "(无输出)"
	}
	text := fmt.Sprintf("$ %s\n%s\n[退出码 %d · %s]", command, output, run.ExitCode, duration)
	result := Result{Output: text, Meta: map[string]any{"exitCode": run.ExitCode}}
	if run.ExitCode != 0 {
		result.IsError = true
	}
	return result
}

func shellPath() string {
	if _, err := os.Stat("/bin/sh"); err == nil {
		return "/bin/sh"
	}
	return "sh"
}

type gitStatusTool struct{}

func (t *gitStatusTool) Spec() Spec {
	return Spec{
		Name:        "git_status",
		Description: "查看当前工作区的 Git 分支与文件状态（只读）。",
		Risk:        policy.R0,
		SummaryText: "git status",
		Schema:      map[string]any{"type": "object", "properties": map[string]any{}},
	}
}

func (t *gitStatusTool) Execute(ctx context.Context, call Call) Result {
	run := platform.Run(ctx, platform.RunSpec{
		Path: "git", Args: []string{"status", "--short", "--branch"}, Dir: call.Workspace,
		Timeout: 20 * time.Second, OutputLimit: maxOutputBytes,
	})
	if run.TimedOut {
		return errorResult("git status 超时")
	}
	if run.Err != nil && run.ExitCode < 0 {
		return errorResult("执行 git status 失败: %v", run.Err)
	}
	output := strings.TrimSpace(run.Output)
	if output == "" {
		return Result{Output: "工作区干净（没有未提交的改动）"}
	}
	if run.ExitCode != 0 {
		return Result{Output: output, IsError: true}
	}
	return Result{Output: output}
}
