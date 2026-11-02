package agent

import (
	"context"
	"strings"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/model"
)

func TestEstimateMessageTokens(t *testing.T) {
	messages := []model.Message{
		{Role: "user", Content: strings.Repeat("a", 100)},
		{Role: "assistant", Content: "你好", ToolCalls: []model.ToolCall{{Name: "fs_read", Args: map[string]any{"path": "note.txt"}}}},
	}
	tokens := estimateMessageTokens(messages)
	if tokens <= 0 {
		t.Fatalf("token 估算应大于 0: %d", tokens)
	}
	if estimateMessageTokens([]model.Message{{Role: "user", Content: strings.Repeat("a", 1000)}}) <= tokens {
		t.Fatal("更长内容应估算出更多 token")
	}
}

func TestPlanCompressionKeepsRecentMessages(t *testing.T) {
	messages := make([]model.Message, 0, 30)
	for index := 0; index < 30; index++ {
		messages = append(messages, model.Message{
			Role:    "user",
			Content: strings.Repeat("上下文", 400),
		})
	}
	early, recent := planCompression(messages, 10000)
	if len(early) == 0 {
		t.Fatal("应切出需要摘要的早期消息")
	}
	if len(recent) < minKeepMessages {
		t.Fatalf("至少保留 %d 条消息，实际 %d", minKeepMessages, len(recent))
	}
	if len(early)+len(recent) != len(messages) {
		t.Fatalf("切分后消息数量不符: %d + %d != %d", len(early), len(recent), len(messages))
	}
	if &messages[len(early)] != &recent[0] {
		t.Fatal("早期与最近消息应连续切分")
	}
}

func TestPlanCompressionKeepsToolChainTogether(t *testing.T) {
	messages := []model.Message{
		{Role: "user", Content: "一"},
		{Role: "user", Content: "二"},
		{Role: "user", Content: "三"},
		{Role: "user", Content: "四"},
		{Role: "assistant", ToolCalls: []model.ToolCall{{ID: "call_1", Name: "fs_read"}}},
		{Role: "tool", Content: "结果一"},
		{Role: "tool", Content: "结果二"},
		{Role: "user", Content: "继续"},
		{Role: "assistant", Content: "好的"},
		{Role: "user", Content: "谢谢"},
	}
	early, recent := planCompression(messages, 1)
	if len(early) == 0 {
		t.Fatal("应发生切分")
	}
	if recent[0].Role != "assistant" || len(recent[0].ToolCalls) == 0 {
		t.Fatalf("切分点应向前扩展到工具调用: %+v", recent[0])
	}
	for index, message := range recent {
		if message.Role == "tool" && (index == 0 || recent[index-1].Role != "tool" && len(recent[index-1].ToolCalls) == 0) {
			t.Fatalf("工具结果缺少对应的工具调用: %+v", recent)
		}
	}
}

func TestMaybeCompressSummarizesAndPersistsNotice(t *testing.T) {
	h := newHarnessWithFactory(t, fakeFactory{client: textClient{text: "早期对话摘要内容"}})
	history := make([]model.Message, 0, 60)
	for index := 0; index < 60; index++ {
		history = append(history, model.Message{Role: "user", Content: strings.Repeat("长内容", 700)})
	}

	// seqs 与消息一一对应（测试历史没有真实事件，用递增序号即可），
	// 压缩摘要按最后一条被覆盖消息的序号落库。
	seqs := make([]int64, len(history))
	for i := range seqs {
		seqs[i] = int64(i + 1)
	}
	compressed := h.runner.maybeCompress(context.Background(), h.thread, "run_ctx", "msg_ctx", history, seqs)
	if len(compressed) >= len(history) {
		t.Fatalf("压缩后消息数量应减少: %d >= %d", len(compressed), len(history))
	}
	if !strings.Contains(compressed[0].Content, "更早对话摘要") {
		t.Fatalf("首条消息应为摘要: %q", compressed[0].Content)
	}
	if compressed[len(compressed)-1].Content != history[len(history)-1].Content {
		t.Fatal("最近消息应原样保留")
	}

	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	foundNotice := false
	for _, message := range snapshot.Messages {
		for _, part := range message.Parts {
			if part.Kind != "notice" {
				continue
			}
			data, _ := part.Data.(map[string]any)
			if strings.Contains(toString(data["text"]), "上下文已压缩") {
				foundNotice = true
				if !strings.Contains(toString(data["summary"]), "早期对话摘要内容") {
					t.Fatalf("提示应携带可查看的摘要: %+v", data)
				}
			}
		}
	}
	if !foundNotice {
		t.Fatal("应写入可见的压缩提示")
	}
}
