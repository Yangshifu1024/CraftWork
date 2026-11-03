// Package chat 是原生 UI 的聊天列：会话事件合并（reducer）、消息流视图、
// Markdown 渲染与输入区（Composer）。
//
// reducer.go 把 frontend/src/stores/threadReducer.ts 逐 handler 移植为纯函数：
// 给定状态与事件批次返回新状态，不触达服务层，因此可以脱离界面单独测试。
// 事件类型字符串与载荷形状以 internal/events 为唯一契约。
package chat

import (
	"encoding/json"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/ui/theme"
)

// 消息角色与状态（frontend lib/types.ts 的字面量联合类型）。
const (
	RoleUser      = "user"
	RoleAssistant = "assistant"

	StatusStreaming  = "streaming"
	StatusComplete   = "complete"
	StatusIncomplete = "incomplete"
	StatusError      = "error"

	// ThreadStatus 是会话状态（ThreadView.status 的取值域）。
	ThreadStatusRunning   = "running"
	ThreadStatusAwaiting  = "awaiting"
	ThreadStatusIdle      = "idle"
	ThreadStatusErrorStat = "error"
)

// 部件类型（PartKind）。
const (
	PartText      = "text"
	PartReasoning = "reasoning"
	PartTool      = "tool"
	PartApproval  = "approval"
	PartTodo      = "todo"
	PartError     = "error"
	PartNotice    = "notice"
	PartDraft     = "draft"
	PartCallout   = "callout"
)

// Part 是消息中的一个内容块。Data 的形状由 Kind 决定；事件载荷统一在这里
// 归一化为 map（实时事件是强类型结构体，快照载荷是 JSON 解出的 map）。
type Part struct {
	ID   string
	Kind string
	Seq  int64
	Data map[string]any
}

// Message 是一条聊天消息。指针语义：切片在状态转移间共享未变化的消息，
// 被修改的消息克隆后替换，与 React 版的引用保持一致。
type Message struct {
	ID             string
	ThreadID       string
	Role           string
	Status         string
	CreatedAt      int64
	WorkDurationMs int64
	// SteerQueued 是运行中引导消息的「待送达」标记：已显示但尚未并入
	// 下一次模型请求。收到本会话任意后续活动事件即清除。
	SteerQueued bool
	Parts       []Part
}

// ThreadInfo 是 reducer 维护的会话视图（ThreadView 的子集）：聊天头部与
// Composer 需要的状态、标题、模型与用量字段。
type ThreadInfo struct {
	ID                 string
	Title              string
	Mode               string
	ModelID            string
	ModelName          string
	Status             string
	Tokens             int64
	ContextTokens      int64
	ContextCacheTokens int64
	ContextSpeed       float64
}

// Slice 是事件合并涉及的界面状态子集（threadReducer.ts 的 ThreadSlice）。
// threads 按 Id 索引，两种模式合一：React 版双列表扫描只为侧栏展示，
// 原生侧侧栏列表归 04 号票的 sidebar 包，这里的表只服务聊天列自身。
type Slice struct {
	Messages         []*Message
	Running          bool
	RunningMessageID string
	PendingApprovals []map[string]any
	Threads          map[string]ThreadInfo
}

// Result 在 Slice 之上多出刷新标记：批次里出现了其他会话的变更或一次
// 运行收尾，调用方需要重新拉取会话列表（对齐 React 版 refreshNeeded）。
type Result struct {
	Slice
	RefreshNeeded bool
}

// draft 是合并过程中的可变草稿。
type draft struct {
	Slice
	activeThreadID string
	refreshNeeded  bool
}

// ---------------------------------------------------------------------------
// 载荷归一化
// ---------------------------------------------------------------------------

// dataOf 把事件或部件的载荷归一化为 map。同进程派发的实时事件可能是
// 强类型结构体（events.TextDelta 等），快照读回的是 JSON map；统一转成
// map 后，handler 与视图只面对一种形状。
func dataOf(v any) map[string]any {
	switch t := v.(type) {
	case map[string]any:
		return t
	case nil:
		return map[string]any{}
	case events.TextDelta:
		return map[string]any{"partId": t.PartID, "text": t.Text}
	case string, bool, float64, int, int64:
		return map[string]any{"text": t}
	default:
		raw, err := json.Marshal(t)
		if err != nil {
			return map[string]any{}
		}
		var m map[string]any
		if json.Unmarshal(raw, &m) != nil || m == nil {
			return map[string]any{}
		}
		return m
	}
}

