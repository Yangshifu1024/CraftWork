package store

import (
	"fmt"
	"path/filepath"

	"craftwork/internal/id"
)

// SeedDemo 在空数据库中写入演示工作区与线程，供本地演示与视觉走查使用。
// 只有显式设置 CRAFTWORK_DEMO=1 时才会调用。
func (db *DB) SeedDemo(workspacePath string) error {
	threads, err := db.ListThreads("code", true)
	if err != nil {
		return err
	}
	if len(threads) > 0 {
		return nil
	}

	now := NowMillis()
	name := filepath.Base(workspacePath)
	workspace := &Workspace{
		ID:           id.New("ws"),
		Name:         name,
		Path:         workspacePath,
		Trust:        "trusted",
		CreatedAt:    now,
		LastOpenedAt: now,
	}
	if err := db.UpsertWorkspace(workspace); err != nil {
		return err
	}

	if err := db.seedThread(workspace, "code", "修复登录刷新超时", "local-demo", now-8*60*1000, []EventRecord{
		{Kind: "user_message", Payload: map[string]any{"text": "登录后 15 分钟刷新令牌就超时了，帮我看看 internal/auth/session.go 里的刷新逻辑。"}},
		{Kind: "reasoning", Payload: map[string]any{"text": "先确认 session.go 的刷新判断，再看测试覆盖。"}},
		{Kind: "text", Payload: map[string]any{"text": "我先读取 `internal/auth/session.go`，确认刷新窗口的计算方式。\n\n关键代码：\n\n```go\nif time.Since(session.RefreshedAt) > refreshWindow {\n    return ErrSessionExpired\n}\n```\n\n问题在于 `refreshWindow` 固定为 10 分钟，而令牌实际有效期是 15 分钟。"}},
		{Kind: "tool_call", Payload: map[string]any{
			"toolCallId": "call_demo_read",
			"name":       "fs_read",
			"args":       map[string]any{"path": "internal/auth/session.go"},
			"risk":       "R0",
			"status":     "done",
			"output":     "internal/auth/session.go 共 128 行，已读取。",
			"isError":    false,
			"durationMs": 42,
			"summary":    "internal/auth/session.go",
		}},
		{Kind: "text", Payload: map[string]any{"text": "建议把刷新窗口改为令牌有效期的 80%，并在刷新失败时回退到重新登录。改动集中在 `internal/auth/session.go:64` 与对应单测。"}},
	}); err != nil {
		return err
	}

	return db.seedThread(workspace, "work", "整理季度会议纪要", "local-demo", now-42*60*1000, []EventRecord{
		{Kind: "user_message", Payload: map[string]any{"text": "把 Q3 会议纪要里的待办整理成表格，并修正人名笔误。"}},
		{Kind: "text", Payload: map[string]any{"text": "已读取文档并整理出 4 条待办：\n\n| 负责人 | 事项 | 截止 |\n| --- | --- | --- |\n| 林岚 | 补充预算说明 | 10/24 |\n| 周野 | 对齐发布节奏 | 10/26 |\n| 陈默 | 更新接口文档 | 10/28 |\n\n文档写入需要审批；批准前不会修改原文件。"}},
	})
}

func (db *DB) seedThread(workspace *Workspace, mode, title, modelID string, startedAt int64, records []EventRecord) error {
	threadID := id.New("th")
	messageID := id.New("msg")
	runID := id.New("run")

	if err := db.CreateThread(&Thread{
		ID:          threadID,
		Mode:        mode,
		Title:       title,
		ModelID:     modelID,
		WorkspaceID: workspace.ID,
		Status:      "idle",
		CreatedAt:   startedAt,
		UpdatedAt:   startedAt,
	}); err != nil {
		return err
	}
	if err := db.CreateRun(&Run{
		ID:        runID,
		ThreadID:  threadID,
		MessageID: messageID,
		Status:    "done",
		ModelID:   modelID,
		StartedAt: startedAt,
	}); err != nil {
		return err
	}

	offset := int64(0)
	for _, record := range records {
		offset += 120
		event := record
		event.ID = id.New("part")
		event.ThreadID = threadID
		event.MessageID = messageID
		event.RunID = runID
		event.CreatedAt = startedAt + offset
		if event.Kind == "user_message" {
			event.MessageID = id.New("msg")
		}
		if _, err := db.AppendEvent(&event); err != nil {
			return fmt.Errorf("写入演示事件: %w", err)
		}
	}
	if _, err := db.AppendEvent(&EventRecord{
		ID:        id.New("ev"),
		ThreadID:  threadID,
		MessageID: messageID,
		RunID:     runID,
		Kind:      "run_finished",
		Payload:   map[string]any{"status": "done"},
		CreatedAt: startedAt + offset + 120,
	}); err != nil {
		return err
	}

	_ = db.AddAudit(&AuditEntry{
		TS:       startedAt + offset,
		ThreadID: threadID,
		RunID:    runID,
		Tool:     "fs_read",
		Risk:     "R0",
		Decision: "auto",
		Detail:   "读取 internal/auth/session.go",
	})
	return nil
}
