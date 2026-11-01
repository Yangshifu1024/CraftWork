// 本文件负责事件表的读写：为线程内事件分配单调递增的序号，把事件重建为
// 前端直接可用的消息快照，并维护运行记录（runs）的生命周期。
//
// 事件表是对话的唯一事实源：消息与部件都是从事件行重建出来的，而不是另外存一份。

package store

import (
	"database/sql"
	"fmt"

	"craftwork/internal/id"
)

// AppendEvent 在事务内分配线程内单调递增的 seq 并写入事件。
//
// 序号分配与插入必须在同一个事务里完成，否则并发写入会拿到相同的 seq。
// 事务同时更新线程的最近活动时间，让侧边栏排序反映新消息。
func (db *DB) AppendEvent(event *EventRecord) (int64, error) {
	if event.CreatedAt == 0 {
		event.CreatedAt = nowMillis()
	}

	// writeMu 把所有写入串行化，减少 SQLite 的锁竞争与 busy 重试。
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	tx, err := db.sqlDB.Begin()
	if err != nil {
		return 0, fmt.Errorf("开始写入事件事务: %w", err)
	}
	defer tx.Rollback()

	seq, err := appendEventTx(tx, *event)
	if err != nil {
		return 0, err
	}
	if _, err := tx.Exec(`UPDATE threads SET updated_at = ? WHERE id = ?`, event.CreatedAt, event.ThreadID); err != nil {
		return 0, fmt.Errorf("更新线程时间: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return 0, fmt.Errorf("提交事件: %w", err)
	}
	return seq, nil
}

// appendEventTx 是事件写入的唯一实现：分配 seq 并插入 events。
//
// 调用方必须已经持有 writeMu 并处于事务中；这样批量写入（例如启动时中断旧运行）
// 也能复用同一套序号规则。
func appendEventTx(tx *sql.Tx, event EventRecord) (int64, error) {
	payload, err := MarshalPayload(event.Payload)
	if err != nil {
		return 0, err
	}

	// 取当前线程最大序号加一。事务内的读写在 SQLite 中是串行的，
	// 因此不会出现两个事务拿到同一个 seq 的情况。
	var seq int64
	if err := tx.QueryRow(
		`SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE thread_id = ?`, event.ThreadID).Scan(&seq); err != nil {
		return 0, fmt.Errorf("分配事件序号: %w", err)
	}
	if _, err := tx.Exec(
		`INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
		event.ID, event.ThreadID, event.MessageID, event.RunID, seq, event.Kind, payload, event.CreatedAt); err != nil {
		return 0, fmt.Errorf("写入事件: %w", err)
	}
	return seq, nil
}

// Snapshot 读取线程、消息、待审批与最后事件序号。
//
// 重建过程只有一次顺序扫描：按 seq 升序读事件，用 message_id 归组，把每个
// 事件转成消息里的一个部件；审批决定事件不单独成为部件，而是回填到对应卡片上。
// 运行状态最后统一覆盖，保证“流式中 / 出错 / 中断”能反映到消息上。
func (db *DB) Snapshot(threadID string) (*Snapshot, error) {
	thread, err := db.GetThread(threadID)
	if err != nil {
		return nil, err
	}

	rows, err := db.sqlDB.Query(
		`SELECT id, message_id, seq, kind, payload, created_at
		 FROM events WHERE thread_id = ? ORDER BY seq`, threadID)
	if err != nil {
		return nil, fmt.Errorf("读取线程事件: %w", err)
	}

	// messageIndex 按消息 ID 聚合部件；order 记录消息首次出现的顺序。
	messageIndex := make(map[string]*Message)
	approvalIndex := make(map[string]approvalLocation)
	var order []string
	var lastSeq int64
	for rows.Next() {
		var id, messageID, kind, payload string
		var seq, createdAt int64
		if err := rows.Scan(&id, &messageID, &seq, &kind, &payload, &createdAt); err != nil {
			rows.Close()
			return nil, fmt.Errorf("扫描事件: %w", err)
		}
		if seq > lastSeq {
			lastSeq = seq
		}
		// 运行生命周期事件只驱动状态，不产生消息部件。
		if kind == "run_started" || kind == "run_finished" {
			continue
		}

		data, err := UnmarshalPayload(payload)
		if err != nil {
			rows.Close()
			return nil, err
		}

		// 审批决定是独立事件，但界面上属于同一个审批卡片。
		if kind == "approval_resolved" {
			if object, ok := data.(map[string]any); ok {
				if partID, ok := object["partId"].(string); ok {
					if location, ok := approvalIndex[partID]; ok && location.index < len(location.message.Parts) {
						if partData, ok := location.message.Parts[location.index].Data.(map[string]any); ok {
							partData["status"] = object["status"]
							partData["decision"] = object["decision"]
							partData["scope"] = object["scope"]
						}
					}
				}
			}
			continue
		}

		// 消息第一次出现时创建，角色由事件类型决定：用户消息与助手回复分开。
		message := messageIndex[messageID]
		if message == nil {
			role := "assistant"
			if kind == "user_message" {
				role = "user"
			}
			message = &Message{
				ID:        messageID,
				ThreadID:  threadID,
				Role:      role,
				Status:    "complete",
				CreatedAt: createdAt,
			}
			messageIndex[messageID] = message
			order = append(order, messageID)
		}

		partKind := snapshotPartKind(kind)
		if partKind == "" {
			continue
		}
		// 用户消息事件里可能带有额外字段，只保留前端需要的文本与附件。
		if kind == "user_message" {
			if object, ok := data.(map[string]any); ok {
				preserved := map[string]any{"text": object["text"]}
				if attachments, ok := object["attachments"]; ok {
					preserved["attachments"] = attachments
				}
				data = preserved
			}
		}
		part := Part{ID: id, Kind: partKind, Seq: seq, Data: data}
		message.Parts = append(message.Parts, part)
		// 记录审批卡片在消息中的位置，供上面的决定事件回填。
		if partKind == "approval" {
			approvalIndex[id] = approvalLocation{message: message, index: len(message.Parts) - 1}
		}
	}
	if err := rows.Err(); err != nil {
		rows.Close()
		return nil, fmt.Errorf("遍历事件: %w", err)
	}
	rows.Close()

	// 运行记录是状态的事实源：即使没有对应对应的事件，也要把消息状态补上。
	runs, err := db.threadRuns(threadID)
	if err != nil {
		return nil, err
	}
	for _, run := range runs {
		message := messageIndex[run.MessageID]
		if message == nil {
			message = &Message{
				ID:        run.MessageID,
				ThreadID:  threadID,
				Role:      "assistant",
				CreatedAt: run.StartedAt,
			}
			messageIndex[run.MessageID] = message
			order = append(order, run.MessageID)
		}
		// 线程未完成的运行要显示为流式中，异常结束则显示为错误或不完整。
		switch run.Status {
		case "running":
			message.Status = "streaming"
		case "error":
			message.Status = "error"
		case "canceled", "interrupted":
			message.Status = "incomplete"
		default:
			message.Status = "complete"
		}
	}

	messages := make([]Message, 0, len(order))
	for _, messageID := range order {
		messages = append(messages, *messageIndex[messageID])
	}

	approvals, err := db.PendingApprovals(threadID)
	if err != nil {
		return nil, err
	}

	return &Snapshot{
		Thread:    *thread,
		Messages:  messages,
		LastSeq:   lastSeq,
		Approvals: approvals,
		OldestSeq: firstPartSeq(messages),
	}, nil
}

// firstPartSeq 返回第一条有部件消息中的最小序号。
func firstPartSeq(messages []Message) int64 {
	for _, message := range messages {
		if len(message.Parts) == 0 {
			continue
		}
		smallest := message.Parts[0].Seq
		for _, part := range message.Parts[1:] {
			if part.Seq < smallest {
				smallest = part.Seq
			}
		}
		return smallest
	}
	return 0
}

// approvalLocation 指向某条消息里审批部件的位置，用于回填审批决定。
type approvalLocation struct {
	// message 是审批卡片所属的消息。
	message *Message
	// index 是审批部件在 message.Parts 中的下标。
	index int
}

// snapshotPartKind 把事件类型映射成消息部件类型；返回空字符串表示不产生部件。
func snapshotPartKind(kind string) string {
	switch kind {
	// 用户消息事件在界面上表现为一个文本部件。
	case "user_message":
		return "text"
	// 这些事件与部件类型同名，直接复用。
	case "text", "reasoning", "approval", "todo", "error", "notice", "draft", "callout":
		return kind
	// 工具调用事件统一渲染成工具部件。
	case "tool_call":
		return "tool"
	default:
		return ""
	}
}

// CreateRun 写入一次运行记录。
func (db *DB) CreateRun(run *Run) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`INSERT INTO runs (id, thread_id, message_id, status, model_id, started_at)
		 VALUES (?, ?, ?, ?, ?, ?)`,
		run.ID, run.ThreadID, run.MessageID, run.Status, run.ModelID, run.StartedAt); err != nil {
		return fmt.Errorf("创建运行记录: %w", err)
	}
	return nil
}

// FinishRun 结束一次运行记录。
func (db *DB) FinishRun(id, status, code, message string) error {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()
	if _, err := db.sqlDB.Exec(
		`UPDATE runs SET status = ?, finished_at = ?, error_code = ?, error_message = ? WHERE id = ?`,
		status, nowMillis(), nullString(code), nullString(message), id); err != nil {
		return fmt.Errorf("结束运行记录: %w", err)
	}
	return nil
}

// InterruptStaleRuns 在启动时把未完成的运行与线程标记为中断。
//
// 返回被中断的运行数量。除了更新 runs / threads / approvals，还会为每个
// 中断的 run 写入一条 error 事件，让界面显示可重试的错误卡片。
func (db *DB) InterruptStaleRuns() (int, error) {
	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	tx, err := db.sqlDB.Begin()
	if err != nil {
		return 0, fmt.Errorf("开始中断事务: %w", err)
	}
	defer tx.Rollback()

	rows, err := tx.Query(`SELECT id, thread_id, message_id FROM runs WHERE status = 'running'`)
	if err != nil {
		return 0, fmt.Errorf("读取未完成运行: %w", err)
	}
	// staleRun 是启动时发现的、状态仍为 running 的历史运行。
	type staleRun struct {
		// id、threadID、messageID 分别对应运行、线程与消息标识。
		id, threadID, messageID string
	}
	var stale []staleRun
	for rows.Next() {
		var run staleRun
		if err := rows.Scan(&run.id, &run.threadID, &run.messageID); err != nil {
			rows.Close()
			return 0, fmt.Errorf("扫描未完成运行: %w", err)
		}
		stale = append(stale, run)
	}
	if err := rows.Err(); err != nil {
		rows.Close()
		return 0, fmt.Errorf("遍历未完成运行: %w", err)
	}
	rows.Close()

	now := nowMillis()
	message := "应用退出导致运行中断，已保留已产生的消息，可以重试。"
	// 先把所有未完成的运行统一标记为 interrupted，避免下次启动重复处理。
	if _, err := tx.Exec(
		`UPDATE runs SET status = 'interrupted', finished_at = ?,
		        error_code = 'interrupted', error_message = ?
		 WHERE status = 'running'`, now, message); err != nil {
		return 0, fmt.Errorf("中断旧运行: %w", err)
	}
	// 挂起的审批已经没有前端可以回答，统一按取消写入数据库。
	if _, err := tx.Exec(
		`UPDATE approvals SET decision = 'canceled', decided_at = ?
		 WHERE decision IS NULL`, now); err != nil {
		return 0, fmt.Errorf("取消挂起审批: %w", err)
	}
	// 线程状态先统一恢复为空闲，随后再按每个中断的运行标记为错误。
	if _, err := tx.Exec(
		`UPDATE threads SET status = 'idle', updated_at = ?
		 WHERE status IN ('running', 'awaiting')`, now); err != nil {
		return 0, fmt.Errorf("重置线程状态: %w", err)
	}

	// 为每个被中断的运行写一条 error 事件，界面据此显示可重试的错误卡片。
	for _, run := range stale {
		if _, err := tx.Exec(
			`UPDATE threads SET status = 'error', updated_at = ? WHERE id = ?`,
			now, run.threadID); err != nil {
			return 0, fmt.Errorf("标记中断线程: %w", err)
		}
		partID := id.New("part")
		if _, err := appendEventTx(tx, EventRecord{
			ID:        id.New("ev"),
			ThreadID:  run.threadID,
			MessageID: run.messageID,
			RunID:     run.id,
			Kind:      "error",
			CreatedAt: now,
			Payload: map[string]any{
				"partId":    partID,
				"code":      "interrupted",
				"message":   message,
				"retryable": true,
			},
		}); err != nil {
			return 0, fmt.Errorf("写入中断事件: %w", err)
		}
	}

	if err := tx.Commit(); err != nil {
		return 0, fmt.Errorf("提交中断事务: %w", err)
	}
	return len(stale), nil
}

// threadRuns 按开始时间读取线程的全部运行记录。
//
// 空值字段用 COALESCE 转成零值，扫描时就不需要额外的可空类型。
func (db *DB) threadRuns(threadID string) ([]Run, error) {
	rows, err := db.sqlDB.Query(
		`SELECT id, thread_id, message_id, status, model_id, started_at,
		        COALESCE(finished_at, 0), COALESCE(error_code, ''), COALESCE(error_message, '')
		 FROM runs WHERE thread_id = ? ORDER BY started_at`, threadID)
	if err != nil {
		return nil, fmt.Errorf("读取运行记录: %w", err)
	}
	defer rows.Close()

	var result []Run
	for rows.Next() {
		var run Run
		if err := rows.Scan(&run.ID, &run.ThreadID, &run.MessageID, &run.Status, &run.ModelID,
			&run.StartedAt, &run.FinishedAt, &run.ErrorCode, &run.ErrorMessage); err != nil {
			return nil, fmt.Errorf("扫描运行记录: %w", err)
		}
		result = append(result, run)
	}
	return result, rows.Err()
}

// nullString 把空字符串转成 SQL NULL，避免可空列里混入无意义的空串。
func nullString(value string) any {
	if value == "" {
		return nil
	}
	return value
}
