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

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/tools"
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
	// 应用没有内置提供商，测试同样通过用户目录注册一个假模型；
	// 这样 Resolve、上下文窗口与失败转移都走真实路径。
	// 个别测试会在调用 harness 前自行注册模型（例如验证 MaxTokens），
	// 此时保留测试自己的配置。
	if len(model.Providers()) == 0 {
		model.SetCustomProviders([]model.Provider{{
			ID:          "test",
			Name:        "测试提供商",
			Kind:        "openai-compatible",
			Endpoint:    "https://example.com/v1",
			KeyRequired: false,
			Models: []model.ModelInfo{{
				ID: "fake", Name: "Fake", Provider: "test", ContextK: 32,
				Caps: model.Capabilities{Stream: true, Tools: true},
			}, {
				ID: "backup", Name: "Backup", Provider: "test", ContextK: 32,
				Caps: model.Capabilities{Stream: true, Tools: true},
			}},
		}})
	}
	t.Cleanup(func() { model.SetCustomProviders(nil) })

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

	if err := db.UpsertWorkspace(&store.Workspace{
		ID: "ws_test", Name: "workspace", Path: workspace, Trust: "trusted",
		Purpose: store.WorkspacePurposeProject, CreatedAt: store.NowMillis(), LastOpenedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}

	registry := tools.NewRegistry(tools.Options{
		BackupDir: filepath.Join(root, "backups"),
		RecordBackup: func(string, string, string, int64) error {
			return nil
		},
	})
	engine := policy.New(policy.LevelGraded)
	recorder := record.New(db, bus)
	approvals := NewApprovals(db, recorder)
	runner := NewRunner(db, recorder, registry, engine, approvals, factory, "", "")

	thread := store.Thread{
		ID: "th_1", Mode: "code", Title: "测试", ModelID: "test/fake", WorkspaceID: "ws_test",
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
		Status: "running", ModelID: "test/fake", StartedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}
	if err := h.db.SetThreadStatus("th_1", "running"); err != nil {
		t.Fatal(err)
	}
	h.runner.start(h.thread, "run_1", "msg_assistant", "code")
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
			"test/fake":   failingClient{err: model.NewError(model.CodeRateLimit, "主模型", "请求过于频繁")},
			"test/backup": textClient{text: "已切换模型"},
		},
		fallback: "test/backup",
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
			"test/fake":   failingClient{err: model.NewError(model.CodeAuth, "主模型", "密钥无效")},
			"test/backup": textClient{text: "不应出现"},
		},
		fallback: "test/backup",
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

	history, _, err := h.runner.history(h.thread, h.workspace)
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

