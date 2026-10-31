// Package events 定义 Go 与前端之间的事件契约，以及把高频增量合并成
// 批量事件的总线。
//
// 设计要点：
//   - 前端只订阅一个事件名（cw:events），载荷是 Batch；
//   - 文本与推理增量在服务端按 33ms 合并且同一部件内合并，避免逐 token 过 IPC；
//   - 非增量事件（工具、审批、生命周期）立即推送，保证顺序语义。
package events

import (
	"sync"
	"time"
)

// 事件类型。前端按这些字符串分派 reducer。
const (
	TypeRunStarted         = "run.started"
	TypeRunCompleted       = "run.completed"
	TypeRunCanceled        = "run.canceled"
	TypeRunFailed          = "run.failed"
	TypeTextDelta          = "text.delta"
	TypeTextCompleted      = "text.completed"
	TypeReasoningDelta     = "reasoning.delta"
	TypeReasoningCompleted = "reasoning.completed"
	TypeToolStarted        = "tool.started"
	TypeToolCompleted      = "tool.completed"
	TypeApprovalRequested  = "approval.requested"
	TypeApprovalResolved   = "approval.resolved"
	TypeTodoUpdated        = "todo.updated"
	TypeNotice             = "notice"
	TypeContextCompressed  = "context.compressed"
	TypeError              = "error"
	TypeThreadUpdated      = "thread.updated"
	TypeUsageUpdated       = "usage.updated"
	TypeSettingsUpdated    = "settings.updated"
	TypeWorkspaceUpdated   = "workspace.updated"
	TypeTerminalOutput     = "terminal.output"
	TypeTerminalExit       = "terminal.exit"
)

// BatchName 是前端订阅的固定事件名。
const BatchName = "cw:events"

// Envelope 是单个事件的公共字段。
type Envelope struct {
	Seq      int64  `json:"seq"`
	Type     string `json:"type"`
	ThreadID string `json:"threadId,omitempty"`
	RunID    string `json:"runId,omitempty"`
	TS       int64  `json:"ts"`
	Data     any    `json:"data,omitempty"`
}

// Batch 是一次 IPC 推送的事件集合。
type Batch struct {
	V        int         `json:"v"`
	ThreadID string      `json:"threadId,omitempty"`
	Events   []*Envelope `json:"events"`
}

// TextDelta 是文本或推理增量的载荷。
type TextDelta struct {
	PartID string `json:"partId"`
	Text   string `json:"text"`
}

// TerminalDelta 是终端输出增量。
type TerminalDelta struct {
	SessionID string `json:"sessionId"`
	Data      string `json:"data"`
}

// Emitter 由调用方提供，通常是 Wails 的 app.Event.Emit 包装。
type Emitter func(name string, data any)

// Bus 按线程缓冲事件，并以固定节拍合并推送。
type Bus struct {
	emit  Emitter
	clock time.Duration

	mu      sync.Mutex
	buffers map[string]*threadBuffer
	closed  bool
	done    chan struct{}
}

type threadBuffer struct {
	seq    int64
	events []*Envelope
}

// NewBus 创建事件总线并启动后台冲刷协程。
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

// Publish 写入一个事件。Data 中的增量载荷需要是 TextDelta。
func (b *Bus) Publish(eventType, threadID, runID string, data any) {
	b.mu.Lock()
	if b.closed {
		b.mu.Unlock()
		return
	}
	buffer := b.buffers[threadID]
	if buffer == nil {
		buffer = &threadBuffer{}
		b.buffers[threadID] = buffer
	}

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
		pending = buffer.events
		buffer.events = nil
	} else if len(buffer.events) >= 32 {
		pending = buffer.events
		buffer.events = nil
	}
	b.mu.Unlock()

	if len(pending) > 0 {
		b.emitBatch(threadID, pending)
	}
}

// Flush 立即推送指定线程的缓冲。
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

// FlushAll 推送所有线程的缓冲。
func (b *Bus) FlushAll() {
	b.mu.Lock()
	type item struct {
		threadID string
		events   []*Envelope
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

// Close 停止后台协程并冲刷剩余事件。
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

func (b *Bus) emitBatch(threadID string, list []*Envelope) {
	if len(list) == 0 || b.emit == nil {
		return
	}
	b.emit(BatchName, Batch{V: 1, ThreadID: threadID, Events: list})
}

func nowMillis() int64 {
	return time.Now().UnixMilli()
}
