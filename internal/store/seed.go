// 本文件负责写入演示数据：在空数据库中创建演示工作区、两个演示线程及其
// 消息事件与一条审计记录，供本地演示与视觉走查使用。

package store

import (
	"fmt"
	"path/filepath"

	"craftwork/internal/id"
)

// SeedDemo 在空数据库中写入演示工作区与线程，供本地演示与视觉走查使用。
// 只有显式设置 CRAFTWORK_DEMO=1 时才会调用。
//
// 幂等性：如果已经存在代码模式线程，说明之前写过演示数据，直接返回，不会
// 重复创建。演示内容固定写在代码里，不读取磁盘上的真实文件。
func (db *DB) SeedDemo(workspacePath string) error {
	threads, err := db.ListThreads("code", true)
	if err != nil {
		return err
	}
	if len(threads) > 0 {
		// 已有代码线程时视为已初始化，避免重复写入。
		return nil
	}

	now := NowMillis()
	// 工作区名称取目录名，路径使用调用方传入的真实目录。
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

	// 第一个演示线程：代码模式，展示推理、正文、工具调用与审批提示。
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

	// 第二个演示线程：工作模式，展示表格与文档写入需要审批的说明。
	return db.seedThread(workspace, "work", "整理季度会议纪要", "local-demo", now-42*60*1000, []EventRecord{
		{Kind: "user_message", Payload: map[string]any{"text": "把 Q3 会议纪要里的待办整理成表格，并修正人名笔误。"}},
		{Kind: "text", Payload: map[string]any{"text": "已读取文档并整理出 4 条待办：\n\n| 负责人 | 事项 | 截止 |\n| --- | --- | --- |\n| 林岚 | 补充预算说明 | 10/24 |\n| 周野 | 对齐发布节奏 | 10/26 |\n| 陈默 | 更新接口文档 | 10/28 |\n\n文档写入需要审批；批准前不会修改原文件。"}},
	})
}

// seedThread 写入一个演示线程及其运行、消息事件与审计记录。
//
// 事件时间从 startedAt 起每条递增 120 毫秒，保证顺序稳定；用户消息会分配
// 独立的消息标识，其余部件归到助手消息下；最后补一条运行结束事件。
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

	// 逐条写入事件，时间戳按 120 毫秒递增，模拟真实的先后顺序。
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
			// 用户消息属于独立消息，需要单独的消息标识。
			event.MessageID = id.New("msg")
		}
		if _, err := db.AppendEvent(&event); err != nil {
			return fmt.Errorf("写入演示事件: %w", err)
		}
	}
	// 补一条运行结束事件，让演示线程在界面里显示为已完成。
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

	// 演示审计记录：忽略写入错误，审计失败不阻塞演示数据初始化。
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