// strOf 取 map 里的字符串字段，缺失或类型不符返回空串。
func strOf(m map[string]any, key string) string {
	if m == nil {
		return ""
	}
	if s, ok := m[key].(string); ok {
		return s
	}
	return ""
}

// numOf 取 map 里的数值字段；实时 map 里的整数与 JSON 解出的浮点都接受。
func numOf(m map[string]any, key string) float64 {
	if m == nil {
		return 0
	}
	switch v := m[key].(type) {
	case float64:
		return v
	case int64:
		return float64(v)
	case int:
		return float64(v)
	case json.Number:
		f, _ := v.Float64()
		return f
	default:
		return 0
	}
}

// boolOf 取 map 里的布尔字段：实时事件是 Go 原生 bool，快照 JSON 解出的
// 也是 bool；兼容旧的字符串 "true" 形态。缺失按 false。
func boolOf(m map[string]any, key string) bool {
	if m == nil {
		return false
	}
	switch v := m[key].(type) {
	case bool:
		return v
	case string:
		return v == "true" || v == "1"
	case float64:
		return v != 0
	default:
		return false
	}
}

// ---------------------------------------------------------------------------
// 消息级纯函数
// ---------------------------------------------------------------------------

// cloneMessages 复制消息切片的顶层（消息指针共享）。
func cloneMessages(messages []*Message) []*Message {
	out := make([]*Message, len(messages))
	copy(out, messages)
	return out
}

// cloneMessage 深拷贝一条消息（部件切片一并复制，部件内容共享）。
func cloneMessage(m *Message) *Message {
	c := *m
	c.Parts = make([]Part, len(m.Parts))
	copy(c.Parts, m.Parts)
	return &c
}

// ensureMessage 保证列表中存在指定 id 的 assistant 消息并标记为流式输出中。
// 事件可能早于消息列表加载完成就到达，找不到时补一条空消息；
// 已存在的消息只更新状态，避免覆盖已有部件。
func ensureMessage(messages []*Message, messageID, threadID string) []*Message {
	for i, m := range messages {
		if m.ID == messageID {
			if m.Status == StatusStreaming {
				return messages
			}
			out := cloneMessages(messages)
			c := cloneMessage(out[i])
			c.Status = StatusStreaming
			out[i] = c
			return out
		}
	}
	return append(cloneMessages(messages), &Message{
		ID:        messageID,
		ThreadID:  threadID,
		Role:      RoleAssistant,
		Status:    StatusStreaming,
		CreatedAt: theme.Now(),
	})
}

// appendText 把一段文本增量追加到指定消息的指定部件上。
// 部件不存在时新建，seq 取当前部件数量，保证顺序与后端一致。
func appendText(messages []*Message, messageID, partID, kind, text string) []*Message {
	for i, m := range messages {
		if m.ID != messageID {
			continue
		}
		out := cloneMessages(messages)
		c := cloneMessage(out[i])
		for j := range c.Parts {
			if c.Parts[j].ID != partID {
				continue
			}
			// 文本增量合并：旧内容在前、新片段在后，拼成完整正文。
			// Data 复制后写：旧状态与新状态共享消息指针之外的部件内容，
			// 原地写会污染合并前的状态。
			old := c.Parts[j].Data
			data := make(map[string]any, len(old)+1)
			for k, v := range old {
				data[k] = v
			}
			data["text"] = strOf(old, "text") + text
			c.Parts[j].Data = data
			out[i] = c
			return out
		}
		// 部件还不存在：追加到消息末尾。
		c.Parts = append(c.Parts, Part{
			ID:   partID,
			Kind: kind,
			Seq:  int64(len(c.Parts)),
			Data: map[string]any{"text": text},
		})
		out[i] = c
		return out
	}
	return messages
}

