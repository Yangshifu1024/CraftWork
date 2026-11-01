package store

import (
	"database/sql"
	"path/filepath"
	"testing"
)

func TestThreadAndSnapshot(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	now := NowMillis()
	thread := &Thread{ID: "th_1", Mode: "code", Title: "测试线程", ModelID: "openai/gpt-5.1",
		Status: "idle", CreatedAt: now, UpdatedAt: now}
	if err := db.CreateThread(thread); err != nil {
		t.Fatal(err)
	}
	if err := db.RenameThread("th_1", "改名后"); err != nil {
		t.Fatal(err)
	}
	list, err := db.ListThreads("code", false)
	if err != nil || len(list) != 1 || list[0].Title != "改名后" {
		t.Fatalf("线程列表不符: %v %+v", err, list)
	}

	if _, err := db.AppendEvent(&EventRecord{
		ID: "p_user", ThreadID: "th_1", MessageID: "msg_user",
		Kind: "user_message", Payload: map[string]any{"text": "你好"},
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.AppendEvent(&EventRecord{
		ID: "p_text", ThreadID: "th_1", MessageID: "msg_assistant",
		Kind: "text", Payload: map[string]any{"text": "你好，有什么可以帮你？"},
	}); err != nil {
		t.Fatal(err)
	}
	if err := db.CreateRun(&Run{
		ID: "run_1", ThreadID: "th_1", MessageID: "msg_assistant",
		Status: "done", ModelID: "openai/gpt-5.1", StartedAt: now,
	}); err != nil {
		t.Fatal(err)
	}

	snapshot, err := db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	if len(snapshot.Messages) != 2 {
		t.Fatalf("期望 2 条消息，实际 %d", len(snapshot.Messages))
	}
	if snapshot.Messages[0].Role != "user" || snapshot.Messages[1].Role != "assistant" {
		t.Fatalf("消息角色不符: %+v", snapshot.Messages)
	}
	if snapshot.LastSeq != 2 {
		t.Fatalf("最后序号应为 2，实际 %d", snapshot.LastSeq)
	}
	if snapshot.Messages[1].Status != "complete" {
		t.Fatalf("消息状态应为 complete，实际 %s", snapshot.Messages[1].Status)
	}
}

func TestInterruptStaleRuns(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	now := NowMillis()
	thread := &Thread{ID: "th_stale", Mode: "code", Title: "中断", ModelID: "openai/gpt-5.1",
		Status: "running", CreatedAt: now, UpdatedAt: now}
	if err := db.CreateThread(thread); err != nil {
		t.Fatal(err)
	}
	if err := db.CreateRun(&Run{
		ID: "run_stale", ThreadID: "th_stale", MessageID: "msg_stale",
		Status: "running", ModelID: "openai/gpt-5.1", StartedAt: now,
	}); err != nil {
		t.Fatal(err)
	}
	if err := db.CreateApproval(&Approval{
		ID: "ap_stale", ThreadID: "th_stale", RunID: "run_stale", ToolCallID: "call_1",
		Tool: "fs_write", Risk: "R1", RequestedAt: now,
	}); err != nil {
		t.Fatal(err)
	}

	count, err := db.InterruptStaleRuns()
	if err != nil {
		t.Fatal(err)
	}
	if count != 1 {
		t.Fatalf("应中断 1 个运行，实际 %d", count)
	}

	recovered, err := db.GetThread("th_stale")
	if err != nil {
		t.Fatal(err)
	}
	if recovered.Status != "error" {
		t.Fatalf("线程状态应为 error，实际 %s", recovered.Status)
	}
	runs, err := db.threadRuns("th_stale")
	if err != nil || len(runs) != 1 {
		t.Fatalf("运行记录不符: %v %+v", err, runs)
	}
	if runs[0].Status != "interrupted" || runs[0].ErrorCode != "interrupted" {
		t.Fatalf("运行中断标记不符: %+v", runs[0])
	}
	approval, err := db.GetApproval("ap_stale")
	if err != nil || approval.Decision != "canceled" {
		t.Fatalf("挂起审批应被取消: %v %+v", err, approval)
	}

	snapshot, err := db.Snapshot("th_stale")
	if err != nil {
		t.Fatal(err)
	}
	foundError := false
	for _, message := range snapshot.Messages {
		for _, part := range message.Parts {
			if part.Kind != "error" {
				continue
			}
			data, _ := part.Data.(map[string]any)
			if data["code"] == "interrupted" {
				foundError = true
			}
		}
	}
	if !foundError {
		t.Fatal("应写入可重试的中断错误事件")
	}

	again, err := db.InterruptStaleRuns()
	if err != nil {
		t.Fatal(err)
	}
	if again != 0 {
		t.Fatalf("重复恢复不应再次中断: %d", again)
	}
}

func TestApprovalResolution(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	approval := &Approval{
		ID: "ap_1", ThreadID: "th_1", RunID: "run_1", ToolCallID: "call_1",
		Tool: "fs_write", Risk: "R1", RequestedAt: NowMillis(),
	}
	if err := db.CreateApproval(approval); err != nil {
		t.Fatal(err)
	}
	pending, err := db.PendingApprovals("th_1")
	if err != nil || len(pending) != 1 {
		t.Fatalf("应有 1 条待审批: %v %d", err, len(pending))
	}
	if err := db.ResolveApproval("ap_1", "allow", "session"); err != nil {
		t.Fatal(err)
	}
	if err := db.ResolveApproval("ap_1", "deny", "once"); err == nil {
		t.Fatal("重复处理审批应失败")
	}
	resolved, err := db.GetApproval("ap_1")
	if err != nil || resolved.Decision != "allow" || resolved.Scope != "session" {
		t.Fatalf("审批结果不符: %v %+v", err, resolved)
	}
}

func TestSettingsAndUsage(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	if err := db.SetSetting("app", map[string]any{"theme": "dark"}); err != nil {
		t.Fatal(err)
	}
	var settings map[string]any
	if err := db.GetSetting("app", &settings); err != nil || settings["theme"] != "dark" {
		t.Fatalf("设置读写失败: %v %+v", err, settings)
	}

	if err := db.AddUsage(&UsageRow{ModelID: "openai/gpt-5.1", InputTokens: 100, OutputTokens: 20}); err != nil {
		t.Fatal(err)
	}
	summary, err := db.UsageSummary()
	if err != nil {
		t.Fatal(err)
	}
	if summary.Today.InputTokens != 100 || summary.Today.OutputTokens != 20 {
		t.Fatalf("用量聚合不符: %+v", summary.Today)
	}
}

func TestAutoTitleThreadOnlyOnce(t *testing.T) {
	db, err := Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	thread := &Thread{
		ID: "th_title", Mode: "code", Title: "新对话", TitleAuto: true,
		ModelID: "openai/gpt-5.1", Status: "idle",
		CreatedAt: NowMillis(), UpdatedAt: NowMillis(),
	}
	if err := db.CreateThread(thread); err != nil {
		t.Fatal(err)
	}
	changed, err := db.AutoTitleThread(thread.ID, "修复登录刷新超时", "新对话")
	if err != nil || !changed {
		t.Fatalf("首次自动命名应成功: changed=%v err=%v", changed, err)
	}
	stored, err := db.GetThread(thread.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.Title != "修复登录刷新超时" || stored.TitleAuto {
		t.Fatalf("自动命名结果不符: %+v", stored)
	}

	// 同一线程再次自动命名不应覆盖。
	if changed, err := db.AutoTitleThread(thread.ID, "第二个标题", "新对话"); err != nil || changed {
		t.Fatalf("重复自动命名应被忽略: changed=%v err=%v", changed, err)
	}

	// 用户手动改名后，即使把 title_auto 恢复也不能被自动覆盖。
	if err := db.RenameThread(thread.ID, "我的标题"); err != nil {
		t.Fatal(err)
	}
	if changed, err := db.AutoTitleThread(thread.ID, "自动标题", "我的标题"); err != nil || changed {
		t.Fatalf("手动改名后不应自动覆盖: changed=%v err=%v", changed, err)
	}
}

func TestMigrateV1AddsTitleAuto(t *testing.T) {
	path := filepath.Join(t.TempDir(), "legacy.db")
	legacy, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := legacy.Exec(schemaV1); err != nil {
		t.Fatal(err)
	}
	if _, err := legacy.Exec(
		`INSERT INTO threads (id, mode, title, model_id, workspace_id, status, archived, tokens, created_at, updated_at)
		 VALUES ('th_legacy', 'code', '新对话', 'openai/gpt-5.1', '', 'idle', 0, 0, 1, 1)`,
	); err != nil {
		t.Fatal(err)
	}
	if _, err := legacy.Exec("PRAGMA user_version = 1"); err != nil {
		t.Fatal(err)
	}
	if err := legacy.Close(); err != nil {
		t.Fatal(err)
	}

	db, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var count int
	if err := db.sqlDB.QueryRow(
		"SELECT COUNT(*) FROM pragma_table_info('threads') WHERE name = 'title_auto'",
	).Scan(&count); err != nil {
		t.Fatal(err)
	}
	if count != 1 {
		t.Fatal("v1 数据库迁移后应存在 title_auto 列")
	}
	legacyThread, err := db.GetThread("th_legacy")
	if err != nil {
		t.Fatal(err)
	}
	if !legacyThread.TitleAuto {
		t.Fatal("旧的“新对话”线程应允许首轮自动命名")
	}
}
