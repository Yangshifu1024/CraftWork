package prompt

import (
	"strings"
	"testing"
)

// 本文件的测试是系统提示词的契约：块序、缓存标记、稳定块与输入无关等约束
// 一旦被实现改动悄悄破坏，提示缓存就会失效或模型行为回退。改文案时先改这里的预期。

// TestBuildBlockOrderAndCacheMarks 验证块序与缓存标记：
// 前缀块无标记在最前，稳定块与动态块都带标记依次跟随。
func TestBuildBlockOrderAndCacheMarks(t *testing.T) {
	blocks, _ := Build(Input{Mode: "code", Workspace: "/tmp/ws"})
	if len(blocks) != 3 {
		t.Fatalf("应产生三个分块: %d", len(blocks))
	}
	if !strings.HasPrefix(blocks[0].Content, "你是 CraftWork") {
		t.Fatalf("前缀块应以产品身份开头: %q", blocks[0].Content)
	}
	if blocks[0].CacheControl {
		t.Fatalf("前缀块不应携带缓存标记: %+v", blocks[0])
	}
	if !blocks[1].CacheControl || !blocks[2].CacheControl {
		t.Fatalf("稳定块与动态块都应携带缓存标记: %+v", blocks[1:])
	}
	if !strings.Contains(blocks[1].Content, "安全测试") || !strings.Contains(blocks[1].Content, "# 行为契约") {
		t.Fatalf("稳定块应包含安全边界与行为契约: %q", blocks[1].Content)
	}
	if !strings.Contains(blocks[2].Content, "当前模式：Code") {
		t.Fatalf("动态块应包含模式声明: %q", blocks[2].Content)
	}
}

// TestStableBlockIgnoresInput 是缓存稳定性的核心契约：
// 稳定块的内容不得随模式、工作区或受限状态变化——否则每次会话切换
// 都会击穿所有会话的稳定缓存。
func TestStableBlockIgnoresInput(t *testing.T) {
	_, baseline := Build(Input{Mode: "code", Workspace: "/tmp/a", Restricted: false})
	var baselineStable []Stat
	for _, stat := range baseline {
		if stat.Hint == HintStable {
			baselineStable = append(baselineStable, stat)
		}
	}
	if len(baselineStable) == 0 {
		t.Fatal("缺少稳定节统计")
	}

	for _, in := range []Input{
		{Mode: "work", Workspace: "/tmp/b", Restricted: true},
		{Mode: "work", Workspace: "", Restricted: false},
		{Mode: "code", Workspace: "/很长的/中文/路径", Restricted: true},
	} {
		_, stats := Build(in)
		var stable []Stat
		for _, stat := range stats {
			if stat.Hint == HintStable {
				stable = append(stable, stat)
			}
		}
		if len(stable) != len(baselineStable) {
			t.Fatalf("稳定节数量随输入变化: %d != %d", len(stable), len(baselineStable))
		}
		for i := range stable {
			if stable[i].Chars != baselineStable[i].Chars {
				t.Fatalf("稳定节 %s 的长度随输入变化: %d != %d",
					stable[i].Source, stable[i].Chars, baselineStable[i].Chars)
			}
		}
	}
}

// TestStableBlockExcludesVolatileFacts 验证易变事实不进稳定块：
// 工作区路径只允许出现在动态块。
func TestStableBlockExcludesVolatileFacts(t *testing.T) {
	const workspace = "/Users/demo/项目"
	blocks, _ := Build(Input{Mode: "code", Workspace: workspace})
	if strings.Contains(blocks[1].Content, workspace) {
		t.Fatalf("稳定块不应包含工作区路径: %q", blocks[1].Content)
	}
	if !strings.Contains(blocks[2].Content, workspace) {
		t.Fatalf("动态块应包含工作区路径: %q", blocks[2].Content)
	}
}

// TestWorkModeRules 验证 Work 模式的动态规则：声明无 shell 与 Git 能力，
// 并提示切换模式；Code 模式不出现该限制。
func TestWorkModeRules(t *testing.T) {
	blocks, _ := Build(Input{Mode: "work", Workspace: "/tmp/ws"})
	if !strings.Contains(blocks[2].Content, "当前模式：Work") {
		t.Fatalf("Work 模式声明缺失: %q", blocks[2].Content)
	}
	if !strings.Contains(blocks[2].Content, "没有 shell 与 Git") {
		t.Fatalf("Work 模式应声明能力边界: %q", blocks[2].Content)
	}

	blocks, _ = Build(Input{Mode: "code", Workspace: "/tmp/ws"})
	if strings.Contains(blocks[2].Content, "没有 shell 与 Git") {
		t.Fatalf("Code 模式不应出现 Work 限制: %q", blocks[2].Content)
	}
}

// TestRestrictedClause 验证受限工作区条款只在受限时出现，
// 并包含「不提供永久授权」与「不要反复重试」两个关键约束。
func TestRestrictedClause(t *testing.T) {
	blocks, _ := Build(Input{Mode: "code", Workspace: "/tmp/ws", Restricted: true})
	if !strings.Contains(blocks[2].Content, "不提供永久授权") || !strings.Contains(blocks[2].Content, "不要反复重试") {
		t.Fatalf("受限条款缺失: %q", blocks[2].Content)
	}

	blocks, _ = Build(Input{Mode: "code", Workspace: "/tmp/ws", Restricted: false})
	if strings.Contains(blocks[2].Content, "未标记为可信") {
		t.Fatalf("非受限工作区不应出现受限条款: %q", blocks[2].Content)
	}
}

// TestStatsMatchContent 验证用量统计：三个来源齐全，各层当前只有一节，
// 因此各节字符数之和应等于所有分块内容的字符数之和。
func TestStatsMatchContent(t *testing.T) {
	blocks, stats := Build(Input{Mode: "work", Workspace: "/tmp/ws", Restricted: true})
	if len(stats) != 3 {
		t.Fatalf("应有三条节统计: %d", len(stats))
	}
	wantSources := []string{"prefix", "identity", "mode_rules"}
	for i, source := range wantSources {
		if stats[i].Source != source {
			t.Fatalf("节顺序不符: %s != %s", stats[i].Source, source)
		}
	}
	total := 0
	for _, block := range blocks {
		total += len([]rune(block.Content))
	}
	counted := 0
	for _, stat := range stats {
		counted += stat.Chars
	}
	if counted != total {
		t.Fatalf("节统计之和 %d 应等于分块内容之和 %d", counted, total)
	}
}
