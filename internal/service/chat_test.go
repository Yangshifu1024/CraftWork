package service

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"craftwork/internal/agent"
	"craftwork/internal/events"
	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/record"
	"craftwork/internal/store"
	"craftwork/internal/tools"
)

// echoClient 每次请求都直接返回收尾文本；requests 记录到达的请求，
// count 是请求计数（接力会产生第二次请求，等待时以它为准）。
type echoClient struct {
	requests chan model.Request
	count    atomic.Int32
}

func (c *echoClient) Stream(_ context.Context, request model.Request) (<-chan model.Chunk, error) {
	c.count.Add(1)
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

// blockedClient 请求后保持流打开，直到测试放行，用于构造运行中的会话。
type blockedClient struct {
	entered chan struct{}
	gate    chan struct{}
}

func (c *blockedClient) Stream(context.Context, model.Request) (<-chan model.Chunk, error) {
	channel := make(chan model.Chunk, 1)
	close(c.entered)
	<-c.gate
	close(channel)
	return channel, nil
}

type fixedFactory struct{ client model.Client }

func (f fixedFactory) Client(string) (model.Client, error) { return f.client, nil }

func (f fixedFactory) FallbackModel(string) string { return "" }

// chatHarness 装配对话服务与其依赖，并捕获总线广播供事件断言。
type chatHarness struct {
	chat    *ChatService
	db      *store.DB
	mu      sync.Mutex
	batches []events.Batch
}

func (h *chatHarness) findEvent(eventType string) *events.Envelope {
	h.mu.Lock()
	defer h.mu.Unlock()
	for i := range h.batches {
		for _, envelope := range h.batches[i].Events {
			if envelope.Type == eventType {
				found := *envelope
				return &found
			}
		}
	}
	return nil
}

func newChatHarness(t *testing.T, client model.Client) *chatHarness {
	t.Helper()
	// 测试通过用户目录注册假模型，让模型解析与能力检查走真实路径。
	if len(model.Providers()) == 0 {
		model.SetCustomProviders([]model.Provider{{
			ID: "test", Name: "测试提供商", Kind: "openai-compatible",
			Endpoint: "https://example.com/v1", KeyRequired: false,
			Models: []model.ModelInfo{{
				ID: "fake", Name: "Fake", Provider: "test", ContextK: 32,
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

	h := &chatHarness{}
	bus := events.NewBus(func(_ string, data any) {
		if batch, ok := data.(events.Batch); ok {
			h.mu.Lock()
			h.batches = append(h.batches, batch)
			h.mu.Unlock()
		}
	})
	t.Cleanup(bus.Close)

	if err := db.UpsertWorkspace(&store.Workspace{
		ID: "ws_chat", Name: "workspace", Path: workspace, Trust: "trusted",
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
	approvals := agent.NewApprovals(db, recorder)
	runner := agent.NewRunner(db, recorder, registry, engine, approvals, fixedFactory{client: client}, "", "")
	h.chat = NewChatService(db, recorder, runner, approvals)
	h.db = db

	thread := store.Thread{
		ID: "th_chat", Mode: "code", Title: "测试", ModelID: "test/fake", WorkspaceID: "ws_chat",
		Status: "idle", CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := db.CreateThread(&thread); err != nil {
		t.Fatal(err)
	}
	return h
}

func waitIdle(t *testing.T, db *store.DB, threadID string) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	current := ""
	for time.Now().Before(deadline) {
		thread, err := db.GetThread(threadID)
		if err == nil {
			current = thread.Status
			if current == "idle" {
				return
			}
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("等待会话空闲超时，当前 %s", current)
}

// TestSteerFallbackToSend 验证会话未在运行时，Steer 回退为普通发送：
// 消息写入事件表并随新 run 进入模型历史，user.message 事件广播给前端。
func TestSteerFallbackToSend(t *testing.T) {
	client := &echoClient{requests: make(chan model.Request, 4)}
	h := newChatHarness(t, client)

	messageID, err := h.chat.Steer("th_chat", "补充一条指令", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(messageID, "msg_") {
		t.Fatalf("应返回用户消息 ID: %q", messageID)
	}
	waitIdle(t, h.db, "th_chat")

	snapshot, err := h.db.Snapshot("th_chat")
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, message := range snapshot.Messages {
		if message.Role != "user" {
			continue
		}
		for _, part := range message.Parts {
			if data, ok := part.Data.(map[string]any); ok && data["text"] == "补充一条指令" {
				found = true
			}
		}
	}
	if !found {
		t.Fatal("回退发送应把引导写入用户消息")
	}

	envelope := h.findEvent(events.TypeUserMessage)
	if envelope == nil {
		t.Fatal("应广播 user.message 事件")
	}
	data, _ := envelope.Data.(map[string]any)
	if data["messageId"] != messageID || data["via"] != "steer" {
		t.Fatalf("user.message 载荷不符: %+v", data)
	}

	requests := make([]model.Request, 0, 1)
	for {
		select {
		case request := <-client.requests:
			requests = append(requests, request)
			continue
		default:
		}
		break
	}
	if len(requests) != 1 {
		t.Fatalf("应恰好请求模型 1 次，实际 %d", len(requests))
	}
	last := requests[0].Messages[len(requests[0].Messages)-1]
	if last.Role != "user" || last.Content != "补充一条指令" {
		t.Fatalf("模型历史末尾应是引导消息: %+v", last)
	}
}

// TestSteerDuringRunDeliveredByFollowup 验证完整链路：运行中 Steer 只入队
// 不开新 run；本次运行正常结束后接力 run 把引导作为新的用户输入执行。
func TestSteerDuringRunDeliveredByFollowup(t *testing.T) {
	client := &echoClient{requests: make(chan model.Request, 4)}
	h := newChatHarness(t, client)

	if _, err := h.chat.Send("th_chat", "开始任务", nil); err != nil {
		t.Fatal(err)
	}
	// 等第一次请求真正到达后再引导，保证入队时运行已在途。
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) && client.count.Load() == 0 {
		time.Sleep(5 * time.Millisecond)
	}
	if client.count.Load() == 0 {
		t.Fatal("没有等到第一次模型请求")
	}

	messageID, err := h.chat.Steer("th_chat", "补充指令", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(messageID, "msg_") {
		t.Fatalf("应返回用户消息 ID: %q", messageID)
	}

	// 接力 run 会发起第二次请求；等待它到达后再确认会话最终空闲，
	// 避免把收尾瞬间的 idle 误判为整条运行链结束。
	deadline = time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) && client.count.Load() < 2 {
		time.Sleep(5 * time.Millisecond)
	}
	if client.count.Load() < 2 {
		t.Fatal("运行结束后应产生接力 run 的第二次请求")
	}
	waitIdle(t, h.db, "th_chat")

	requests := make([]model.Request, 0, 2)
	for {
		select {
		case request := <-client.requests:
			requests = append(requests, request)
			continue
		default:
		}
		break
	}
	if len(requests) != 2 {
		t.Fatalf("应恰好请求模型 2 次，实际 %d", len(requests))
	}
	last := requests[1].Messages[len(requests[1].Messages)-1]
	if last.Role != "user" || last.Content != "补充指令" {
		t.Fatalf("接力请求末尾应是引导消息: %+v", last)
	}

	// 两条 assistant 消息：原始 run 与接力 run 各一条。
	snapshot, err := h.db.Snapshot("th_chat")
	if err != nil {
		t.Fatal(err)
	}
	assistants := 0
	for _, message := range snapshot.Messages {
		if message.Role == "assistant" {
			assistants++
		}
	}
	if assistants != 2 {
		t.Fatalf("接力后应有两条 assistant 消息，实际 %d", assistants)
	}
}

// TestSteerValidations 验证参数校验：空消息拒绝、不存在的会话拒绝。
func TestSteerValidations(t *testing.T) {
	client := &echoClient{requests: make(chan model.Request, 4)}
	h := newChatHarness(t, client)

	if _, err := h.chat.Steer("th_chat", "", nil); err == nil {
		t.Fatal("空消息应被拒绝")
	}
	if _, err := h.chat.Steer("th_missing", "任意", nil); err == nil {
		t.Fatal("不存在的会话应被拒绝")
	}
	if client.count.Load() != 0 {
		t.Fatal("被拒绝的引导不应触发模型请求")
	}
}
