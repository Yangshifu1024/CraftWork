package agent

import (
	"context"
	"strings"
	"testing"
	"time"

	"craftwork/internal/model"
	"craftwork/internal/store"
	"craftwork/internal/title"
)

// titleClient 返回固定的标题文本，用于验证模型生成路径。
type titleClient struct {
	text string
}

func (c titleClient) Stream(context.Context, model.Request) (<-chan model.Chunk, error) {
	channel := make(chan model.Chunk, 2)
	channel <- model.Chunk{Text: c.text}
	channel <- model.Chunk{Usage: &model.Usage{InputTokens: 12, OutputTokens: 4}, Done: true}
	close(channel)
	return channel, nil
}

// blockingTitleClient 阻塞到上下文超时，用于验证降级路径。
type blockingTitleClient struct{}

func (blockingTitleClient) Stream(ctx context.Context, _ model.Request) (<-chan model.Chunk, error) {
	<-ctx.Done()
	return nil, ctx.Err()
}

// neverClosingTitleClient 返回一个永不关闭的通道，模拟不响应取消的适配器。
type neverClosingTitleClient struct{}

func (neverClosingTitleClient) Stream(context.Context, model.Request) (<-chan model.Chunk, error) {
	return make(chan model.Chunk), nil
}

func TestSanitizeModelTitle(t *testing.T) {
	cases := map[string]string{
		"  标题：修复登录刷新超时。 ":          "修复登录刷新超时",
		"“写文件”":                    "写文件",
		"！！！":                      "",
		strings.Repeat("超长标题", 10): "超长标题超长标题超长标题超长标题超长标…",
	}
	for input, want := range cases {
		if got := sanitizeModelTitle(input); got != want {
			t.Fatalf("标题清洗不符: got %q want %q", got, want)
		}
	}
}

func TestAutoTitleUsesModel(t *testing.T) {
	h := newHarnessWithFactory(t, fakeFactory{client: titleClient{text: "标题：修复登录刷新超时。"}})
	thread := h.createTitleThread(t, "帮我看看登录后 15 分钟刷新令牌就超时的问题")
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Title != "修复登录刷新超时" || stored.TitleAuto {
		t.Fatalf("模型标题未写入: %+v", stored)
	}
}

func TestAutoTitleFallsBackOnTimeout(t *testing.T) {
	original := titleRequestTimeout
	titleRequestTimeout = 20 * time.Millisecond
	t.Cleanup(func() { titleRequestTimeout = original })

	h := newHarnessWithFactory(t, fakeFactory{client: blockingTitleClient{}})
	thread := h.createTitleThread(t, "帮我修复登录刷新超时")
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Title != "帮我修复登录刷新超时" {
		t.Fatalf("超时后应使用降级标题: %+v", stored)
	}
}

func TestAutoTitleFallsBackWhenStreamNeverCloses(t *testing.T) {
	original := titleRequestTimeout
	titleRequestTimeout = 20 * time.Millisecond
	t.Cleanup(func() { titleRequestTimeout = original })

	h := newHarnessWithFactory(t, fakeFactory{client: neverClosingTitleClient{}})
	thread := h.createTitleThread(t, "整理季度会议纪要")
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Title != "整理季度会议纪要" {
		t.Fatalf("流未关闭时也应在超时后降级: %+v", stored)
	}
}

func TestAutoTitleDoesNotOverwriteManualRename(t *testing.T) {
	h := newHarnessWithFactory(t, fakeFactory{client: titleClient{text: "模型标题"}})
	thread := h.createTitleThread(t, "帮我修复登录刷新超时")
	if err := h.db.RenameThread(thread.ID, "我的标题"); err != nil {
		t.Fatal(err)
	}
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Title != "我的标题" {
		t.Fatalf("手动标题被覆盖: %s", stored.Title)
	}
}

// createTitleThread 创建一个仍允许自动命名的线程，并写入首条用户消息。
func (h *harness) createTitleThread(t *testing.T, text string) store.Thread {
	t.Helper()
	thread := store.Thread{
		ID: "th_title", Mode: "code", Title: title.Default, TitleAuto: true,
		ModelID: "test/fake", Status: "idle",
		CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := h.db.CreateThread(&thread); err != nil {
		t.Fatal(err)
	}
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "p_title_user", ThreadID: thread.ID, MessageID: "msg_title",
		Kind: "user_message", Payload: map[string]any{"text": text},
	}); err != nil {
		t.Fatal(err)
	}
	return thread
}
