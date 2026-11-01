package service

import (
	"fmt"
	"path/filepath"
	"testing"

	"craftwork/internal/events"
	"craftwork/internal/record"
	"craftwork/internal/store"
)

func TestSnapshotPagePagination(t *testing.T) {
	db, err := store.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })

	thread := store.Thread{
		ID: "th_page", Mode: "code", Title: "分页", ModelID: "openai/gpt-5.1",
		Status: "idle", CreatedAt: store.NowMillis(), UpdatedAt: store.NowMillis(),
	}
	if err := db.CreateThread(&thread); err != nil {
		t.Fatal(err)
	}
	seqs := make([]int64, 0, 10)
	for index := 0; index < 10; index++ {
		seq, err := db.AppendEvent(&store.EventRecord{
			ID:        fmt.Sprintf("p%d", index),
			ThreadID:  thread.ID,
			MessageID: fmt.Sprintf("m%d", index),
			Kind:      "user_message",
			Payload:   map[string]any{"text": fmt.Sprintf("消息 %d", index)},
		})
		if err != nil {
			t.Fatal(err)
		}
		seqs = append(seqs, seq)
	}

	bus := events.NewBus(func(string, any) {})
	t.Cleanup(bus.Close)
	service := NewThreadService(db, record.New(db, bus))

	page, err := service.SnapshotPage(thread.ID, 4, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(page.Messages) != 4 || !page.HasMore {
		t.Fatalf("第一页不符: len=%d hasMore=%v", len(page.Messages), page.HasMore)
	}
	if page.OldestSeq != seqs[6] {
		t.Fatalf("第一页起始序号不符: got %d want %d", page.OldestSeq, seqs[6])
	}

	page2, err := service.SnapshotPage(thread.ID, 4, page.OldestSeq)
	if err != nil {
		t.Fatal(err)
	}
	if len(page2.Messages) != 4 || !page2.HasMore {
		t.Fatalf("第二页不符: len=%d hasMore=%v", len(page2.Messages), page2.HasMore)
	}
	if page2.OldestSeq != seqs[2] {
		t.Fatalf("第二页起始序号不符: got %d want %d", page2.OldestSeq, seqs[2])
	}

	page3, err := service.SnapshotPage(thread.ID, 4, page2.OldestSeq)
	if err != nil {
		t.Fatal(err)
	}
	if len(page3.Messages) != 2 || page3.HasMore {
		t.Fatalf("最后一页不符: len=%d hasMore=%v", len(page3.Messages), page3.HasMore)
	}
	if page3.OldestSeq != seqs[0] {
		t.Fatalf("最后一页起始序号不符: got %d want %d", page3.OldestSeq, seqs[0])
	}
}
