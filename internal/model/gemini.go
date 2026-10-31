package model

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"

	"craftwork/internal/id"
)

// Gemini 是 Google Gemini generateContent / streamGenerateContent 的客户端。
//
// 工具以 functionDeclarations 暴露，模型返回 functionCall，回填时使用
// functionResponse；调用 ID 由本地生成，并通过 ToolCallID 关联。
type Gemini struct {
	provider Provider
	endpoint string
	apiKey   string
	http     *http.Client
}

// NewGemini 创建 Gemini 客户端。
func NewGemini(provider Provider, endpoint, apiKey string) *Gemini {
	if endpoint == "" {
		endpoint = provider.Endpoint
	}
	return &Gemini{
		provider: provider,
		endpoint: strings.TrimRight(endpoint, "/"),
		apiKey:   apiKey,
		http:     &http.Client{},
	}
}

// Stream 发起流式请求并按片段返回。
func (g *Gemini) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	body, err := g.requestBody(req)
	if err != nil {
		return nil, err
	}
	raw, err := json.Marshal(body)
	if err != nil {
		return nil, fmt.Errorf("编码模型请求: %w", err)
	}

	target := joinAPIPath(g.endpoint, "/v1beta/models/"+url.PathEscape(req.Model)+":streamGenerateContent") + "?alt=sse"
	httpReq, err := http.NewRequestWithContext(ctx, http.MethodPost, target, bytes.NewReader(raw))
	if err != nil {
		return nil, fmt.Errorf("创建模型请求: %w", err)
	}
	httpReq.Header.Set("Content-Type", "application/json")
	httpReq.Header.Set("Accept", "text/event-stream")
	httpReq.Header.Set("x-goog-api-key", g.apiKey)

	resp, err := g.http.Do(httpReq)
	if err != nil {
		return nil, TransportError(g.provider.Name, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		snippet, _ := io.ReadAll(io.LimitReader(resp.Body, 8192))
		resp.Body.Close()
		return nil, HTTPError(g.provider.Name, resp.StatusCode, snippet)
	}

	channel := make(chan Chunk, 16)
	go func() {
		defer close(channel)
		defer resp.Body.Close()
		g.consume(ctx, resp.Body, channel)
	}()
	return channel, nil
}

type geminiPart struct {
	Text             string                  `json:"text,omitempty"`
	Thought          bool                    `json:"thought,omitempty"`
	ThoughtSignature string                  `json:"thoughtSignature,omitempty"`
	FunctionCall     *geminiFunctionCall     `json:"functionCall,omitempty"`
	FunctionResponse *geminiFunctionResponse `json:"functionResponse,omitempty"`
	InlineData       *geminiInlineData       `json:"inlineData,omitempty"`
}

type geminiInlineData struct {
	MimeType string `json:"mimeType"`
	Data     string `json:"data"`
}

type geminiFunctionCall struct {
	Name string         `json:"name"`
	Args map[string]any `json:"args,omitempty"`
}

type geminiFunctionResponse struct {
	Name     string         `json:"name"`
	Response map[string]any `json:"response"`
}

type geminiContent struct {
	Role  string       `json:"role,omitempty"`
	Parts []geminiPart `json:"parts"`
}

