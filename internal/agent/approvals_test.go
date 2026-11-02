package agent

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/store"
)

func newApprovalHarness(t *testing.T) (*Approvals, *store.DB, store.Thread) {
	t.Helper()
	root := t.TempDir()
	db, err := store.Open(filepath.Join(root, "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	bus := events.NewBus(func(string, any) {})
	t.Cleanup(bus.Close)

	approvals := NewApprovals(db, record.New(db, bus))

	thread := store.Thread{
		ID: "th_1", Mode: "code", Title: "测试", ModelID: "fake",
		Status: "idle", CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := db.CreateThread(&thread); err != nil {
		t.Fatal(err)
	}
	return approvals, db, thread
}

func baseApprovalRequest() ApprovalRequest {
	return ApprovalRequest{
		ID: "ap_1", PartID: "part_1", ThreadID: "th_1", RunID: "run_1", MessageID: "msg_1",
		ToolCallID: "call_1", Tool: "fs_write", Risk: "R1", Title: "写入文件",
		Target: "/workspace/a.txt", Detail: "写入工作区",
		Facts: []Fact{{Label: "风险等级", Value: "R1"}},
		Args:  map[string]any{"path": "a.txt"}, AllowPermanent: true,
	}
}

func requestApproval(approvals *Approvals, ctx context.Context) <-chan Outcome {
	return requestApprovalWith(approvals, ctx, baseApprovalRequest())
}

func requestApprovalWith(approvals *Approvals, ctx context.Context, request ApprovalRequest) <-chan Outcome {
	outcome := make(chan Outcome, 1)
	go func() {
		result, _ := approvals.Request(ctx, request)
		outcome <- result
	}()
	return outcome
}

func waitPendingApproval(t *testing.T, db *store.DB, threadID string) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		pending, err := db.PendingApprovals(threadID)
		if err == nil && len(pending) > 0 {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("审批未进入等待状态")
}

func TestApprovalsApproveWritesCardAndResolvedEvent(t *testing.T) {
	approvals, db, thread := newApprovalHarness(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	outcome := requestApproval(approvals, ctx)
	waitPendingApproval(t, db, thread.ID)

	if err := approvals.Resolve("ap_1", Decision{Allow: true, Scope: "session"}); err != nil {
		t.Fatal(err)
	}
	result := <-outcome
	if !result.Allow || result.Scope != "session" || result.Status != "approved" {
		t.Fatalf("审批结果不符: %+v", result)
	}

	approval, err := db.GetApproval("ap_1")
	if err != nil {
		t.Fatal(err)
	}
	if approval.Decision != "allow" || approval.Scope != "session" {
		t.Fatalf("审批落库不符: %+v", approval)
	}

	snapshot, err := db.Snapshot(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	cards := 0
	for _, message := range snapshot.Messages {
		for _, part := range message.Parts {
			if part.Kind == "approval" {
				cards++
			}
		}
	}
	if cards != 1 {
		t.Fatalf("应写入 1 个审批卡部件，实际 %d", cards)
	}
}

func TestApprovalsRejectPermanentWhenNotAllowed(t *testing.T) {
	approvals, db, thread := newApprovalHarness(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	request := baseApprovalRequest()
	request.AllowPermanent = false
	outcome := requestApprovalWith(approvals, ctx, request)
	waitPendingApproval(t, db, thread.ID)

	if err := approvals.Resolve("ap_1", Decision{Allow: true, Scope: "workspace"}); err == nil {
		t.Fatal("不允许永久的审批应拒绝 workspace 范围")
	}
	if err := approvals.Resolve("ap_1", Decision{Allow: true, Scope: "once"}); err != nil {
		t.Fatalf("允许一次应成功: %v", err)
	}
	result := <-outcome
	if !result.Allow || result.Scope != "once" {
		t.Fatalf("审批结果不符: %+v", result)
	}
}

func TestApprovalsDenyForcesOnce(t *testing.T) {
	approvals, db, thread := newApprovalHarness(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	outcome := requestApproval(approvals, ctx)
	waitPendingApproval(t, db, thread.ID)

	if err := approvals.Resolve("ap_1", Decision{Allow: false, Scope: "workspace"}); err != nil {
		t.Fatal(err)
	}
	result := <-outcome
	if result.Allow || result.Scope != "once" || result.Status != "denied" {
		t.Fatalf("拒绝时应强制 once: %+v", result)
	}
}

func TestApprovalsInvalidScopeAndCancel(t *testing.T) {
	approvals, db, thread := newApprovalHarness(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	outcome := requestApproval(approvals, ctx)
	waitPendingApproval(t, db, thread.ID)

	if err := approvals.Resolve("ap_1", Decision{Allow: true, Scope: "forever"}); err == nil {
		t.Fatal("非法授权范围应被拒绝")
	}
	cancel()
	result := <-outcome
	if result.Status != "canceled" {
		t.Fatalf("取消后状态应为 canceled: %+v", result)
	}
	approval, err := db.GetApproval("ap_1")
	if err != nil {
		t.Fatal(err)
	}
	if approval.Decision != "canceled" {
		t.Fatalf("取消应落库: %+v", approval)
	}
}
