package tools

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/policy"
)

type stubTool struct {
	spec   Spec
	output string
}

func (t *stubTool) Spec() Spec {
	return t.spec
}

func (t *stubTool) Execute(context.Context, Call) Result {
	return Result{Output: t.output}
}

func TestAssessMarksPathOutsideWorkspace(t *testing.T) {
	registry := NewRegistry(Options{})
	workspace := t.TempDir()

	assessment, call, exists := registry.Assess([]string{workspace}, "fs_read", map[string]any{"path": "../secret"})
	if !exists {
		t.Fatal("fs_read 应已注册")
	}
	if call.Err == nil {
		t.Fatal("越界路径应带解析错误，阻止执行")
	}
	if assessment.Risk != policy.R3 {
		t.Fatalf("越界路径应为 R3，实际 %s", assessment.Risk)
	}
	if result := registry.Execute(context.Background(), call); !result.IsError {
		t.Fatalf("越界调用不应执行: %+v", result)
	}
}

func TestAssessClassifiesCommands(t *testing.T) {
	registry := NewRegistry(Options{})

	network, _, _ := registry.Assess([]string{t.TempDir()}, "shell_exec", map[string]any{"command": "curl https://example.com"})
	if network.Risk != policy.R3 {
		t.Fatalf("联网命令应为 R3，实际 %s", network.Risk)
	}
	destructive, _, _ := registry.Assess([]string{t.TempDir()}, "shell_exec", map[string]any{"command": "rm -rf /"})
	if destructive.Risk != policy.R4 {
		t.Fatalf("破坏性命令应为 R4，实际 %s", destructive.Risk)
	}
}

func TestExecuteTruncatesDynamicToolOutput(t *testing.T) {
	registry := NewRegistry(Options{})
	registry.Register(&stubTool{
		spec:   Spec{Name: "mcp__demo__big", Risk: policy.R2},
		output: strings.Repeat("a", maxOutputBytes+100),
	})

	_, call, exists := registry.Assess([]string{t.TempDir()}, "mcp__demo__big", map[string]any{})
	if !exists {
		t.Fatal("动态工具应已注册")
	}
	result := registry.Execute(context.Background(), call)
	if !strings.Contains(result.Output, "输出已截断") {
		t.Fatalf("动态工具输出应被截断: %d 字节", len(result.Output))
	}
	if len(result.Output) > maxOutputBytes+100 {
		t.Fatalf("截断后仍然过长: %d 字节", len(result.Output))
	}
}

func TestSpecDrivesSummaryTitleAndWorkMode(t *testing.T) {
	registry := NewRegistry(Options{})

	spec, exists := registry.SpecFor("fs_write")
	if !exists {
		t.Fatal("fs_write 应已注册")
	}
	if spec.TitleOf() != "写入文件" {
		t.Fatalf("审批标题应来自 Spec: %s", spec.TitleOf())
	}
	if summary := spec.SummaryOf(map[string]any{"path": "a.txt"}); summary != "a.txt" {
		t.Fatalf("摘要应取 path 参数: %s", summary)
	}

	if registry.Allows("work", "shell_exec") {
		t.Fatal("Work 模式不应开放 shell_exec")
	}
	if !registry.Allows("work", "fs_write") {
		t.Fatal("Work 模式应开放 fs_write")
	}
	registerStub(registry)
	if !registry.Allows("work", "mcp__demo__echo") {
		t.Fatal("Work 模式应开放 MCP 工具")
	}
}

func registerStub(registry *Registry) {
	registry.Register(&stubTool{spec: Spec{Name: "mcp__demo__echo", Risk: policy.R2}})
}

func TestAssessResolvesAdditionalRoot(t *testing.T) {
	registry := NewRegistry(Options{})
	primary := t.TempDir()
	additional := t.TempDir()
	// macOS 的 /var 是 /private/var 的别名：归一化后再断言绝对路径。
	if canonical, err := filepath.EvalSymlinks(additional); err == nil {
		additional = canonical
	}
	target := filepath.Join(additional, "note.md")
	if err := os.WriteFile(target, []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}

	// 附加目录内的路径应命中并解析为绝对路径，展示路径相对附加根。
	assessment, call, exists := registry.Assess([]string{primary, additional}, "fs_read", map[string]any{"path": "note.md"})
	if !exists || call.Err != nil {
		t.Fatalf("附加目录内的路径应正常解析: %v", call.Err)
	}
	if call.Target != target {
		t.Fatalf("Target 应为附加目录内绝对路径: %s", call.Target)
	}
	if call.Display != "note.md" {
		t.Fatalf("Display 应相对附加根: %s", call.Display)
	}
	if call.TargetRoot != additional {
		t.Fatalf("TargetRoot 应为附加根: %s", call.TargetRoot)
	}
	if assessment.Risk != policy.R0 {
		t.Fatalf("读取文件应为 R0，实际 %s", assessment.Risk)
	}

	// git 工具保持主目录仓库语义：附加目录路径直接报错。
	_, call, _ = registry.Assess([]string{primary, additional}, "git_diff", map[string]any{"path": "note.md"})
	if call.Err == nil {
		t.Fatal("git 工具命中附加目录应报错")
	}
}

func TestCallResolveRejectsAdditionalRootForGit(t *testing.T) {
	registry := NewRegistry(Options{})
	primary := t.TempDir()
	additional := t.TempDir()

	_, call, _ := registry.Assess([]string{primary, additional}, "git_commit", map[string]any{"message": "x"})
	// 相对路径会按根顺序先命中主根，这是正确行为；用附加根内实际存在的
	// 文件验证「命中附加根时报错」。
	if err := os.WriteFile(filepath.Join(additional, "tracked.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, _, err := call.Resolve("tracked.txt"); err == nil {
		t.Fatal("git_commit 的 paths 命中附加目录应报错")
	}
	// 主根路径不受影响。
	if _, _, err := call.Resolve("."); err != nil {
		t.Fatalf("主根路径应正常解析: %v", err)
	}
}
