package agent

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/record"
	"craftwork/internal/store"
)

// Fact 是审批卡片上展示的一条事实。
type Fact struct {
	Label string `json:"label"`
	Value string `json:"value"`
}

// ApprovalRequest 是一次审批请求的完整输入。
type ApprovalRequest struct {
	ID             string
	PartID         string
	ThreadID       string
	RunID          string
	MessageID      string
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

// Outcome 是审批结束后的结果。
type Outcome struct {
	Allow    bool
	Status   string // approved / denied / canceled
	Decision string // allow / deny / canceled
	Scope    string
}

// cardPayload 是审批卡片唯一的一份载荷，事件与落库共用。
func (r ApprovalRequest) cardPayload(status string) map[string]any {
	return map[string]any{
		"partId":         r.PartID,
		"approvalId":     r.ID,
		"toolCallId":     r.ToolCallID,
		"tool":           r.Tool,
		"risk":           r.Risk,
		"title":          r.Title,
		"target":         r.Target,
		"detail":         r.Detail,
		"facts":          r.Facts,
		"args":           r.Args,
		"allowPermanent": r.AllowPermanent,
		"status":         status,
	}
}

func (r ApprovalRequest) resolvedPayload(status, decision, scope string) map[string]any {
	return map[string]any{
		"partId":     r.PartID,
		"approvalId": r.ID,
		"status":     status,
		"decision":   decision,
		"scope":      scope,
	}
}

type pendingApproval struct {
	channel   chan Decision
	partID    string
	target    string
	permanent bool
}

// Approvals 是审批的唯一入口。
//
// 状态机（waiting → approved / denied / canceled）、内存等待表、数据库行、
// 事件广播、授权缓存与审计都在这个模块内部完成；调用方只提供一次请求，
// 或回传一次决定。
type Approvals struct {
	mu      sync.Mutex
	pending map[string]*pendingApproval
	db      *store.DB
	records *record.Recorder
	grants  *Grants
}

// NewApprovals 创建审批模块。
func NewApprovals(db *store.DB, records *record.Recorder) *Approvals {
	return &Approvals{
		pending: map[string]*pendingApproval{},
		db:      db,
		records: records,
		grants:  NewGrants(db),
	}
}

// Allowed 判断工具调用是否命中已有授权。
func (a *Approvals) Allowed(tool, target string) bool {
	return a.grants.Allowed(tool, target)
}

// Request 写入审批卡、挂起当前 run，直到前端回传决定或上下文取消。
func (a *Approvals) Request(ctx context.Context, request ApprovalRequest) (Outcome, error) {
	channel := make(chan Decision, 1)

	a.mu.Lock()
	a.pending[request.ID] = &pendingApproval{
		channel:   channel,
		partID:    request.PartID,
		target:    request.Target,
		permanent: request.AllowPermanent,
	}
	a.mu.Unlock()

	removePending := func() {
		a.mu.Lock()
		delete(a.pending, request.ID)
		a.mu.Unlock()
	}

	if err := a.db.CreateApproval(&store.Approval{
		ID:          request.ID,
		ThreadID:    request.ThreadID,
		RunID:       request.RunID,
		ToolCallID:  request.ToolCallID,
		Tool:        request.Tool,
		Risk:        request.Risk,
		RequestedAt: nowMillis(),
	}); err != nil {
		removePending()
		return Outcome{}, err
	}

	awaiting := request.cardPayload("waiting")
	if err := a.records.Append(store.EventRecord{
		ID:        request.PartID,
		ThreadID:  request.ThreadID,
		MessageID: request.MessageID,
		RunID:     request.RunID,
		Kind:      "approval",
		Payload:   awaiting,
	}); err != nil {
		removePending()
		_ = a.db.ResolveApproval(request.ID, "canceled", "")
		return Outcome{}, err
	}
	_ = a.records.SetThreadStatus(request.ThreadID, request.RunID, "awaiting")

	select {
	case decision := <-channel:
		scope := decision.Scope
		if scope == "" {
			scope = "once"
		}
		status, decisionText := "denied", "deny"
		if decision.Allow {
			status, decisionText = "approved", "allow"
		}
		_ = a.records.SetThreadStatus(request.ThreadID, request.RunID, "running")
		a.persist(request, id.New("ev"), "approval_resolved", request.resolvedPayload(status, decisionText, scope))
		return Outcome{Allow: decision.Allow, Status: status, Decision: decisionText, Scope: scope}, nil

	case <-ctx.Done():
		removePending()
		_ = a.db.ResolveApproval(request.ID, "canceled", "")
		_ = a.records.SetThreadStatus(request.ThreadID, request.RunID, "running")
		a.persist(request, id.New("ev"), "approval_resolved", request.resolvedPayload("canceled", "canceled", ""))
		return Outcome{Status: "canceled", Decision: "canceled"}, ctx.Err()
	}
}

// Resolve 校验并记录用户决定，然后唤醒等待中的 run。
//
// scope 的合法性、拒绝时强制 once、永久授权是否可用，都在这里判断，
// 调用方不需要重复实现状态机规则。
func (a *Approvals) Resolve(approvalID string, decision Decision) error {
	approval, err := a.db.GetApproval(approvalID)
	if err != nil {
		return err
	}
	if approval.Decision != "" {
		return fmt.Errorf("审批已处理：%s", approval.Decision)
	}

	scope := decision.Scope
	if !decision.Allow || scope == "" {
		scope = "once"
	}
	switch scope {
	case "once", "session", "workspace":
	default:
		return errors.New("无效的授权范围")
	}

	a.mu.Lock()
	entry := a.pending[approvalID]
	a.mu.Unlock()
	if entry == nil {
		return errors.New("审批等待已失效（可能已取消）")
	}
	if decision.Allow && scope != "once" && !entry.permanent {
		return errors.New("该审批不提供永久授权，只能允许一次")
	}

	status, dbDecision := "denied", "deny"
	if decision.Allow {
		status, dbDecision = "approved", "allow"
	}
	if err := a.db.ResolveApproval(approvalID, dbDecision, scope); err != nil {
		return err
	}

	a.mu.Lock()
	delete(a.pending, approvalID)
	a.mu.Unlock()

	if decision.Allow && scope != "once" {
		if err := a.grants.Grant(approval.Tool, entry.target, scope); err != nil {
			return err
		}
	}

	_ = a.db.AddAudit(&store.AuditEntry{
		ThreadID: approval.ThreadID,
		RunID:    approval.RunID,
		Tool:     approval.Tool,
		Risk:     approval.Risk,
		Decision: dbDecision,
		Scope:    scope,
		Detail:   "用户审批",
	})

	a.records.Emit(events.TypeApprovalResolved, approval.ThreadID, approval.RunID, map[string]any{
		"partId":     entry.partID,
		"approvalId": approvalID,
		"status":     status,
		"decision":   dbDecision,
		"scope":      scope,
	})

	entry.channel <- Decision{Allow: decision.Allow, Scope: scope}
	return nil
}

func (a *Approvals) persist(request ApprovalRequest, eventID, kind string, payload any) {
	a.records.AppendOrWarn(store.EventRecord{
		ID:        eventID,
		ThreadID:  request.ThreadID,
		MessageID: request.MessageID,
		RunID:     request.RunID,
		Kind:      kind,
		Payload:   payload,
	})
}

func nowMillis() int64 {
	return time.Now().UnixMilli()
}
