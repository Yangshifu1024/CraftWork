package app

// 端到端冒烟（10 号票任务 7）：按 Run 的装配链路组装真实后端（SQLite、
// 策略、工具、审批、运行器、聊天服务），模型指向 httptest 假端点，跑通
// 「发消息 → 工具调用 → 审批允许 → 执行 → 完成 → 审计」全链。
//
// 不启动 GUI：Run 的窗口部分由各 UI 票的 ui.Tester 测试与 cwsmoke 冒烟
// 覆盖，这里锁的是装配语义本身（服务间的接线与 02 号票解耦后的形状）。

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/yangshifu1024/craftwork/internal/agent"
	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/id"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/tools"
)

// sseToolCall 构造一轮「工具调用」的 SSE 响应（finish_reason=tool_calls）。
func sseToolCall(name, args string) string {
	call := map[string]any{
		"index": 0,
		"id":    "call_1",
		"type":  "function",
		"function": map[string]any{
			"name":      name,
			"arguments": args,
		},
	}
	return sseResponse(map[string]any{
		"choices": []map[string]any{{
			"index":         0,
			"delta":         map[string]any{"tool_calls": []any{call}},
			"finish_reason": "tool_calls",
		}},
	})
}

// sseText 构造一轮「正文」的 SSE 响应（finish_reason=stop）。
func sseText(text string) string {
	return sseResponse(map[string]any{
		"choices": []map[string]any{{
			"index":         0,
			"delta":         map[string]any{"content": text},
			"finish_reason": "stop",
		}},
	})
}

// sseResponse 把一条 Chat Completions 事件包成 SSE 帧。
func sseResponse(event map[string]any) string {
	data, _ := json.Marshal(event)
	return "data: " + string(data) + "\n\ndata: [DONE]\n\n"
}

// newFakeEndpoint 起一个 OpenAI 兼容假端点：第一轮回 fs_write 工具调用，
// 之后回正文。返回端点地址与收到的对话轮数。
func newFakeEndpoint(t *testing.T, toolArgs string, finalText string) (string, *atomicCounter) {
	t.Helper()
	rounds := &atomicCounter{}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !strings.HasSuffix(r.URL.Path, "/chat/completions") {
			http.NotFound(w, r)
			return
		}
		n := rounds.Add()
		w.Header().Set("Content-Type", "text/event-stream")
		var body string
		if n == 1 {
			body = sseToolCall("fs_write", toolArgs)
		} else {
			body = sseText(finalText)
		}
		_, _ = fmt.Fprint(w, body)
	}))
	t.Cleanup(server.Close)
	return server.URL, rounds
}

// atomicCounter 是测试里的简单并发计数器。
type atomicCounter struct {
	mu sync.Mutex
	n  int
}

func (c *atomicCounter) Add() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.n++
	return c.n
}

// waitFor 等待条件成立或超时（默认 10 秒），超时直接判定失败。
func waitFor(t *testing.T, what string, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("等待 %s 超时", what)
}