// upsertPart 按部件 id 新增或替换一个部件。同一部件可能先以「开始」事件
// 出现、再以「完成」事件带完整数据到达，替换保持位置不变；消息本身不
// 存在时补一条 assistant 流式消息，避免事件丢失。
func upsertPart(messages []*Message, messageID, threadID string, part Part) []*Message {
	for i, m := range messages {
		if m.ID != messageID {
			continue
		}
		out := cloneMessages(messages)
		c := cloneMessage(out[i])
		for j := range c.Parts {
			if c.Parts[j].ID == part.ID {
				c.Parts[j] = part
				out[i] = c
				return out
			}
		}
		c.Parts = append(c.Parts, part)
		out[i] = c
		return out
	}
	return append(cloneMessages(messages), &Message{
		ID:        messageID,
		ThreadID:  threadID,
		Role:      RoleAssistant,
		Status:    StatusStreaming,
		CreatedAt: theme.Now(),
		Parts:     []Part{part},
	})
}

// setMessageStatus 更新单条消息的状态；没有目标 id 时原样返回。
func setMessageStatus(messages []*Message, messageID, status string) []*Message {
	if messageID == "" {
		return messages
	}
	for i, m := range messages {
		if m.ID == messageID {
			out := cloneMessages(messages)
			c := cloneMessage(out[i])
			c.Status = status
			out[i] = c
			return out
		}
	}
	return messages
}

// clearSteerQueued 清除所有引导消息的「待送达」标记。运行中引导显示后，
// 收到本会话任意后续活动事件（新 run、流式增量、工具开始）即视为排队
// 引导已被并入模型请求；运行终态也清除。没有标记时原样返回。
func clearSteerQueued(messages []*Message) []*Message {
	found := false
	for _, m := range messages {
		if m.SteerQueued {
			found = true
			break
		}
	}
	if !found {
		return messages
	}
	out := cloneMessages(messages)
	for i, m := range out {
		if m.SteerQueued {
			c := cloneMessage(m)
			c.SteerQueued = false
			out[i] = c
		}
	}
	return out
}

// updateApprovalPart 就地更新审批部件的字段（approval.resolved 后把
// 「等待」改成最终结果）；只匹配 id 相同且类型为审批的部件。
func updateApprovalPart(messages []*Message, partID string, patch map[string]any) []*Message {
	changed := false
	out := cloneMessages(messages)
	for i, m := range out {
		mc := cloneMessage(m)
		partChanged := false
		for j := range mc.Parts {
			if mc.Parts[j].ID == partID && mc.Parts[j].Kind == PartApproval {
				old := mc.Parts[j].Data
				data := make(map[string]any, len(old)+len(patch))
				for k, v := range old {
					data[k] = v
				}
				for k, v := range patch {
					data[k] = v
				}
				mc.Parts[j].Data = data
				partChanged = true
			}
		}
		if partChanged {
			out[i] = mc
			changed = true
		}
	}
	if !changed {
		return messages
	}
	return out
}

// ---------------------------------------------------------------------------
// 会话级纯函数（React 版 updateThread / updateThreadWith 的键表等价物）
// ---------------------------------------------------------------------------

// updateThread 在会话表里合并某个会话的字段，返回新表（写时复制）。
func updateThread(threads map[string]ThreadInfo, threadID string, patch ThreadInfo) map[string]ThreadInfo {
	if threadID == "" {
		return threads
	}
	out := make(map[string]ThreadInfo, len(threads)+1)
	for id, t := range threads {
		out[id] = t
	}
	t := out[threadID]
	t.ID = threadID
	if patch.Title != "" {
		t.Title = patch.Title
	}
	if patch.Mode != "" {
		t.Mode = patch.Mode
	}
	if patch.ModelID != "" {
		t.ModelID = patch.ModelID
	}
	if patch.ModelName != "" {
		t.ModelName = patch.ModelName
	}
	if patch.Status != "" {
		t.Status = patch.Status
	}
	if patch.Tokens != 0 {
		t.Tokens = patch.Tokens
	}
	if patch.ContextTokens != 0 {
		t.ContextTokens = patch.ContextTokens
	}
	if patch.ContextCacheTokens != 0 {
		t.ContextCacheTokens = patch.ContextCacheTokens
	}
	if patch.ContextSpeed != 0 {
		t.ContextSpeed = patch.ContextSpeed
	}
	out[threadID] = t
	return out
}

// updateThreadWith 用函数改写某个会话（用量事件的累加语义）。
func updateThreadWith(threads map[string]ThreadInfo, threadID string, update func(ThreadInfo) ThreadInfo) map[string]ThreadInfo {
	if threadID == "" {
		return threads
	}
	out := make(map[string]ThreadInfo, len(threads)+1)
	for id, t := range threads {
		if id == threadID {
			out[id] = update(t)
		} else {
			out[id] = t
		}
	}
	return out
}

