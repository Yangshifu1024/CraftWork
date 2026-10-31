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
	raw, err := json.Marshal(body)
	if err != nil {
		return nil, fmt.Errorf("编码模型请求: %w", err)
	}

	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, joinAPIPath(c.endpoint, "/v1/messages"), bytes.NewReader(raw))
	if err != nil {
		return nil, fmt.Errorf("创建模型请求: %w", err)
	}
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("Accept", "text/event-stream")
	httpReq.Header.Set("x-api-key", c.apiKey)
	httpReq.Header.Set("anthropic-version", anthropicVersion)

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
			if message.Content == "" {
				continue
			}
			appendMessage("user", []map[string]any{{"type": "text", "text": message.Content}})
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

type anthropicToolBlock struct {
	mu   sync.Mutex
	id   string
	name string
	raw  strings.Builder
}

func (c *Anthropic) consume(ctx context.Context, body io.Reader, channel chan<- Chunk) {
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 64*1024), 4*1024*1024)

	blocks := map[int]*anthropicToolBlock{}
	var order []int
	var usage Usage
	usageSeen := false

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

		var event anthropicEvent
		if err := json.Unmarshal([]byte(payload), &event); err != nil {
			continue
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
			send(ctx, channel, Chunk{Err: StreamError(c.provider.Name, message, kind)})
			return
		case "message_start":
			if event.Message != nil {
				usage.InputTokens = event.Message.Usage.InputTokens
				usage.CacheTokens = event.Message.Usage.CacheCreationInputTokens + event.Message.Usage.CacheReadInputTokens
				usageSeen = true
			}
		case "content_block_start":
			if event.ContentBlock != nil && event.ContentBlock.Type == "tool_use" {
				block := &anthropicToolBlock{id: event.ContentBlock.ID, name: event.ContentBlock.Name}
				if raw := strings.TrimSpace(string(event.ContentBlock.Input)); raw != "" && raw != "{}" {
					block.raw.WriteString(raw)
				}
				blocks[event.Index] = block
				order = append(order, event.Index)
			}
		case "content_block_delta":
			if event.Delta == nil {
				continue
			}
			switch event.Delta.Type {
			case "text_delta":
				if event.Delta.Text != "" {
					if !send(ctx, channel, Chunk{Text: event.Delta.Text}) {
						return
					}
				}
			case "thinking_delta":
				if event.Delta.Thinking != "" {
					if !send(ctx, channel, Chunk{Reasoning: event.Delta.Thinking}) {
						return
					}
				}
			case "input_json_delta":
				if block := blocks[event.Index]; block != nil {
					block.mu.Lock()
					block.raw.WriteString(event.Delta.PartialJSON)
					block.mu.Unlock()
				}
			}
		case "message_delta":
			if event.Usage != nil {
				usage.OutputTokens = event.Usage.OutputTokens
				if event.Usage.InputTokens > 0 {
					usage.InputTokens = event.Usage.InputTokens
				}
				usageSeen = true
			}
		case "message_stop":
			c.emitToolBlocks(ctx, channel, blocks, order)
			if !usageSeen {
				usage = Usage{}
			}
			send(ctx, channel, Chunk{Usage: &usage, Done: true})
			return
		}
	}

	if ctx.Err() != nil {
		return
	}
	// 连接提前结束时仍然交付已经累积的内容，避免整轮结果丢失。
	c.emitToolBlocks(ctx, channel, blocks, order)
	send(ctx, channel, Chunk{Usage: &usage, Done: true})
}

func (c *Anthropic) emitToolBlocks(ctx context.Context, channel chan<- Chunk, blocks map[int]*anthropicToolBlock, order []int) {
	sort.Ints(order)
	for _, index := range order {
		block := blocks[index]
		if block == nil {
			continue
		}
		block.mu.Lock()
		call := ToolCall{
			ID:   block.id,
			Name: block.name,
			Args: parseArguments(block.raw.String()),
		}
		block.mu.Unlock()
		if call.ID == "" {
			call.ID = id.New("call")
		}
		if !send(ctx, channel, Chunk{ToolCall: &call}) {
			return
		}
	}
}

var _ Client = (*Anthropic)(nil)
