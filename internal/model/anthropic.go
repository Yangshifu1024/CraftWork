// 本文件负责 Anthropic Messages API 客户端。核心工作是把内部消息
// 翻译成 user / assistant 交替的内容块（tools 用 tool_use 与 tool_result），
// 并把 Messages 的 SSE 事件翻译成统一片段。

package model

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
)

// anthropicVersion 是 Messages API 要求的版本头，必须随每次请求发送。
const anthropicVersion = "2023-06-01"

// Anthropic 是 Anthropic Messages API 的客户端。
//
// 请求使用 /v1/messages + SSE；工具调用以 tool_use / tool_result 内容块
// 表达，与内部 ToolCall / tool 消息互相转换。
type Anthropic struct {
	provider Provider     // 提供商标识与默认配置
	endpoint string       // 已去掉末尾斜杠的基础地址
	apiKey   string       // 通过 x-api-key 头发送
	http     *http.Client // 使用默认传输设置
}

// NewAnthropic 创建 Anthropic 客户端。
func NewAnthropic(provider Provider, endpoint, apiKey string) *Anthropic {
	if endpoint == "" {
		endpoint = provider.Endpoint
	}
	return &Anthropic{
		provider: provider,
		endpoint: strings.TrimRight(endpoint, "/"),
		apiKey:   apiKey,
		http:     &http.Client{},
	}
}

// Stream 发起流式请求并按片段返回。
//
// Messages API 用 x-api-key 鉴权，并要求携带 anthropic-version；
// 解码器会累积流式工具调用，并在结束时交付用量。
func (c *Anthropic) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	body, err := c.requestBody(req)
	if err != nil {
		return nil, err
	}
	return streamSSE(ctx, streamRequest{
		provider: c.provider.Name,
		client:   c.http,
		endpoint: joinAPIPath(c.endpoint, "/v1/messages"),
		headers: map[string]string{
			"x-api-key":         c.apiKey,
			"anthropic-version": anthropicVersion,
		},
		body: body,
	}, &anthropicDecoder{provider: c.provider.Name, tools: newToolCallBuilder()})
}

// requestBody 把内部请求翻译成 Messages API 的 JSON 结构。
//
// system 不是普通消息角色，而是顶层字段，所以这里会把 system 消息
// 合并进 req.System。工具结果必须以 user 身份、tool_result 内容块回填。
// 连续同角色的消息会被合并，满足 API 对 user / assistant 交替的要求。
func (c *Anthropic) requestBody(req Request) (map[string]any, error) {
	system := strings.TrimSpace(req.System)
	messages := make([]map[string]any, 0, len(req.Messages)+1)

	// appendMessage 追加一条消息；若上一条角色相同则把内容块拼在一起。
	appendMessage := func(role string, blocks []map[string]any) {
		if len(blocks) == 0 {
			return
		}
		if len(messages) > 0 && messages[len(messages)-1]["role"] == role {
			existing, _ := messages[len(messages)-1]["content"].([]map[string]any)
			messages[len(messages)-1]["content"] = append(existing, blocks...)
			return
		}
		messages = append(messages, map[string]any{"role": role, "content": blocks})
	}

	for _, message := range req.Messages {
		switch message.Role {
		case "system":
			// 多个 system 消息用空行拼接，最终写入顶层 system 字段。
			if system != "" {
				system += "\n\n"
			}
			system += message.Content
		case "assistant":
			var blocks []map[string]any
			if message.Content != "" {
				blocks = append(blocks, map[string]any{"type": "text", "text": message.Content})
			}
			for _, call := range message.ToolCalls {
				input := call.Args
				if input == nil {
					input = map[string]any{}
				}
				blocks = append(blocks, map[string]any{
					"type":  "tool_use",
					"id":    call.ID,
					"name":  call.Name,
					"input": input,
				})
			}
			appendMessage("assistant", blocks)
		case "tool":
			// tool_result 只能出现在 user 消息里，通过 tool_use_id 关联调用。
			appendMessage("user", []map[string]any{{
				"type":        "tool_result",
				"tool_use_id": message.ToolCallID,
				"content":     message.Content,
			}})
		default:
			var blocks []map[string]any
			if message.Content != "" {
				blocks = append(blocks, map[string]any{"type": "text", "text": message.Content})
			}
			for _, attachment := range message.Attachments {
				if attachment.Skipped != "" {
					blocks = append(blocks, map[string]any{"type": "text", "text": attachment.SkipNotice()})
					continue
				}
				if attachment.Text != "" {
					blocks = append(blocks, map[string]any{"type": "text", "text": attachment.PromptText()})
				}
				if attachment.IsImage() {
					blocks = append(blocks, map[string]any{
						"type": "image",
						"source": map[string]any{
							"type":       "base64",
							"media_type": attachment.MIME,
							"data":       attachment.Data,
						},
					})
				}
			}
			appendMessage("user", blocks)
		}
	}

	// max_tokens 在 Messages API 中是必填项，未指定时给一个安全默认值。
	maxTokens := req.MaxTokens
	if maxTokens <= 0 {
		maxTokens = 4096
	}
	body := map[string]any{
		"model":      req.Model,
		"max_tokens": maxTokens,
		"messages":   messages,
		"stream":     true,
	}
	if system != "" {
		body["system"] = system
	}
	if len(req.Tools) > 0 {
		tools := make([]map[string]any, 0, len(req.Tools))
		for _, tool := range req.Tools {
			schema := tool.Schema
			if schema == nil {
				// input_schema 必填，没有参数的工具也要给出空对象模式。
				schema = map[string]any{"type": "object", "properties": map[string]any{}}
			}
			tools = append(tools, map[string]any{
				"name":         tool.Name,
				"description":  tool.Description,
				"input_schema": schema,
			})
		}
		body["tools"] = tools
	}
	return body, nil
}

