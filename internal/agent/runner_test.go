package agent

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"craftwork/internal/events"
	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/store"
	"craftwork/internal/tools"
)

type fakeFactory struct{ client model.Client }

func (f fakeFactory) Client(string) (model.Client, error) { return f.client, nil }

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
	broker    *Broker
	workspace string
	thread    store.Thread
}

func newHarness(t *testing.T) *harness {
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
	broker := NewBroker(db, bus)
	runner := NewRunner(db, bus, registry, engine, broker, fakeFactory{client: &scriptedClient{}})

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

	return &harness{runner: runner, db: db, broker: broker, workspace: workspace, thread: thread}
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
	if err := h.broker.Resolve(approval.ID, Decision{Allow: true, Scope: "once"}); err != nil {
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
	if err := h.broker.Resolve(approval.ID, Decision{Allow: false}); err != nil {
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