// threadUpdatedData 把 thread.updated 事件载荷里的整条会话对象换算成
// ThreadInfo 补丁（缺失字段为零值，合并时被跳过）。
func threadUpdatedData(m map[string]any) ThreadInfo {
	return ThreadInfo{
		ID:                 strOf(m, "id"),
		Title:              strOf(m, "title"),
		Mode:               strOf(m, "mode"),
		ModelID:            strOf(m, "modelId"),
		ModelName:          strOf(m, "modelName"),
		Status:             strOf(m, "status"),
		Tokens:             int64(numOf(m, "tokens")),
		ContextTokens:      int64(numOf(m, "contextTokens")),
		ContextCacheTokens: int64(numOf(m, "contextCacheTokens")),
		ContextSpeed:       numOf(m, "contextSpeed"),
	}
}

// ---------------------------------------------------------------------------
// 事件级 handler
// ---------------------------------------------------------------------------

// upsertFromEvent 把整块部件事件写入当前正在流式输出的消息。用于工具、
// 审批、待办、通知、错误等「一次发送完整数据」的事件；没有运行中的消息
// 或事件缺少部件 id 时直接忽略。
func (d *draft) upsertFromEvent(kind string, e *events.Envelope) {
	data := dataOf(e.Data)
	partID := strOf(data, "partId")
	if d.RunningMessageID == "" || partID == "" {
		return
	}
	d.Messages = upsertPart(d.Messages, d.RunningMessageID, d.activeThreadID, Part{
		ID:   partID,
		Kind: kind,
		Seq:  0,
		Data: data,
	})
}

// appendFromEvent 把文本类增量事件追加到当前正在流式输出的消息。
func (d *draft) appendFromEvent(kind string, e *events.Envelope) {
	partID := ""
	text := ""
	if td, ok := e.Data.(events.TextDelta); ok {
		partID, text = td.PartID, td.Text
	} else {
		data := dataOf(e.Data)
		partID, text = strOf(data, "partId"), strOf(data, "text")
	}
	if d.RunningMessageID == "" || partID == "" {
		return
	}
	d.Messages = appendText(d.Messages, d.RunningMessageID, partID, kind, text)
}

// finishRun 收尾一次运行：给当前消息写入终态、把事件携带的运行时长回填
// 为「已工作」工时、清掉待送达标记、清空运行标记、把会话状态改成终态，
// 最后标记需要刷新列表。
func (d *draft) finishRun(messageStatus, threadStatus string, e *events.Envelope) {
	d.Messages = setMessageStatus(d.Messages, d.RunningMessageID, messageStatus)
	// 运行结束事件带后端计算的耗时，回填给消息用于「已工作 N」折叠显示；
	// 与历史快照的回填字段一致，两条路径拿到的都是持久化事实。
	if durationMs := int64(numOf(dataOf(e.Data), "durationMs")); durationMs > 0 && d.RunningMessageID != "" {
		messageID := d.RunningMessageID
		d.Messages = cloneMessages(d.Messages)
		for i, m := range d.Messages {
			if m.ID == messageID {
				c := cloneMessage(m)
				c.WorkDurationMs = durationMs
				d.Messages[i] = c
			}
		}
	}
	// 运行终态后不再有「即将投递」的语义：取消或失败时未投递的引导
	// 会留在会话记录里，随下一次运行的模型历史重建生效。
	d.Messages = clearSteerQueued(d.Messages)
	d.Running = false
	d.RunningMessageID = ""
	if d.activeThreadID != "" {
		d.Threads = updateThread(d.Threads, d.activeThreadID, ThreadInfo{Status: threadStatus})
	}
	d.refreshNeeded = true
}

// appendSteerMessage 落库一条运行中引导：按消息 id 幂等插入一条用户消息。
// 引导不走乐观插入，这条事件就是显示的唯一来源；重复推送按 id 去重。
func (d *draft) appendSteerMessage(e *events.Envelope) {
	data := dataOf(e.Data)
	messageID := strOf(data, "messageId")
	if messageID == "" {
		return
	}
	for _, m := range d.Messages {
		if m.ID == messageID {
			return
		}
	}
	d.Messages = append(cloneMessages(d.Messages), &Message{
		ID:          messageID,
		ThreadID:    d.activeThreadID,
		Role:        RoleUser,
		Status:      StatusComplete,
		CreatedAt:   theme.Now(),
		SteerQueued: true,
		Parts: []Part{{
			ID:   messageID + "-text",
			Kind: PartText,
			Seq:  0,
			Data: map[string]any{"text": strOf(data, "text"), "attachments": data["attachments"]},
		}},
	})
}

