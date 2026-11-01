package model

import (
	"context"
	"encoding/json"
	"net/http"
	"net/url"
	"strings"
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
	target := joinAPIPath(g.endpoint, "/v1beta/models/"+url.PathEscape(req.Model)+":streamGenerateContent") + "?alt=sse"
	return streamSSE(ctx, streamRequest{
		provider: g.provider.Name,
		client:   g.http,
		endpoint: target,
		headers:  map[string]string{"x-goog-api-key": g.apiKey},
		body:     body,
	}, &geminiDecoder{provider: g.provider.Name})
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

// geminiDecoder 把 generateContent 的 SSE 事件翻译成片段。
type geminiDecoder struct {
	provider  string
	usage     Usage
	usageSeen bool
}

func (d *geminiDecoder) decode(payload []byte, emit emitFunc) (bool, error) {
	var event geminiResponse
	if err := json.Unmarshal(payload, &event); err != nil {
		return false, nil
	}
	if event.Error != nil {
		message := event.Error.Message
		if event.Error.Code > 0 {
			return false, StatusError(d.provider, event.Error.Code, message)
		}
		return false, StreamError(d.provider, message, event.Error.Status)
	}
	if event.UsageMetadata != nil {
		d.usage = Usage{
			InputTokens:     event.UsageMetadata.PromptTokenCount,
			OutputTokens:    event.UsageMetadata.CandidatesTokenCount,
			CacheTokens:     event.UsageMetadata.CachedContentTokenCount,
			ReasoningTokens: event.UsageMetadata.ThoughtsTokenCount,
		}
		d.usageSeen = true
	}
	for _, candidate := range event.Candidates {
		for _, part := range candidate.Content.Parts {
			if part.FunctionCall != nil {
				call := ToolCall{
					ID:        newCallID(),
					Name:      part.FunctionCall.Name,
					Args:      part.FunctionCall.Args,
					Signature: part.ThoughtSignature,
				}
				if call.Args == nil {
					call.Args = map[string]any{}
				}
				if !emit(Chunk{ToolCall: &call}) {
					return true, nil
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
			if !emit(chunk) {
				return true, nil
			}
		}
	}
	return false, nil
}

func (d *geminiDecoder) finish(emit emitFunc) {
	usage := d.usage
	if !d.usageSeen {
		usage = Usage{}
	}
	emit(Chunk{Usage: &usage, Done: true})
}

var _ Client = (*Gemini)(nil)
