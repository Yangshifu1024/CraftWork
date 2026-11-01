package agent

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"craftwork/internal/events"
	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/record"
	"craftwork/internal/store"
	"craftwork/internal/tools"
)

type fakeFactory struct{ client model.Client }

func (f fakeFactory) Client(string) (model.Client, error) { return f.client, nil }

func (f fakeFactory) FallbackModel(string) string { return "" }

// scriptedClient 第一次返回写文件工具调用，第二次返回收尾文本。
type scriptedClient struct{ step int }

func (c *scriptedClient) Stream(_ context.Context, _ model.Request) (<-chan model.Chunk, error) {
	step := c.step
	c.step++
	channel := make(chan model.Chunk, 4)
	go func() {
		defer close(channel)
		if step == 0 {
			channel <- model.Chunk{ToolCall: &model.ToolCall{
				ID:   "call_1",
				Name: "fs_write",
				Args: map[string]any{"path": "note.txt", "content": "hello"},
			}}
		} else {
			channel <- model.Chunk{Text: "完成"}
		}
		channel <- model.Chunk{Usage: &model.Usage{InputTokens: 10, OutputTokens: 2}, Done: true}
	}()
	return channel, nil
}

type harness struct {
	runner    *Runner
	db        *store.DB
	approvals *Approvals
	workspace string
	thread    store.Thread
}

func newHarness(t *testing.T) *harness {
	t.Helper()
	return newHarnessWithFactory(t, fakeFactory{client: &scriptedClient{}})
}

