// Package record 是线程事件写入与广播的唯一出口。
//
// 调用方不再分别调用 store.AppendEvent 与 events.Bus.Publish：
// 一次 Record 同时完成序号分配、落库与推送，part kind 到事件类型的
// 映射也只存在于这里。
package record

import (
	"log/slog"

	"craftwork/internal/events"
	"craftwork/internal/store"
)

// partEvents 是持久化部件 kind 与前端完成事件的对应关系。
//
// 只存在于这里的映射保证“写进历史”和“推给界面”不会各自漂移。
var partEvents = map[string]string{
	"text":      events.TypeTextCompleted,
	"reasoning": events.TypeReasoningCompleted,
	"tool_call": events.TypeToolCompleted,
	"approval":  events.TypeApprovalRequested,
	"todo":      events.TypeTodoUpdated,
	"error":     events.TypeError,
	"notice":    events.TypeNotice,
}

// Recorder 把事件落库与广播合并成一个动作。
type Recorder struct {
	db  *store.DB
	bus *events.Bus
}

// New 创建事件记录器。
func New(db *store.DB, bus *events.Bus) *Recorder {
	return &Recorder{db: db, bus: bus}
}

// Append 写入事件；部件 kind 会同时广播对应的完成事件。
func (r *Recorder) Append(event store.EventRecord) error {
	if _, err := r.db.AppendEvent(&event); err != nil {
		return err
	}
	if eventType, ok := partEvents[event.Kind]; ok {
		r.bus.Publish(eventType, event.ThreadID, event.RunID, event.Payload)
	}
	return nil
}

// AppendOrWarn 写入事件，失败只记日志。用于运行过程中不阻断主流程的部件。
func (r *Recorder) AppendOrWarn(event store.EventRecord) {
	if err := r.Append(event); err != nil {
		slog.Warn("写入事件失败", "kind", event.Kind, "err", err)
	}
}

// Emit 只广播，不落库。用于增量片段与生命周期事件。
func (r *Recorder) Emit(eventType, threadID, runID string, payload any) {
	r.bus.Publish(eventType, threadID, runID, payload)
}

// SetThreadStatus 更新线程状态并广播 thread.updated。
func (r *Recorder) SetThreadStatus(threadID, runID, status string) error {
	if err := r.db.SetThreadStatus(threadID, status); err != nil {
		return err
	}
	r.Emit(events.TypeThreadUpdated, threadID, runID, map[string]any{
		"threadId": threadID,
		"status":   status,
	})
	return nil
}

// Flush 把线程内合并中的增量事件立即推送。
func (r *Recorder) Flush(threadID string) {
	r.bus.Flush(threadID)
}
