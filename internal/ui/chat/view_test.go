// 聊天列 view 的 ui.Tester 走查：空态建议句、消息流渲染（气泡 / 占位卡 /
// 元信息标签）、Composer 的键盘分流与附件条、事件批次驱动流式标签。
// Actions 注入本地记录函数（不接服务），同路径覆盖单测回退语义。

package chat

import (
	"strings"
	"testing"
	"time"

	mygo "github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// recorded 收集注入动作的调用，供断言。
type recorded struct {
	submits     []string
	cancels     int
	regens      int
	picks       int
	pastes      int
	opens       []string
	externals   []string
	copies      []string
	loadEarlies int
	decides     []decision
}

// newChatTester 在 1200×800 下起一个注入了数据与本地动作的聊天列 view。
func newChatTester(t *testing.T) (*Chat, *recorded, *mygo.Tester) {
	t.Helper()
	s := NewChat()
	rec := &recorded{}
	s.SetActions(&Actions{
		Submit:          func(text string, _ []Attachment) { rec.submits = append(rec.submits, text) },
		Cancel:          func() { rec.cancels++ },
		Regenerate:      func() { rec.regens++ },
		PickFiles:       func() { rec.picks++ },
		PasteImage:      func() { rec.pastes++ },
		LoadEarlier:     func() { rec.loadEarlies++ },
		OpenFile:        func(path string, _ int) { rec.opens = append(rec.opens, path) },
		ConfirmExternal: func(url string) { rec.externals = append(rec.externals, url) },
		Copy:            func(text string) { rec.copies = append(rec.copies, text) },
	})
	view := func(c *mygo.Context) {
		pal := theme.Apply(c)
		s.View(c, pal)
	}
	tt := mygo.NewTester(view, 1200, 800)
	return s, rec, tt
}

// clickText 点击含指定文字的元素（helper 与 sidebar 走查同款）。
func clickText(t *testing.T, tt *mygo.Tester, label string) {
	t.Helper()
	if err := tt.Click(label); err != nil {
		t.Fatalf("点击 %q 失败: %v", label, err)
	}
}

// settleClick 是点击后的统一等待：推进一帧再加动效时长（连点需要
// Advance 动效时长，04 号票坑）。
func settleClick(tt *mygo.Tester) {
	tt.Frame()
	tt.Advance(theme.Duration + 20*time.Millisecond)
	tt.Frame()
}

func TestChatViewEmptyStateSuggestions(t *testing.T) {
	s, rec, tt := newChatTester(t)
	tt.Frame()
	for _, want := range []string{"Code 模式已就绪", "解释这个仓库的结构"} {
		if !tt.HasText(want) {
			t.Fatalf("空态应显示 %q", want)
		}
	}
	clickText(t, tt, "运行测试并修复失败用例")
	settleClick(tt)
	if len(rec.submits) != 1 || rec.submits[0] != "运行测试并修复失败用例" {
		t.Fatalf("建议句点击应触发 Submit: %v", rec.submits)
	}
	_ = s
}

// 事件批次驱动：run.started 后消息区出现流式光标与「生成中」标签；
// 文本增量渲染正文；run.completed 后标签消失。
func TestChatViewStreamsAndFinishes(t *testing.T) {
	s, _, tt := newChatTester(t)
	tt.Frame()

	s.ApplyBatch(batchOf(
		events.TypeRunStarted, map[string]any{"messageId": "m1"}, 1, "th",
	))
	tt.Frame()
	if !tt.HasText("生成中") {
		t.Fatal("运行开始后应显示生成中标签")
	}

	s.ApplyBatch(batchOf(
		events.TypeTextDelta, events.TextDelta{PartID: "p1", Text: "你好，世界"}, 2, "th",
	))
	// 词级 reveal 有节奏延迟：推进一个 CatchUp 周期让文本全部浮现。
	tt.Advance(400 * time.Millisecond)
	tt.Frame()
	if !tt.HasText("你好") {
		t.Fatal("流式正文应渲染")
	}

	s.ApplyBatch(batchOf(events.TypeRunCompleted, map[string]any{"durationMs": 120}, 3, "th"))
	tt.Frame()
	if tt.HasText("生成中") {
		t.Fatal("运行结束后不应再显示生成中标签")
	}
}

// 部件渲染：用户消息右对齐气泡（文本可查）、助手消息结束后过程部件
// 收进「已工作」折叠区（展开可见工具卡）、系统提示细条。
func TestChatViewMessageParts(t *testing.T) {
	s, _, tt := newChatTester(t)
	s.LoadSnapshot("th", []*Message{
		{
			ID: "u1", ThreadID: "th", Role: RoleUser, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
			Parts: []Part{{ID: "up", Kind: PartText, Data: map[string]any{"text": "帮我看看 runner.go"}}},
		},
		{
			ID: "a1", ThreadID: "th", Role: RoleAssistant, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
			Parts: []Part{
				{ID: "at", Kind: PartText, Data: map[string]any{"text": "这是回答"}},
				{ID: "ap", Kind: PartTool, Data: map[string]any{"partId": "ap", "name": "fs_read", "status": "done"}},
				{ID: "an", Kind: PartNotice, Data: map[string]any{"text": "上下文已压缩"}},
			},
		},
	}, ThreadInfo{ID: "th", Title: "测试会话", Status: ThreadStatusIdle}, false, 0)
	tt.Frame()

	if !tt.HasText("帮我看看 runner.go") || !tt.HasText("这是回答") {
		t.Fatal("消息文本应渲染")
	}
	// 终态消息的过程段默认收拢：折叠头可见、工具名不可见。
	if !tt.HasText("已工作") {
		t.Fatal("终态助手消息应显示已工作折叠头")
	}
	if tt.HasText("fs_read") {
		t.Fatal("收拢状态不应显示工具卡")
	}
	// 展开后工具卡可见（点击折叠头）。
	clickText(t, tt, "已工作")
	settleClick(tt)
	if !tt.HasText("fs_read") {
		t.Fatal("展开后应显示工具卡")
	}
	if !tt.HasText("上下文已压缩") {
		t.Fatal("系统提示细条应渲染")
	}
}

// Composer：输入 + Enter 提交并清空；Shift+Enter 不提交。
func TestComposerEnterSubmits(t *testing.T) {
	s, rec, tt := newChatTester(t)
	tt.Frame()
	tt.Type(" hello ")
	tt.Key(0, mygo.KeyEnter)
	tt.Frame()
	if len(rec.submits) != 1 || rec.submits[0] != "hello" {
		t.Fatalf("Enter 应提交去空格后的文本: %v", rec.submits)
	}
	if strings.TrimSpace(*s.DraftRef()) != "" {
		t.Fatalf("提交后草稿应清空: %q", *s.DraftRef())
	}

	// Shift+Enter 换行：不提交。
	tt.Type("line1")
	tt.Key(mygo.Shift, mygo.KeyEnter)
	tt.Frame()
	if len(rec.submits) != 1 {
		t.Fatalf("Shift+Enter 不应提交: %v", rec.submits)
	}
}

// Composer 运行中形态：空输入显示停止按钮；有输入时回车转引导。
func TestComposerRunningModes(t *testing.T) {
	s, rec, tt := newChatTester(t)
	s.LoadSnapshot("th", nil, ThreadInfo{ID: "th", Title: "会话", Status: ThreadStatusRunning}, false, 0)
	s.SetRunning(true)
	tt.Frame()
	if !tt.HasText("引导") {
		t.Fatal("运行中提示行应显示引导文案")
	}

	// 空输入点击停止（icon 按钮以可访问名定位）。
	clickText(t, tt, "停止生成")
	settleClick(tt)
	if rec.cancels != 1 {
		t.Fatalf("停止按钮应触发 Cancel: %d", rec.cancels)
	}
}

// 附件条：加入草稿后出现 chip，点击移除后消失。
func TestAttachmentStrip(t *testing.T) {
	s, _, tt := newChatTester(t)
	tt.Frame()
	s.AddAttachments([]string{"/tmp/report.md", "/tmp/report.md"})
	tt.Frame()
	if !tt.HasText("report.md") {
		t.Fatal("附件 chip 应显示文件名")
	}
	// 去重：同一文件只出现一条草稿。
	if len(s.Attachments()) != 1 {
		t.Fatalf("重复路径应去重: %d", len(s.Attachments()))
	}
	// 点击移除按钮（x 图标无文字，用 chip 行内的按钮文字回退：直接删）。
	s.RemoveAttachment("/tmp/report.md")
	tt.Frame()
	if len(s.Attachments()) != 0 {
		t.Fatal("移除后附件条应为空")
	}
}

// batchOf 是测试用的事件批次构造。
func batchOf(eventType string, data any, seq int64, threadID string) events.Batch {
	return events.Batch{V: 1, ThreadID: threadID, Events: []*events.Envelope{{
		Seq: seq, Type: eventType, ThreadID: threadID, TS: 0, Data: data,
	}}}
}

// ---------------------------------------------------------------------------
// 14 号票：聊天正文列居中限宽（两侧 margin 各 10%）与保底宽度
// ---------------------------------------------------------------------------

// TestChatColumnWidthFormula 是正文列宽公式：所在列宽的 80%
// （两侧 margin 各 10%，用户给定参数），不低于保底宽度——Markdown 在
// 收缩布局里会塌零宽（AGENTS.md 第 7 节），正文列必须显式保底。
func TestChatColumnWidthFormula(t *testing.T) {
	cases := []struct{ avail, want float32 }{
		{2000, 1600}, // 超宽窗（约 2000px）：80% 限宽生效
		{1200, 960},
		{400, 320}, // 80% 低于保底：保底生效
		{300, 320}, // 列本身比保底还窄：仍取保底（溢出被消息流裁剪）
	}
	for _, c := range cases {
		if got := chatColumnWidth(c.avail); got != c.want {
			t.Errorf("chatColumnWidth(%v) = %v, want %v", c.avail, got, c.want)
		}
	}
}

// TestChatViewColumnWidthWideAndFloored 验证 view 真按公式构建正文列：
// 超宽窗下限宽居中、极窄窗下保底生效、零宽不塌；用户气泡不超出正文列
// （78% 上限改为相对正文列后的真实布局验证）。
func TestChatViewColumnWidthWideAndFloored(t *testing.T) {
	s, _, tt := newChatTester(t)
	long := strings.Repeat("这是一段很长的用户消息内容，", 40) // 560 字，远超列宽
	s.LoadSnapshot("th", []*Message{
		{
			ID: "u1", ThreadID: "th", Role: RoleUser, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
			Parts: []Part{{ID: "up", Kind: PartText, Data: map[string]any{"text": long}}},
		},
		{
			ID: "a1", ThreadID: "th", Role: RoleAssistant, Status: StatusComplete, CreatedAt: time.Now().UnixMilli(),
			Parts: []Part{{ID: "at", Kind: PartText, Data: map[string]any{"text": "回答"}}},
		},
	}, ThreadInfo{ID: "th", Title: "限宽会话", Status: ThreadStatusIdle}, false, 0)

	// 超宽窗（约 2000px）：正文列 = 80% 限宽，消息行帧内记录的列宽与
	// 公式一致（messageRow 每条消息都写，取任意一条即整列的值）。
	tt.SetSize(2000, 800)
	tt.Frame()
	if w := chatColumnWidth(2000); w != 1600 {
		t.Fatalf("公式应在 2000 列宽下给出 1600，得 %v", w)
	}
	if s.lastColumnW != 1600 {
		t.Fatalf("超宽窗下正文列宽 = %v, want 1600（80%% 限宽）", s.lastColumnW)
	}
	rect, ok := tt.Find(long)
	if !ok {
		t.Fatal("用户消息文本应渲染（零宽不塌的文本可见性底线）")
	}
	if rect.W <= 0 {
		t.Fatalf("用户消息文本宽度 = %v，不得塌零宽", rect.W)
	}
	// 用户气泡右对齐且不超出正文列右缘（列右缘 = 2000×90% = 1800）。
	if right := rect.X + rect.W; right > 2000*0.9+1 {
		t.Fatalf("用户气泡文本右缘 = %v，超出正文列右缘 1800", right)
	}

	// 极窄窗：保底宽度生效，正文列不随列宽跌落、不为零。
	tt.SetSize(360, 800)
	tt.Frame()
	if s.lastColumnW != theme.ChatColumnMinWidth {
		t.Fatalf("极窄窗下正文列宽 = %v, want 保底 %v", s.lastColumnW, theme.ChatColumnMinWidth)
	}
	rect, ok = tt.Find(long)
	if !ok || rect.W <= 0 {
		t.Fatalf("极窄窗下用户消息文本应可见且非零宽: rect=%v ok=%v", rect, ok)
	}
}

// TestComposerFocusHighlightsFrame 验证 composer 聚焦态标记整个外框：
// AutoFocus 让输入框取得焦点后外框为聚焦态（FocusWithin），焦点移出后
// 失焦恢复；外框宽度与消息正文列同参数（80% 限宽），两段对齐。
func TestComposerFocusHighlightsFrame(t *testing.T) {
	s, _, tt := newChatTester(t)
	tt.Frame() // AutoFocus：输入框取得焦点
	if !s.ComposerFocused() {
		t.Fatal("输入框聚焦时 composer 外框应为聚焦态")
	}
	// 1200 视口：外框 = 80% 限宽 = 960，与消息正文列同宽。
	if s.lastComposerW != 960 {
		t.Fatalf("composer 外框宽 = %v, want 960（80%% 限宽）", s.lastComposerW)
	}
	// 焦点移出（点击空态建议按钮，可聚焦元素）：外框失焦。
	clickText(t, tt, "解释这个仓库的结构")
	settleClick(tt)
	if s.ComposerFocused() {
		t.Fatal("焦点移出后 composer 外框应为失焦态")
	}
}