func (g *Gemini) requestBody(req Request) (map[string]any, error) {
	contents := make([]geminiContent, 0, len(req.Messages))
	toolNames := map[string]string{}
	system := strings.TrimSpace(req.System)

	appendContent := func(role string, parts []geminiPart) {
		if len(parts) == 0 {
			return
		}
		if len(contents) > 0 && contents[len(contents)-1].Role == role {
			contents[len(contents)-1].Parts = append(contents[len(contents)-1].Parts, parts...)
			return
		}
		contents = append(contents, geminiContent{Role: role, Parts: parts})
	}

	for _, message := range req.Messages {
		switch message.Role {
		case "system":
			if system != "" {
				system += "\n\n"
			}
			system += message.Content
		case "assistant":
			var parts []geminiPart
			if message.Content != "" {
				parts = append(parts, geminiPart{Text: message.Content})
			}
			for _, call := range message.ToolCalls {
				toolNames[call.ID] = call.Name
				parts = append(parts, geminiPart{
					ThoughtSignature: call.Signature,
					FunctionCall: &geminiFunctionCall{
						Name: call.Name,
						Args: call.Args,
					},
				})
			}
			appendContent("model", parts)
		case "tool":
			name := toolNames[message.ToolCallID]
			if name == "" {
				name = "tool_result"
			}
			appendContent("user", []geminiPart{{
				FunctionResponse: &geminiFunctionResponse{
					Name:     name,
					Response: map[string]any{"result": message.Content},
				},
			}})
		default:
			var parts []geminiPart
			if message.Content != "" {
				parts = append(parts, geminiPart{Text: message.Content})
			}
			for _, attachment := range message.Attachments {
				if attachment.Skipped != "" {
					parts = append(parts, geminiPart{Text: attachment.SkipNotice()})
					continue
				}
				if attachment.Text != "" {
					parts = append(parts, geminiPart{Text: attachment.PromptText()})
				}
				if attachment.IsImage() {
					parts = append(parts, geminiPart{
						InlineData: &geminiInlineData{MimeType: attachment.MIME, Data: attachment.Data},
					})
				}
			}
			appendContent("user", parts)
		}
	}

	body := map[string]any{"contents": contents}
	if system != "" {
		body["systemInstruction"] = map[string]any{
			"parts": []map[string]any{{"text": system}},
		}
	}
	if len(req.Tools) > 0 {
		declarations := make([]map[string]any, 0, len(req.Tools))
		for _, tool := range req.Tools {
			declaration := map[string]any{
				"name":        tool.Name,
				"description": tool.Description,
			}
			if tool.Schema != nil {
				declaration["parameters"] = tool.Schema
			}
			declarations = append(declarations, declaration)
		}
		body["tools"] = []map[string]any{{"functionDeclarations": declarations}}
	}
	if req.MaxTokens > 0 {
		body["generationConfig"] = map[string]any{"maxOutputTokens": req.MaxTokens}
	}
	return body, nil
}

type geminiResponse struct {
	Candidates []struct {
		Content struct {
			Parts []struct {
				Text             string `json:"text"`
				Thought          bool   `json:"thought"`
				ThoughtSignature string `json:"thoughtSignature"`
				FunctionCall     *struct {
					Name string         `json:"name"`
					Args map[string]any `json:"args"`
				} `json:"functionCall"`
			} `json:"parts"`
		} `json:"content"`
		FinishReason string `json:"finishReason"`
	} `json:"candidates"`
	UsageMetadata *struct {
		PromptTokenCount        int64 `json:"promptTokenCount"`
		CandidatesTokenCount    int64 `json:"candidatesTokenCount"`
		CachedContentTokenCount int64 `json:"cachedContentTokenCount"`
		ThoughtsTokenCount      int64 `json:"thoughtsTokenCount"`
	} `json:"usageMetadata"`
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
		Status  string `json:"status"`
	} `json:"error"`
}

func (g *Gemini) consume(ctx context.Context, body io.Reader, channel chan<- Chunk) {
	scanner := bufio.NewScanner(body)
	scanner.Buffer(make([]byte, 64*1024), 4*1024*1024)

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
		if payload == "" || payload == "[DONE]" {
			continue
		}

		var event geminiResponse
		if err := json.Unmarshal([]byte(payload), &event); err != nil {
			continue
		}
		if event.Error != nil {
			message := event.Error.Message
			code := event.Error.Code
			var streamErr error
			if code > 0 {
				streamErr = StatusError(g.provider.Name, code, message)
			} else {
				streamErr = StreamError(g.provider.Name, message, event.Error.Status)
			}
			send(ctx, channel, Chunk{Err: streamErr})
			return
		}
		if event.UsageMetadata != nil {
			usage = Usage{
				InputTokens:     event.UsageMetadata.PromptTokenCount,
				OutputTokens:    event.UsageMetadata.CandidatesTokenCount,
				CacheTokens:     event.UsageMetadata.CachedContentTokenCount,
				ReasoningTokens: event.UsageMetadata.ThoughtsTokenCount,
			}
			usageSeen = true
		}
		for _, candidate := range event.Candidates {
			for _, part := range candidate.Content.Parts {
				if part.FunctionCall != nil {
					call := ToolCall{
						ID:        id.New("call"),
						Name:      part.FunctionCall.Name,
						Args:      part.FunctionCall.Args,
						Signature: part.ThoughtSignature,
					}
					if call.Args == nil {
						call.Args = map[string]any{}
					}
					if !send(ctx, channel, Chunk{ToolCall: &call}) {
						return
					}
					continue
				}
				if part.Text == "" {
					continue
				}
				chunk := Chunk{Text: part.Text}
				if part.Thought {
					chunk = Chunk{Reasoning: part.Text}
				}
				if !send(ctx, channel, chunk) {
					return
				}
			}
		}
	}

	if ctx.Err() != nil {
		return
	}
	if !usageSeen {
		usage = Usage{}
	}
	send(ctx, channel, Chunk{Usage: &usage, Done: true})
}

var _ Client = (*Gemini)(nil)
