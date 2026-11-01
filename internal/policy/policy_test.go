package policy

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestResolveAbsolutePathThroughSymlinkedWorkspace(t *testing.T) {
	root := t.TempDir()
	realDir := filepath.Join(root, "real")
	if err := os.MkdirAll(realDir, 0o755); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(root, "link")
	if err := os.Symlink(realDir, link); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(realDir, "note.txt")
	if err := os.WriteFile(file, []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}

	canonical, err := filepath.EvalSymlinks(file)
	if err != nil {
		t.Fatal(err)
	}
	resolved, err := Resolve(link, filepath.Join(link, "note.txt"))
	if err != nil {
		t.Fatalf("符号链接工作区中的绝对路径不应失败: %v", err)
	}
	if resolved != canonical {
		t.Fatalf("解析结果不符: got %s want %s", resolved, canonical)
	}
}

func TestResolveRejectsEscape(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	if err := os.WriteFile(filepath.Join(outside, "secret.txt"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, filepath.Join(root, "link")); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(root, "internal"), 0o755); err != nil {
		t.Fatal(err)
	}

	if _, err := Resolve(root, "link/secret.txt"); !errors.Is(err, ErrOutsideWorkspace) {
		t.Fatalf("符号链接逃逸未被拒绝: %v", err)
	}
	if _, err := Resolve(root, "../etc/passwd"); !errors.Is(err, ErrOutsideWorkspace) {
		t.Fatalf("路径穿越未被拒绝: %v", err)
	}

	canonicalRoot, err := filepath.EvalSymlinks(root)
	if err != nil {
		t.Fatal(err)
	}
	resolved, err := Resolve(root, "internal/new.go")
	if err != nil {
		t.Fatalf("工作区内路径不应失败: %v", err)
	}
	if resolved != filepath.Join(canonicalRoot, "internal", "new.go") {
		t.Fatalf("路径解析结果不符: %s", resolved)
	}
}

func TestActionByLevel(t *testing.T) {
	graded := New(LevelGraded, "/tmp")
	if graded.Action(R0) != ActionAllow || graded.Action(R1) != ActionAsk ||
		graded.Action(R3) != ActionAsk || graded.Action(R4) != ActionDeny {
		t.Fatal("分级档位处置不符合预期")
	}
	if !graded.AllowsPermanent(R2) {
		t.Fatal("分级档位应允许 R2 永久授权")
	}
	if graded.AllowsPermanent(R3) {
		t.Fatal("R3 不应允许永久授权")
	}

	strict := New(LevelStrict, "/tmp")
	if strict.Action(R1) != ActionAsk || strict.AllowsPermanent(R1) {
		t.Fatal("严格档位不应提供永久授权")
	}

	readonly := New(LevelReadonly, "/tmp")
	if readonly.Action(R1) != ActionDeny || readonly.Action(R0) != ActionAllow {
		t.Fatal("只读档位处置不符合预期")
	}

	restricted := New(LevelGraded, "/tmp")
	restricted.SetRestricted(true)
	if restricted.Action(R3) != ActionDeny {
		t.Fatal("受限工作区应直接拒绝网络访问")
	}
	if restricted.Action(R1) != ActionAsk {
		t.Fatal("受限工作区写入仍需审批")
	}
	if restricted.AllowsPermanent(R1) {
		t.Fatal("受限工作区不应提供永久授权")
	}
}

func TestClassifyCommand(t *testing.T) {
	if risk, _ := ClassifyCommand("rm -rf /"); risk != R4 {
		t.Fatalf("破坏性命令应为 R4，实际 %s", risk)
	}
	if risk, _ := ClassifyCommand("curl https://example.com"); risk != R3 {
		t.Fatalf("联网命令应为 R3，实际 %s", risk)
	}
	if risk, _ := ClassifyCommand("go test ./..."); risk != R2 {
		t.Fatalf("普通命令应为 R2，实际 %s", risk)
	}
	if risk, _ := ClassifyCommand(""); risk != R2 {
		t.Fatalf("空命令应为 R2，实际 %s", risk)
	}
}