// 运行环境必须跟随会话绑定的工作区，而不是全局激活工作区：
// 激活状态漂移（切换、新建或移除工作区）曾导致运行中的会话拿到空路径，
// 所有文件与 Shell 工具报「工作区未设置」。
func TestExecuteUsesThreadWorkspace(t *testing.T) {
	h := newHarness(t)

	// 工作区 B：会话真正绑定的工作区；写入目标落在 B 才算正确。
	workspaceB := filepath.Join(filepath.Dir(h.workspace), "workspace-b")
	if err := os.MkdirAll(workspaceB, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := h.db.UpsertWorkspace(&store.Workspace{
		ID: "ws_b", Name: "B", Path: workspaceB, Trust: "trusted",
		CreatedAt: store.NowMillis(), LastOpenedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}

	// 会话绑定 B；全局注册表仍指向 harness 默认的工作区 A，模拟激活状态漂移。
	threadB := store.Thread{
		ID: "th_b", Mode: "code", Title: "B 会话", ModelID: "test/fake",
		WorkspaceID: "ws_b", Status: "idle",
		CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := h.db.CreateThread(&threadB); err != nil {
		t.Fatal(err)
	}
	if _, err := h.db.AppendEvent(&store.EventRecord{
		ID: "p_user_b", ThreadID: "th_b", MessageID: "msg_user_b",
		Kind: "user_message", Payload: map[string]any{"text": "写一个文件"},
	}); err != nil {
		t.Fatal(err)
	}
	if err := h.db.CreateRun(&store.Run{
		ID: "run_b", ThreadID: "th_b", MessageID: "msg_assistant",
		Status: "running", ModelID: "test/fake", StartedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}
	h.runner.start(threadB, "run_b", "msg_assistant", "code")

	approval := waitApproval(t, h.db, "th_b")
	if err := h.approvals.Resolve(approval.ID, Decision{Allow: true, Scope: "once"}); err != nil {
		t.Fatal(err)
	}
	waitStatus(t, h.db, "th_b", "idle")

	// 脚本模型请求写入 note.txt；修复后它必须落在会话绑定的 B，而不是 A。
	data, err := os.ReadFile(filepath.Join(workspaceB, "note.txt"))
	if err != nil || string(data) != "hello" {
		t.Fatalf("工具应写入会话绑定的工作区 B: %v %q", err, string(data))
	}
	if _, err := os.Stat(filepath.Join(h.workspace, "note.txt")); !os.IsNotExist(err) {
		t.Fatal("不应写入全局激活的工作区 A")
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
	t.Fatalf("等待会话状态 %s 超时，当前 %s", want, current)
}

func TestEstimateCostMicros(t *testing.T) {
	t.Cleanup(func() { model.SetCustomProviders(nil) })
	model.SetCustomProviders([]model.Provider{
		{
			ID: "compat", Name: "兼容", Kind: "openai-compatible", Endpoint: "https://example.com/v1",
			Models: []model.ModelInfo{{
				ID: "model", Name: "Model", Provider: "compat", ContextK: 32,
				Cost: &model.Cost{InputPerMTok: 1, OutputPerMTok: 2, CacheReadPerMTok: 0.1},
			}},
		},
		{
			ID: "anthropic", Name: "Anthropic", Kind: "anthropic", Endpoint: "https://api.anthropic.com",
			Models: []model.ModelInfo{{
				ID: "claude", Name: "Claude", Provider: "anthropic", ContextK: 200,
				Cost: &model.Cost{InputPerMTok: 1, OutputPerMTok: 2, CacheReadPerMTok: 0.1, CacheWritePerMTok: 1.25},
			}},
		},
		{
			ID: "manual", Name: "手动", Kind: "openai-compatible", Endpoint: "https://example.com/v1",
			Models: []model.ModelInfo{{
				ID: "plain", Name: "Plain", Provider: "manual", ContextK: 32,
			}},
		},
	})

	// OpenAI 兼容接口：输入已包含缓存，未缓存输入按输入价，缓存读取按缓存价。
	compat := estimateCostMicros("compat/model", &model.Usage{
		InputTokens: 1_000_000, OutputTokens: 100_000, CacheTokens: 200_000,
	})
	if compat != 1_020_000 {
		t.Fatalf("OpenAI 兼容费用不符: %d", compat)
	}

	// Anthropic：input_tokens 不包含缓存，缓存写入与读取分别计价。
	anthropic := estimateCostMicros("anthropic/claude", &model.Usage{
		InputTokens: 800_000, OutputTokens: 100_000,
		CacheTokens: 300_000, CacheWriteTokens: 100_000,
	})
	if anthropic != 1_145_000 {
		t.Fatalf("Anthropic 费用不符: %d", anthropic)
	}

	// 没有价格信息的模型记 0，而不是猜测默认价格。
	if cost := estimateCostMicros("manual/plain", &model.Usage{InputTokens: 1000, OutputTokens: 1000}); cost != 0 {
		t.Fatalf("未知价格应为 0: %d", cost)
	}
}

// toolLoopClient 每次调用都返回一个 fs_list 工具调用（R0 只读，不产生审批阻塞）；
// calls 记录实际被请求的次数，用于断言上限恰好生效。
type toolLoopClient struct{ calls int }

func (c *toolLoopClient) Stream(_ context.Context, _ model.Request) (<-chan model.Chunk, error) {
	c.calls++
	channel := make(chan model.Chunk, 2)
	go func() {
		defer close(channel)
		channel <- model.Chunk{ToolCall: &model.ToolCall{
			ID:   fmt.Sprintf("call_%d", c.calls),
			Name: "fs_list",
			Args: map[string]any{"path": "."},
		}}
		channel <- model.Chunk{Usage: &model.Usage{InputTokens: 10, OutputTokens: 2}, Done: true}
	}()
	return channel, nil
}

// finishAfterClient 前 until 次返回工具调用，之后返回收尾文本；step 记录总调用次数。
type finishAfterClient struct {
	until int
	step  int
}

func (c *finishAfterClient) Stream(_ context.Context, _ model.Request) (<-chan model.Chunk, error) {
	step := c.step
	c.step++
	channel := make(chan model.Chunk, 2)
	go func() {
		defer close(channel)
		if step < c.until {
			channel <- model.Chunk{ToolCall: &model.ToolCall{
				ID:   fmt.Sprintf("call_%d", step+1),
				Name: "fs_list",
				Args: map[string]any{"path": "."},
			}}
		} else {
			channel <- model.Chunk{Text: "完成"}
		}
		channel <- model.Chunk{Usage: &model.Usage{InputTokens: 10, OutputTokens: 2}, Done: true}
	}()
	return channel, nil
}

// TestRunnerMaxIterationsLimit 验证设置回调返回的往返上限恰好生效：
// 模型永远调用工具时，应在第 N 次往返后被 max_iterations 终止。
func TestRunnerMaxIterationsLimit(t *testing.T) {
	client := &toolLoopClient{}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.runner.SetMaxIterationsResolver(func() int { return 3 })
	h.start(t)

	waitStatus(t, h.db, "th_1", "error")

	if client.calls != 3 {
		t.Fatalf("应恰好调用模型 3 次，实际 %d", client.calls)
	}
	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, message := range snapshot.Messages {
		for _, part := range message.Parts {
			if part.Kind != "error" {
				continue
			}
			data, _ := part.Data.(map[string]any)
			if data["code"] != "max_iterations" {
				continue
			}
			found = true
			if msg, _ := data["message"].(string); !strings.Contains(msg, "3") {
				t.Fatalf("错误信息应包含上限轮数 3: %q", msg)
			}
		}
	}
	if !found {
		t.Fatal("应记录 max_iterations 错误部件")
	}
}

// TestRunnerUnlimitedIterations 验证上限为 0 时不限制往返次数：
// 模型连续调用 5 轮工具后收尾，应正常完成而不是被终止。
func TestRunnerUnlimitedIterations(t *testing.T) {
	client := &finishAfterClient{until: 5}
	h := newHarnessWithFactory(t, fakeFactory{client: client})
	h.runner.SetMaxIterationsResolver(func() int { return 0 })
	h.start(t)

	waitStatus(t, h.db, "th_1", "idle")

	if client.step != 6 {
		t.Fatalf("应调用模型 6 次（5 轮工具 + 1 轮收尾），实际 %d", client.step)
	}
	snapshot, err := h.db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	for _, message := range snapshot.Messages {
		for _, part := range message.Parts {
			if part.Kind == "error" {
				t.Fatalf("不限制模式不应产生错误部件: %+v", part.Data)
			}
		}
	}
}
