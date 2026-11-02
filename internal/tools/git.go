// 本文件负责 Git 相关工具：查看差异、查看提交历史与提交代码。
// 所有命令都直接启动 git 子进程，不经过 Shell，参数以数组形式传递，
// 避免拼接字符串带来的注入风险。

package tools

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/yangshifu1024/craftwork/internal/platform"
	"github.com/yangshifu1024/craftwork/internal/policy"
)

// gitTimeout 是单次 Git 命令的默认超时时间。
const gitTimeout = 30 * time.Second

// ---------- git_diff ----------

// gitDiffTool 查看工作区或暂存区的差异，只读。
type gitDiffTool struct{}

// Spec 声明 git_diff 的元数据：可选 path 与 staged，只读操作。
func (t *gitDiffTool) Spec() Spec {
	return Spec{
		Name:        "git_diff",
		Description: "查看工作区的 Git 差异（只读）。可以只看某个路径或暂存区。",
		Risk:        policy.R0,
		PathArg:     "path",
		SummaryArg:  "path",
		SummaryText: "git diff",
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"path":   map[string]any{"type": "string", "description": "可选的相对路径"},
				"staged": map[string]any{"type": "boolean", "description": "是否查看暂存区差异"},
			},
		},
	}
}

// Execute 按参数拼装 git diff 命令并返回差异文本。
func (t *gitDiffTool) Execute(ctx context.Context, call Call) Result {
	args := call.Args
	// 参数逐项追加，路径放在 -- 之后，防止把文件名误当成选项。
	arguments := []string{"diff", "--no-color"}
	if boolArg(args, "staged") {
		arguments = append(arguments, "--staged")
	}
	if stringArg(args, "path") != "" {
		arguments = append(arguments, "--", call.Display)
	}
	output, exitCode, timedOut, err := runGit(ctx, call.Workspace, gitTimeout, arguments...)
	if timedOut {
		return errorResult("git diff 超时")
	}
	if err != nil && exitCode < 0 {
		return errorResult("执行 git diff 失败: %v", err)
	}
	output = strings.TrimRight(output, "\n")
	if strings.TrimSpace(output) == "" {
		return Result{Output: "没有差异"}
	}
	result := Result{Output: output, Meta: map[string]any{"staged": boolArg(args, "staged")}}
	if exitCode != 0 {
		result.IsError = true
	}
	return result
}

// ---------- git_log ----------

// gitLogTool 查看最近的提交记录，只读。
type gitLogTool struct{}

// Spec 声明 git_log 的元数据：limit 控制条数，path 可选。
func (t *gitLogTool) Spec() Spec {
	return Spec{
		Name:        "git_log",
		Description: "查看最近的 Git 提交记录（只读），默认 20 条。",
		Risk:        policy.R0,
		PathArg:     "path",
		SummaryArg:  "path",
		SummaryText: "git log",
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"limit": map[string]any{"type": "integer", "minimum": 1, "maximum": 100, "description": "提交条数，默认 20"},
				"path":  map[string]any{"type": "string", "description": "可选的相对路径"},
			},
		},
	}
}

// Execute 输出单行格式的提交列表，条数夹取到 1–100。
func (t *gitLogTool) Execute(ctx context.Context, call Call) Result {
	args := call.Args
	limit := intArg(args, "limit", 20)
	if limit < 1 {
		limit = 1
	}
	if limit > 100 {
		limit = 100
	}
	arguments := []string{"log", "--oneline", "--decorate", "-n", fmt.Sprintf("%d", limit)}
	if stringArg(args, "path") != "" {
		arguments = append(arguments, "--", call.Display)
	}
	output, exitCode, timedOut, err := runGit(ctx, call.Workspace, gitTimeout, arguments...)
	if timedOut {
		return errorResult("git log 超时")
	}
	if err != nil && exitCode < 0 {
		return errorResult("执行 git log 失败: %v", err)
	}
	output = strings.TrimSpace(output)
	if output == "" {
		return Result{Output: "还没有提交记录"}
	}
	result := Result{Output: output, Meta: map[string]any{"limit": limit}}
	if exitCode != 0 {
		result.IsError = true
	}
	return result
}

// ---------- git_commit ----------

// gitCommitTool 提交已修改或指定路径的文件，不执行推送。
type gitCommitTool struct{}

