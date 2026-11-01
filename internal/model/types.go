// Package model 定义模型接入的统一接口与用户提供商目录。
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

// Cost 是模型价格，单位为美元 / 百万 token。
type Cost struct {
	InputPerMTok      float64 `json:"inputPerMTok"`
	OutputPerMTok     float64 `json:"outputPerMTok"`
	CacheReadPerMTok  float64 `json:"cacheReadPerMTok,omitempty"`
	CacheWritePerMTok float64 `json:"cacheWritePerMTok,omitempty"`
}

// ModelInfo 是用户提供商下的一个模型。
type ModelInfo struct {
	// ID 是调用提供商接口时使用的原始模型 ID。
	ID string `json:"id"`
	// Name 是展示名称。
	Name string `json:"name"`
	// Provider 是所属提供商标识。
	Provider string `json:"provider"`
	// ContextK 是上下文窗口，单位为千 token。
	ContextK int `json:"contextK"`
	// MaxTokens 是单次最大输出 token 数，0 表示不限制。
	MaxTokens int `json:"maxTokens,omitempty"`
	// Caps 是模型能力。
	Caps Capabilities `json:"caps"`
	// Cost 是价格信息；nil 表示未知。
	Cost *Cost `json:"cost,omitempty"`
	// Status 是模型状态：alpha、beta 或 deprecated。
	Status string `json:"status,omitempty"`
	// CanonicalID 是跨托管方的原始模型标识。
	CanonicalID string `json:"canonicalId,omitempty"`
	// ReleaseDate 是发布日期。
	ReleaseDate string `json:"releaseDate,omitempty"`
	// Source 是模型来源：catalog 或 manual。
	Source string `json:"source,omitempty"`
}

// Ref 返回“提供商/模型”的稳定引用，用于区分不同提供商下的同名模型。
func (m ModelInfo) Ref() string {
	return Ref(m.Provider, m.ID)
}

// Provider 是一组共享密钥与端点的模型，全部由用户添加。
type Provider struct {
	// ID 是提供商标识，同时也是密钥文件中的键。
	ID string `json:"id"`
	// Name 是展示名称。
	Name string `json:"name"`
	// Kind 是协议类型：openai、anthropic 或 openai-compatible。
	Kind string `json:"kind"`
	// Endpoint 是接口端点。
	Endpoint string `json:"endpoint"`
	// KeyRequired 表示调用该提供商必须配置密钥。
	KeyRequired bool `json:"keyRequired"`
	// Note 是用户可读的配置说明。
	Note string `json:"note,omitempty"`
	// Source 是配置来源：catalog 或 manual。
	Source string `json:"source,omitempty"`
	// CatalogID 是对应的 models.dev 提供商标识。
	CatalogID string `json:"catalogId,omitempty"`
	// CatalogVersion 是导入时使用的目录版本。
	CatalogVersion string `json:"catalogVersion,omitempty"`
	// EnvVars 是目录给出的可用环境变量名。
	EnvVars []string `json:"envVars,omitempty"`
	// Models 是该提供商下的模型。
	Models []ModelInfo `json:"models"`
}

// ToolCall 是模型请求执行的一次工具调用。
type ToolCall struct {
	ID   string         `json:"id"`
	Name string         `json:"name"`
	Args map[string]any `json:"args"`
}

// Message 是发送给模型的对话消息。
type Message struct {
	Role       string     `json:"role"` // system / user / assistant / tool
	Content    string     `json:"content"`
	ToolCalls  []ToolCall `json:"toolCalls,omitempty"`
	ToolCallID string     `json:"toolCallId,omitempty"`
	// Attachments 是用户消息携带的附件内容。Text 为文本内容，Data 为图片
	// 的 base64 数据；Skipped 表示未读取的原因。
	Attachments []Attachment `json:"attachments,omitempty"`
}

// Attachment 是随用户消息发送给模型的附件。
type Attachment struct {
	Name    string `json:"name"`
	MIME    string `json:"mime,omitempty"`
	Text    string `json:"text,omitempty"`
	Data    string `json:"data,omitempty"`
	Size    int64  `json:"size,omitempty"`
	Skipped string `json:"skipped,omitempty"`
}

// ToolSpec 是暴露给模型的工具定义。
type ToolSpec struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Schema      map[string]any `json:"schema"`
}

// SystemBlock 是系统提示词的一个分块。
//
// 系统提示词按「变化频率」分成多个块：几乎不变的稳定内容（身份与行为契约）
// 在前，偶尔变化的动态内容（模式规则、环境信息）在后。变化频率越高的块越靠后，
// 提供商的前缀缓存就能在动态块变化时继续命中前面的稳定块。
type SystemBlock struct {
	// Content 是块的正文；调用方负责保证内容干净（无多余首尾空白）。
	Content string
	// CacheControl 表示该块携带提示缓存标记。Anthropic 协议会写入
	// cache_control: ephemeral；OpenAI 兼容协议依赖自动前缀缓存，忽略该字段。
	CacheControl bool
}

// SingleSystem 把一段系统提示词包装成单块、不带缓存标记的形式，
// 供摘要、标题等辅助请求使用。
func SingleSystem(text string) []SystemBlock {
	if text == "" {
		return nil
	}
	return []SystemBlock{{Content: text}}
}

// Request 是一次模型调用。
type Request struct {
	Model     string
	Mode      string
	System    []SystemBlock
	Messages  []Message
	Tools     []ToolSpec
	MaxTokens int
}

// Usage 是模型返回的用量。
type Usage struct {
	InputTokens      int64 `json:"inputTokens"`
	OutputTokens     int64 `json:"outputTokens"`
	CacheTokens      int64 `json:"cacheTokens"`
	CacheWriteTokens int64 `json:"cacheWriteTokens,omitempty"`
	ReasoningTokens  int64 `json:"reasoningTokens"`
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
