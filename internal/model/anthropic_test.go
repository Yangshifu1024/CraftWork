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
		System    []struct {
			Text         string         `json:"text"`
			CacheControl map[string]any `json:"cache_control"`
		} `json:"system"`
		Messages  []struct {
			Role    string `json:"role"`
			Content []struct {
				Type         string         `json:"type"`
				Text         string         `json:"text"`
				ID           string         `json:"id"`
				Name         string         `json:"name"`
				Input        map[string]any `json:"input"`
				ToolUseID    string         `json:"tool_use_id"`
				Content      string         `json:"content"`
				CacheControl map[string]any `json:"cache_control"`
				Source       struct {
					Type      string `json:"type"`
					MediaType string `json:"media_type"`
					Data      string `json:"data"`
				} `json:"source"`
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
		System:    []SystemBlock{{Content: "你是助手"}, {Content: "行为规则", CacheControl: true}},
		MaxTokens: 256,
		Messages: []Message{
			{Role: "user", Content: "列出文件", Attachments: []Attachment{
				{Name: "note.txt", Text: "附件内容"},
				{Name: "pic.png", MIME: "image/png", Data: "cGlj"},
			}},
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
	if body.Model != "claude-sonnet-4" || body.MaxTokens != 256 {
		t.Fatalf("请求基础字段不符: %+v", body)
	}
	// system 应为数组：第一块不带缓存标记，第二块带 ephemeral。
	if len(body.System) != 2 || body.System[0].Text != "你是助手" || body.System[1].Text != "行为规则" {
		t.Fatalf("system 块不符: %+v", body.System)
	}
	if body.System[0].CacheControl != nil {
		t.Fatalf("未标记的块不应携带 cache_control: %+v", body.System[0])
	}
	if body.System[1].CacheControl["type"] != "ephemeral" {
		t.Fatalf("标记块应携带 ephemeral 缓存标记: %+v", body.System[1])
	}
	if len(body.Messages) != 3 {
		t.Fatalf("消息数量不符: %d", len(body.Messages))
	}
	first := body.Messages[0]
	if len(first.Content) != 3 {
		t.Fatalf("用户消息应包含文本与附件块: %+v", first.Content)
	}
	if !strings.Contains(first.Content[1].Text, "不可信") || !strings.Contains(first.Content[1].Text, "附件内容") {
		t.Fatalf("文本附件包装不符: %+v", first.Content[1])
	}
	if first.Content[2].Type != "image" || first.Content[2].Source.MediaType != "image/png" ||
		first.Content[2].Source.Data != "cGlj" {
		t.Fatalf("图片附件块不符: %+v", first.Content[2])
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
	// 历史尾部断点：最后一条消息的末尾块带缓存标记，其余消息不带。
	if result.Content[0].CacheControl["type"] != "ephemeral" {
		t.Fatalf("历史尾部应携带缓存断点: %+v", result.Content[0])
	}
	if body.Messages[0].Content[0].CacheControl != nil {
		t.Fatalf("非尾部消息不应带缓存标记: %+v", body.Messages[0].Content[0])
	}
	if len(body.Tools) != 1 || body.Tools[0].Name != "fs_list" {
		t.Fatalf("工具定义不符: %+v", body.Tools)
	}
}

// TestAnthropicSystemBlocksAndOmission 验证请求体转换的边界：空白块跳过、
// 全空 system 不写入顶层字段、消息序列里的 system 消息作为不带标记的块汇入。
func TestAnthropicSystemBlocksAndOmission(t *testing.T) {
	type requestBody struct {
		System []struct {
			Text         string         `json:"text"`
			CacheControl map[string]any `json:"cache_control"`
		} `json:"system"`
		Messages []struct {
			Role string `json:"role"`
		} `json:"messages"`
	}

	newBody := func(req Request) requestBody {
		requests := make(chan requestBody, 1)
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			var body requestBody
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
				t.Errorf("解析请求失败: %v", err)
			}
			requests <- body
			w.Header().Set("Content-Type", "text/event-stream")
			fmt.Fprint(w, strings.Join([]string{
				`event: message_start`,
				`data: {"type":"message_start","message":{}}`,
				``,
				`event: message_stop`,
				`data: {"type":"message_stop"}`,
				``,
			}, "\n"))
		}))
		defer server.Close()

		provider := Provider{ID: "anthropic", Name: "Anthropic", Kind: "anthropic", Endpoint: server.URL}
		client := NewAnthropic(provider, server.URL, "test-key")
		stream, err := client.Stream(context.Background(), req)
		if err != nil {
			t.Fatalf("建立流失败: %v", err)
		}
		collectStream(t, stream)
		return <-requests
	}

	// 空白块跳过；消息序列里的 system 消息作为不带缓存标记的块汇入数组末尾。
	body := newBody(Request{
		Model:  "claude-sonnet-4",
		System: []SystemBlock{{Content: "你是助手。"}, {Content: " "}, {Content: "行为规则", CacheControl: true}},
		Messages: []Message{
			{Role: "system", Content: "补充说明"},
			{Role: "user", Content: "列出文件"},
		},
	})
	if len(body.System) != 3 {
		t.Fatalf("system 块数量不符: %+v", body.System)
	}
	if body.System[0].Text != "你是助手。" || body.System[1].Text != "行为规则" || body.System[2].Text != "补充说明" {
		t.Fatalf("system 块内容不符: %+v", body.System)
	}
	if body.System[2].CacheControl != nil {
		t.Fatalf("消息汇入的 system 块不应携带 cache_control: %+v", body.System[2])
	}

	// 全空白的系统提示词不应产生顶层 system 字段。
	empty := newBody(Request{
		Model:    "claude-sonnet-4",
		System:   []SystemBlock{{Content: "  "}},
		Messages: []Message{{Role: "user", Content: "列出文件"}},
	})
	if len(empty.System) != 0 {
		t.Fatalf("全空 system 不应写入请求体: %+v", empty.System)
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