func newHarnessWithFactory(t *testing.T, factory ModelFactory) *harness {
	t.Helper()
	root := t.TempDir()
	workspace := filepath.Join(root, "workspace")
	if err := os.MkdirAll(workspace, 0o755); err != nil {
		t.Fatal(err)
	}
	db, err := store.Open(filepath.Join(root, "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	bus := events.NewBus(func(string, any) {})
	t.Cleanup(bus.Close)

	registry := tools.NewRegistry(tools.Options{
		Workspace: workspace,
		BackupDir: filepath.Join(root, "backups"),
		RecordBackup: func(string, string, string, int64) error {
			return nil
		},
	})
	engine := policy.New(policy.LevelGraded, workspace)
	recorder := record.New(db, bus)
	approvals := NewApprovals(db, recorder)
	runner := NewRunner(db, recorder, registry, engine, approvals, factory)

	thread := store.Thread{
		ID: "th_1", Mode: "code", Title: "测试", ModelID: "fake",
		Status: "idle", CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := db.CreateThread(&thread); err != nil {
		t.Fatal(err)
	}
	if _, err := db.AppendEvent(&store.EventRecord{
		ID: "p_user", ThreadID: "th_1", MessageID: "msg_user",
		Kind: "user_message", Payload: map[string]any{"text": "帮我写一个文件"},
	}); err != nil {
		t.Fatal(err)
	}

	return &harness{runner: runner, db: db, approvals: approvals, workspace: workspace, thread: thread}
}

func (h *harness) start(t *testing.T) {
	t.Helper()
	if err := h.db.CreateRun(&store.Run{
		ID: "run_1", ThreadID: "th_1", MessageID: "msg_assistant",
		Status: "running", ModelID: "fake", StartedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}
	if err := h.db.SetThreadStatus("th_1", "running"); err != nil {
		t.Fatal(err)
	}
	h.runner.Start(h.thread, "run_1", "msg_assistant", "code")
}

func TestRunnerApprovalAllow(t *testing.T) {
	h := newHarness(t)
	h.start(t)

	approval := waitApproval(t, h.db, "th_1")
	if approval.Tool != "fs_write" || approval.Risk != "R1" {
		t.Fatalf("审批内容不符: %+v", approval)
	}
	if err := h.approvals.Resolve(approval.ID, Decision{Allow: true, Scope: "once"}); err != nil {
		t.Fatal(err)
	}
	waitStatus(t, h.db, "th_1", "idle")

	data, err := os.ReadFile(filepath.Join(h.workspace, "note.txt"))
	if err != nil || string(data) != "hello" {
		t.Fatalf("工具应已写入文件: %v %q", err, string(data))
	}
	audit, err := h.db.ListAudit(5)
	if err != nil || len(audit) == 0 || audit[0].Decision != "allow" {
		t.Fatalf("审计应记录 allow: %v %+v", err, audit)
	}
}

func TestRunnerApprovalDeny(t *testing.T) {
	h := newHarness(t)
	h.start(t)

	approval := waitApproval(t, h.db, "th_1")
	if err := h.approvals.Resolve(approval.ID, Decision{Allow: false}); err != nil {
		t.Fatal(err)
	}
	waitStatus(t, h.db, "th_1", "idle")

	if _, err := os.Stat(filepath.Join(h.workspace, "note.txt")); !os.IsNotExist(err) {
		t.Fatal("拒绝审批后不应写入文件")
	}
	audit, err := h.db.ListAudit(5)
	if err != nil || len(audit) == 0 || audit[0].Decision != "deny" {
		t.Fatalf("审计应记录 deny: %v %+v", err, audit)
	}
}

type failoverFactory struct {
	clients  map[string]model.Client
	fallback string
}

func (f failoverFactory) Client(modelID string) (model.Client, error) { return f.clients[modelID], nil }
func (f failoverFactory) FallbackModel(string) string                 { return f.fallback }

type failingClient struct{ err error }

func (c failingClient) Stream(context.Context, model.Request) (<-chan model.Chunk, error) {
	return nil, c.err
}

type textClient struct{ text string }

func (c textClient) Stream(context.Context, model.Request) (<-chan model.Chunk, error) {
	channel := make(chan model.Chunk, 2)
	channel <- model.Chunk{Text: c.text}
	channel <- model.Chunk{Usage: &model.Usage{}, Done: true}
	close(channel)
	return channel, nil
}

func TestRunnerFailsOverOnRetryableError(t *testing.T) {
	factory := failoverFactory{
		clients: map[string]model.Client{
			"fake":   failingClient{err: model.NewError(model.CodeRateLimit, "主模型", "请求过于频繁")},
			"backup": textClient{text: "已切换模型"},
		},
		fallback: "backup",
	}
	h := newHarnessWithFactory(t, factory)
	h.start(t)
	waitStatus(t, h.db, "th_1", "idle")

	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	var notice, text string
	for _, message := range snapshot.Messages {
		for _, part := range message.Parts {
			switch part.Kind {
			case "notice":
				data, _ := part.Data.(map[string]any)
				notice, _ = data["text"].(string)
			case "text":
				data, _ := part.Data.(map[string]any)
				text, _ = data["text"].(string)
			}
		}
	}
	if !strings.Contains(notice, "已切换到") {
		t.Fatalf("应记录失败转移提示: %q", notice)
	}
	if text != "已切换模型" {
		t.Fatalf("应使用失败转移模型的输出: %q", text)
	}
}

func TestRunnerDoesNotFailoverOnAuthError(t *testing.T) {
	factory := failoverFactory{
		clients: map[string]model.Client{
			"fake":   failingClient{err: model.NewError(model.CodeAuth, "主模型", "密钥无效")},
			"backup": textClient{text: "不应出现"},
		},
		fallback: "backup",
	}
	h := newHarnessWithFactory(t, factory)
	h.start(t)
	waitStatus(t, h.db, "th_1", "error")

	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	foundAuth := false
	for _, message := range snapshot.Messages {
		if message.Role != "assistant" {
			continue
		}
		for _, part := range message.Parts {
			if part.Kind == "error" {
				data, _ := part.Data.(map[string]any)
				if data["code"] == string(model.CodeAuth) {
					foundAuth = true
				}
			}
			if part.Kind == "text" {
				t.Fatalf("认证错误不应触发失败转移: %+v", part.Data)
			}
		}
	}
	if !foundAuth {
		t.Fatal("应记录 auth 错误")
	}
}

// TestRunnerAnthropicToolLoop 验证 Anthropic 适配器与 Agent 循环的完整链路：
// 文本 → tool_use → 工具执行 → tool_result 回填 → 收尾文本。
func TestRunnerAnthropicToolLoop(t *testing.T) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		requestNumber := calls.Add(1)
		if requestNumber == 2 && !strings.Contains(string(body), `"tool_result"`) {
			t.Errorf("第二次请求应包含 tool_result: %s", string(body))
		}
		w.Header().Set("Content-Type", "text/event-stream")
		if requestNumber == 1 {
			fmt.Fprint(w, strings.Join([]string{
				`event: message_start`,
				`data: {"type":"message_start","message":{"usage":{"input_tokens":8}}}`,
				``,
				`event: content_block_start`,
				`data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}`,
				``,
				`event: content_block_delta`,
				`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"读取文件"}}`,
				``,
				`event: content_block_start`,
				`data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"fs_read","input":{}}}`,
				``,
				`event: content_block_delta`,
				`data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"note.txt\"}"}}`,
				``,
				`event: message_delta`,
				`data: {"type":"message_delta","usage":{"output_tokens":5}}`,
				``,
				`event: message_stop`,
				`data: {"type":"message_stop"}`,
				``,
			}, "\n"))
			return
		}
		fmt.Fprint(w, strings.Join([]string{
			`event: message_start`,
			`data: {"type":"message_start","message":{"usage":{"input_tokens":20}}}`,
			``,
			`event: content_block_start`,
			`data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"文件内容是 hello"}}`,
			``,
			`event: message_delta`,
			`data: {"type":"message_delta","usage":{"output_tokens":6}}`,
			``,
			`event: message_stop`,
			`data: {"type":"message_stop"}`,
			``,
		}, "\n"))
	}))
	defer server.Close()

	h := newHarnessWithFactory(t, fakeFactory{client: model.NewAnthropic(
		model.Provider{ID: "anthropic", Name: "Anthropic", Kind: "anthropic", Endpoint: server.URL},
		server.URL,
		"test-key",
	)})
	if err := os.WriteFile(filepath.Join(h.workspace, "note.txt"), []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}
	h.start(t)
	waitStatus(t, h.db, "th_1", "idle")

	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	var finalText string
	var toolOutput string
	for _, message := range snapshot.Messages {
		if message.Role != "assistant" {
			continue
		}
		for _, part := range message.Parts {
			data, _ := part.Data.(map[string]any)
			switch part.Kind {
			case "text":
				if text, _ := data["text"].(string); text != "" {
					finalText = text
				}
			case "tool":
				toolOutput, _ = data["output"].(string)
			}
		}
	}
	if !strings.Contains(finalText, "文件内容是 hello") {
		t.Fatalf("收尾文本不符: %q", finalText)
	}
	if !strings.Contains(toolOutput, "hello") {
		t.Fatalf("工具输出不符: %q", toolOutput)
	}
	if calls.Load() != 2 {
		t.Fatalf("模型请求次数不符: %d", calls.Load())
	}
}

