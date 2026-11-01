package model

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strings"
	"sync"

	"craftwork/internal/id"
)

// maxSSELineBytes 是单条 SSE 行的上限，避免超长事件撑爆内存。
const maxSSELineBytes = 4 * 1024 * 1024

// streamRequest 描述一次流式调用中与协议无关的传输部分。
type streamRequest struct {
	provider string
	client   *http.Client
	endpoint string
	headers  map[string]string
	body     any
}

// emitFunc 交付一个片段；返回 false 表示调用方已经取消。
type emitFunc func(Chunk) bool

// sseDecoder 是各提供商唯一需要实现的协议适配器。
//
// decode 只负责把一条 data: 载荷翻译成片段；传输、分帧、错误归一与
// 流结束的统一收尾都由 streamSSE 承担。done 表示本次流已经结束，
// 随后会调用一次 finish。
type sseDecoder interface {
	decode(payload []byte, emit emitFunc) (done bool, err error)
	finish(emit emitFunc)
}

// streamSSE 是所有提供商的唯一流入口。
func streamSSE(ctx context.Context, spec streamRequest, decoder sseDecoder) (<-chan Chunk, error) {
	raw, err := json.Marshal(spec.body)
	if err != nil {
		return nil, fmt.Errorf("编码模型请求: %w", err)
	}
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, spec.endpoint, bytes.NewReader(raw))
	if err != nil {
		return nil, fmt.Errorf("创建模型请求: %w", err)
	}
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("Accept", "text/event-stream")
	for key, value := range spec.headers {
		httpReq.Header.Set(key, value)
	}

	resp, err := spec.client.Do(httpReq)
	if err != nil {
		return nil, TransportError(spec.provider, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 8192))
		resp.Body.Close()
		return nil, HTTPError(spec.provider, resp.StatusCode, snippet)
	}

	channel := make(chan Chunk, 16)
	go func() {
		defer close(channel)
		defer resp.Body.Close()
		consumeSSE(ctx, resp.Body, channel, decoder)
	}()
	return channel, nil
}

// consumeSSE 读取 data: 行并交给解码器，结束时统一收尾。
func consumeSSE(ctx context.Context, body io.Reader, channel chan<- Chunk, decoder sseDecoder) {
	emit := func(chunk Chunk) bool { return send(ctx, channel, chunk) }

	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 64*1024), maxSSELineBytes)

	for scanner.Scan() {
		if ctx.Err() != nil {
			return
		}
		line := strings.TrimSpace(scanner.Text())
		if !strings.HasPrefix(line, "data:") {
			continue
		}
		payload := strings.TrimSpace(strings.TrimPrefix(line, "data:"))
		if payload == "" {
			continue
		}
		if payload == "[DONE]" {
			decoder.finish(emit)
			return
		}
		done, err := decoder.decode([]byte(payload), emit)
		if err != nil {
			emit(Chunk{Err: err})
			return
		}
		if done {
			decoder.finish(emit)
			return
		}
	}
	if ctx.Err() != nil {
		return
	}
	decoder.finish(emit)
}

func send(ctx context.Context, channel chan<- Chunk, chunk Chunk) bool {
	select {
	case channel <- chunk:
		return true
	case <-ctx.Done():
		return false
	}
}

// newCallID 为没有服务端 ID 的工具调用生成本地标识。
func newCallID() string {
	return id.New("call")
}

// parseArguments 把累积的 JSON 参数解析成对象。
//
// 解析失败时保留原文，便于模型与用户看到实际收到的内容。
func parseArguments(raw string) map[string]any {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return map[string]any{}
	}
	var args map[string]any
	if err := json.Unmarshal([]byte(raw), &args); err != nil {
		return map[string]any{"_raw": raw}
	}
	return args
}

// toolCallBuilder 按 index 累积流式工具调用，是 OpenAI 与 Anthropic
// 两套增量协议共享的唯一实现。
type toolCallBuilder struct {
	mu    sync.Mutex
	calls map[int]*toolCallEntry
}

type toolCallEntry struct {
	id   string
	name string
	args strings.Builder
}

func newToolCallBuilder() *toolCallBuilder {
	return &toolCallBuilder{calls: map[int]*toolCallEntry{}}
}

// add 追加一个工具调用的增量片段。
func (b *toolCallBuilder) add(index int, id, name, args string) {
	b.mu.Lock()
	defer b.mu.Unlock()
	entry := b.calls[index]
	if entry == nil {
		entry = &toolCallEntry{}
		b.calls[index] = entry
	}
	if id != "" {
		entry.id = id
	}
	if name != "" {
		entry.name = name
	}
	if args != "" {
		entry.args.WriteString(args)
	}
}

// flush 按 index 升序交付已经完整的工具调用。
func (b *toolCallBuilder) flush(emit emitFunc) {
	b.mu.Lock()
	if len(b.calls) == 0 {
		b.mu.Unlock()
		return
	}
	indices := make([]int, 0, len(b.calls))
	for index := range b.calls {
		indices = append(indices, index)
	}
	sort.Ints(indices)
	calls := make([]ToolCall, 0, len(indices))
	for _, index := range indices {
		entry := b.calls[index]
		call := ToolCall{
			ID:   entry.id,
			Name: entry.name,
			Args: parseArguments(entry.args.String()),
		}
		if call.ID == "" {
			call.ID = newCallID()
		}
		calls = append(calls, call)
	}
	b.mu.Unlock()

	for index := range calls {
		if !emit(Chunk{ToolCall: &calls[index]}) {
			return
		}
	}
}
