package model

import (
	"context"
	"strings"
	"testing"
)

type fakeSSEDecoder struct {
	payloads []string
	finishes int
}

func (d *fakeSSEDecoder) decode(payload []byte, emit emitFunc) (bool, error) {
	d.payloads = append(d.payloads, string(payload))
	return false, nil
}

func (d *fakeSSEDecoder) finish(emit emitFunc) {
	d.finishes++
	emit(Chunk{Text: "done"})
}

// collectSSE 驱动共享流核心，便于在不启动 HTTP 服务的情况下断言分帧行为。
func collectSSE(t *testing.T, body string, decoder sseDecoder) []Chunk {
	t.Helper()
	channel := make(chan Chunk, 32)
	done := make(chan struct{})
	go func() {
		defer close(done)
		consumeSSE(context.Background(), strings.NewReader(body), channel, decoder)
	}()
	<-done
	close(channel)

	var chunks []Chunk
	for chunk := range channel {
		chunks = append(chunks, chunk)
	}
	return chunks
}

func TestConsumeSSESkipsNonDataLinesAndFinishesOnce(t *testing.T) {
	decoder := &fakeSSEDecoder{}
	chunks := collectSSE(t, `event: message
data: {"a":1}

: keep-alive
data: {"b":2}
data: [DONE]
data: {"after":"done"}
`, decoder)

	if len(decoder.payloads) != 2 {
		t.Fatalf("期望 2 条载荷，实际 %d：%v", len(decoder.payloads), decoder.payloads)
	}
	if decoder.payloads[0] != `{"a":1}` || decoder.payloads[1] != `{"b":2}` {
		t.Fatalf("载荷解析错误：%v", decoder.payloads)
	}
	if decoder.finishes != 1 {
		t.Fatalf("finish 应只调用一次，实际 %d", decoder.finishes)
	}
	if len(chunks) != 1 || chunks[0].Text != "done" {
		t.Fatalf("结束片段错误：%+v", chunks)
	}
}

func TestToolCallBuilderMergesPartialArgumentsInIndexOrder(t *testing.T) {
	builder := newToolCallBuilder()
	builder.add(1, "call_b", "fs_read", `{"path":`)
	builder.add(1, "", "", `"a.txt"}`)
	builder.add(0, "", "todo_write", `{"items":[]}`)

	var calls []ToolCall
	builder.flush(func(chunk Chunk) bool {
		if chunk.ToolCall != nil {
			calls = append(calls, *chunk.ToolCall)
		}
		return true
	})

	if len(calls) != 2 {
		t.Fatalf("期望 2 个工具调用，实际 %d", len(calls))
	}
	if calls[0].Name != "todo_write" || calls[0].ID == "" {
		t.Fatalf("索引 0 的调用错误：%+v", calls[0])
	}
	if calls[1].ID != "call_b" || calls[1].Name != "fs_read" {
		t.Fatalf("索引 1 的调用错误：%+v", calls[1])
	}
	if got := calls[1].Args["path"]; got != "a.txt" {
		t.Fatalf("参数未合并：%v", calls[1].Args)
	}
}
