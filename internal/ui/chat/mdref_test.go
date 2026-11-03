// mdref 单测：对齐 frontend/src/features/chat/markdownRef.test.ts 的用例
// 与 Markdown.tsx preprocess 的行为（三种写法、围栏保护、解析回引）。

package chat

import (
	"strings"
	"testing"
)

// 对齐「matchPathLineRef」的 11 个用例。
func TestMatchPathLineRef(t *testing.T) {
	cases := []struct {
		text string
		want PathLineRef
		ok   bool
	}{
		{"runner.go:31", PathLineRef{"runner.go", 31}, true},
		{"internal/agent/runner.go:208", PathLineRef{"internal/agent/runner.go", 208}, true},
		{"internal/app.v2/MessageView.test.tsx:12", PathLineRef{"internal/app.v2/MessageView.test.tsx", 12}, true},
		{"a.go:1", PathLineRef{"a.go", 1}, true},
		{"a.go:99999", PathLineRef{"a.go", 99999}, true},
		{"a.go:0", PathLineRef{"a.go", 0}, true}, // 行号 0 原样返回
		{"见 runner.go:31 的定义", PathLineRef{}, false},
		{"foo.go:1 bar.go:2", PathLineRef{}, false},
		{"internal/agent/runner.go", PathLineRef{}, false},
		{"runner.go:31:5", PathLineRef{}, false},
		{"Makefile:3", PathLineRef{}, false},
		{"a.go:123456", PathLineRef{}, false},
		{"backup.abcdefghij:1", PathLineRef{}, false},
	}
	for _, c := range cases {
		got, ok := MatchPathLineRef(c.text)
		if ok != c.ok || got != c.want {
			t.Errorf("MatchPathLineRef(%q) = %v,%v; want %v,%v", c.text, got, ok, c.want, c.ok)
		}
	}
}

// 对齐「preprocess」：围栏外裸文本转内部链接，围栏内不动。
func TestPreprocessRefsBare(t *testing.T) {
	in := "看 internal/agent/runner.go:208 的定义。\n```go\nfmt.Println(\"a.go:12\")\n```\n再看 runner.go:31。"
	out := PreprocessRefs(in)
	want := "看 [internal/agent/runner.go:208](craftwork://internal/agent/runner.go:208) 的定义。\n" +
		"```go\nfmt.Println(\"a.go:12\")\n```\n" +
		"再看 [runner.go:31](craftwork://runner.go:31)。"
	if out != want {
		t.Fatalf("预处理结果不符:\n got  %q\n want %q", out, want)
	}
}

// 空目标链接与行内代码引用同样转为内部链接；已是链接的文本不二次包裹。
func TestPreprocessRefsLinkAndCode(t *testing.T) {
	in := "空目标 [policy.go:176]() 与行内代码 `main.go:42`，已转换 [x.go:1](craftwork://x.go:1) 保持。"
	out := PreprocessRefs(in)
	want := "空目标 [policy.go:176](craftwork://policy.go:176) 与行内代码 [main.go:42](craftwork://main.go:42)，" +
		"已转换 [x.go:1](craftwork://x.go:1) 保持。"
	if out != want {
		t.Fatalf("预处理结果不符:\n got  %q\n want %q", out, want)
	}
}

// 中文引号与全角括号后的引用同样命中。
func TestPreprocessRefsCJKPrefix(t *testing.T) {
	out := PreprocessRefs("见（cmd/main.go:10）与「a/b.go:2」。")
	if !strings.Contains(out, "[cmd/main.go:10](craftwork://cmd/main.go:10)") ||
		!strings.Contains(out, "[a/b.go:2](craftwork://a/b.go:2)") {
		t.Fatalf("中文标点后的引用应被转换: %q", out)
	}
}

// ParseRefLink 从内部链接换算回路径与行号；非内部链接被拒绝。
func TestParseRefLink(t *testing.T) {
	ref, ok := ParseRefLink("craftwork://internal/agent/runner.go:208")
	if !ok || ref.Path != "internal/agent/runner.go" || ref.Line != 208 {
		t.Fatalf("内部链接应解析回引用: %+v %v", ref, ok)
	}
	if _, ok := ParseRefLink("https://example.com/a.go:1"); ok {
		t.Fatal("外部链接不应被当作内部引用")
	}
}