// TestRunnerGeminiToolLoop 验证 Gemini 适配器与 Agent 循环的完整链路，
// 包括 functionCall / functionResponse 与 thoughtSignature 回填。
func TestRunnerGeminiToolLoop(t *testing.T) {
	var calls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, _ := io.ReadAll(r.Body)
		requestNumber := calls.Add(1)
		if requestNumber == 2 {
			if !strings.Contains(string(body), `"functionResponse"`) {
				t.Errorf("第二次请求应包含 functionResponse: %s", string(body))
			}
			if !strings.Contains(string(body), `"thoughtSignature":"sig-1"`) {
				t.Errorf("第二次请求应回填 thoughtSignature: %s", string(body))
			}
		}
		w.Header().Set("Content-Type", "text/event-stream")
		if requestNumber == 1 {
			fmt.Fprint(w, `data: {"candidates":[{"content":{"role":"model","parts":[{"text":"读取文件"},{"functionCall":{"name":"fs_read","args":{"path":"note.txt"}},"thoughtSignature":"sig-1"}]}}],"usageMetadata":{"promptTokenCount":8,"candidatesTokenCount":5}}`+"\n\n")
			return
		}
		fmt.Fprint(w, `data: {"candidates":[{"content":{"role":"model","parts":[{"text":"文件内容是 hello"}]}}],"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":6}}`+"\n\n")
	}))
	defer server.Close()

	h := newHarnessWithFactory(t, fakeFactory{client: model.NewGemini(
		model.Provider{ID: "gemini", Name: "Google Gemini", Kind: "gemini", Endpoint: server.URL},
		server.URL,
		"test-key",
	)})
	if err := os.WriteFile(filepath.Join(h.workspace, "note.txt"), []byte("hello"), 0o644); err != nil {
		t.Fatal(err)
	}
	h.start(t)
	waitStatus(t, h.db, "th_1", "idle")

	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	var finalText, toolOutput string
	for _, message := range snapshot.Messages {
		if message.Role != "assistant" {
			continue
		}
		for _, part := range message.Parts {
			data, _ := part.Data.(map[string]any)
			switch part.Kind {
			case "text":
				if text, _ := data["text"].(string); text != "" {
					finalText = text
				}
			case "tool":
				toolOutput, _ = data["output"].(string)
			}
		}
	}
	if !strings.Contains(finalText, "文件内容是 hello") {
		t.Fatalf("收尾文本不符: %q", finalText)
	}
	if !strings.Contains(toolOutput, "hello") {
		t.Fatalf("工具输出不符: %q", toolOutput)
	}
	if calls.Load() != 2 {
		t.Fatalf("模型请求次数不符: %d", calls.Load())
	}
}

