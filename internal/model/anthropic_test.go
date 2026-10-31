package model

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestAnthropicStreamConvertsMessagesAndTools(t *testing.T) {
	type requestBody struct {
		Model     string `json:"model"`
		MaxTokens int    `json:"max_tokens"`
		System    string `json:"system"`
		Messages  []struct {
			Role    string `json:"role"`
			Content []struct {
				Type      string         `json:"type"`
				Text      string         `json:"text"`
				ID        string         `json:"id"`
				Name      string         `json:"name"`
				Input     map[string]any `json:"input"`
				ToolUseID string         `json:"tool_use_id"`
				Content   string         `json:"content"`
			} `json:"content"`
		} `json:"messages"`
		Tools []struct {
			Name        string         `json:"name"`
			InputSchema map[string]any `json:"input_schema"`
		} `json:"tools"`
	}

	requests := make(chan requestBody, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/messages" {
			t.Errorf("路径不符: %s", r.URL.Path)
		}
		if got := r.Header.Get("x-api-key"); got != "test-key" {
			t.Errorf("x-api-key 不符: %q", got)
		}
		if got := r.Header.Get("anthropic-version"); got != anthropicVersion {
			t.Errorf("anthropic-version 不符: %q", got)
		}
		var body requestBody
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("解析请求失败: %v", err)
		}
		requests <- body

		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprint(w, strings.Join([]string{
			`event: message_start`,
			`data: {"type":"message_start","message":{"usage":{"input_tokens":11,"cache_read_input_tokens":3}}}`,
			``,
			`event: content_block_start`,
			`data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"我来"}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"列文件"}}`,
			``,
			`event: content_block_start`,
			`data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_2","name":"fs_list","input":{}}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}`,
			``,
			`event: content_block_delta`,
			`data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\".\"}"}}`,
			``,
			`event: message_delta`,
			`data: {"type":"message_delta","usage":{"output_tokens":7}}`,
			``,
			`event: message_stop`,
			`data: {"type":"message_stop"}`,
			``,
		}, "\n"))
	}))
	defer server.Close()

	provider := Provider{ID: "anthropic", Name: "Anthropic", Kind: "anthropic", Endpoint: server.URL}
	client := NewAnthropic(provider, server.URL, "test-key")

	stream, err := client.Stream(context.Background(), Request{
		Model:     "claude-sonnet-4",
		System:    "你是助手",
		MaxTokens: 256,
		Messages: []Message{
			{Role: "user", Content: "列出文件"},
			{Role: "assistant", Content: "我来看看。", ToolCalls: []ToolCall{
				{ID: "toolu_1", Name: "fs_list", Args: map[string]any{"path": "."}},
			}},
			{Role: "tool", ToolCallID: "toolu_1", Content: "a.txt"},
		},
		Tools: []ToolSpec{{Name: "fs_list", Description: "列出文件", Schema: map[string]any{"type": "object"}}},
	})
	if err != nil {
		t.Fatalf("建立流失败: %v", err)
	}

	text, calls, usage := collectStream(t, stream)
	if text != "我来列文件" {
		t.Fatalf("文本不符: %q", text)
	}
	if len(calls) != 1 {
		t.Fatalf("工具调用数量不符: %d", len(calls))
	}
	if calls[0].Name != "fs_list" || calls[0].ID != "toolu_2" {
		t.Fatalf("工具调用不符: %+v", calls[0])
	}
	if path, _ := calls[0].Args["path"].(string); path != "." {
		t.Fatalf("工具参数不符: %+v", calls[0].Args)
	}
	if usage == nil || usage.InputTokens != 11 || usage.OutputTokens != 7 || usage.CacheTokens != 3 {
		t.Fatalf("用量不符: %+v", usage)
	}

	body := <-requests
	if body.Model != "claude-sonnet-4" || body.MaxTokens != 256 || body.System != "你是助手" {
		t.Fatalf("请求基础字段不符: %+v", body)
	}
	if len(body.Messages) != 3 {
		t.Fatalf("消息数量不符: %d", len(body.Messages))
	}
	if body.Messages[1].Role != "assistant" || len(body.Messages[1].Content) != 2 {
		t.Fatalf("assistant 内容块不符: %+v", body.Messages[1])
	}
	if body.Messages[1].Content[1].Type != "tool_use" || body.Messages[1].Content[1].Name != "fs_list" {
		t.Fatalf("tool_use 块不符: %+v", body.Messages[1].Content[1])
	}
	result := body.Messages[2]
	if result.Role != "user" || len(result.Content) != 1 || result.Content[0].Type != "tool_result" {
		t.Fatalf("tool_result 消息不符: %+v", result)
	}
	if result.Content[0].ToolUseID != "toolu_1" || result.Content[0].Content != "a.txt" {
		t.Fatalf("tool_result 内容不符: %+v", result.Content[0])
	}
	if len(body.Tools) != 1 || body.Tools[0].Name != "fs_list" {
		t.Fatalf("工具定义不符: %+v", body.Tools)
	}
}

func TestAnthropicErrorMapping(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusTooManyRequests)
		fmt.Fprint(w, `{"type":"error","error":{"type":"rate_limit_error","message":"请求过于频繁"}}`)
	}))
	defer server.Close()

	provider := Provider{ID: "anthropic", Name: "Anthropic", Kind: "anthropic", Endpoint: server.URL}
	client := NewAnthropic(provider, server.URL, "test-key")
	_, err := client.Stream(context.Background(), Request{Model: "claude-sonnet-4"})
	if err == nil {
		t.Fatal("应返回错误")
	}
	if CodeOf(err) != CodeRateLimit {
		t.Fatalf("错误码不符: %s", CodeOf(err))
	}
	if !Retryable(err) {
		t.Fatal("限流错误应可重试")
	}
	if !strings.Contains(err.Error(), "请求过于频繁") {
		t.Fatalf("错误信息应包含服务端文案: %v", err)
	}
}

func collectStream(t *testing.T, stream <-chan Chunk) (string, []*ToolCall, *Usage) {
	t.Helper()
	var text strings.Builder
	var calls []*ToolCall
	var usage *Usage
	for chunk := range stream {
		if chunk.Err != nil {
			t.Fatalf("流式错误: %v", chunk.Err)
		}
		text.WriteString(chunk.Text)
		if chunk.ToolCall != nil {
			calls = append(calls, chunk.ToolCall)
		}
		if chunk.Usage != nil {
			usage = chunk.Usage
		}
	}
	return text.String(), calls, usage
}
