package record

import (
	"path/filepath"
	"testing"

	"craftwork/internal/events"
	"craftwork/internal/store"
)

func newRecorder(t *testing.T) (*Recorder, *store.DB, *[]string) {
	t.Helper()
	db, err := store.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	var names []string
	bus := events.NewBus(func(_ string, data any) {
		batch, ok := data.(events.Batch)
		if !ok {
			return
		}
		for _, event := range batch.Events {
			names = append(names, event.Type)
		}
	})
	t.Cleanup(bus.Close)

	thread := store.Thread{
		ID: "th_1", Mode: "code", Title: "测试", ModelID: "fake",
		Status: "idle", CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := db.CreateThread(&thread); err != nil {
		t.Fatal(err)
	}
	return New(db, bus), db, &names
}

func TestAppendPersistsAndBroadcastsPartEvent(t *testing.T) {
	recorder, db, names := newRecorder(t)

	err := recorder.Append(store.EventRecord{
		ID: "part_1", ThreadID: "th_1", MessageID: "msg_1",
		Kind: "text", Payload: map[string]any{"text": "你好"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(*names) != 1 || (*names)[0] != events.TypeTextCompleted {
		t.Fatalf("应广播 text.completed，实际 %v", *names)
	}

	snapshot, err := db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	if len(snapshot.Messages) != 1 || len(snapshot.Messages[0].Parts) != 1 {
		t.Fatalf("事件未落库: %+v", snapshot.Messages)
	}
	if snapshot.Messages[0].Parts[0].Kind != "text" {
		t.Fatalf("部件类型不符: %+v", snapshot.Messages[0].Parts[0])
	}
}

func TestAppendUnmappedKindOnlyPersists(t *testing.T) {
	recorder, db, names := newRecorder(t)

	if err := recorder.Append(store.EventRecord{
		ID: "ev_1", ThreadID: "th_1", MessageID: "msg_1",
		Kind: "approval_resolved", Payload: map[string]any{"status": "approved"},
	}); err != nil {
		t.Fatal(err)
	}
	if len(*names) != 0 {
		t.Fatalf("非部件事件不应广播，实际 %v", *names)
	}
	snapshot, err := db.Snapshot("th_1")
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.LastSeq != 1 {
		t.Fatalf("事件应落库，lastSeq=%d", snapshot.LastSeq)
	}
}

func TestAppendFailureDoesNotBroadcast(t *testing.T) {
	recorder, _, names := newRecorder(t)
	event := store.EventRecord{
		ID: "part_1", ThreadID: "th_1", MessageID: "msg_1",
		Kind: "text", Payload: map[string]any{"text": "你好"},
	}
	if err := recorder.Append(event); err != nil {
		t.Fatal(err)
	}
	if err := recorder.Append(event); err == nil {
		t.Fatal("重复事件 ID 应写入失败")
	}
	if len(*names) != 1 {
		t.Fatalf("写入失败不应重复广播，实际 %v", *names)
	}
}

func TestSetThreadStatusUpdatesAndBroadcasts(t *testing.T) {
	recorder, db, names := newRecorder(t)

	if err := recorder.SetThreadStatus("th_1", "run_1", "running"); err != nil {
		t.Fatal(err)
	}
	thread, err := db.GetThread("th_1")
	if err != nil {
		t.Fatal(err)
	}
	if thread.Status != "running" {
		t.Fatalf("线程状态未更新: %s", thread.Status)
	}
	if len(*names) != 1 || (*names)[0] != events.TypeThreadUpdated {
		t.Fatalf("应广播 thread.updated，实际 %v", *names)
	}
}