// ---------------------------------------------------------------------------
// 合并入口
// ---------------------------------------------------------------------------

// Reduce 是会话事件的唯一合并入口（threadReducer.ts reduceThread 的移植）。
//
// 纯函数：给定状态与批次返回新状态。处理顺序严格按批次里的事件顺序进行，
// 因此同一批次内的增量不会乱序。state.Threads 允许为 nil（首次使用）。
func Reduce(state Slice, batch events.Batch, activeThreadID string) Result {
	d := &draft{
		Slice: Slice{
			Messages:         state.Messages,
			Running:          state.Running,
			RunningMessageID: state.RunningMessageID,
			PendingApprovals: state.PendingApprovals,
			Threads:          state.Threads,
		},
		activeThreadID: activeThreadID,
	}
	if d.Threads == nil {
		d.Threads = map[string]ThreadInfo{}
	}

	for _, e := range batch.Events {
		// 事件批次过滤：终端事件挂在 "terminal:" 伪会话下，与聊天消息无关。
		if strings.HasPrefix(e.ThreadID, "terminal:") {
			continue
		}
		// 当前会话判定：明确属于其他会话的事件不进入本会话的消息与运行状态。
		// 但会话列表仍要跟上，遇到其他会话的更新事件只打刷新标记。
		if activeThreadID != "" && e.ThreadID != "" && e.ThreadID != activeThreadID {
			if e.Type == events.TypeThreadUpdated {
				d.refreshNeeded = true
			}
			continue
		}
		d.dispatch(e)
		// 没有登记的事件类型（后端新增、界面尚未跟上）静默忽略。
	}

	return Result{
		Slice:         d.Slice,
		RefreshNeeded: d.refreshNeeded,
	}
}

