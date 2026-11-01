package model

import (
	"context"
	"encoding/json"
	"net/http"
	"strings"
)

const anthropicVersion = "2023-06-01"

// Anthropic 是 Anthropic Messages API 的客户端。
//
// 请求使用 /v1/messages + SSE；工具调用以 tool_use / tool_result 内容块
// 表达，与内部 ToolCall / tool 消息互相转换。
type Anthropic struct {
	provider Provider
	endpoint string
	apiKey   string
	http     *http.Client
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

func (c *Anthropic) requestBody(req Request) (map[string]any, error) {
	system := strings.TrimSpace(req.System)
	messages := make([]map[string]any, 0, len(req.Messages)+1)

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

type anthropicUsage struct {
	InputTokens              int64 `json:"input_tokens"`
	OutputTokens             int64 `json:"output_tokens"`
	CacheCreationInputTokens int64 `json:"cache_creation_input_tokens"`
	CacheReadInputTokens     int64 `json:"cache_read_input_tokens"`
}

type anthropicEvent struct {
	Type  string `json:"type"`
	Index int    `json:"index"`
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
	Delta *struct {
		Type        string `json:"type"`
		Text        string `json:"text"`
		Thinking    string `json:"thinking"`
		PartialJSON string `json:"partial_json"`
	} `json:"delta"`
	// Usage 只在 message_delta 中出现。
	Usage *anthropicUsage `json:"usage"`
	Error *struct {
		Type    string `json:"type"`
		Message string `json:"message"`
	} `json:"error"`
}

// anthropicDecoder 把 Messages API 的 SSE 事件翻译成片段。
type anthropicDecoder struct {
	provider  string
	tools     *toolCallBuilder
	usage     Usage
	usageSeen bool
}

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
			d.usageSeen = true
		}
	case "content_block_start":
		if event.ContentBlock != nil && event.ContentBlock.Type == "tool_use" {
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
			d.tools.add(event.Index, "", "", event.Delta.PartialJSON)
		}
	case "message_delta":
		if event.Usage != nil {
			d.usage.OutputTokens = event.Usage.OutputTokens
			if event.Usage.InputTokens > 0 {
				d.usage.InputTokens = event.Usage.InputTokens
			}
			d.usageSeen = true
		}
	case "message_stop":
		return true, nil
	}
	return false, nil
}

func (d *anthropicDecoder) finish(emit emitFunc) {
	d.tools.flush(emit)
	usage := d.usage
	if !d.usageSeen {
		usage = Usage{}
	}
	emit(Chunk{Usage: &usage, Done: true})
}

var _ Client = (*Anthropic)(nil)
