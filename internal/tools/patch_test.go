package tools

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const twoHunkPatch = `@@ -2,3 +2,3 @@
 b
-c
+C
 d
@@ -4,2 +4,2 @@
 d
-e
+E
`

func TestParseAndApplyPatch(t *testing.T) {
	hunks, err := parseUnifiedPatch(twoHunkPatch)
	if err != nil {
		t.Fatal(err)
	}
	updated, applied, additions, deletions, err := applyHunks("a\nb\nc\nd\ne", hunks, nil)
	if err != nil {
		t.Fatal(err)
	}
	if updated != "a\nb\nC\nd\nE" {
		t.Fatalf("应用结果不符: %q", updated)
	}
	if applied != 2 || additions != 2 || deletions != 2 {
		t.Fatalf("统计不符: applied=%d +%d -%d", applied, additions, deletions)
	}
}

func TestApplySelectedHunks(t *testing.T) {
	hunks, err := parseUnifiedPatch(twoHunkPatch)
	if err != nil {
		t.Fatal(err)
	}
	updated, applied, _, _, err := applyHunks("a\nb\nc\nd\ne", hunks, []int{1})
	if err != nil {
		t.Fatal(err)
	}
	if updated != "a\nb\nC\nd\ne" || applied != 1 {
		t.Fatalf("逐块应用结果不符: %q applied=%d", updated, applied)
	}
}

func TestApplyPatchRejectsMismatchedContext(t *testing.T) {
	hunks, err := parseUnifiedPatch(`@@ -1,2 +1,2 @@
 missing
-old
+new
`)
	if err != nil {
		t.Fatal(err)
	}
	if _, _, _, _, err := applyHunks("a\nb\nc", hunks, nil); err == nil {
		t.Fatal("上下文不匹配时应报错")
	}
}

func TestApplyPatchToolDryRunAndWrite(t *testing.T) {
	workspace := t.TempDir()
	path := filepath.Join(workspace, "note.txt")
	if err := os.WriteFile(path, []byte("a\nb\nc\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	backups := 0
	registry := NewRegistry(Options{
		Workspace: workspace,
		BackupDir: filepath.Join(workspace, "backups"),
		RecordBackup: func(string, string, string, int64) error {
			backups++
			return nil
		},
	})
	tool := &applyPatchTool{registry: registry}
	patch := "@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"

	dry := tool.Execute(context.Background(), map[string]any{
		"path": "note.txt", "patch": patch, "dry_run": true,
	})
	if dry.IsError || !strings.Contains(dry.Output, "预演") {
		t.Fatalf("预演结果不符: %+v", dry)
	}
	content, _ := os.ReadFile(path)
	if string(content) != "a\nb\nc\n" {
		t.Fatalf("预演不应写入文件: %q", string(content))
	}
	if backups != 0 {
		t.Fatalf("预演不应备份: %d", backups)
	}

	applied := tool.Execute(context.Background(), map[string]any{"path": "note.txt", "patch": patch})
	if applied.IsError || !strings.Contains(applied.Output, "已应用") {
		t.Fatalf("应用结果不符: %+v", applied)
	}
	content, _ = os.ReadFile(path)
	if string(content) != "a\nB\nc\n" {
		t.Fatalf("应用结果不符: %q", string(content))
	}
	if backups != 1 {
		t.Fatalf("写入前应备份一次: %d", backups)
	}
}

func TestApplyPatchToolRejectsUnknownFile(t *testing.T) {
	registry := NewRegistry(Options{Workspace: t.TempDir()})
	tool := &applyPatchTool{registry: registry}
	result := tool.Execute(context.Background(), map[string]any{
		"path": "missing.txt", "patch": twoHunkPatch,
	})
	if !result.IsError || !strings.Contains(result.Output, "只修改已有文件") {
		t.Fatalf("不存在的文件应被拒绝: %+v", result)
	}
}