// dispatch 按事件类型分派到 handler（23 种事件一一对应 threadReducer.ts
// 的 handlers 表；新增 events.Type 时编译不强制补表，靠本包单测覆盖）。
func (d *draft) dispatch(e *events.Envelope) {
	switch e.Type {
	case events.TypeRunStarted:
		// 运行开始：记录正在流式输出的消息 id，并把会话标记为运行中。
		// 接力运行也走这里；开始即视为排队引导即将投递，清掉「待送达」标记。
		messageID := strOf(dataOf(e.Data), "messageId")
		if messageID == "" {
			return
		}
		d.RunningMessageID = messageID
		d.Running = true
		d.Messages = clearSteerQueued(ensureMessage(d.Messages, messageID, d.activeThreadID))
		if d.activeThreadID != "" {
			d.Threads = updateThread(d.Threads, d.activeThreadID, ThreadInfo{Status: ThreadStatusRunning})
		}
	case events.TypeTextDelta:
		// 正文增量：把片段追加到对应文本部件后面；出现新内容说明引导已投递。
		d.appendFromEvent(PartText, e)
		d.Messages = clearSteerQueued(d.Messages)
	case events.TypeTextCompleted:
		// 正文片段完成：用后端给出的完整文本覆盖该部件。
		d.upsertFromEvent(PartText, e)
	case events.TypeReasoningDelta:
		// 推理过程增量：与正文分开存放，供界面折叠展示。
		d.appendFromEvent(PartReasoning, e)
		d.Messages = clearSteerQueued(d.Messages)
	case events.TypeReasoningCompleted:
		d.upsertFromEvent(PartReasoning, e)
	case events.TypeToolStarted:
		// 工具调用开始：写入工具部件；出现新活动说明引导已投递。
		d.upsertFromEvent(PartTool, e)
		d.Messages = clearSteerQueued(d.Messages)
	case events.TypeToolCompleted:
		// 工具调用结束：同一部件 id 合并输出、耗时与错误标记。
		d.upsertFromEvent(PartTool, e)
	case events.TypeApprovalRequested:
		// 先落一个审批部件，界面据此渲染允许 / 拒绝按钮；去重后放入
		// 待审批队列；会话切到等待审批状态。
		d.upsertFromEvent(PartApproval, e)
		approval := dataOf(e.Data)
		approvalID := strOf(approval, "approvalId")
		kept := make([]map[string]any, 0, len(d.PendingApprovals)+1)
		for _, item := range d.PendingApprovals {
			if strOf(item, "approvalId") != approvalID {
				kept = append(kept, item)
			}
		}
		d.PendingApprovals = append(kept, approval)
		if d.activeThreadID != "" {
			d.Threads = updateThread(d.Threads, d.activeThreadID, ThreadInfo{Status: ThreadStatusAwaiting})
		}
	case events.TypeApprovalResolved:
		// 审批状态更新：部件里的等待状态改成最终结果，并移出待审批队列。
		data := dataOf(e.Data)
		if partID := strOf(data, "partId"); partID != "" {
			d.Messages = updateApprovalPart(d.Messages, partID, map[string]any{
				"status":   data["status"],
				"decision": data["decision"],
				"scope":    data["scope"],
			})
		}
		approvalID := strOf(data, "approvalId")
		kept := make([]map[string]any, 0, len(d.PendingApprovals))
		for _, item := range d.PendingApprovals {
			if strOf(item, "approvalId") != approvalID {
				kept = append(kept, item)
			}
		}
		d.PendingApprovals = kept
		if d.activeThreadID != "" {
			d.Threads = updateThread(d.Threads, d.activeThreadID, ThreadInfo{Status: ThreadStatusRunning})
		}
	case events.TypeTodoUpdated:
		// 待办清单更新：整份清单一次性替换。
		d.upsertFromEvent(PartTodo, e)
	case events.TypeUserMessage:
		// 运行中引导落库：插入带「待送达」标记的用户消息气泡。
		d.appendSteerMessage(e)
	case events.TypeNotice:
		// 普通提示：例如上下文压缩说明，作为消息内的通知部件展示。
		d.upsertFromEvent(PartNotice, e)
	case events.TypeError:
		// 错误事件：写成错误部件，展示错误码与可读文案。
		d.upsertFromEvent(PartError, e)
	case events.TypeRunCompleted:
		d.finishRun(StatusComplete, ThreadStatusIdle, e)
	case events.TypeRunCanceled:
		d.finishRun(StatusIncomplete, ThreadStatusIdle, e)
	case events.TypeRunFailed:
		d.finishRun(StatusError, ThreadStatusErrorStat, e)
	case events.TypeThreadUpdated:
		// 事件带整条会话时直接合并；只带状态时更新当前会话的状态字段。
		data := dataOf(e.Data)
		if th, ok := data["thread"].(map[string]any); ok {
			info := threadUpdatedData(th)
			if info.ID != "" {
				d.Threads = updateThread(d.Threads, info.ID, info)
			}
		} else if d.activeThreadID != "" && strOf(data, "status") != "" {
			d.Threads = updateThread(d.Threads, d.activeThreadID, ThreadInfo{Status: strOf(data, "status")})
		}
	case events.TypeUsageUpdated:
		// 用量是累加语义：在已有计数上加本次输入与输出之和；上下文占用、
		// 缓存命中与输出速度是覆盖语义，只有主对话请求携带这些字段
		//（sidecar 小请求不带），缺失时保持原值。
		data := dataOf(e.Data)
		input := numOf(data, "inputTokens")
		output := numOf(data, "outputTokens")
		if d.activeThreadID == "" || input+output <= 0 {
			return
		}
		d.Threads = updateThreadWith(d.Threads, d.activeThreadID, func(t ThreadInfo) ThreadInfo {
			t.Tokens += int64(input + output)
			return t
		})
		if contextTokens := int64(numOf(data, "contextTokens")); contextTokens > 0 {
			d.Threads = updateThreadWith(d.Threads, d.activeThreadID, func(t ThreadInfo) ThreadInfo {
				t.ContextTokens = contextTokens
				t.ContextCacheTokens = int64(numOf(data, "cacheTokens"))
				t.ContextSpeed = numOf(data, "tokensPerSecond")
				return t
			})
		}
	case events.TypeContextCompressed, events.TypeSettingsUpdated, events.TypeWorkspaceUpdated,
		events.TypeTerminalOutput, events.TypeTerminalExit:
		// 压缩文案由 notice 部件承载；设置 / 工作区 / 终端事件由各自的
		// 接线处理，会话合并只负责会话内状态。
	}
}
