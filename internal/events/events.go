// Package events 定义 Go 与前端之间的事件契约，以及把高频增量合并成
// 批量事件的总线。
//
// 设计要点：
//   - 前端只订阅一个事件名（cw:events），载荷是 Batch；
//   - 文本与推理增量在服务端按 33ms 合并且同一部件内合并，避免逐 token 通过进程间通信（IPC）传输；
//   - 非增量事件（工具、审批、生命周期）立即推送，保证顺序语义。
//
// 本文件负责定义事件类型常量、事件信封与批量载荷，以及按会话缓冲、合并、
// 节流推送的事件总线（Bus）。事件类型字符串与前端 src/lib/types.ts 必须
// 保持一致，改动即视为协议变更。
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
	// TypeTerminalOutput 是终端输出增量，载荷为 TerminalDelta。
	TypeTerminalOutput = "terminal.output"
	// TypeTerminalExit 表示终端进程退出。
	TypeTerminalExit = "terminal.exit"
)

// BatchName 是前端订阅的固定事件名。前端只监听这一个通道，再按 Envelope.Type
// 分派到各自的 reducer。
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

// Batch 是一次 IPC 推送的事件集合。
type Batch struct {
	// V 是协议版本，当前固定为 1。
	V int `json:"v"`
	// ThreadID 是这批事件所属会话；批量推送按会话分组。
	ThreadID string `json:"threadId,omitempty"`
	// Events 是按产生顺序排列的事件。
	Events []*Envelope `json:"events"`
}

// TextDelta 是文本或推理增量的载荷。同一部件（PartID）的连续增量会在总线
// 内合并成一段再推送。
type TextDelta struct {
	// PartID 是消息部件标识，用于判断增量是否属于同一段文本。
	PartID string `json:"partId"`
	// Text 是本批新增的文本。
	Text string `json:"text"`
}

// TerminalDelta 是终端输出增量。
type TerminalDelta struct {
	// SessionID 是终端会话标识。
	SessionID string `json:"sessionId"`
	// Data 是本批新增的终端输出。
	Data string `json:"data"`
}

// Emitter 由调用方提供，通常是 Wails 的 app.Event.Emit 包装。
type Emitter func(name string, data any)

// Bus 按会话缓冲事件，并以固定节拍合并推送。
//
// 会话缓冲保证同一会话内事件有序；合并只发生在相邻且属于同一部件的增量上，
// 其他事件到达时会把积压的增量一起冲刷，维持先后顺序。
type Bus struct {
	// emit 是实际推送函数，可为空（此时只缓冲不推送）。
	emit Emitter
	// clock 是后台冲刷节拍，默认 33 毫秒。
	clock time.Duration

	// mu 保护 buffers、closed 与各会话缓冲。
	mu sync.Mutex
	// buffers 按会话保存待推送事件；没有缓冲的会话不会出现在这里。
	buffers map[string]*threadBuffer
	// closed 表示总线已关闭，关闭后的写入会被丢弃。
	closed bool
	// done 关闭后通知后台协程退出。
	done chan struct{}
}

// threadBuffer 是单个会话的待推送事件。
type threadBuffer struct {
	// seq 是该会话已分配的最大事件序号。
	seq int64
	// events 是尚未推送的事件，按产生顺序排列。
	events []*Envelope
}

// NewBus 创建事件总线并启动后台冲刷协程。冲刷节拍固定为 33 毫秒，与文本
// 增量的合并窗口一致。
func NewBus(emit Emitter) *Bus {
	bus := &Bus{
		emit:    emit,
		clock:   33 * time.Millisecond,
		buffers: make(map[string]*threadBuffer),
		done:    make(chan struct{}),
	}
	go bus.loop()
	return bus
}

