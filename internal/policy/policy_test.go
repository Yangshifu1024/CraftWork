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
	graded := New(LevelGraded)
	if graded.Action(false, R0) != ActionAllow || graded.Action(false, R1) != ActionAsk ||
		graded.Action(false, R3) != ActionAsk || graded.Action(false, R4) != ActionDeny {
		t.Fatal("分级档位处置不符合预期")
	}
	if !graded.AllowsPermanent(false, R2) {
		t.Fatal("分级档位应允许 R2 永久授权")
	}
	if graded.AllowsPermanent(false, R3) {
		t.Fatal("R3 不应允许永久授权")
	}

	strict := New(LevelStrict)
	if strict.Action(false, R1) != ActionAsk || strict.AllowsPermanent(false, R1) {
		t.Fatal("严格档位不应提供永久授权")
	}

	readonly := New(LevelReadonly)
	if readonly.Action(false, R1) != ActionDeny || readonly.Action(false, R0) != ActionAllow {
		t.Fatal("只读档位处置不符合预期")
	}

	// 受限工作区作为参数传入，引擎不再持有可变的信任状态。
	restricted := New(LevelGraded)
	if restricted.Action(true, R3) != ActionDeny {
		t.Fatal("受限工作区应直接拒绝网络访问")
	}
	if restricted.Action(true, R1) != ActionAsk {
		t.Fatal("受限工作区写入仍需审批")
	}
	if restricted.AllowsPermanent(true, R1) {
		t.Fatal("受限工作区不应提供永久授权")
	}

	// 自由档位放行一切，包括 R4 与受限工作区的 R3；
	// 审批卡不会出现，因此也不需要永久授权。
	free := New(LevelFree)
	for _, risk := range []Risk{R0, R1, R2, R3, R4} {
		if free.Action(false, risk) != ActionAllow {
			t.Fatalf("自由档位应放行 %s", risk)
		}
	}
	if free.Action(true, R3) != ActionAllow || free.Action(true, R4) != ActionAllow {
		t.Fatal("自由档位在受限工作区也应放行（用户已显式确认）")
	}
	if free.AllowsPermanent(false, R1) || free.AllowsPermanent(false, R2) {
		t.Fatal("自由档位不应提供永久授权：审批卡不会出现")
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

func TestResolveAnyMultiRoot(t *testing.T) {
	primary := t.TempDir()
	additional := t.TempDir()
	missing := filepath.Join(t.TempDir(), "gone")
	if err := os.MkdirAll(filepath.Join(additional, "docs"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(additional, "docs", "a.md"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	// macOS 的 /var 是 /private/var 的别名：先归一化再构造根列表与断言，
	// 与 ResolveAny 返回的真实路径口径一致。
	var canonicalErr error
	if primary, canonicalErr = filepath.EvalSymlinks(primary); canonicalErr != nil {
		t.Fatal(canonicalErr)
	}
	if additional, canonicalErr = filepath.EvalSymlinks(additional); canonicalErr != nil {
		t.Fatal(canonicalErr)
	}
	roots := []string{primary, additional, missing}

	// 相对路径按根顺序命中：主根优先。
	hit, err := ResolveAny(roots, filepath.Join(primary, "in-primary.txt"))
	if err != nil {
		t.Fatalf("主根内的绝对路径应命中主根: %v", err)
	}
	if hit.RootIndex != 0 {
		t.Fatalf("主根内的路径应命中下标 0，实际 %d", hit.RootIndex)
	}

	// 主根未命中时落到附加目录。
	hit, err = ResolveAny(roots, filepath.Join(additional, "docs", "a.md"))
	if err != nil {
		t.Fatalf("附加目录内的路径应命中: %v", err)
	}
	if hit.RootIndex != 1 {
		t.Fatalf("附加目录路径应命中下标 1，实际 %d", hit.RootIndex)
	}
	if hit.Rel != filepath.Join("docs", "a.md") {
		t.Fatalf("展示路径应相对命中根，实际 %s", hit.Rel)
	}

	// 相对路径在主根与附加根各有一个同名目录时，主根优先。
	nested := filepath.Join(primary, "docs")
	if err := os.MkdirAll(nested, 0o755); err != nil {
		t.Fatal(err)
	}
	hit, err = ResolveAny(roots, "docs")
	if err != nil {
		t.Fatalf("同名目录应命中主根: %v", err)
	}
	if hit.RootIndex != 0 {
		t.Fatalf("同名目录应优先主根，实际命中下标 %d", hit.RootIndex)
	}

	// 即将新建的文件（目标不存在）按词法命中主根：主根 docs 目录存在，
	// 按根顺序优先命中。
	hit, err = ResolveAny(roots, "docs/new-file.txt")
	if err != nil {
		t.Fatalf("新建文件应按词法命中: %v", err)
	}
	if hit.RootIndex != 0 || hit.Abs != filepath.Join(nested, "new-file.txt") {
		t.Fatalf("新建文件应命中主根，实际 %+v", hit)
	}

	// 主根没有对应目录、附加根有时，落到附加根（目标不存在但父目录存在，
	// 按词法判定命中后解析父目录拼回文件名）。
	if err := os.MkdirAll(filepath.Join(additional, "notes"), 0o755); err != nil {
		t.Fatal(err)
	}
	hit, err = ResolveAny(roots, "notes/new-note.md")
	if err != nil {
		t.Fatalf("主根未命中时应尝试附加根: %v", err)
	}
	if hit.RootIndex != 1 || hit.Abs != filepath.Join(additional, "notes", "new-note.md") {
		t.Fatalf("应命中附加根的新建路径，实际 %+v", hit)
	}

	// 全部根都未命中时报越界。
	if _, err := ResolveAny(roots, filepath.Join(t.TempDir(), "elsewhere.txt")); !errors.Is(err, ErrOutsideWorkspace) {
		t.Fatalf("越界路径应返回 ErrOutsideWorkspace，实际 %v", err)
	}

	// 主要目录不可用时报错；附加目录不可用则跳过。
	if _, err := ResolveAny([]string{missing, additional}, "docs/a.md"); err == nil {
		t.Fatal("主要目录不可用时应报错")
	}
	// 仅附加目录可用时（主根缺失且不可用），解析应成功落到附加根——但主要
	// 目录不可用按约定直接报错，因此这里验证的正是「不跳过主根」。
	if _, err := ResolveAny([]string{missing}, "docs/a.md"); err == nil {
		t.Fatal("仅有不可用根时应报错")
	}
}

func TestResolveAnyRejectsSymlinkEscapeInAdditionalRoot(t *testing.T) {
	primary := t.TempDir()
	additional := t.TempDir()
	outside := t.TempDir()
	if err := os.WriteFile(filepath.Join(outside, "secret.txt"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, filepath.Join(additional, "link")); err != nil {
		t.Fatal(err)
	}

	// 附加目录内的符号链接逃逸必须被拒绝，不能因为「词法在附加目录内」而放行。
	if _, err := ResolveAny([]string{primary, additional}, "link/secret.txt"); !errors.Is(err, ErrOutsideWorkspace) {
		t.Fatalf("附加目录内的链接逃逸应被拒绝，实际 %v", err)
	}
}
