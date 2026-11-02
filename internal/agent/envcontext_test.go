package agent

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestCollectInstructions 验证两级指令文件的读取顺序与标签：
// 全局在前、项目在后；缺失的文件跳过；空文件跳过。
func TestCollectInstructions(t *testing.T) {
	dataDir := t.TempDir()
	workspace := t.TempDir()
	if err := os.WriteFile(filepath.Join(dataDir, "AGENTS.md"), []byte("全局约定"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(workspace, "AGENTS.md"), []byte("项目约定"), 0o644); err != nil {
		t.Fatal(err)
	}

	snapshot := collectEnvContext(context.Background(), dataDir, workspace)
	if len(snapshot.Instructions) != 2 {
		t.Fatalf("应读到两份指令文件: %d", len(snapshot.Instructions))
	}
	if snapshot.Instructions[0].Scope != "global" || snapshot.Instructions[0].Content != "全局约定" {
		t.Fatalf("全局指令不符: %+v", snapshot.Instructions[0])
	}
	if snapshot.Instructions[1].Scope != "project" || snapshot.Instructions[1].Content != "项目约定" {
		t.Fatalf("项目指令不符: %+v", snapshot.Instructions[1])
	}
	if snapshot.Env.Today == "" || snapshot.Env.Platform == "" {
		t.Fatalf("环境事实缺失: %+v", snapshot.Env)
	}
	// 临时目录不是 Git 仓库，快照应为 nil。
	if snapshot.Git != nil {
		t.Fatalf("非仓库不应产生 Git 快照: %+v", snapshot.Git)
	}
}

// TestCollectInstructionsMissing 验证没有指令文件时不产生内容。
func TestCollectInstructionsMissing(t *testing.T) {
	snapshot := collectEnvContext(context.Background(), t.TempDir(), t.TempDir())
	if len(snapshot.Instructions) != 0 {
		t.Fatalf("不应读到指令文件: %+v", snapshot.Instructions)
	}
}

// TestReadInstructionTruncation 验证行数截断与截断标记。
func TestReadInstructionTruncation(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "AGENTS.md")
	content := ""
	for i := 0; i < instructionMaxLines+50; i++ {
		content += "规则行\n"
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	item := readInstruction(path, "project")
	if item == nil {
		t.Fatal("应读到指令文件")
	}
	if !item.Truncated {
		t.Fatal("超行数文件应标记截断")
	}
	if lines := strings.Count(item.Content, "\n") + 1; lines != instructionMaxLines {
		t.Fatalf("应只读取 %d 行: %d", instructionMaxLines, lines)
	}
}

// TestReadInstructionEmpty 验证空白文件不产生指令。
func TestReadInstructionEmpty(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "AGENTS.md")
	if err := os.WriteFile(path, []byte("\n  \n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if item := readInstruction(path, "project"); item != nil {
		t.Fatalf("空文件不应产生指令: %+v", item)
	}
}

// TestCollectGitSnapshot 验证在真实 Git 仓库中能读到分支名。
// 用当前仓库自身做样本（测试运行目录就在仓库内），临时目录场景由
// collectEnvContext 的非仓库分支覆盖。
func TestCollectGitSnapshot(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	// PR CI 以 detached HEAD 检出（merge commit），此时没有「当前分支」
	// 的概念，collectGitSnapshot 的 Branch 合法为空；实现不区分该场景，
	// 测试在这里跳过分支断言，本地与 main push 不受影响。
	if _, exitCode, _, _ := runGitCommand(context.Background(), wd, "symbolic-ref", "--short", "-q", "HEAD"); exitCode != 0 {
		t.Skip("detached HEAD（如 PR CI 的 merge 检出）没有当前分支，跳过分支断言")
	}
	snapshot := collectGitSnapshot(context.Background(), wd)
	if snapshot == nil {
		t.Fatal("当前目录是 Git 仓库，应产生快照")
	}
	if snapshot.Branch == "" {
		t.Fatalf("分支名不应为空: %+v", snapshot)
	}
}
