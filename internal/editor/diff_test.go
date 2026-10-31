package editor

import "testing"

func TestDiffCounts(t *testing.T) {
	oldText := "a\nb\nc\nd\ne\n"
	newText := "a\nb\nX\nd\ne\n"
	result := Diff(oldText, newText)
	if result.Additions != 1 || result.Deletions != 1 {
		t.Fatalf("期望 1 增 1 删，实际 %d/%d", result.Additions, result.Deletions)
	}
	if len(result.Hunks) != 1 {
		t.Fatalf("期望 1 个差异块，实际 %d", len(result.Hunks))
	}
	if result.Truncated {
		t.Fatal("小文件不应被标记截断")
	}
}

func TestDiffNoChange(t *testing.T) {
	result := Diff("same\n", "same\n")
	if result.Additions != 0 || result.Deletions != 0 || len(result.Hunks) != 0 {
		t.Fatal("相同文本不应产生差异")
	}
}

func TestDiffInsertAtEnd(t *testing.T) {
	result := Diff("a\n", "a\nb\n")
	if result.Additions != 1 || result.Deletions != 0 {
		t.Fatalf("期望 1 行新增，实际 %d/%d", result.Additions, result.Deletions)
	}
}
