// 本文件负责 Google Gemini generateContent 客户端。核心工作是把内部
// 消息翻译成 contents / parts，把工具翻译成 functionDeclarations，并将
// streamGenerateContent 的 SSE 响应翻译成统一片段。

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
	provider Provider     // 提供商标识与默认配置
	endpoint string       // 已去掉末尾斜杠的基础地址
	apiKey   string       // 通过 x-goog-api-key 头发送
	http     *http.Client // 使用默认传输设置
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
//
// 地址包含模型名，并追加 ?alt=sse 让服务端返回 SSE 而不是 JSON 数组；
// Gemini 本身不返回工具调用 ID，解码时会生成本地标识。
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

// geminiPart 是 contents 中的一段内容。
//
// 一段只承载一种载荷：普通文本、推理文本、函数调用、函数结果或内联
// 二进制数据，具体由哪个字段非空决定。
type geminiPart struct {
	Text             string                  `json:"text,omitempty"`             // 普通文本或推理文本
	Thought          bool                    `json:"thought,omitempty"`          // 为 true 时表示这段是推理内容
	ThoughtSignature string                  `json:"thoughtSignature,omitempty"` // 推理签名，回传时保持原样
	FunctionCall     *geminiFunctionCall     `json:"functionCall,omitempty"`     // 模型发起的工具调用
	FunctionResponse *geminiFunctionResponse `json:"functionResponse,omitempty"` // 工具执行结果
	InlineData       *geminiInlineData       `json:"inlineData,omitempty"`       // 内联图片等二进制数据
}

// geminiInlineData 是内联二进制数据，图片附件以 base64 形式放入。
type geminiInlineData struct {
	MimeType string `json:"mimeType"` // 数据的媒体类型，例如 image/png
	Data     string `json:"data"`     // base64 编码后的内容
}

// geminiFunctionCall 是模型发起的一次工具调用。
type geminiFunctionCall struct {
	Name string         `json:"name"`           // 工具名称
	Args map[string]any `json:"args,omitempty"` // 工具参数对象
}

// geminiFunctionResponse 是回填给模型的工具执行结果。
type geminiFunctionResponse struct {
	Name     string         `json:"name"`     // 对应的工具名称
	Response map[string]any `json:"response"` // 包在 result 字段中的执行结果
}

// geminiContent 是 contents 中的一条消息，role 只能是 user 或 model。
type geminiContent struct {
	Role  string       `json:"role,omitempty"` // user 或 model
	Parts []geminiPart `json:"parts"`          // 消息内容，按顺序排列
}

// requestBody 把内部请求翻译成 generateContent 的 JSON 结构。
//
// 角色映射：assistant 对应 model，工具结果以 user 身份回填。Gemini 的
// functionResponse 只认工具名，因此这里记录本次请求中调用 ID 到工具名
// 的对应关系，再按 ToolCallID 找回。连续同角色消息会被合并。
func (g *Gemini) requestBody(req Request) (map[string]any, error) {
	contents := make([]geminiContent, 0, len(req.Messages))
	toolNames := map[string]string{}
	system := strings.TrimSpace(req.System)

	// appendContent 追加一条消息；若上一条角色相同则合并 parts。
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
			// system 消息不放进 contents，而是合并到顶层 systemInstruction。
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
				// 记录调用 ID 与工具名的对应关系，供后续 tool 消息回填。
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
				// 找不到原名时用占位名，至少保证请求结构合法。
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
	// 输出上限在 Gemini 中位于 generationConfig，而不是顶层字段。
	if req.MaxTokens > 0 {
		body["generationConfig"] = map[string]any{"maxOutputTokens": req.MaxTokens}
	}
	return body, nil
}

// geminiResponse 是 streamGenerateContent 单条 SSE 事件的响应结构。
//
// 文本、推理与函数调用都放在 candidates[].content.parts 中；用量在
// usageMetadata，错误可能以顶层 error 字段返回。
type geminiResponse struct {
	// Candidates 是候选结果，通常只有一个候选。
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
	// UsageMetadata 是本次调用的 token 用量。
	UsageMetadata *struct {
		PromptTokenCount        int64 `json:"promptTokenCount"`
		CandidatesTokenCount    int64 `json:"candidatesTokenCount"`
		CachedContentTokenCount int64 `json:"cachedContentTokenCount"`
		ThoughtsTokenCount      int64 `json:"thoughtsTokenCount"`
	} `json:"usageMetadata"`
	// Error 是服务端返回的错误。
	Error *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
		Status  string `json:"status"`
	} `json:"error"`
}

// geminiDecoder 把 generateContent 的 SSE 事件翻译成片段。
type geminiDecoder struct {
	provider  string // 提供商名称，用于错误信息
	usage     Usage  // 汇总后的用量
	usageSeen bool   // 是否至少收到过一次用量信息
}

// decode 处理一条 Gemini SSE 事件。
//
// 每个 part 字段要么是函数调用，要么是文本；thought 为 true 的文本属于
// 推理内容，要转成 Reasoning 而不是正文。函数调用没有服务端 ID，
// 这里为每次调用生成本地标识，并保留 thoughtSignature 供回传使用。
func (d *geminiDecoder) decode(payload []byte, emit emitFunc) (bool, error) {
	var event geminiResponse
	if err := json.Unmarshal(payload, &event); err != nil {
		return false, nil
	}
	if event.Error != nil {
		message := event.Error.Message
		// 有 HTTP 状态码时按状态码归一，否则按流内错误处理。
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

// finish 在流结束时交付用量与结束标记。
//
// Gemini 没有独立的结束事件，读到响应体 EOF 后由这里收尾；
// 没有收到用量时交付空 Usage。
func (d *geminiDecoder) finish(emit emitFunc) {
	usage := d.usage
	if !d.usageSeen {
		usage = Usage{}
	}
	emit(Chunk{Usage: &usage, Done: true})
}

// 编译期确认 Gemini 实现了 Client 接口。
var _ Client = (*Gemini)(nil)