func TestEndToEndApprovalLoop(t *testing.T) {
	const (
		fileName  = "notes.txt"
		fileBody  = "端到端冒烟写入的内容"
		finalText = "已写入文件，任务完成。"
	)
	endpoint, rounds := newFakeEndpoint(t,
		fmt.Sprintf(`{"path":%q,"content":%q}`, fileName, fileBody), finalText)

	// 用户目录注册假提供商（应用没有内置提供商，测试与生产走同一条
	// Resolve 路径）。
	model.SetCustomProviders([]model.Provider{{
		ID:          "test",
		Name:        "测试提供商",
		Kind:        "openai-compatible",
		Endpoint:    endpoint,
		KeyRequired: false,
		Models: []model.ModelInfo{{
			ID: "fake", Name: "Fake", Provider: "test", ContextK: 32,
			Caps: model.Capabilities{Stream: true, Tools: true},
		}},
	}})
	t.Cleanup(func() { model.SetCustomProviders(nil) })

	root := t.TempDir()
	workspace := filepath.Join(root, "workspace")
	// 工作区目录必须真实存在：路径解析要 EvalSymlinks，不存在时评估
	// 失败、所有工具按不可解析处理（第一次跑测试踩到的坑）。
	if err := os.MkdirAll(workspace, 0o755); err != nil {
		t.Fatal(err)
	}
	db, err := store.Open(filepath.Join(root, "e2e.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	// 事件总线把审批请求与运行完成转成信号，供测试等待。
	approvalIDs := make(chan string, 4)
	completed := make(chan struct{}, 4)
	bus := events.NewBus(func(b events.Batch) {
		for _, e := range b.Events {
			switch e.Type {
			case events.TypeApprovalRequested:
				if m, ok := e.Data.(map[string]any); ok {
					if idStr, _ := m["approvalId"].(string); idStr != "" {
						approvalIDs <- idStr
					}
				}
			case events.TypeRunCompleted:
				completed <- struct{}{}
			}
		}
	})
	t.Cleanup(bus.Close)

	if err := db.UpsertWorkspace(&store.Workspace{
		ID: "ws_e2e", Name: "workspace", Path: workspace, Trust: "trusted",
		Purpose: store.WorkspacePurposeProject, CreatedAt: store.NowMillis(), LastOpenedAt: store.NowMillis(),
	}); err != nil {
		t.Fatal(err)
	}

	// 与 Run 相同的装配链：策略 → 工具（真实 fs_write，备份登记进库）→
	// 记录器 → 审批 → 运行器 → 服务。
	engine := policy.New(policy.LevelGraded)
	registry := tools.NewRegistry(tools.Options{
		BackupDir: filepath.Join(root, "backups"),
		RecordBackup: func(path, backupPath, hash string, size int64) error {
			return db.AddBackup(&store.Backup{
				ID: id.New("bk"), Path: path, BackupPath: backupPath, Hash: hash, Size: size,
			})
		},
	})
	recorder := record.New(db, bus)
	approvals := agent.NewApprovals(db, recorder)
	factory := model.NewFactory(nil)
	runner := agent.NewRunner(db, recorder, registry, engine, approvals, factory, filepath.Join(root, "attachments"), root)
	threadService := service.NewThreadService(db, recorder)
	chatService := service.NewChatService(db, recorder, runner, approvals)

	thread, err := threadService.Create("code", "ws_e2e", "端到端冒烟")
	if err != nil {
		t.Fatal(err)
	}
	if err := threadService.SetModel(thread.ID, "test/fake"); err != nil {
		t.Fatal(err)
	}

	// 发消息：写用户消息并启动 run（ChatService.Send 的完整校验链）。
	if _, err := chatService.Send(thread.ID, "帮我写一个文件", nil); err != nil {
		t.Fatalf("发送失败: %v", err)
	}

	// 工具调用触发审批：允许一次。
	var approvalID string
	select {
	case approvalID = <-approvalIDs:
	case <-time.After(10 * time.Second):
		t.Fatalf("等待审批请求超时（端点轮数 %d）", rounds.snapshot())
	}
	if err := chatService.DecideApproval(approvalID, true, "once"); err != nil {
		t.Fatalf("决定审批失败: %v", err)
	}

	// 等运行完成；会话状态回 idle。
	<-completed
	waitFor(t, "会话回到空闲", func() bool {
		th, err := db.GetThread(thread.ID)
		return err == nil && th != nil && th.Status == "idle"
	})

	// 文件真的写进了工作区。
	body, err := os.ReadFile(filepath.Join(workspace, fileName))
	if err != nil {
		t.Fatalf("工作区文件缺失: %v", err)
	}
	if string(body) != fileBody {
		t.Fatalf("文件内容不符: %q", body)
	}

	// 审计：审批决定与工具执行两条（「审批允许 once」+ 执行记录）。
	entries, err := db.ListAudit(50)
	if err != nil {
		t.Fatal(err)
	}
	var sawDecision, sawExecution bool
	for _, e := range entries {
		if e.Tool != "fs_write" {
			continue
		}
		if e.Decision == "allow" && e.Scope == "once" && strings.Contains(e.Detail, "用户审批") {
			sawDecision = true
		}
		if e.Decision == "allow" && strings.Contains(e.Detail, fileName) {
			sawExecution = true
		}
	}
	if !sawDecision || !sawExecution {
		t.Fatalf("审计缺项：决定=%v 执行=%v，实有 %+v", sawDecision, sawExecution, entries)
	}

	// 审批行已按 allow/once 收尾，没有挂起审批。
	pending, err := db.PendingApprovals(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 0 {
		t.Fatalf("仍有挂起审批: %+v", pending)
	}

	// 助手正文消息已落库（快照里有最终文本）。
	snapshot, err := threadService.SnapshotPage(thread.ID, 20, 0)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, msg := range snapshot.Messages {
		if msg.Role != "assistant" {
			continue
		}
		for _, part := range msg.Parts {
			if text, _ := part.Data.(map[string]any)["text"].(string); strings.Contains(text, finalText) {
				found = true
			}
		}
	}
	if !found {
		t.Fatalf("快照缺少最终正文 %q", finalText)
	}
}

func (c *atomicCounter) snapshot() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.n
}
