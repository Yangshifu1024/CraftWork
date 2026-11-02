package agent

import (
	"context"
	"fmt"
	"strings"
	"testing"

	"craftwork/internal/model"
	"craftwork/internal/store"
)

// TestLatestContextSummary 验证存储层按「最新一条」读取压缩摘要。
func TestLatestContextSummary(t *testing.T) {
	db, err := store.Open(t.TempDir() + "/test.db")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	if _, err := db.AppendEvent(&store.EventRecord{
		ID: "e1", ThreadID: "th_sum", MessageID: "msg_1",
		Kind: "context_summary", Payload: map[string]any{"summary": "旧摘要", "untilSeq": 5},
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.AppendEvent(&store.EventRecord{
		ID: "e2", ThreadID: "th_sum", MessageID: "msg_2",
		Kind: "context_summary", Payload: map[string]any{"summary": "新摘要", "untilSeq": 9},
	}); err != nil {
		t.Fatal(err)
	}

	summary, err := db.LatestContextSummary("th_sum")
	if err != nil {
		t.Fatal(err)
	}
	if summary == nil || summary.Summary != "新摘要" || summary.UntilSeq != 9 {
		t.Fatalf("应返回最新一条摘要: %+v", summary)
	}

	if empty, err := db.LatestContextSummary("th_none"); err != nil || empty != nil {
		t.Fatalf("没有摘要时应返回 nil: %+v, %v", empty, err)
	}
}

// TestHistoryReusesContextSummary 验证历史重建复用已持久化的摘要：
// 覆盖范围内的消息被裁掉，摘要以带标注的背景说明出现在最前。
func TestHistoryReusesContextSummary(t *testing.T) {
	h := newHarness(t)
	thread := h.thread

	// 两组对话：第一组被摘要覆盖（覆盖范围取该消息部件的最大序号），
	// 第二组保持原样。
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "e_user_1", ThreadID: thread.ID, MessageID: "msg_1",
		Kind: "user_message", Payload: map[string]any{"text": "第一条用户消息"},
	}); err != nil {
		t.Fatal(err)
	}
	lastOfFirst, err := h.db.AppendEvent(&store.EventRecord{
		ID: "e_asst_1", ThreadID: thread.ID, MessageID: "msg_1",
		Kind: "text", Payload: map[string]any{"text": "第一条回复"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "e_user_2", ThreadID: thread.ID, MessageID: "msg_2",
		Kind: "user_message", Payload: map[string]any{"text": "第二条用户消息"},
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "e_sum", ThreadID: thread.ID, MessageID: "msg_2",
		Kind: "context_summary", Payload: map[string]any{"summary": "第一组对话的摘要", "untilSeq": lastOfFirst},
	}); err != nil {
		t.Fatal(err)
	}

	messages, seqs, err := h.runner.history(thread, h.workspace)
	if err != nil {
		t.Fatal(err)
	}
	if len(messages) != 2 {
		t.Fatalf("应只有摘要与第二条消息: %d", len(messages))
	}
	if !strings.Contains(messages[0].Content, "【更早对话摘要】") ||
		!strings.Contains(messages[0].Content, "第一组对话的摘要") {
		t.Fatalf("第一条应是带标注的摘要: %q", messages[0].Content)
	}
	if messages[0].Role != "user" {
		t.Fatalf("摘要应以 user 角色注入: %s", messages[0].Role)
	}
	if messages[1].Content != "第二条用户消息" {
		t.Fatalf("第二条应是未覆盖的消息: %q", messages[1].Content)
	}
	if seqs[0] != 0 {
		t.Fatalf("摘要消息的序号应为 0: %d", seqs[0])
	}
	if seqs[1] <= lastOfFirst {
		t.Fatalf("保留消息的序号应大于覆盖范围: %d <= %d", seqs[1], lastOfFirst)
	}
}

// TestMaybeCompressPersistsSummary 验证压缩后摘要落库：下一次 history
// 重建能读到同一份摘要，而不是再次调用模型摘要。
func TestMaybeCompressPersistsSummary(t *testing.T) {
	h := newHarnessWithFactory(t, fakeFactory{client: textClient{text: "压缩后的摘要"}})
	thread := h.thread

	// 构造超过窗口 70% 的历史（fake 模型窗口 32k，估算按字符数一半），
	// 每条消息都是独立的用户消息，避免工具调用链的切分约束。
	// 10 条保证切分后早期消息多于一条（保留预算至少留 4 条）。
	const messageChars = 12000
	big := strings.Repeat("长", messageChars)
	var history []model.Message
	var seqs []int64
	for i := 0; i < 10; i++ {
		seq, err := h.db.AppendEvent(&store.EventRecord{
			ID:        fmt.Sprintf("e_big_%d", i),
			ThreadID:  thread.ID,
			MessageID: fmt.Sprintf("msg_big_%d", i),
			Kind:      "user_message",
			Payload:   map[string]any{"text": big},
		})
		if err != nil {
			t.Fatal(err)
		}
		history = append(history, model.Message{Role: "user", Content: big})
		seqs = append(seqs, seq)
	}

	compressed := h.runner.maybeCompress(context.Background(), thread, "run_1", "msg_big_4", history, seqs)
	// 摘要替换早期消息后，历史应明显变短且第一条是摘要。
	if len(compressed) >= len(history) {
		t.Fatalf("压缩后应减少消息数: %d >= %d", len(compressed), len(history))
	}
	if !strings.Contains(compressed[0].Content, "压缩后的摘要") {
		t.Fatalf("压缩后第一条应是新摘要: %q", compressed[0].Content[:100])
	}

	persisted, err := h.db.LatestContextSummary(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if persisted == nil || persisted.Summary != "压缩后的摘要" {
		t.Fatalf("摘要应已持久化: %+v", persisted)
	}
	// 压缩后的历史 = 1 条摘要 + 保留的最近消息；被摘要条数由切分结果反推。
	earlyCount := len(history) - (len(compressed) - 1)
	if persisted.UntilSeq != seqs[earlyCount-1] {
		t.Fatalf("覆盖范围应截止到被摘要的最后一条: %d != %d", persisted.UntilSeq, seqs[earlyCount-1])
	}
}
