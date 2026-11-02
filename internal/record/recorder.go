// Package record 是会话事件写入与广播的唯一出口。
//
// 调用方不再分别调用 store.AppendEvent 与 events.Bus.Publish：
// 一次 Append 同时完成序号分配、写入数据库与推送，部件类型到事件类型的
// 映射也只存在于这里。本文件实现这个出口，并负责会话状态更新的
// 写入数据库与广播。
package record

import (
	"log/slog"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// partEvents 是持久化部件类型与前端完成事件的对应关系。
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

// Recorder 把事件写入数据库与广播合并成一个动作。
type Recorder struct {
	db  *store.DB   // 负责序号分配与持久化
	bus *events.Bus // 负责推送给前端
}

// New 创建事件记录器。
func New(db *store.DB, bus *events.Bus) *Recorder {
	return &Recorder{db: db, bus: bus}
}

// Append 写入事件；部件类型会同时广播对应的完成事件。
//
// 先写入数据库再广播：只有写入成功的部件才会出现在界面上，避免历史与实时
// 消息不一致。没有映射的部件类型（例如生命周期事件）只写入数据库、不广播。
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

// Emit 只广播，不写入数据库。用于增量片段与生命周期事件。
//
// 增量片段本身会被合并成完整的部件再写入数据库，因此这里不再重复写入。
func (r *Recorder) Emit(eventType, threadID, runID string, payload any) {
	r.bus.Publish(eventType, threadID, runID, payload)
}

// SetThreadStatus 更新会话状态并广播 thread.updated。
//
// 先更新数据库，成功后才广播，保证前端看到的状态与持久化一致。
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

// Flush 把会话内合并中的增量事件立即推送。
//
// 正常收尾时调用，避免最后的增量还停留在合并窗口里没有发出。
func (r *Recorder) Flush(threadID string) {
	r.bus.Flush(threadID)
}