// Publish 写入一个事件。Data 中的增量载荷需要是 TextDelta 或 TerminalDelta
// 才能参与合并。
//
// 合并规则：只有缓冲末尾正好是同类型、同一部件的增量时才合并；其他事件
// 会先入队，因此不会跨事件打乱顺序。非增量事件立即推送，增量事件按节拍
// 或数量上限推送。
func (b *Bus) Publish(eventType, threadID, runID string, data any) {
	b.mu.Lock()
	if b.closed {
		// 关闭后到达的事件直接丢弃，避免向已退出的前端推送。
		b.mu.Unlock()
		return
	}
	buffer := b.buffers[threadID]
	if buffer == nil {
		buffer = &threadBuffer{}
		b.buffers[threadID] = buffer
	}

	// 文本与推理增量：末尾事件同类型且同部件时直接追加文本。
	delta, isDelta := data.(TextDelta)
	if isDelta && len(buffer.events) > 0 {
		if last := buffer.events[len(buffer.events)-1]; last.Type == eventType {
			if previous, ok := last.Data.(TextDelta); ok && previous.PartID == delta.PartID {
				previous.Text += delta.Text
				last.Data = previous
				last.TS = nowMillis()
				b.mu.Unlock()
				return
			}
		}
	}

	// 终端输出增量：按会话标识合并，合并后超过 16KB 时立即推送，避免缓冲过大。
	terminal, isTerminal := data.(TerminalDelta)
	if isTerminal && len(buffer.events) > 0 {
		if last := buffer.events[len(buffer.events)-1]; last.Type == eventType {
			if previous, ok := last.Data.(TerminalDelta); ok && previous.SessionID == terminal.SessionID {
				previous.Data += terminal.Data
				last.Data = previous
				last.TS = nowMillis()
				if len(previous.Data) < 16384 {
					b.mu.Unlock()
					return
				}
				// 合并后的输出达到上限，立即推送并清空缓冲。
				pending := buffer.events
				buffer.events = nil
				b.mu.Unlock()
				b.emitBatch(threadID, pending)
				return
			}
		}
	}
	isDelta = isDelta || isTerminal

	// 分配会话内序号并入队。序号在锁内递增，保证同一会话严格单调。
	buffer.seq++
	envelope := &Envelope{
		Seq:      buffer.seq,
		Type:     eventType,
		ThreadID: threadID,
		RunID:    runID,
		TS:       nowMillis(),
		Data:     data,
	}
	buffer.events = append(buffer.events, envelope)

	var pending []*Envelope
	if !isDelta {
		// 非增量事件立即推送，同时把之前积压的增量一起带走，保持顺序。
		pending = buffer.events
		buffer.events = nil
	} else if len(buffer.events) >= 32 {
		// 增量过快时按数量兜底，避免等待节拍期间缓冲无限增长。
		pending = buffer.events
		buffer.events = nil
	}
	b.mu.Unlock()

	if len(pending) > 0 {
		b.emitBatch(threadID, pending)
	}
}

// Flush 立即推送指定会话的缓冲。没有缓冲时什么也不做。
func (b *Bus) Flush(threadID string) {
	b.mu.Lock()
	buffer := b.buffers[threadID]
	if buffer == nil || len(buffer.events) == 0 {
		b.mu.Unlock()
		return
	}
	pending := buffer.events
	buffer.events = nil
	b.mu.Unlock()
	b.emitBatch(threadID, pending)
}

// FlushAll 推送所有会话的缓冲。先在锁内把各会话缓冲摘下来，再在锁外逐个
// 推送，避免推送过程阻塞其他会话写入。
func (b *Bus) FlushAll() {
	b.mu.Lock()
	type item struct {
		// threadID 是这批事件所属会话。
		threadID string
		// events 是摘下来的待推送事件。
		events []*Envelope
	}
	var items []item
	for threadID, buffer := range b.buffers {
		if len(buffer.events) == 0 {
			continue
		}
		items = append(items, item{threadID: threadID, events: buffer.events})
		buffer.events = nil
	}
	b.mu.Unlock()
	for _, entry := range items {
		b.emitBatch(entry.threadID, entry.events)
	}
}

// Close 停止后台协程并冲刷剩余事件。重复调用是安全的。
func (b *Bus) Close() {
	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		return
	}
	b.closed = true
	close(b.done)
	b.mu.Unlock()
	b.FlushAll()
}

// loop 是后台冲刷协程：按固定节拍推送所有会话的缓冲，收到退出信号后结束。
func (b *Bus) loop() {
	ticker := time.NewTicker(b.clock)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			b.FlushAll()
		case <-b.done:
			return
		}
	}
}

// emitBatch 把一批事件包装成 Batch 后推送。空批次或没有推送函数时直接返回。
func (b *Bus) emitBatch(threadID string, list []*Envelope) {
	if len(list) == 0 || b.emit == nil {
		return
	}
	b.emit(BatchName, Batch{V: 1, ThreadID: threadID, Events: list})
}

// nowMillis 返回当前时间的毫秒时间戳，统一事件时间基准。
func nowMillis() int64 {
	return time.Now().UnixMilli()
}
