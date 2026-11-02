package tools

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func gitTestRepo(t *testing.T) string {
	t.Helper()
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("未安装 git")
	}
	repo := t.TempDir()
	runGitTest(t, repo, "init")
	runGitTest(t, repo, "config", "user.email", "test@example.com")
	runGitTest(t, repo, "config", "user.name", "CraftWork Test")
	return repo
}

func runGitTest(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s 失败: %v\n%s", strings.Join(args, " "), err, output)
	}
	return string(output)
}

func TestGitTools(t *testing.T) {
	repo := gitTestRepo(t)
	path := filepath.Join(repo, "a.txt")
	if err := os.WriteFile(path, []byte("a\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	registry := NewRegistry(Options{})
	run := func(name string, args map[string]any) Result {
		t.Helper()
		_, call, exists := registry.Assess([]string{repo}, name, args)
		if !exists {
			t.Fatalf("%s 未注册", name)
		}
		return registry.Execute(context.Background(), call)
	}

	result := run("git_commit", map[string]any{
		"message": "初始提交",
		"paths":   []any{"a.txt"},
	})
	if result.IsError || !strings.Contains(result.Output, "已提交") || !strings.Contains(result.Output, "未推送") {
		t.Fatalf("首次提交失败: %+v", result)
	}

	logResult := run("git_log", map[string]any{"limit": 5})
	if logResult.IsError || !strings.Contains(logResult.Output, "初始提交") {
		t.Fatalf("git log 不符: %+v", logResult)
	}

	if err := os.WriteFile(path, []byte("b\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	diffResult := run("git_diff", map[string]any{"path": "a.txt"})
	if diffResult.IsError || !strings.Contains(diffResult.Output, "-a") || !strings.Contains(diffResult.Output, "+b") {
		t.Fatalf("git diff 不符: %+v", diffResult)
	}

	dryEmpty := run("git_commit", map[string]any{"message": "空提交"})
	if dryEmpty.IsError {
		t.Fatalf("存在已跟踪修改时提交应成功: %+v", dryEmpty)
	}

	empty := run("git_commit", map[string]any{"message": "空提交"})
	if !empty.IsError {
		t.Fatalf("没有改动时提交应失败: %+v", empty)
	}

	status := runGitTest(t, repo, "status", "--porcelain")
	if strings.TrimSpace(status) != "" {
		t.Fatalf("提交后工作区应干净: %q", status)
	}
}
