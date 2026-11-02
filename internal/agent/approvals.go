// 本文件实现工具审批：把一次需要用户确认的工具调用变成可跨进程交互的审批卡片，
// 并在前端回传决定后唤醒等待中的运行。
//
// 审批是 Go 侧与前端之间唯一的“阻塞点”：运行器调用 Request 挂起，服务层通过
// Resolve 传入用户决定，两边只共享数据库行、事件与内存等待表。

package agent

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/id"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// Fact 是审批卡片上展示的一条事实。
type Fact struct {
	// Label 是事实的名称，例如“风险等级”“目标”。
	Label string `json:"label"`
	// Value 是事实的内容，直接展示给用户。
	Value string `json:"value"`
}

// ApprovalRequest 是一次审批请求的完整输入。
//
// 除 ID 与关联标识外，其余字段都用于渲染审批卡片或校验授权范围，
// 前端不需要也不应该自行推断风险等级。
type ApprovalRequest struct {
	// ID 是审批本身的唯一标识，前端回传决定时使用。
	ID string
	// PartID 是审批卡片在消息中的部件标识。
	PartID string
	// ThreadID 是所属会话，用于事件归属与审计。
	ThreadID string
	// RunID 是触发审批的那次运行。
	RunID string
	// MessageID 是审批卡片所属的消息。
	MessageID string
	// ToolCallID 是模型给出的工具调用标识。
	ToolCallID string
	// Tool 是工具名称。
	Tool string
	// Risk 是风险评估等级的文字形式，例如 R2。
	Risk string
	// Title 是工具的人类可读标题。
	Title string
	// Target 是被操作的目标，例如文件路径或命令。
	Target string
	// Detail 是风险评估给出的原因说明。
	Detail string
	// Facts 是卡片上逐条展示的事实列表。
	Facts []Fact
	// Args 是工具的原始参数，供用户查看完整调用内容。
	Args map[string]any
	// AllowPermanent 表示本次审批是否允许授予会话级或工作区级授权。
	AllowPermanent bool
}

// Decision 是用户对审批的决定。
type Decision struct {
	// Allow 表示是否允许执行。
	Allow bool
	// Scope 是授权范围，取值为 once、session 或 workspace。
	Scope string
}

// Outcome 是审批结束后的结果。
type Outcome struct {
	// Allow 表示最终是否允许执行。
	Allow bool
	// Status 是卡片状态，取值为 approved、denied 或 canceled。
	Status string
	// Decision 是记录到数据库的决定，取值为 allow、deny 或 canceled。
	Decision string
	// Scope 是最终生效的授权范围。
	Scope string
}

// cardPayload 是审批卡片唯一的一份载荷，事件与数据库共用。
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

// resolvedPayload 生成“审批已结束”事件的最小载荷。
//
// 它按 partId 合并回原卡片，所以只需要状态与决定，不需要重复整张卡片的内容。
func (r ApprovalRequest) resolvedPayload(status, decision, scope string) map[string]any {
	return map[string]any{
		"partId":     r.PartID,
		"approvalId": r.ID,
		"status":     status,
		"decision":   decision,
		"scope":      scope,
	}
}

// pendingApproval 是内存里等待用户决定的一条审批。
type pendingApproval struct {
	// channel 用来把用户决定从 Resolve 送回阻塞中的 Request，容量为 1。
	channel chan Decision
	// partID 是审批卡片的部件标识，取消时用来定位。
	partID string
	// target 是这次被授权的目标，授予会话/工作区权限时写入授权缓存。
	target string
	// permanent 表示本次审批是否允许永久授权，拒绝时强制降级为 once。
	permanent bool
}

