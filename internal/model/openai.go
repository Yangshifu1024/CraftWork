// 本文件负责 OpenAI Chat Completions 协议客户端，同时兼容所有采用
// 相同请求与 SSE 格式的第三方服务。核心工作是把内部消息与工具调用
// 翻译成 Chat Completions 结构，再把流式增量翻译回 Chunk。

package model

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
)

// OpenAICompat 是 OpenAI Chat Completions 与兼容服务的客户端。
type OpenAICompat struct {
	provider Provider     // 提供商标识与默认配置
	endpoint string       // 已去掉末尾斜杠的基础地址
	apiKey   string       // 为空时不发送 Authorization 头
	http     *http.Client // 使用默认传输设置
}

// NewOpenAICompat 创建客户端。endpoint 为空时使用目录中的默认地址。
func NewOpenAICompat(provider Provider, endpoint, apiKey string) *OpenAICompat {
	if endpoint == "" {
		endpoint = provider.Endpoint
	}
	return &OpenAICompat{
		provider: provider,
		endpoint: strings.TrimRight(endpoint, "/"),
		apiKey:   apiKey,
		http:     &http.Client{},
	}
}

// Stream 发起流式请求并按片段返回。
//
// 请求发往“基础地址 + /chat/completions”，鉴权使用 Bearer 头；
// 解码器会累积流式工具调用，并在结束时交付用量。
func (c *OpenAICompat) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	body, err := c.requestBody(req)
	if err != nil {
		return nil, err
	}
	headers := map[string]string{}
	if c.apiKey != "" {
		headers["Authorization"] = "Bearer " + c.apiKey
	}
	return streamSSE(ctx, streamRequest{
		provider: c.provider.Name,
		client:   c.http,
		endpoint: c.endpoint + "/chat/completions",
		headers:  headers,
		body:     body,
	}, &openAIDecoder{provider: c.provider.Name, tools: newToolCallBuilder()})
}

// requestBody 把内部请求翻译成 Chat Completions 的 JSON 结构。
//
// 系统提示词的各个分块用空行拼成第一条 system 消息；该协议没有显式的
// 缓存标记，提供商的自动前缀缓存依赖请求前缀字节稳定，因此拼接规则
// 固定、不增减空白。CacheControl 字段在此协议下忽略。
// assistant 的工具调用使用 tool_calls 数组；工具结果使用独立的 tool 消息
// 并通过 tool_call_id 关联。用户消息带附件时会改写成多段 content（文本 + 图片）。
func (c *OpenAICompat) requestBody(req Request) (map[string]any, error) {
	messages := make([]map[string]any, 0, len(req.Messages)+1)
	var system strings.Builder
	for _, block := range req.System {
		if strings.TrimSpace(block.Content) == "" {
			continue
		}
		if system.Len() > 0 {
			system.WriteString("\n\n")
		}
		system.WriteString(block.Content)
	}
	if system.Len() > 0 {
		messages = append(messages, map[string]any{"role": "system", "content": system.String()})
	}
	for _, message := range req.Messages {
		switch message.Role {
		case "assistant":
			item := map[string]any{"role": "assistant", "content": message.Content}
			if len(message.ToolCalls) > 0 {
				// 工具参数内部是对象，协议要求序列化成 JSON 字符串。
				calls := make([]map[string]any, 0, len(message.ToolCalls))
				for _, call := range message.ToolCalls {
					args, err := json.Marshal(call.Args)
					if err != nil {
						return nil, fmt.Errorf("编码工具参数: %w", err)
					}
					calls = append(calls, map[string]any{
						"id":   call.ID,
						"type": "function",
						"function": map[string]any{
							"name":      call.Name,
							"arguments": string(args),
						},
					})
				}
				item["tool_calls"] = calls
			}
			messages = append(messages, item)
		case "tool":
			messages = append(messages, map[string]any{
				"role":         "tool",
				"tool_call_id": message.ToolCallID,
				"content":      message.Content,
			})
		default:
			content := any(message.Content)
			if message.Role == "user" && len(message.Attachments) > 0 {
				content = openAIUserContent(message)
			}
			messages = append(messages, map[string]any{"role": message.Role, "content": content})
		}
	}

	body := map[string]any{
		"model":    req.Model,
		"messages": messages,
		"stream":   true,
		// 显式要求流式响应在最后附带 usage，否则拿不到 token 计量。
		"stream_options": map[string]any{"include_usage": true},
	}
	if req.MaxTokens > 0 {
		body["max_tokens"] = req.MaxTokens
	}
	if len(req.Tools) > 0 {
		tools := make([]map[string]any, 0, len(req.Tools))
		for _, tool := range req.Tools {
			tools = append(tools, map[string]any{
				"type": "function",
				"function": map[string]any{
					"name":        tool.Name,
					"description": tool.Description,
					"parameters":  tool.Schema,
				},
			})
		}
		body["tools"] = tools
	}
	return body, nil
}

