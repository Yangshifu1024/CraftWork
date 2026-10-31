package events

import (
	"testing"
	"time"
)

func newTestBus() (*Bus, chan Batch) {
	received := make(chan Batch, 16)
	bus := &Bus{
		emit: func(name string, data any) {
			if batch, ok := data.(Batch); ok {
				received <- batch
			}
		},
		clock:   time.Hour,
		buffers: map[string]*threadBuffer{},
		done:    make(chan struct{}),
	}
	go bus.loop()
	return bus, received
}

func TestTextDeltasAreMerged(t *testing.T) {
	bus, received := newTestBus()
	defer bus.Close()

	bus.Publish(TypeTextDelta, "th_1", "run_1", TextDelta{PartID: "p1", Text: "你"})
	bus.Publish(TypeTextDelta, "th_1", "run_1", TextDelta{PartID: "p1", Text: "好"})
	bus.Flush("th_1")

	batch := <-received
	if len(batch.Events) != 1 {
		t.Fatalf("期望合并成 1 条事件，实际 %d", len(batch.Events))
	}
	delta, ok := batch.Events[0].Data.(TextDelta)
	if !ok || delta.Text != "你好" {
		t.Fatalf("合并结果不符: %+v", batch.Events[0].Data)
	}
}

func TestNonDeltaFlushesImmediately(t *testing.T) {
	bus, received := newTestBus()
	defer bus.Close()

	bus.Publish(TypeToolStarted, "th_1", "run_1", map[string]any{"name": "fs_read"})
	select {
	case batch := <-received:
		if len(batch.Events) != 1 || batch.Events[0].Type != TypeToolStarted {
			t.Fatalf("事件内容不符: %+v", batch.Events)
		}
	case <-time.After(time.Second):
		t.Fatal("非增量事件应立即推送")
	}
}

func TestTerminalDeltasAreMerged(t *testing.T) {
	bus, received := newTestBus()
	defer bus.Close()

	bus.Publish(TypeTerminalOutput, "terminal:t1", "", TerminalDelta{SessionID: "t1", Data: "a"})
	bus.Publish(TypeTerminalOutput, "terminal:t1", "", TerminalDelta{SessionID: "t1", Data: "b"})
	bus.Flush("terminal:t1")

	batch := <-received
	if len(batch.Events) != 1 {
		t.Fatalf("终端增量应合并，实际 %d 条", len(batch.Events))
	}
	delta, ok := batch.Events[0].Data.(TerminalDelta)
	if !ok || delta.Data != "ab" {
		t.Fatalf("终端合并结果不符: %+v", batch.Events[0].Data)
	}
}
