// 本文件负责所有提供商共享的流式传输骨架：建立 HTTP 请求、按 SSE
//（Server-Sent Events，服务器发送事件）逐行读取、统一错误归一，并在
// 流结束时调用解码器收尾。各提供商只需要实现自己的协议解码器。

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

	"github.com/yangshifu1024/craftwork/internal/id"
)

// maxSSELineBytes 是单条 SSE 行的上限（4 MB），避免异常超长的事件把内存撑爆。
const maxSSELineBytes = 4 * 1024 * 1024

// streamRequest 描述一次流式调用中与协议无关的传输部分。
//
// 请求体由 streamSSE 统一编码成 JSON；提供商只需要填写地址、鉴权头与
// 已经组装好的请求对象。
type streamRequest struct {
	provider string            // 提供商名称，用于拼接可读的错误信息
	client   *http.Client      // HTTP 客户端，便于复用连接池
	endpoint string            // 完整的请求地址
	headers  map[string]string // 提供商特有的请求头，例如鉴权与版本号
	body     any               // 会被 json.Marshal 编码的请求体
}

// emitFunc 交付一个片段；返回 false 表示调用方已经取消。
type emitFunc func(Chunk) bool

// sseDecoder 是各提供商唯一需要实现的协议适配器。
//
// decode 只负责把一条 data: 载荷翻译成片段；传输、分帧、错误归一与
// 流结束的统一收尾都由 streamSSE 承担。done 表示本次流已经结束，
// 随后会调用一次 finish。
type sseDecoder interface {
	// decode 处理一条 data: 载荷。done 为 true 表示协议层已经给出结束信号。
	decode(payload []byte, emit emitFunc) (done bool, err error)
	// finish 在流正常结束时调用一次，用于冲刷累积的工具调用并交付用量与结束标记；
	// 解码出错时不会调用。
	finish(emit emitFunc)
}

// streamSSE 是所有提供商的唯一流入口。
//
// 它负责建立 HTTP 请求、校验响应状态、在后台读取 SSE 行并把片段写入
// 返回的通道，同时保证通道在结束时关闭。正常的流结束会交给解码器收尾；
// 非 2xx 响应会被转换成带状态码与响应片段的结构化错误，网络错误则交给
// TransportError 归一。
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
		// 只读取有限长度的响应体用于错误提示，避免异常服务返回超大内容。
		snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 8192))
		resp.Body.Close()
		return nil, HTTPError(spec.provider, resp.StatusCode, snippet)
	}

	// 通道留出少量缓冲，读取与消费可以并行推进；消费方取消时 send 会返回 false。
	channel := make(chan Chunk, 16)
	go func() {
		defer close(channel)
		defer resp.Body.Close()
		consumeSSE(ctx, resp.Body, channel, decoder)
	}()
	return channel, nil
}

// consumeSSE 读取 data: 行并交给解码器，正常结束时统一收尾。
//
// SSE 还包含 event:、id:、retry: 与注释行，这里只关心 data: 载荷。
// "[DONE]" 是 OpenAI 兼容服务常用的结束哨兵；部分提供商不用哨兵，
// 而是靠解码器返回 done 或响应体读到 EOF 来判断结束。
// 解码出错时只交付一个错误片段，不再调用 finish。
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
	// 上下文被取消时不再补发结束片段，避免向已经放弃的调用方继续投递。
	if ctx.Err() != nil {
		return
	}
	decoder.finish(emit)
}

// send 把片段写入通道；调用方取消时立即返回 false。
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
// 解析失败时保留原文，放在 _raw 字段中，便于模型与用户看到实际收到的
// 内容，而不是静默丢弃参数。
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
//
// 两套协议都会把一次工具调用拆成多条增量：先给名称与 ID，再逐段给出
// JSON 参数字符串。index 是服务端用来区分同一条消息里多个工具调用的序号。
type toolCallBuilder struct {
	mu    sync.Mutex             // 保护 calls，避免并发读写
	calls map[int]*toolCallEntry // 键为服务端给出的调用序号
}

// toolCallEntry 是一条尚未交付的工具调用。
type toolCallEntry struct {
	id   string          // 服务端调用 ID；为空时在交付阶段生成本地 ID
	name string          // 工具名称，可能只在第一条增量中出现
	args strings.Builder // 分片到达的 JSON 参数字符串，需要原样拼接
}

// newToolCallBuilder 创建空的工具调用累积器。
func newToolCallBuilder() *toolCallBuilder {
	return &toolCallBuilder{calls: map[int]*toolCallEntry{}}
}

// add 追加一个工具调用的增量片段。
//
// 空字符串表示本次增量没有携带该字段，不会覆盖已经记录的值。
// 参数片段必须按到达顺序追加，因为服务端会把一段 JSON 任意切开。
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
//
// 先按序号排序，保证同一条消息里的工具调用顺序稳定；缺少 ID 的调用
// （例如部分兼容服务本身不返回调用 ID）会生成本地标识。构造完成后先释放锁，
// 再逐条交付，避免回调阻塞其它增量写入。
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
