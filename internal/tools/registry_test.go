package tools

import (
	"context"
	"strings"
	"testing"

	"craftwork/internal/policy"
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

	assessment, call, exists := registry.Assess(workspace, "fs_read", map[string]any{"path": "../secret"})
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

	network, _, _ := registry.Assess(t.TempDir(), "shell_exec", map[string]any{"command": "curl https://example.com"})
	if network.Risk != policy.R3 {
		t.Fatalf("联网命令应为 R3，实际 %s", network.Risk)
	}
	destructive, _, _ := registry.Assess(t.TempDir(), "shell_exec", map[string]any{"command": "rm -rf /"})
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

	_, call, exists := registry.Assess(t.TempDir(), "mcp__demo__big", map[string]any{})
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
