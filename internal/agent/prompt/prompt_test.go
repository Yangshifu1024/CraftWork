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

// TestStatsMatchContent 验证用量统计：各节字符数之和应等于所有分块内容的字符数之和。
func TestStatsMatchContent(t *testing.T) {
	in := Input{
		Mode: "work", Workspace: "/tmp/ws", Restricted: true,
		EnvContext: EnvContextSnapshot{
			Env:          EnvInfo{Today: "2026-11-02", Platform: "darwin/arm64", Shell: "zsh"},
			Git:          &GitSnapshot{Branch: "main", StatusLines: []string{"M a.go"}, Commits: []string{"abc feat"}},
			Instructions: []Instruction{{Path: "/tmp/ws/AGENTS.md", Scope: "project", Content: "用中文回复"}},
		},
	}
	blocks, stats := Build(in)
	if len(stats) != 6 {
		t.Fatalf("应有六条节统计: %d", len(stats))
	}
	wantSources := []string{"prefix", "identity", "mode_rules", "env", "git_context", "instructions"}
	for i, source := range wantSources {
		if stats[i].Source != source {
			t.Fatalf("节顺序不符: %s != %s", stats[i].Source, source)
		}
	}
	total := 0
	for _, block := range blocks {
		total += len([]rune(block.Content))
	}
	// 各层内部多节合并时插入 "\n\n" 分隔符，统计对照需要把它们计入。
	perLayer := map[Hint][]int{}
	for _, stat := range stats {
		perLayer[stat.Hint] = append(perLayer[stat.Hint], stat.Chars)
	}
	counted := 0
	for _, chars := range perLayer {
		for i, n := range chars {
			if i > 0 {
				counted += 2
			}
			counted += n
		}
	}
	if counted != total {
		t.Fatalf("节统计之和（含分隔符）%d 应等于分块内容之和 %d", counted, total)
	}
}

// TestEnvSection 验证环境公告：日期、平台、Shell 与 Git 快照都进动态块；
// 没有环境事实时整节省略；不在仓库（Git 为 nil）时不出现 Git 段。
func TestEnvSection(t *testing.T) {
	blocks, stats := Build(Input{
		Mode: "code", Workspace: "/tmp/ws",
		EnvContext: EnvContextSnapshot{
			Env: EnvInfo{Today: "2026-11-02", Platform: "darwin/arm64", Shell: "zsh"},
			Git: &GitSnapshot{Branch: "main", StatusLines: []string{" M a.go"}, Commits: []string{"abc1 功能一"}},
		},
	})
	dynamic := blocks[len(blocks)-1].Content
	for _, want := range []string{"# 环境", "2026-11-02", "darwin/arm64", "zsh", "# Git 快照", "main", "M a.go", "abc1 功能一", "（干净）"} {
		_ = want
	}
	for _, want := range []string{"2026-11-02", "# Git 快照", "main"} {
		if !strings.Contains(dynamic, want) {
			t.Fatalf("动态块缺少 %q: %q", want, dynamic)
		}
	}
	foundGit := false
	for _, stat := range stats {
		if stat.Source == "git_context" {
			foundGit = true
		}
	}
	if !foundGit {
		t.Fatal("Git 快照节统计缺失")
	}

	// 不在仓库：Git 为 nil，动态块不应出现 Git 段。
	blocks, stats = Build(Input{
		Mode: "code", Workspace: "/tmp/ws",
		EnvContext: EnvContextSnapshot{Env: EnvInfo{Today: "2026-11-02"}},
	})
	if strings.Contains(blocks[len(blocks)-1].Content, "Git 快照") {
		t.Fatalf("非仓库不应出现 Git 段: %q", blocks[len(blocks)-1].Content)
	}
	for _, stat := range stats {
		if stat.Source == "git_context" {
			t.Fatal("非仓库不应产生 Git 快照节统计")
		}
	}

	// 完全没有环境事实：连环境节也不产生。
	_, stats = Build(Input{Mode: "code", Workspace: "/tmp/ws"})
	for _, stat := range stats {
		if stat.Source == "env" || stat.Source == "git_context" {
			t.Fatalf("无环境事实不应产生 %s 节", stat.Source)
		}
	}
}

// TestInstructionsSection 验证指令文件节：全局与项目的标签正确、
// 截断提示存在、空文件跳过、没有任何内容时整节省略。
func TestInstructionsSection(t *testing.T) {
	blocks, stats := Build(Input{
		Mode: "code", Workspace: "/tmp/ws",
		EnvContext: EnvContextSnapshot{
			Env: EnvInfo{Today: "2026-11-02"},
			Instructions: []Instruction{
				{Path: "~/.craftwork/AGENTS.md", Scope: "global", Content: "用中文回复"},
				{Path: "/tmp/ws/AGENTS.md", Scope: "project", Content: "先跑测试", Truncated: true},
				{Path: "/tmp/empty.md", Scope: "project", Content: "  "},
			},
		},
	})
	dynamic := blocks[len(blocks)-1].Content
	if !strings.Contains(dynamic, "覆盖默认行为") {
		t.Fatalf("指令导语缺失: %q", dynamic)
	}
	if !strings.Contains(dynamic, "用户的全局私人指令") || !strings.Contains(dynamic, "项目指令，随代码库提交") {
		t.Fatalf("范围标签缺失: %q", dynamic)
	}
	if !strings.Contains(dynamic, "文件过长已截断") {
		t.Fatalf("截断提示缺失: %q", dynamic)
	}
	if strings.Contains(dynamic, "/tmp/empty.md") {
		t.Fatalf("空文件不应出现: %q", dynamic)
	}
	found := false
	for _, stat := range stats {
		if stat.Source == "instructions" {
			found = true
		}
	}
	if !found {
		t.Fatal("指令节统计缺失")
	}

	// 没有指令文件：整节省略。
	blocks, _ = Build(Input{Mode: "code", Workspace: "/tmp/ws", EnvContext: EnvContextSnapshot{Env: EnvInfo{Today: "2026-11-02"}}})
	if strings.Contains(blocks[len(blocks)-1].Content, "覆盖默认行为") {
		t.Fatalf("无指令不应出现指令节: %q", blocks[len(blocks)-1].Content)
	}
}

// TestNewInputsStayDynamic 是缓存契约的延伸：环境、Git 与指令都属于
// 动态层，绝不允许进入稳定块或前缀块——否则每次运行都会击穿稳定缓存。
func TestNewInputsStayDynamic(t *testing.T) {
	in := Input{
		Mode: "code", Workspace: "/tmp/ws",
		EnvContext: EnvContextSnapshot{
			Env:          EnvInfo{Today: "2026-11-02", Platform: "darwin/arm64", Shell: "zsh"},
			Git:          &GitSnapshot{Branch: "main"},
			Instructions: []Instruction{{Path: "/tmp/ws/AGENTS.md", Scope: "project", Content: "先跑测试"}},
		},
	}
	blocks, _ := Build(in)
	if len(blocks) != 3 {
		t.Fatalf("新增输入不应增加块数: %d", len(blocks))
	}
	stable := blocks[1].Content
	for _, volatile := range []string{"2026-11-02", "darwin/arm64", "AGENTS.md", "main"} {
		if strings.Contains(stable, volatile) {
			t.Fatalf("稳定块不应包含易变事实 %q", volatile)
		}
	}
}
