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
	provider Provider
	endpoint string
	apiKey   string
	http     *http.Client
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

func (c *OpenAICompat) requestBody(req Request) (map[string]any, error) {
	messages := make([]map[string]any, 0, len(req.Messages)+1)
	if req.System != "" {
		messages = append(messages, map[string]any{"role": "system", "content": req.System})
	}
	for _, message := range req.Messages {
		switch message.Role {
		case "assistant":
			item := map[string]any{"role": "assistant", "content": message.Content}
			if len(message.ToolCalls) > 0 {
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
		"model":          req.Model,
		"messages":       messages,
		"stream":         true,
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

type sseEvent struct {
	Choices []struct {
		Delta struct {
			Content          string `json:"content"`
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
	Error *struct {
		Message string `json:"message"`
		Type    string `json:"type"`
		Code    string `json:"code"`
	} `json:"error"`
}

// openAIDecoder 把 Chat Completions 的 SSE 事件翻译成片段。
type openAIDecoder struct {
	provider string
	tools    *toolCallBuilder
	usage    *Usage
}

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
		for _, call := range delta.ToolCalls {
			d.tools.add(call.Index, call.ID, call.Function.Name, call.Function.Arguments)
		}
	}
	return false, nil
}

func (d *openAIDecoder) finish(emit emitFunc) {
	d.tools.flush(emit)
	usage := d.usage
	if usage == nil {
		usage = &Usage{}
	}
	emit(Chunk{Usage: usage, Done: true})
}

var _ Client = (*OpenAICompat)(nil)
