package model

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"

	"craftwork/internal/id"
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
	raw, err := json.Marshal(body)
	if err != nil {
		return nil, fmt.Errorf("编码模型请求: %w", err)
	}

	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, c.endpoint+"/chat/completions", bytes.NewReader(raw))
	if err != nil {
		return nil, fmt.Errorf("创建模型请求: %w", err)
	}
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("Accept", "text/event-stream")
	if c.apiKey != "" {
		httpReq.Header.Set("Authorization", "Bearer "+c.apiKey)
	}

	resp, err := c.http.Do(httpReq)
	if err != nil {
		return nil, TransportError(c.provider.Name, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 8192))
		resp.Body.Close()
		return nil, HTTPError(c.provider.Name, resp.StatusCode, snippet)
	}

	channel := make(chan Chunk, 16)
	go func() {
		defer close(channel)
		defer resp.Body.Close()
		c.consume(ctx, resp.Body, channel)
	}()
	return channel, nil
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

type toolAccumulator struct {
	mu   sync.Mutex
	id   string
	name string
	args strings.Builder
}

func (c *OpenAICompat) consume(ctx context.Context, body io.Reader, channel chan<- Chunk) {
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 64*1024), 4*1024*1024)

	accumulators := map[int]*toolAccumulator{}
	var usage *Usage

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
			break
		}

		var event sseEvent
		if err := json.Unmarshal([]byte(payload), &event); err != nil {
			continue
		}
		if event.Error != nil {
			kind := firstNonEmpty(event.Error.Type, event.Error.Code)
			select {
			case channel <- Chunk{Err: StreamError(c.provider.Name, event.Error.Message, kind)}:
			case <-ctx.Done():
			}
			return
		}
		if event.Usage != nil {
			usage = &Usage{
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
				if !send(ctx, channel, Chunk{Reasoning: text}) {
					return
				}
			}
			if delta.Content != "" {
				if !send(ctx, channel, Chunk{Text: delta.Content}) {
					return
				}
			}
			for _, toolCall := range delta.ToolCalls {
				accumulator := accumulators[toolCall.Index]
				if accumulator == nil {
					accumulator = &toolAccumulator{}
					accumulators[toolCall.Index] = accumulator
				}
				accumulator.mu.Lock()
				if toolCall.ID != "" {
					accumulator.id = toolCall.ID
				}
				if toolCall.Function.Name != "" {
					accumulator.name = toolCall.Function.Name
				}
				accumulator.args.WriteString(toolCall.Function.Arguments)
				accumulator.mu.Unlock()
			}
		}
	}

	if ctx.Err() != nil {
		return
	}
	for index := 0; index < len(accumulators); index++ {
		accumulator := accumulators[index]
		if accumulator == nil {
			continue
		}
		accumulator.mu.Lock()
		call := ToolCall{
			ID:   accumulator.id,
			Name: accumulator.name,
			Args: parseArguments(accumulator.args.String()),
		}
		accumulator.mu.Unlock()
		if call.ID == "" {
			call.ID = id.New("call")
		}
		if !send(ctx, channel, Chunk{ToolCall: &call}) {
			return
		}
	}
	if usage == nil {
		usage = &Usage{}
	}
	send(ctx, channel, Chunk{Usage: usage, Done: true})
}

func send(ctx context.Context, channel chan<- Chunk, chunk Chunk) bool {
	select {
	case channel <- chunk:
		return true
	case <-ctx.Done():
		return false
	}
}

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

var _ Client = (*OpenAICompat)(nil)