// Approvals 是审批的唯一入口。
//
// 状态机（waiting → approved / denied / canceled）、内存等待表、数据库行、
// 事件广播、授权缓存与审计都在这个模块内部完成；调用方只提供一次请求，
// 或回传一次决定。
type Approvals struct {
	// mu 保护 pending 表，Request 与 Resolve 可能来自不同 goroutine。
	mu sync.Mutex
	// pending 保存尚未决定的审批，以审批 ID 为键。
	pending map[string]*pendingApproval
	// db 负责审批行与审计记录的持久化。
	db *store.DB
	// records 负责事件写入与前端广播。
	records *record.Recorder
	// grants 保存会话级与工作区级的已有授权。
	grants *Grants
}

// NewApprovals 创建审批模块。授权缓存会在这里从数据库恢复。
func NewApprovals(db *store.DB, records *record.Recorder) *Approvals {
	return &Approvals{
		pending: map[string]*pendingApproval{},
		db:      db,
		records: records,
		grants:  NewGrants(db),
	}
}

// Allowed 判断工具调用是否命中已有授权。
//
// 只读查询，不会修改授权表；受限工作区由策略层单独处理，不依赖这里的缓存。
func (a *Approvals) Allowed(tool, target string) bool {
	return a.grants.Allowed(tool, target)
}

// Request 写入审批卡、挂起当前 run，直到前端回传决定或上下文取消。
//
// 流程固定为四步：登记内存等待表 → 写审批行 → 写 waiting 事件并切会话状态 →
// 阻塞等待 channel 或 ctx。任何一步失败都会清理已写入的状态，避免留下孤儿审批。
func (a *Approvals) Request(ctx context.Context, request ApprovalRequest) (Outcome, error) {
	// 容量为 1，保证 Resolve 即使先于 select 到达也不会永久阻塞。
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

	// 审批卡片与普通部件走同一个事件通道，前端按 partId 合并后续的决定事件。
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
	// 会话状态切到 awaiting，界面据此显示“等待确认”。
	_ = a.records.SetThreadStatus(request.ThreadID, request.RunID, "awaiting")

	select {
	case decision := <-channel:
		// 正常路径：Resolve 已经写好数据库与审计，这里只负责恢复会话状态并补一条决定事件。
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
		// 取消路径：审批按 canceled 写入数据库，运行器会据此中止本轮执行。
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
	// 先读数据库行：它记录了真实的工具与风险，也用于拒绝重复决定。
	approval, err := a.db.GetApproval(approvalID)
	if err != nil {
		return err
	}
	if approval.Decision != "" {
		return fmt.Errorf("审批已处理：%s", approval.Decision)
	}

	// 拒绝时一律按 once 记录；允许但没给范围时也按 once 处理，防止误授永久权限。
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
	// 永久授权是否可用由发起方决定：高风险操作即使前端传了 session 也必须拒绝。
	if decision.Allow && scope != "once" && !entry.permanent {
		return errors.New("该审批不提供永久授权，只能允许一次")
	}

	status, dbDecision := "denied", "deny"
	if decision.Allow {
		status, dbDecision = "approved", "allow"
	}
	// 先落数据库：即使后续广播失败，审批状态也不会回退成等待中。
	if err := a.db.ResolveApproval(approvalID, dbDecision, scope); err != nil {
		return err
	}

	a.mu.Lock()
	delete(a.pending, approvalID)
	a.mu.Unlock()

	// 只有“允许且不是一次性”才写入授权缓存，后续同类调用不再询问。
	if decision.Allow && scope != "once" {
		if err := a.grants.Grant(approval.Tool, entry.target, scope); err != nil {
			return err
		}
	}

	// 用户决定也要进审计，保证“谁允许了什么”可以追溯。
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

	// 唤醒挂起中的 Request；channel 有缓冲，不会阻塞在这里。
	entry.channel <- Decision{Allow: decision.Allow, Scope: scope}
	return nil
}

// persist 把审批决定写成事件；写入失败只记录警告，不影响已经完成的决定。
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

// nowMillis 返回当前毫秒时间戳，审批请求与决定时间统一用这个口径。
func nowMillis() int64 {
	return time.Now().UnixMilli()
}
