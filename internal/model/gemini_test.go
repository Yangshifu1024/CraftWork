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

func TestGeminiStreamConvertsContentsAndTools(t *testing.T) {
	type requestBody struct {
		SystemInstruction struct {
			Parts []struct {
				Text string `json:"text"`
			} `json:"parts"`
		} `json:"systemInstruction"`
		Contents []struct {
			Role  string `json:"role"`
			Parts []struct {
				Text             string `json:"text"`
				ThoughtSignature string `json:"thoughtSignature"`
				FunctionCall     *struct {
					Name string         `json:"name"`
					Args map[string]any `json:"args"`
				} `json:"functionCall"`
				FunctionResponse *struct {
					Name     string         `json:"name"`
					Response map[string]any `json:"response"`
				} `json:"functionResponse"`
				InlineData *struct {
					MimeType string `json:"mimeType"`
					Data     string `json:"data"`
				} `json:"inlineData"`
			} `json:"parts"`
		} `json:"contents"`
		Tools []struct {
			FunctionDeclarations []struct {
				Name string `json:"name"`
			} `json:"functionDeclarations"`
		} `json:"tools"`
	}

	requests := make(chan requestBody, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1beta/models/gemini-2.5-pro:streamGenerateContent" {
			t.Errorf("路径不符: %s", r.URL.Path)
		}
		if got := r.URL.Query().Get("alt"); got != "sse" {
			t.Errorf("alt 参数不符: %q", got)
		}
		if got := r.Header.Get("x-goog-api-key"); got != "test-key" {
			t.Errorf("x-goog-api-key 不符: %q", got)
		}
		var body requestBody
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("解析请求失败: %v", err)
		}
		requests <- body

		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprint(w, strings.Join([]string{
			`data: {"candidates":[{"content":{"role":"model","parts":[{"text":"思考中","thought":true}]}}]}`,
			``,
			`data: {"candidates":[{"content":{"role":"model","parts":[{"text":"我来"}]}}]}`,
			``,
			`data: {"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"fs_list","args":{"path":"."}},"thoughtSignature":"sig-2"}]}}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":4,"cachedContentTokenCount":2,"thoughtsTokenCount":1}}`,
			``,
		}, "\n"))
	}))
	defer server.Close()

	provider := Provider{ID: "gemini", Name: "Google Gemini", Kind: "gemini", Endpoint: server.URL}
	client := NewGemini(provider, server.URL, "test-key")

	stream, err := client.Stream(context.Background(), Request{
		Model:  "gemini-2.5-pro",
		System: "你是助手",
		Messages: []Message{
			{Role: "user", Content: "列出文件", Attachments: []Attachment{
				{Name: "note.txt", Text: "附件内容"},
				{Name: "pic.png", MIME: "image/png", Data: "cGlj"},
			}},
			{Role: "assistant", ToolCalls: []ToolCall{
				{ID: "call_1", Name: "fs_list", Args: map[string]any{"path": "."}, Signature: "sig-1"},
			}},
			{Role: "tool", ToolCallID: "call_1", Content: "a.txt"},
		},
		Tools: []ToolSpec{{Name: "fs_list", Description: "列出文件", Schema: map[string]any{"type": "object"}}},
	})
	if err != nil {
		t.Fatalf("建立流失败: %v", err)
	}

	text, calls, usage := collectStream(t, stream)
	if text != "我来" {
		t.Fatalf("文本不符: %q", text)
	}
	if len(calls) != 1 || calls[0].Name != "fs_list" || calls[0].Signature != "sig-2" {
		t.Fatalf("工具调用不符: %+v", calls)
	}
	if path, _ := calls[0].Args["path"].(string); path != "." {
		t.Fatalf("工具参数不符: %+v", calls[0].Args)
	}
	if usage == nil || usage.InputTokens != 9 || usage.OutputTokens != 4 ||
		usage.CacheTokens != 2 || usage.ReasoningTokens != 1 {
		t.Fatalf("用量不符: %+v", usage)
	}

	body := <-requests
	if body.SystemInstruction.Parts[0].Text != "你是助手" {
		t.Fatalf("systemInstruction 不符: %+v", body.SystemInstruction)
	}
	if len(body.Contents) != 3 {
		t.Fatalf("contents 数量不符: %d", len(body.Contents))
	}
	if body.Contents[0].Role != "user" || body.Contents[0].Parts[0].Text != "列出文件" {
		t.Fatalf("用户内容不符: %+v", body.Contents[0])
	}
	if len(body.Contents[0].Parts) != 3 {
		t.Fatalf("用户内容应包含文本与附件: %+v", body.Contents[0].Parts)
	}
	if !strings.Contains(body.Contents[0].Parts[1].Text, "不可信") {
		t.Fatalf("文本附件包装不符: %+v", body.Contents[0].Parts[1])
	}
	inline := body.Contents[0].Parts[2].InlineData
	if inline == nil || inline.MimeType != "image/png" || inline.Data != "cGlj" {
		t.Fatalf("图片附件不符: %+v", body.Contents[0].Parts[2])
	}
	modelContent := body.Contents[1]
	if modelContent.Role != "model" || modelContent.Parts[0].FunctionCall == nil {
		t.Fatalf("模型内容不符: %+v", modelContent)
	}
	if modelContent.Parts[0].FunctionCall.Name != "fs_list" || modelContent.Parts[0].ThoughtSignature != "sig-1" {
		t.Fatalf("functionCall 不符: %+v", modelContent.Parts[0])
	}
	toolContent := body.Contents[2]
	if toolContent.Role != "user" || toolContent.Parts[0].FunctionResponse == nil {
		t.Fatalf("工具回填不符: %+v", toolContent)
	}
	if toolContent.Parts[0].FunctionResponse.Name != "fs_list" {
		t.Fatalf("functionResponse 名称不符: %+v", toolContent.Parts[0].FunctionResponse)
	}
	if result, _ := toolContent.Parts[0].FunctionResponse.Response["result"].(string); result != "a.txt" {
		t.Fatalf("functionResponse 内容不符: %+v", toolContent.Parts[0].FunctionResponse.Response)
	}
	if len(body.Tools) != 1 || body.Tools[0].FunctionDeclarations[0].Name != "fs_list" {
		t.Fatalf("工具定义不符: %+v", body.Tools)
	}
}

func TestGeminiErrorMapping(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusTooManyRequests)
		fmt.Fprint(w, `{"error":{"code":429,"message":"配额不足","status":"RESOURCE_EXHAUSTED"}}`)
	}))
	defer server.Close()

	provider := Provider{ID: "gemini", Name: "Google Gemini", Kind: "gemini", Endpoint: server.URL}
	client := NewGemini(provider, server.URL, "test-key")
	_, err := client.Stream(context.Background(), Request{Model: "gemini-2.5-pro"})
	if err == nil {
		t.Fatal("应返回错误")
	}
	if CodeOf(err) != CodeRateLimit || !Retryable(err) {
		t.Fatalf("错误分类不符: code=%s retryable=%v", CodeOf(err), Retryable(err))
	}
}
