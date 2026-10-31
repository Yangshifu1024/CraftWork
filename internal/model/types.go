// Package model 定义模型接入的统一接口、内置提供商目录与客户端实现。
package model

import "context"

// Capabilities 描述模型能力，前端据此禁用不支持的控件。
type Capabilities struct {
	Stream     bool `json:"stream"`
	Tools      bool `json:"tools"`
	Vision     bool `json:"vision"`
	Reasoning  bool `json:"reasoning"`
	Structured bool `json:"structured"`
}

// ModelInfo 是目录中的一个模型。
type ModelInfo struct {
	ID       string       `json:"id"`
	Name     string       `json:"name"`
	Provider string       `json:"provider"`
	ContextK int          `json:"contextK"`
	Caps     Capabilities `json:"caps"`
	Default  bool         `json:"default"`
}

// Provider 是一组共享密钥与端点的模型。
type Provider struct {
	ID          string      `json:"id"`
	Name        string      `json:"name"`
	Kind        string      `json:"kind"` // mock / openai / openai-compatible / unsupported
	Endpoint    string      `json:"endpoint"`
	KeyRequired bool        `json:"keyRequired"`
	Note        string      `json:"note,omitempty"`
	Models      []ModelInfo `json:"models"`
}

// ToolCall 是模型请求执行的一次工具调用。
type ToolCall struct {
	ID   string         `json:"id"`
	Name string         `json:"name"`
	Args map[string]any `json:"args"`
	// Signature 是 Gemini 思考模型的 thoughtSignature，用于在回填工具结果时
	// 保持上下文一致。其他提供商忽略该字段。
	Signature string `json:"signature,omitempty"`
}

// Message 是发送给模型的对话消息。
type Message struct {
	Role       string     `json:"role"` // system / user / assistant / tool
	Content    string     `json:"content"`
	ToolCalls  []ToolCall `json:"toolCalls,omitempty"`
	ToolCallID string     `json:"toolCallId,omitempty"`
}

// ToolSpec 是暴露给模型的工具定义。
type ToolSpec struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Schema      map[string]any `json:"schema"`
}

// Request 是一次模型调用。
type Request struct {
	Model     string
	Mode      string
	System    string
	Messages  []Message
	Tools     []ToolSpec
	MaxTokens int
}

// Usage 是模型返回的用量。
type Usage struct {
	InputTokens     int64 `json:"inputTokens"`
	OutputTokens    int64 `json:"outputTokens"`
	CacheTokens     int64 `json:"cacheTokens"`
	ReasoningTokens int64 `json:"reasoningTokens"`
}

// Chunk 是流式返回的一个片段。
type Chunk struct {
	Text      string    `json:"text,omitempty"`
	Reasoning string    `json:"reasoning,omitempty"`
	ToolCall  *ToolCall `json:"toolCall,omitempty"`
	Usage     *Usage    `json:"usage,omitempty"`
	Done      bool      `json:"done,omitempty"`
	// Err 只在 Go 内部流传，不会序列化给前端。
	Err error `json:"-"`
}

// Client 是模型客户端的统一接口。
type Client interface {
	Stream(ctx context.Context, req Request) (<-chan Chunk, error)
}
