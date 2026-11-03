package events

import (
	"testing"
	"time"
)

// newTestBus 创建把收到的批次送进通道的总线，便于测试断言派发结果。
func newTestBus() (*Bus, chan Batch) {
	received := make(chan Batch, 16)
	bus := NewBus(func(batch Batch) {
		received <- batch
	})
	return bus, received
}

func TestPublishPassesThroughImmediately(t *testing.T) {
	bus, received := newTestBus()
	defer bus.Close()

	bus.Publish(TypeToolStarted, "th_1", "run_1", map[string]any{"name": "fs_read"})
	select {
	case batch := <-received:
		if len(batch.Events) != 1 {
			t.Fatalf("透传应为单事件批次，实际 %d 条", len(batch.Events))
		}
		event := batch.Events[0]
		if event.Type != TypeToolStarted || event.Seq != 1 || event.ThreadID != "th_1" || event.RunID != "run_1" {
			t.Fatalf("事件信封不符: %+v", event)
		}
		if event.TS == 0 {
			t.Fatal("事件应携带时间戳")
		}
		if batch.V != 1 || batch.ThreadID != "th_1" {
			t.Fatalf("批次信封不符: v=%d threadId=%s", batch.V, batch.ThreadID)
		}
	case <-time.After(time.Second):
		t.Fatal("事件应立即派发，不进缓冲")
	}
}

func TestDeltasAreNotMerged(t *testing.T) {
	bus, received := newTestBus()
	defer bus.Close()

	// 同一部件的连续增量不再合并：每条增量立即派发，由界面层接成整段。
	bus.Publish(TypeTextDelta, "th_1", "run_1", TextDelta{PartID: "p1", Text: "你"})
	bus.Publish(TypeTextDelta, "th_1", "run_1", TextDelta{PartID: "p1", Text: "好"})

	for i, want := range []string{"你", "好"} {
		select {
		case batch := <-received:
			if len(batch.Events) != 1 {
				t.Fatalf("第 %d 批应为单事件，实际 %d 条", i+1, len(batch.Events))
			}
			delta, ok := batch.Events[0].Data.(TextDelta)
			if !ok || delta.Text != want {
				t.Fatalf("第 %d 条增量不符: %+v", i+1, batch.Events[0].Data)
			}
			if batch.Events[0].Seq != int64(i+1) {
				t.Fatalf("序号应单调递增，实际 %d", batch.Events[0].Seq)
			}
		case <-time.After(time.Second):
			t.Fatalf("第 %d 条增量未派发", i+1)
		}
	}
}

func TestSeqIsMonotonicPerThread(t *testing.T) {
	bus, received := newTestBus()
	defer bus.Close()

	bus.Publish(TypeNotice, "th_1", "", nil)
	bus.Publish(TypeNotice, "th_2", "", nil)
	bus.Publish(TypeNotice, "th_1", "", nil)

	seqOf := map[string]int64{}
	for range 3 {
		batch := <-received
		seqOf[batch.Events[0].ThreadID] = batch.Events[0].Seq
	}
	if seqOf["th_1"] != 2 || seqOf["th_2"] != 1 {
		t.Fatalf("序号应按会话独立单调: %v", seqOf)
	}
}

func TestCloseDropsFurtherEvents(t *testing.T) {
	bus, received := newTestBus()
	bus.Publish(TypeNotice, "th_1", "", nil)
	<-received

	bus.Close()
	bus.Publish(TypeNotice, "th_1", "", nil)
	select {
	case batch := <-received:
		t.Fatalf("关闭后的事件不应派发: %+v", batch)
	default:
	}
}