// anthropicUsage 是 Messages API 的用量结构。
//
// 缓存相关字段在 message_start 中出现，输入与输出 token 会分摊在
// message_start 与 message_delta 两条事件里。
type anthropicUsage struct {
	InputTokens              int64 `json:"input_tokens"`                // 输入 token 数
	OutputTokens             int64 `json:"output_tokens"`               // 输出 token 数
	CacheCreationInputTokens int64 `json:"cache_creation_input_tokens"` // 写入缓存的 token 数
	CacheReadInputTokens     int64 `json:"cache_read_input_tokens"`     // 从缓存读取的 token 数
}

// anthropicEvent 是 Messages API 流式事件的最小公共结构。
//
// 不同事件类型只填充各自相关的字段，其余字段保持为零值。
type anthropicEvent struct {
	Type  string `json:"type"`  // 事件类型，例如 message_start、content_block_delta
	Index int    `json:"index"` // 内容块序号，工具参数增量按它归属到对应调用
	// Message 只在 message_start 中出现。
	Message *struct {
		Usage anthropicUsage `json:"usage"`
	} `json:"message"`
	// ContentBlock 只在 content_block_start 中出现。
	ContentBlock *struct {
		Type  string          `json:"type"`
		ID    string          `json:"id"`
		Name  string          `json:"name"`
		Input json.RawMessage `json:"input"`
	} `json:"content_block"`
	// Delta 是内容块增量，只在 content_block_delta 中出现。
	Delta *struct {
		Type        string `json:"type"`
		Text        string `json:"text"`
		Thinking    string `json:"thinking"`
		PartialJSON string `json:"partial_json"`
	} `json:"delta"`
	// Usage 只在 message_delta 中出现。
	Usage *anthropicUsage `json:"usage"`
	// Error 是错误详情，只在 error 事件中出现。
	Error *struct {
		Type    string `json:"type"`
		Message string `json:"message"`
	} `json:"error"`
}

// anthropicDecoder 把 Messages API 的 SSE 事件翻译成片段。
type anthropicDecoder struct {
	provider  string           // 提供商名称，用于错误信息
	tools     *toolCallBuilder // 累积分片到达的工具调用
	usage     Usage            // 跨事件汇总后的用量
	usageSeen bool             // 是否至少收到过一次用量信息
}

// decode 处理一条 Messages 事件。
//
// 事件类型决定载荷形态：message_start 给输入用量，content_block_delta
// 给文本或工具参数增量，message_delta 给输出用量，message_stop 表示结束。
func (d *anthropicDecoder) decode(payload []byte, emit emitFunc) (bool, error) {
	var event anthropicEvent
	if err := json.Unmarshal(payload, &event); err != nil {
		return false, nil
	}

	switch event.Type {
	case "error":
		message := "模型服务返回错误"
		kind := ""
		if event.Error != nil {
			if event.Error.Message != "" {
				message = event.Error.Message
			}
			kind = event.Error.Type
		}
		return false, StreamError(d.provider, message, kind)
	case "message_start":
		if event.Message != nil {
			d.usage.InputTokens = event.Message.Usage.InputTokens
			d.usage.CacheTokens = event.Message.Usage.CacheCreationInputTokens + event.Message.Usage.CacheReadInputTokens
			// 缓存写入单独记录，费用计算需要用写入价而不是读取价。
			d.usage.CacheWriteTokens = event.Message.Usage.CacheCreationInputTokens
			d.usageSeen = true
		}
	case "content_block_start":
		if event.ContentBlock != nil && event.ContentBlock.Type == "tool_use" {
			// 工具调用开始：先记录 ID 与名称。部分服务会在这里给出完整输入，
			// 空对象 {} 视为“稍后通过 input_json_delta 增量给出”。
			initial := strings.TrimSpace(string(event.ContentBlock.Input))
			if initial == "{}" {
				initial = ""
			}
			d.tools.add(event.Index, event.ContentBlock.ID, event.ContentBlock.Name, initial)
		}
	case "content_block_delta":
		if event.Delta == nil {
			return false, nil
		}
		switch event.Delta.Type {
		case "text_delta":
			if event.Delta.Text != "" && !emit(Chunk{Text: event.Delta.Text}) {
				return true, nil
			}
		case "thinking_delta":
			if event.Delta.Thinking != "" && !emit(Chunk{Reasoning: event.Delta.Thinking}) {
				return true, nil
			}
		case "input_json_delta":
			// 工具参数是逐段到达的 JSON 文本，必须按顺序拼接后再解析。
			d.tools.add(event.Index, "", "", event.Delta.PartialJSON)
		}
	case "message_delta":
		if event.Usage != nil {
			d.usage.OutputTokens = event.Usage.OutputTokens
			// 输入用量有时只在 message_delta 中给出，非零时覆盖初值。
			if event.Usage.InputTokens > 0 {
				d.usage.InputTokens = event.Usage.InputTokens
			}
			d.usageSeen = true
		}
	case "message_stop":
		// 协议层明确结束；调用方会紧接着执行 finish。
		return true, nil
	}
	return false, nil
}

// finish 在流结束时冲刷工具调用，并交付用量与结束标记。
//
// 没有收到任何用量事件时交付空 Usage，保证下游总能拿到结束片段。
func (d *anthropicDecoder) finish(emit emitFunc) {
	d.tools.flush(emit)
	usage := d.usage
	if !d.usageSeen {
		usage = Usage{}
	}
	emit(Chunk{Usage: &usage, Done: true})
}

// 编译期确认 Anthropic 实现了 Client 接口。
var _ Client = (*Anthropic)(nil)
