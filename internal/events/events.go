// Package events 定义 Go 与界面之间的事件契约，以及立即透传事件的总线。
//
// 设计要点：
//   - 事件类型字符串、信封（Envelope）与批量载荷（Batch）是唯一契约，迁移
//     Wails 时期只走一个批量通道；native UI 时期保留同一套类型表，供
//     chat reducer 等界面层按类型分派；
//   - 事件产生后立即派发，不在 Go 侧节流或合并：进程间通道消失后，
//     界面的 win.Update 自带按帧合并，服务端再做一次只增加延迟。
//
// 本文件定义事件类型常量、事件信封与批量载荷，以及透传的事件总线（Bus）。
package events

import (
	"sync"
	"time"
)

// 事件类型。前端按这些字符串分派 reducer。
const (
	// TypeRunStarted 表示一次运行开始。
	TypeRunStarted = "run.started"
	// TypeRunCompleted 表示一次运行正常结束。
	TypeRunCompleted = "run.completed"
	// TypeRunCanceled 表示一次运行被取消。
	TypeRunCanceled = "run.canceled"
	// TypeRunFailed 表示一次运行因错误结束。
	TypeRunFailed = "run.failed"
	// TypeTextDelta 是正文增量，载荷为 TextDelta。
	TypeTextDelta = "text.delta"
	// TypeTextCompleted 表示正文部件生成完毕。
	TypeTextCompleted = "text.completed"
	// TypeReasoningDelta 是推理增量，载荷为 TextDelta。
	TypeReasoningDelta = "reasoning.delta"
	// TypeReasoningCompleted 表示推理部件生成完毕。
	TypeReasoningCompleted = "reasoning.completed"
	// TypeToolStarted 表示工具开始执行。
	TypeToolStarted = "tool.started"
	// TypeToolCompleted 表示工具执行结束（成功或失败）。
	TypeToolCompleted = "tool.completed"
	// TypeApprovalRequested 表示需要用户审批，前端据此弹出审批浮层。
	TypeApprovalRequested = "approval.requested"
	// TypeApprovalResolved 表示审批已有结论。
	TypeApprovalResolved = "approval.resolved"
	// TypeTodoUpdated 表示待办列表发生变化。
	TypeTodoUpdated = "todo.updated"
	// TypeUserMessage 表示一条用户消息已写入事件表。当前只有运行中引导
	// （steer）广播该事件：普通发送由前端乐观插入显示，再广播会出双份。
	TypeUserMessage = "user.message"
	// TypeNotice 是普通提示消息。
	TypeNotice = "notice"
	// TypeContextCompressed 表示上下文被压缩。
	TypeContextCompressed = "context.compressed"
	// TypeError 表示后台错误，前端以提示条或 toast 展示。
	TypeError = "error"
	// TypeThreadUpdated 表示会话元信息变化（标题、状态等）。
	TypeThreadUpdated = "thread.updated"
	// TypeUsageUpdated 表示用量统计变化。
	TypeUsageUpdated = "usage.updated"
	// TypeSettingsUpdated 表示设置变化。
	TypeSettingsUpdated = "settings.updated"
	// TypeWorkspaceUpdated 表示工作区变化。
	TypeWorkspaceUpdated = "workspace.updated"
)

// BatchName 是 Wails 时期的批量事件通道名。native UI 下不再有事件通道，
// 但名字仍是契约的一部分（历史事件表与文档引用它），保留不改值。
const BatchName = "cw:events"

// Envelope 是单个事件的公共字段。
type Envelope struct {
	// Seq 是会话内单调递增的序号，前端可据此发现丢失或乱序。
	Seq int64 `json:"seq"`
	// Type 是事件类型，取值为本文件定义的 Type 常量。
	Type string `json:"type"`
	// ThreadID 是所属会话；全局事件可以为空。
	ThreadID string `json:"threadId,omitempty"`
	// RunID 是所属运行；非运行期事件可以为空。
	RunID string `json:"runId,omitempty"`
	// TS 是事件产生时间，毫秒时间戳。
	TS int64 `json:"ts"`
	// Data 是事件载荷，具体类型由 Type 决定。
	Data any `json:"data,omitempty"`
}

// Batch 是一次派发给界面的事件集合。透传后每个 Batch 只装一个事件，
// 结构保留为契约，界面层的消费代码不因迁移而改写。
type Batch struct {
	// V 是协议版本，当前固定为 1。
	V int `json:"v"`
	// ThreadID 是这批事件所属会话；批量推送按会话分组。
	ThreadID string `json:"threadId,omitempty"`
	// Events 是按产生顺序排列的事件。
	Events []*Envelope `json:"events"`
}

// TextDelta 是文本或推理增量的载荷。增量不再在 Go 侧合并，界面层按
// PartID 把连续增量接到同一段文本上。
type TextDelta struct {
	// PartID 是消息部件标识，用于判断增量是否属于同一段文本。
	PartID string `json:"partId"`
	// Text 是本批新增的文本。
	Text string `json:"text"`
}

// Emitter 由调用方提供，把事件派发到界面。native UI 下它是 win.Update 桥：
// 派发动作进主线程队列，界面在下一帧统一消费。
type Emitter func(batch Batch)

// Bus 把事件立即透传给派发函数。
//
// Wails 时期这里还负责按 33ms 合并增量、按会话缓冲批量推送（IPC 优化）；
// 进程间通道消失后这些都不再需要，只保留会话内序号的分配，让界面仍能
// 依据 Seq 发现丢失或乱序。
type Bus struct {
	// emit 是派发函数，可为空（此时事件只分配序号后被丢弃）。
	emit Emitter

	// mu 保护 seqs 与 closed。
	mu sync.Mutex
	// seqs 按会话记录已分配的最大事件序号。
	seqs map[string]int64
	// closed 表示总线已关闭，关闭后的写入会被丢弃。
	closed bool
}

// NewBus 创建事件总线。
func NewBus(emit Emitter) *Bus {
	return &Bus{emit: emit, seqs: make(map[string]int64)}
}

// Publish 派发一个事件：包装成单事件 Batch 后立即交给 Emitter。
//
// 序号在锁内分配，保证同一会话严格单调；派发在锁外进行，Emitter 变慢
// 不会阻塞其他会话写入。
func (b *Bus) Publish(eventType, threadID, runID string, data any) {
	b.mu.Lock()
	if b.closed {
		// 关闭后到达的事件直接丢弃，避免向已退出的界面派发。
		b.mu.Unlock()
		return
	}
	b.seqs[threadID]++
	envelope := &Envelope{
		Seq:      b.seqs[threadID],
		Type:     eventType,
		ThreadID: threadID,
		RunID:    runID,
		TS:       nowMillis(),
		Data:     data,
	}
	b.mu.Unlock()

	if b.emit != nil {
		b.emit(Batch{V: 1, ThreadID: threadID, Events: []*Envelope{envelope}})
	}
}

// Close 关闭总线，之后的事件会被丢弃。重复调用是安全的。
func (b *Bus) Close() {
	b.mu.Lock()
	b.closed = true
	b.mu.Unlock()
}

// nowMillis 返回当前时间的毫秒时间戳，统一事件时间基准。
func nowMillis() int64 {
	return time.Now().UnixMilli()
}
