package store

import (
	"fmt"

	"craftwork/internal/id"
)

// AppendEvent 在事务内分配线程内单调递增的 seq 并写入事件。
func (db *DB) AppendEvent(event *EventRecord) (int64, error) {
	if event.CreatedAt == 0 {
		event.CreatedAt = nowMillis()
	}
	payload, err := MarshalPayload(event.Payload)
	if err != nil {
		return 0, err
	}

	db.writeMu.Lock()
	defer db.writeMu.Unlock()

	tx, err := db.sqlDB.Begin()
	if err != nil {
		return 0, fmt.Errorf("开始写入事件事务: %w", err)
	}
	defer tx.Rollback()

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
	if _, err := tx.Exec(`UPDATE threads SET updated_at = ? WHERE id = ?`, event.CreatedAt, event.ThreadID); err != nil {
		return 0, fmt.Errorf("更新线程时间: %w", err)
	}
	if err := tx.Commit(); err != nil {
		return 0, fmt.Errorf("提交事件: %w", err)
	}
	return seq, nil
}

// Snapshot 读取线程、消息、待审批与最后事件序号。
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
		if kind == "user_message" {
			if object, ok := data.(map[string]any); ok {
				data = map[string]any{"text": object["text"]}
			}
		}
		part := Part{ID: id, Kind: partKind, Seq: seq, Data: data}
		message.Parts = append(message.Parts, part)
		if partKind == "approval" {
			approvalIndex[id] = approvalLocation{message: message, index: len(message.Parts) - 1}
		}
	}
	if err := rows.Err(); err != nil {
		rows.Close()
		return nil, fmt.Errorf("遍历事件: %w", err)
	}
	rows.Close()

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

type approvalLocation struct {
	message *Message
	index   int
}

func snapshotPartKind(kind string) string {
	switch kind {
	case "user_message":
		return "text"
	case "text", "reasoning", "approval", "todo", "error", "notice", "draft", "callout":
		return kind
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
	type staleRun struct {
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
	if _, err := tx.Exec(
		`UPDATE runs SET status = 'interrupted', finished_at = ?,
		        error_code = 'interrupted', error_message = ?
		 WHERE status = 'running'`, now, message); err != nil {
		return 0, fmt.Errorf("中断旧运行: %w", err)
	}
	if _, err := tx.Exec(
		`UPDATE approvals SET decision = 'canceled', decided_at = ?
		 WHERE decision IS NULL`, now); err != nil {
		return 0, fmt.Errorf("取消挂起审批: %w", err)
	}
	if _, err := tx.Exec(
		`UPDATE threads SET status = 'idle', updated_at = ?
		 WHERE status IN ('running', 'awaiting')`, now); err != nil {
		return 0, fmt.Errorf("重置线程状态: %w", err)
	}

	for _, run := range stale {
		if _, err := tx.Exec(
			`UPDATE threads SET status = 'error', updated_at = ? WHERE id = ?`,
			now, run.threadID); err != nil {
			return 0, fmt.Errorf("标记中断线程: %w", err)
		}
		partID := id.New("part")
		payload, err := MarshalPayload(map[string]any{
			"partId":    partID,
			"code":      "interrupted",
			"message":   message,
			"retryable": true,
		})
		if err != nil {
			return 0, err
		}
		if _, err := tx.Exec(
			`INSERT INTO events (id, thread_id, message_id, run_id, seq, kind, payload, created_at)
			 VALUES (?, ?, ?, ?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE thread_id = ?),
			         'error', ?, ?)`,
			id.New("ev"), run.threadID, run.messageID, run.id, run.threadID, payload, now); err != nil {
			return 0, fmt.Errorf("写入中断事件: %w", err)
		}
	}

	if err := tx.Commit(); err != nil {
		return 0, fmt.Errorf("提交中断事务: %w", err)
	}
	return len(stale), nil
}

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

func nullString(value string) any {
	if value == "" {
		return nil
	}
	return value
}