// Spec 声明 git_commit 的元数据：message 必填，paths 可选，风险等级 R1。
func (t *gitCommitTool) Spec() Spec {
	return Spec{
		Name:        "git_commit",
		Description: "提交已修改或指定的文件。不会推送；没有指定 paths 时只提交已跟踪文件的修改。",
		Risk:        policy.R1,
		Title:       "提交代码",
		SummaryArg:  "message",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"message"},
			"properties": map[string]any{
				"message": map[string]any{"type": "string", "description": "提交信息"},
				"paths": map[string]any{
					"type":        "array",
					"items":       map[string]any{"type": "string"},
					"description": "要提交的文件路径；省略时提交已跟踪文件的修改",
				},
			},
		},
	}
}

// Execute 校验提交信息与路径后暂存并提交；未指定路径时只提交已跟踪文件。
func (t *gitCommitTool) Execute(ctx context.Context, call Call) Result {
	args := call.Args
	message := strings.TrimSpace(stringArg(args, "message"))
	if message == "" {
		return errorResult("message 不能为空")
	}
	// 提交信息里出现 NUL 字节说明输入异常，直接拒绝而不是传给子进程。
	if strings.ContainsRune(message, 0) {
		return errorResult("message 包含非法字符")
	}
	if len(message) > 2000 {
		return errorResult("message 过长（最多 2000 字节）")
	}

	var paths []string
	if raw, ok := args["paths"].([]any); ok {
		for _, item := range raw {
			path, _ := item.(string)
			if strings.TrimSpace(path) == "" {
				continue
			}
			// 路径列表同样要经过工作区边界校验，并转换成相对路径交给 git。
			_, display, err := call.Resolve(path)
			if err != nil {
				return errorResult("%v", err)
			}
			paths = append(paths, display)
		}
	}

	if len(paths) > 0 {
		if result := t.gitAdd(ctx, call.Workspace, paths); result != nil {
			return *result
		}
	}

	// 指定路径时先暂存再提交这些路径；否则用 -a 提交全部已跟踪文件的修改。
	arguments := []string{"commit", "-m", message}
	if len(paths) > 0 {
		arguments = append(arguments, "--")
		arguments = append(arguments, paths...)
	} else {
		arguments = []string{"commit", "-am", message}
	}
	output, exitCode, timedOut, err := runGit(ctx, call.Workspace, gitTimeout, arguments...)
	if timedOut {
		return errorResult("git commit 超时")
	}
	if err != nil && exitCode < 0 {
		return errorResult("执行 git commit 失败: %v", err)
	}
	if exitCode != 0 {
		return errorResult("提交失败: %s", truncate(strings.TrimSpace(output), maxOutputBytes))
	}

	// 提交成功后取短哈希用于回显；这一步失败不影响提交本身。
	hash, _, _, _ := runGit(ctx, call.Workspace, 10*time.Second, "rev-parse", "--short", "HEAD")
	hash = strings.TrimSpace(hash)
	subject := strings.SplitN(message, "\n", 2)[0]
	line := fmt.Sprintf("已提交 %s：%s\n（未推送）", hash, subject)
	return Result{Output: line, Meta: map[string]any{"hash": hash, "message": message, "paths": paths}}
}

// gitAdd 暂存指定路径。返回 nil 表示成功，否则返回可直接回给模型的失败结果。
func (t *gitCommitTool) gitAdd(ctx context.Context, workspace string, paths []string) *Result {
	arguments := append([]string{"add", "--"}, paths...)
	output, exitCode, timedOut, err := runGit(ctx, workspace, gitTimeout, arguments...)
	if timedOut {
		result := errorResult("git add 超时")
		return &result
	}
	if err != nil && exitCode < 0 {
		result := errorResult("执行 git add 失败: %v", err)
		return &result
	}
	if exitCode != 0 {
		result := errorResult("暂存文件失败: %s", strings.TrimSpace(output))
		return &result
	}
	return nil
}

// runGit 直接执行 git 子命令，不经过 shell，避免参数注入。
func runGit(parent context.Context, dir string, timeout time.Duration, args ...string) (string, int, bool, error) {
	result := platform.Run(parent, platform.RunSpec{
		Path: "git", Args: args, Dir: dir, Timeout: timeout, OutputLimit: maxOutputBytes,
	})
	return result.Output, result.ExitCode, result.TimedOut, result.Err
}
