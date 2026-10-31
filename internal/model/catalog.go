package model

// Providers 返回内置提供商目录。
//
// 这里只描述“有哪些提供商与模型”，不包含任何密钥。密钥由
// internal/secret 从 ~/.craftwork/keys.json 读取。
func Providers() []Provider {
	return []Provider{
		{
			ID:          "local",
			Name:        "本地演示",
			Kind:        "mock",
			Endpoint:    "",
			KeyRequired: false,
			Note:        "无需密钥，用于本地演示与端到端测试。",
			Models: []ModelInfo{
				{ID: "local-demo", Name: "Local Demo", Provider: "local", ContextK: 32,
					Caps: Capabilities{Stream: true, Tools: true, Reasoning: true}, Default: true},
			},
		},
		{
			ID:          "openai",
			Name:        "OpenAI",
			Kind:        "openai",
			Endpoint:    "https://api.openai.com/v1",
			KeyRequired: true,
			Models: []ModelInfo{
				{ID: "gpt-4o", Name: "GPT-4o", Provider: "openai", ContextK: 128,
					Caps: Capabilities{Stream: true, Tools: true, Vision: true, Structured: true}},
				{ID: "gpt-4o-mini", Name: "GPT-4o mini", Provider: "openai", ContextK: 128,
					Caps: Capabilities{Stream: true, Tools: true, Vision: true, Structured: true}},
			},
		},
		{
			ID:          "deepseek",
			Name:        "DeepSeek",
			Kind:        "openai-compatible",
			Endpoint:    "https://api.deepseek.com/v1",
			KeyRequired: true,
			Models: []ModelInfo{
				{ID: "deepseek-chat", Name: "DeepSeek Chat", Provider: "deepseek", ContextK: 64,
					Caps: Capabilities{Stream: true, Tools: true, Structured: true}},
				{ID: "deepseek-reasoner", Name: "DeepSeek Reasoner", Provider: "deepseek", ContextK: 64,
					Caps: Capabilities{Stream: true, Tools: true, Reasoning: true}},
			},
		},
		{
			ID:          "glm",
			Name:        "智谱 GLM",
			Kind:        "openai-compatible",
			Endpoint:    "https://open.bigmodel.cn/api/paas/v4",
			KeyRequired: true,
			Models: []ModelInfo{
				{ID: "glm-4.6", Name: "GLM-4.6", Provider: "glm", ContextK: 128,
					Caps: Capabilities{Stream: true, Tools: true, Reasoning: true, Structured: true}},
				{ID: "glm-4.5-air", Name: "GLM-4.5 Air", Provider: "glm", ContextK: 128,
					Caps: Capabilities{Stream: true, Tools: true}},
			},
		},
		{
			ID:          "qwen",
			Name:        "通义千问",
			Kind:        "openai-compatible",
			Endpoint:    "https://dashscope.aliyuncs.com/compatible-mode/v1",
			KeyRequired: true,
			Models: []ModelInfo{
				{ID: "qwen3-max", Name: "Qwen3 Max", Provider: "qwen", ContextK: 128,
					Caps: Capabilities{Stream: true, Tools: true, Structured: true}},
				{ID: "qwen3-coder-plus", Name: "Qwen3 Coder Plus", Provider: "qwen", ContextK: 256,
					Caps: Capabilities{Stream: true, Tools: true}},
			},
		},
		{
			ID:          "ollama",
			Name:        "Ollama",
			Kind:        "openai-compatible",
			Endpoint:    "http://127.0.0.1:11434/v1",
			KeyRequired: false,
			Note:        "本地推理服务，需要在 Ollama 中提前拉取模型。",
			Models: []ModelInfo{
				{ID: "qwen3:8b", Name: "Qwen3 8B", Provider: "ollama", ContextK: 32,
					Caps: Capabilities{Stream: true, Tools: true}},
			},
		},
		{
			ID:          "anthropic",
			Name:        "Anthropic",
			Kind:        "anthropic",
			Endpoint:    "https://api.anthropic.com",
			KeyRequired: true,
			Note:        "原生 Messages 协议，支持流式输出、工具调用与图片输入。",
			Models: []ModelInfo{
				{ID: "claude-sonnet-4", Name: "Claude Sonnet 4", Provider: "anthropic", ContextK: 200,
					Caps: Capabilities{Stream: true, Tools: true, Vision: true}},
			},
		},
		{
			ID:          "gemini",
			Name:        "Google Gemini",
			Kind:        "gemini",
			Endpoint:    "https://generativelanguage.googleapis.com",
			KeyRequired: true,
			Note:        "原生 generateContent 协议，支持流式输出、函数调用与图片输入。",
			Models: []ModelInfo{
				{ID: "gemini-2.5-pro", Name: "Gemini 2.5 Pro", Provider: "gemini", ContextK: 1000,
					Caps: Capabilities{Stream: true, Tools: true, Vision: true, Reasoning: true}},
			},
		},
	}
}

// FindModel 在目录中查找模型及其提供商。
func FindModel(modelID string) (Provider, ModelInfo, bool) {
	for _, provider := range Providers() {
		for _, item := range provider.Models {
			if item.ID == modelID {
				return provider, item, true
			}
		}
	}
	return Provider{}, ModelInfo{}, false
}

// DefaultModel 返回目录中的默认模型。
func DefaultModel() (Provider, ModelInfo) {
	for _, provider := range Providers() {
		for _, item := range provider.Models {
			if item.Default {
				return provider, item
			}
		}
	}
	providers := Providers()
	return providers[0], providers[0].Models[0]
}
