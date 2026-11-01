package title

import (
	"strings"
	"testing"
)

func TestDerive(t *testing.T) {
	cases := []struct {
		name        string
		text        string
		attachments []string
		want        string
	}{
		{name: "普通中文", text: "帮我修复登录刷新超时", want: "帮我修复登录刷新超时"},
		{name: "清理 Markdown 标题", text: "## 修复登录问题\n正文", want: "修复登录问题"},
		{name: "跳过代码围栏", text: "```go\nfunc main() {}\n```", want: "func main() {}"},
		{name: "纯附件消息", text: "", attachments: []string{"/tmp/季度会议纪要.docx"}, want: "季度会议纪要.docx"},
		{name: "纯标点不命名", text: "！！！", want: ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := Derive(tc.text, tc.attachments); got != tc.want {
				t.Fatalf("标题不符: got %q want %q", got, tc.want)
			}
		})
	}
}

func TestLimitKeepsMaxRunes(t *testing.T) {
	text := "登录后 15 分钟刷新令牌就超时了，帮我看看 internal/auth/session.go 里的刷新逻辑。"
	limited := Limit(Normalize(text))
	if len([]rune(limited)) > MaxRunes {
		t.Fatalf("标题超过 %d 个字符: %q", MaxRunes, limited)
	}
	if !strings.HasSuffix(limited, "…") {
		t.Fatalf("截断后应带省略号: %q", limited)
	}
	if !strings.HasPrefix(text, strings.TrimSuffix(limited, "…")) {
		t.Fatalf("标题应来自原文: %q", limited)
	}
}

func TestShortUsesOriginalTextWithinLimit(t *testing.T) {
	exact := strings.Repeat("超", MaxRunes)
	cases := map[string]string{
		"帮我修复登录刷新超时":      "帮我修复登录刷新超时",
		"## 排查终端问题\n多余内容": "排查终端问题",
		"  “整理季度会议纪要”  ":  "整理季度会议纪要",
		"！！！":             "",
		exact:             exact,
		exact + "超":       "",
	}
	for input, want := range cases {
		if got := Short(input); got != want {
			t.Fatalf("Short(%q) = %q, want %q", input, got, want)
		}
	}
}

func TestNormalizeModelOutput(t *testing.T) {
	cases := map[string]string{
		"  “修复登录刷新超时”  ": "修复登录刷新超时",
		"标题：写文件。":        "标题：写文件",
		"# 排查终端问题\n多余内容": "排查终端问题",
	}
	for input, want := range cases {
		if got := Normalize(input); got != want {
			t.Fatalf("清洗不符: got %q want %q", got, want)
		}
	}
}