func TestHistoryLoadsAttachmentContents(t *testing.T) {
	h := newHarness(t)
	path := filepath.Join(h.workspace, "note.txt")
	if err := os.WriteFile(path, []byte("附件内容"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "p_attach", ThreadID: "th_1", MessageID: "msg_attach",
		Kind: "user_message",
		Payload: map[string]any{
			"text":        "看看附件",
			"attachments": []string{path},
		},
	}); err != nil {
		t.Fatal(err)
	}

	history, err := h.runner.history(h.thread)
	if err != nil {
		t.Fatal(err)
	}
	if len(history) == 0 {
		t.Fatal("历史不应为空")
	}
	last := history[len(history)-1]
	if last.Role != "user" || last.Content != "看看附件" {
		t.Fatalf("最后一条用户消息不符: %+v", last)
	}
	if len(last.Attachments) != 1 || last.Attachments[0].Text != "附件内容" {
		t.Fatalf("附件内容未进入模型历史: %+v", last.Attachments)
	}
}

func TestRunnerUsesModelMaxTokens(t *testing.T) {
	requests := make(chan model.Request, 1)
	capture := &captureClient{requests: requests}
	t.Cleanup(func() { model.SetCustomProviders(nil) })
	model.SetCustomProviders([]model.Provider{{
		ID: "test", Name: "Test", Kind: "openai-compatible",
		Endpoint: "https://example.com/v1", KeyRequired: false,
		Models: []model.ModelInfo{{
			ID: "fake", Name: "Fake", Provider: "test", ContextK: 32, MaxTokens: 512,
			Caps: model.Capabilities{Stream: true},
		}},
	}})

	h := newHarnessWithFactory(t, fakeFactory{client: capture})
	h.start(t)
	waitStatus(t, h.db, "th_1", "idle")

	select {
	case request := <-requests:
		if request.MaxTokens != 512 {
			t.Fatalf("模型最大输出未传入请求: %+v", request)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("没有捕获到模型请求")
	}
}

type captureClient struct {
	requests chan model.Request
}

func (c *captureClient) Stream(_ context.Context, request model.Request) (<-chan model.Chunk, error) {
	select {
	case c.requests <- request:
	default:
	}
	channel := make(chan model.Chunk, 2)
	channel <- model.Chunk{Text: "完成"}
	channel <- model.Chunk{Usage: &model.Usage{}, Done: true}
	close(channel)
	return channel, nil
}

func waitApproval(t *testing.T, db *store.DB, threadID string) store.Approval {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		pending, err := db.PendingApprovals(threadID)
		if err == nil && len(pending) > 0 {
			return pending[0]
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatal("等待审批超时")
	return store.Approval{}
}

func waitStatus(t *testing.T, db *store.DB, threadID, want string) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	current := ""
	for time.Now().Before(deadline) {
		thread, err := db.GetThread(threadID)
		if err == nil {
			current = thread.Status
			if current == want {
				return
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("等待线程状态 %s 超时，当前 %s", want, current)
}