// openAIUserContent 把用户文本与附件拼成 OpenAI 的多段 content。
//
// 没有图片时直接返回纯文本；有图片时返回 content 数组，文本放在第一段，
// 图片以 data URL 形式内联，避免依赖可被外部访问的图片地址。
func openAIUserContent(message Message) any {
	texts := make([]string, 0, len(message.Attachments)+1)
	if message.Content != "" {
		texts = append(texts, message.Content)
	}
	var images []map[string]any
	for _, attachment := range message.Attachments {
		if attachment.Skipped != "" {
			texts = append(texts, attachment.SkipNotice())
			continue
		}
		if attachment.Text != "" {
			texts = append(texts, attachment.PromptText())
		}
		if attachment.IsImage() {
			images = append(images, map[string]any{
				"type": "image_url",
				"image_url": map[string]any{
					"url": "data:" + attachment.MIME + ";base64," + attachment.Data,
				},
			})
		}
	}
	text := strings.Join(texts, "\n\n")
	if len(images) == 0 {
		return text
	}
	parts := make([]map[string]any, 0, len(images)+1)
	if text != "" {
		parts = append(parts, map[string]any{"type": "text", "text": text})
	}
	parts = append(parts, images...)
	return parts
}

// sseEvent 是 Chat Completions 流式响应的单条事件。
//
// 同一条事件可能只包含增量文本，也可能只包含用量（此时 choices 为空）；
// 服务端错误还可能以 200 状态码加 error 字段的形式出现在流中间。
type sseEvent struct {
	// Choices 是本次增量的候选结果，通常只有一个。
	Choices []struct {
		Delta struct {
			Content string `json:"content"`
			// 推理内容没有统一字段名：OpenAI 兼容服务常见 reasoning_content，
			// 部分服务使用 reasoning，两者只需要取其中一个。
			ReasoningContent string `json:"reasoning_content"`
			Reasoning        string `json:"reasoning"`
			ToolCalls        []struct {
				Index    int    `json:"index"`
				ID       string `json:"id"`
				Function struct {
					Name      string `json:"name"`
					Arguments string `json:"arguments"`
				} `json:"function"`
			} `json:"tool_calls"`
		} `json:"delta"`
		FinishReason string `json:"finish_reason"`
	} `json:"choices"`
	// Usage 只在请求了 include_usage 时出现在最后一条事件中。
	Usage *struct {
		PromptTokens        int64 `json:"prompt_tokens"`
		CompletionTokens    int64 `json:"completion_tokens"`
		PromptTokensDetails struct {
			CachedTokens int64 `json:"cached_tokens"`
		} `json:"prompt_tokens_details"`
		CompletionTokensDetails struct {
			ReasoningTokens int64 `json:"reasoning_tokens"`
		} `json:"completion_tokens_details"`
	} `json:"usage"`
	// Error 是流内错误，HTTP 状态码可能仍然是 200。
	Error *struct {
		Message string `json:"message"`
		Type    string `json:"type"`
		Code    string `json:"code"`
	} `json:"error"`
}

// openAIDecoder 把 Chat Completions 的 SSE 事件翻译成片段。
type openAIDecoder struct {
	provider string           // 提供商名称，用于错误信息
	tools    *toolCallBuilder // 累积分片到达的工具调用
	usage    *Usage           // 最后一条事件里的用量，可能为空
}

// decode 处理一条 Chat Completions 事件。
//
// 无法解析为 JSON 的载荷会被直接忽略，因为部分兼容服务会发送心跳或
// 非标准注释。工具调用不在这里交付，而是先累积到 finish 统一冲刷。
func (d *openAIDecoder) decode(payload []byte, emit emitFunc) (bool, error) {
	var event sseEvent
	if err := json.Unmarshal(payload, &event); err != nil {
		return false, nil
	}
	if event.Error != nil {
		kind := firstNonEmpty(event.Error.Type, event.Error.Code)
		return false, StreamError(d.provider, event.Error.Message, kind)
	}
	if event.Usage != nil {
		d.usage = &Usage{
			InputTokens:     event.Usage.PromptTokens,
			OutputTokens:    event.Usage.CompletionTokens,
			CacheTokens:     event.Usage.PromptTokensDetails.CachedTokens,
			ReasoningTokens: event.Usage.CompletionTokensDetails.ReasoningTokens,
		}
	}
	for _, choice := range event.Choices {
		delta := choice.Delta
		if delta.ReasoningContent != "" || delta.Reasoning != "" {
			text := delta.ReasoningContent
			if text == "" {
				text = delta.Reasoning
			}
			if !emit(Chunk{Reasoning: text}) {
				return true, nil
			}
		}
		if delta.Content != "" {
			if !emit(Chunk{Text: delta.Content}) {
				return true, nil
			}
		}
		// 工具调用只累积、不立即交付，等流结束后再统一解析参数。
		for _, call := range delta.ToolCalls {
			d.tools.add(call.Index, call.ID, call.Function.Name, call.Function.Arguments)
		}
	}
	return false, nil
}

// finish 在流结束时冲刷工具调用，并交付用量与结束标记。
//
// 没有收到用量事件时仍然交付一个空 Usage，保证下游总能拿到结束片段。
func (d *openAIDecoder) finish(emit emitFunc) {
	d.tools.flush(emit)
	usage := d.usage
	if usage == nil {
		usage = &Usage{}
	}
	emit(Chunk{Usage: usage, Done: true})
}

// 编译期确认 OpenAICompat 实现了 Client 接口。
var _ Client = (*OpenAICompat)(nil)
