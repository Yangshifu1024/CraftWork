package agent

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/title"
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

func TestAutoTitleUsesShortTextWithoutModel(t *testing.T) {
	// 模型会返回一个明显不同的标题；最终标题仍是原文，说明没有发起请求。
	h := newHarnessWithFactory(t, fakeFactory{client: titleClient{text: "模型生成的标题"}})
	text := "帮我修复登录刷新超时"
	thread := h.createTitleThread(t, text)
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Title != text || stored.TitleAuto {
		t.Fatalf("短消息应直接使用原文: %+v", stored)
	}
}

func TestAutoTitleFallsBackOnTimeout(t *testing.T) {
	original := titleRequestTimeout
	titleRequestTimeout = 20 * time.Millisecond
	t.Cleanup(func() { titleRequestTimeout = original })

	// 超过 20 个字符才会进入模型路径，超时后应降级为首条消息的截断标题。
	longText := "登录后 15 分钟刷新令牌就超时了，帮我看看 internal/auth/session.go 里的刷新逻辑。"
	h := newHarnessWithFactory(t, fakeFactory{client: blockingTitleClient{}})
	thread := h.createTitleThread(t, longText)
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if want := title.Derive(longText, nil); stored.Title != want {
		t.Fatalf("超时后应使用降级标题: got %q want %q", stored.Title, want)
	}
}

func TestAutoTitleFallsBackWhenStreamNeverCloses(t *testing.T) {
	original := titleRequestTimeout
	titleRequestTimeout = 20 * time.Millisecond
	t.Cleanup(func() { titleRequestTimeout = original })

	longText := "帮我整理这份季度会议纪要，并列出所有待办事项和负责人"
	h := newHarnessWithFactory(t, fakeFactory{client: neverClosingTitleClient{}})
	thread := h.createTitleThread(t, longText)
	h.runner.autoTitle(thread, "test/fake")

	stored, err := h.db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if want := title.Derive(longText, nil); stored.Title != want {
		t.Fatalf("流未关闭时也应在超时后降级: got %q want %q", stored.Title, want)
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

// createTitleThread 创建一个仍允许自动命名的会话，并写入首条用户消息。
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
