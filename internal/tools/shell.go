package tools

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"sort"
	"strings"
	"time"
)

type shellExecTool struct{ registry *Registry }

func (t *shellExecTool) Name() string { return "shell_exec" }

func (t *shellExecTool) Description() string {
	return "在工作区内执行 Shell 命令，默认超时 120 秒，输出截断到 32 KB。命令需要经过审批。"
}

func (t *shellExecTool) Schema() map[string]any {
	return map[string]any{
		"type":     "object",
		"required": []string{"command"},
		"properties": map[string]any{
			"command": map[string]any{"type": "string", "description": "要执行的命令"},
			"cwd":     map[string]any{"type": "string", "description": "工作目录（相对工作区），默认工作区根"},
			"timeout": map[string]any{"type": "integer", "minimum": 1, "maximum": 600, "description": "超时秒数，默认 120"},
		},
	}
}

func (t *shellExecTool) Execute(ctx context.Context, args map[string]any) Result {
	command := stringArg(args, "command")
	if strings.TrimSpace(command) == "" {
		return errorResult("command 不能为空")
	}
	cwd := t.registry.Workspace()
	if value := stringArg(args, "cwd"); value != "" {
		resolved, _, err := t.registry.resolve(value)
		if err != nil {
			return errorResult("%v", err)
		}
		cwd = resolved
	}
	timeoutSeconds := intArg(args, "timeout", 120)
	if timeoutSeconds < 1 {
		timeoutSeconds = 1
	}
	if timeoutSeconds > 600 {
		timeoutSeconds = 600
	}

	start := time.Now()
	output, exitCode, timedOut, err := runShell(ctx, cwd, command, time.Duration(timeoutSeconds)*time.Second)
	duration := time.Since(start).Round(time.Millisecond)
	if timedOut {
		return errorResult("命令超时（%s）：%s", duration, truncate(output, maxOutputBytes))
	}
	if err != nil && exitCode < 0 {
		return errorResult("执行命令失败: %v", err)
	}
	output = strings.TrimRight(output, "\n")
	if output == "" {
		output = "(无输出)"
	}
	text := fmt.Sprintf("$ %s\n%s\n[退出码 %d · %s]", command, output, exitCode, duration)
	result := Result{Output: truncate(text, maxOutputBytes), Meta: map[string]any{"exitCode": exitCode}}
	if exitCode != 0 {
		result.IsError = true
	}
	return result
}

type gitStatusTool struct{ registry *Registry }

func (t *gitStatusTool) Name() string { return "git_status" }

func (t *gitStatusTool) Description() string {
	return "查看当前工作区的 Git 分支与文件状态（只读）。"
}

func (t *gitStatusTool) Schema() map[string]any {
	return map[string]any{"type": "object", "properties": map[string]any{}}
}

func (t *gitStatusTool) Execute(ctx context.Context, _ map[string]any) Result {
	output, exitCode, timedOut, err := runShell(ctx, t.registry.Workspace(), "git status --short --branch", 20*time.Second)
	if timedOut {
		return errorResult("git status 超时")
	}
	if err != nil && exitCode < 0 {
		return errorResult("执行 git status 失败: %v", err)
	}
	output = strings.TrimSpace(output)
	if output == "" {
		return Result{Output: "工作区干净（没有未提交的改动）"}
	}
	if exitCode != 0 {
		return Result{Output: output, IsError: true}
	}
	return Result{Output: truncate(output, maxOutputBytes)}
}

// runShell 在工作目录中执行命令，返回合并输出、退出码与是否超时。
func runShell(parent context.Context, dir, command string, timeout time.Duration) (string, int, bool, error) {
	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()

	shell := "/bin/sh"
	if _, err := os.Stat(shell); err != nil {
		shell = "sh"
	}
	cmd := exec.CommandContext(ctx, shell, "-c", command)
	cmd.Dir = dir
	cmd.Env = sanitizedEnv()
	cmd.SysProcAttr = processGroupAttr()
	cmd.Cancel = func() error {
		return killProcessGroup(cmd)
	}
	cmd.WaitDelay = 2 * time.Second

	buffer := &limitedBuffer{limit: maxOutputBytes}
	cmd.Stdout = buffer
	cmd.Stderr = buffer

	err := cmd.Run()
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return buffer.String(), -1, true, ctx.Err()
	}
	exitCode := 0
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			exitCode = exitErr.ExitCode()
			err = nil
		}
	}
	return buffer.String(), exitCode, false, err
}

type limitedBuffer struct {
	buffer    bytes.Buffer
	limit     int
	truncated bool
}

func (b *limitedBuffer) Write(data []byte) (int, error) {
	if b.buffer.Len() >= b.limit {
		b.truncated = true
		return len(data), nil
	}
	remaining := b.limit - b.buffer.Len()
	if len(data) > remaining {
		b.buffer.Write(data[:remaining])
		b.truncated = true
		return len(data), nil
	}
	b.buffer.Write(data)
	return len(data), nil
}

func (b *limitedBuffer) String() string {
	text := b.buffer.String()
	if b.truncated {
		text += "\n…（输出已截断）"
	}
	return text
}

// sanitizedEnv 移除可能把密钥泄漏给子进程的环境变量。
func sanitizedEnv() []string {
	keep := map[string]bool{
		"PATH": true, "HOME": true, "USER": true, "LOGNAME": true,
		"SHELL": true, "LANG": true, "TERM": true, "TMPDIR": true,
		"TZ": true, "PWD": true, "COLORTERM": true, "SSH_AUTH_SOCK": true,
	}
	var result []string
	for _, entry := range os.Environ() {
		key := entry
		if index := strings.Index(entry, "="); index >= 0 {
			key = entry[:index]
		}
		upper := strings.ToUpper(key)
		if keep[key] || strings.HasPrefix(key, "LC_") {
			result = append(result, entry)
			continue
		}
		if strings.Contains(upper, "API_KEY") || strings.Contains(upper, "TOKEN") ||
			strings.Contains(upper, "SECRET") || strings.Contains(upper, "PASSWORD") ||
			strings.Contains(upper, "CREDENTIAL") {
			continue
		}
		switch {
		case strings.HasPrefix(upper, "AWS_"),
			strings.HasPrefix(upper, "GITHUB_"),
			strings.HasPrefix(upper, "GH_"),
			strings.HasPrefix(upper, "OPENAI_"),
			strings.HasPrefix(upper, "ANTHROPIC_"),
			strings.HasPrefix(upper, "DEEPSEEK_"),
			strings.HasPrefix(upper, "ZHIPU_"),
			strings.HasPrefix(upper, "DASHSCOPE_"):
			continue
		}
		result = append(result, entry)
	}
	result = append(result, "CRAFTWORK=1")
	sort.Strings(result)
	return result
}
