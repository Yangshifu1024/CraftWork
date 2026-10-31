package agent

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"craftwork/internal/events"
	"craftwork/internal/store"
)

// Fact 是审批卡片上展示的一条事实。
type Fact struct {
	Label string `json:"label"`
	Value string `json:"value"`
}

// ApprovalRequest 是一次审批请求。
type ApprovalRequest struct {
	ID             string
	PartID         string
	ThreadID       string
	RunID          string
	ToolCallID     string
	Tool           string
	Risk           string
	Title          string
	Target         string
	Detail         string
	Facts          []Fact
	Args           map[string]any
	AllowPermanent bool
}

// Decision 是用户对审批的决定。
type Decision struct {
	Allow bool
	Scope string // once / session / workspace
}

// Broker 把进程内阻塞的审批转换成跨 IPC 的请求-响应。
type Broker struct {
	mu        sync.Mutex
	pending   map[string]chan Decision
	parts     map[string]string
	targets   map[string]string
	permanent map[string]bool
	db        *store.DB
	bus       *events.Bus
	grants    *Grants
}

// NewBroker 创建审批 Broker。
func NewBroker(db *store.DB, bus *events.Bus) *Broker {
	return &Broker{
		pending:   map[string]chan Decision{},
		parts:     map[string]string{},
		targets:   map[string]string{},
		permanent: map[string]bool{},
		db:        db,
		bus:       bus,
		grants:    NewGrants(db),
	}
}

// Grants 返回授权缓存。
func (b *Broker) Grants() *Grants {
	return b.grants
}

// Ask 挂起当前 run，等待前端回传决定。
func (b *Broker) Ask(ctx context.Context, request ApprovalRequest) (Decision, error) {
	channel := make(chan Decision, 1)

	b.mu.Lock()
	b.pending[request.ID] = channel
	b.parts[request.ID] = request.PartID
	b.targets[request.ID] = request.Target
	b.permanent[request.ID] = request.AllowPermanent
	b.mu.Unlock()

	removePending := func() {
		b.mu.Lock()
		delete(b.pending, request.ID)
		delete(b.parts, request.ID)
		delete(b.targets, request.ID)
		delete(b.permanent, request.ID)
		b.mu.Unlock()
	}

	if err := b.db.CreateApproval(&store.Approval{
		ID:          request.ID,
		ThreadID:    request.ThreadID,
		RunID:       request.RunID,
		ToolCallID:  request.ToolCallID,
		Tool:        request.Tool,
		Risk:        request.Risk,
		RequestedAt: nowMillis(),
	}); err != nil {
		removePending()
		return Decision{}, err
	}

	b.bus.Publish(events.TypeApprovalRequested, request.ThreadID, request.RunID, map[string]any{
		"partId":         request.PartID,
		"approvalId":     request.ID,
		"toolCallId":     request.ToolCallID,
		"tool":           request.Tool,
		"risk":           request.Risk,
		"title":          request.Title,
		"target":         request.Target,
		"detail":         request.Detail,
		"facts":          request.Facts,
		"args":           request.Args,
		"allowPermanent": request.AllowPermanent,
		"status":         "waiting",
	})

	select {
	case decision := <-channel:
		return decision, nil
	case <-ctx.Done():
		removePending()
		_ = b.db.ResolveApproval(request.ID, "canceled", "")
		b.bus.Publish(events.TypeApprovalResolved, request.ThreadID, request.RunID, map[string]any{
			"partId":     request.PartID,
			"approvalId": request.ID,
			"status":     "canceled",
			"decision":   "canceled",
			"scope":      "",
		})
		return Decision{}, ctx.Err()
	}
}

// Resolve 记录用户决定并唤醒等待中的 run。
func (b *Broker) Resolve(approvalID string, decision Decision) error {
	approval, err := b.db.GetApproval(approvalID)
	if err != nil {
		return err
	}
	if approval.Decision != "" {
		return fmt.Errorf("审批已处理：%s", approval.Decision)
	}

	scope := decision.Scope
	if scope == "" {
		scope = "once"
	}
	if decision.Allow && scope != "once" {
		b.mu.Lock()
		allowPermanent := b.permanent[approvalID]
		b.mu.Unlock()
		if !allowPermanent {
			return errors.New("该审批不提供永久授权，只能允许一次")
		}
	}
	status := "denied"
	dbDecision := "deny"
	if decision.Allow {
		status = "approved"
		dbDecision = "allow"
	}
	if err := b.db.ResolveApproval(approvalID, dbDecision, scope); err != nil {
		return err
	}

	b.mu.Lock()
	channel := b.pending[approvalID]
	partID := b.parts[approvalID]
	target := b.targets[approvalID]
	delete(b.pending, approvalID)
	delete(b.parts, approvalID)
	delete(b.targets, approvalID)
	delete(b.permanent, approvalID)
	b.mu.Unlock()

	if channel == nil {
		return errors.New("审批等待已失效（可能已取消）")
	}

	if decision.Allow && scope != "once" {
		if err := b.grants.Grant(approval.Tool, target, scope); err != nil {
			return err
		}
	}

	_ = b.db.AddAudit(&store.AuditEntry{
		ThreadID: approval.ThreadID,
		RunID:    approval.RunID,
		Tool:     approval.Tool,
		Risk:     approval.Risk,
		Decision: dbDecision,
		Scope:    scope,
		Detail:   "用户审批",
	})

	b.bus.Publish(events.TypeApprovalResolved, approval.ThreadID, approval.RunID, map[string]any{
		"partId":     partID,
		"approvalId": approvalID,
		"status":     status,
		"decision":   dbDecision,
		"scope":      scope,
	})

	channel <- decision
	return nil
}

// HasPending 判断线程是否有等待中的审批。
func (b *Broker) HasPending(threadID string) bool {
	approvals, err := b.db.PendingApprovals(threadID)
	return err == nil && len(approvals) > 0
}

func nowMillis() int64 {
	return time.Now().UnixMilli()
}
