// 本文件负责维护模型提供商目录：内置目录、模型查找、默认模型选择，以及
// 用户自定义目录的合并与深拷贝，保证调用方拿到的切片不会改动共享状态。

package model

import "sync"

// builtinProviders 返回内置提供商目录。
//
// 目录中的每个提供商描述连接方式（Kind：mock / openai / openai-compatible /
// anthropic / gemini，由工厂据此选择客户端实现）、端点、是否需要密钥，以及
// 该提供商下每个模型的上下文窗口（ContextK，单位是千 token）与能力矩阵
// （Capabilities：流式、工具、图片、推理、结构化输出）。前端根据能力矩阵
// 禁用不支持的控件，因此这里的取值必须与适配器实际支持的能力一致。
func builtinProviders() []Provider {
	return []Provider{
		{
			// 本地演示提供商：不访问网络，用于端到端测试与视觉走查。
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
			// OpenAI 官方接口，使用 OpenAI 兼容协议，需要密钥。
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
			// DeepSeek 提供 OpenAI 兼容接口，只需替换端点与密钥。
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
			// 智谱 GLM 提供 OpenAI 兼容接口。
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
			// 通义千问通过 DashScope 的 OpenAI 兼容模式接入。
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
			// Ollama 是本地推理服务，默认监听本机端口，不需要密钥。
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
			// Anthropic 使用原生 Messages 协议，需要单独的适配器。
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
			// Google Gemini 使用原生 generateContent 协议。
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

// FindModel 在合并后的目录中按模型标识查找模型及其提供商。
//
// 返回值第三个表示是否找到；未找到时前两个返回值是零值。如果同一模型标识
// 出现在多个提供商下，返回第一个匹配项。
func FindModel(modelID string) (Provider, ModelInfo, bool) {
	// 顺序遍历所有提供商；目录规模很小，线性查找足够。
	for _, provider := range Providers() {
		for _, item := range provider.Models {
			if item.ID == modelID {
				return provider, item, true
			}
		}
	}
	return Provider{}, ModelInfo{}, false
}

// 目录的全局状态。读写自定义提供商时都先拷贝，避免调用方持有内部切片后
// 并发修改，也避免调用方把内部状态改坏。
var (
	// catalogMu 保护 customProviders 的读写锁。
	catalogMu sync.RWMutex
	// customProviders 是用户自定义提供商配置；与内置提供商 ID 相同时覆盖内置条目。
	customProviders []Provider
)

// BuiltinProviders 返回内置提供商目录，不包含用户自定义配置。
func BuiltinProviders() []Provider {
	return cloneProviders(builtinProviders())
}

// Providers 返回内置与用户自定义合并后的提供商目录。
//
// 自定义提供商与内置提供商 ID 相同时覆盖内置配置；密钥仍按提供商 ID
// 保存在 keys.json，不进入这份目录。
func Providers() []Provider {
	catalogMu.RLock()
	defer catalogMu.RUnlock()
	return mergeProviders(builtinProviders(), customProviders)
}

// CustomProviders 返回用户自定义提供商配置的副本。
func CustomProviders() []Provider {
	catalogMu.RLock()
	defer catalogMu.RUnlock()
	return cloneProviders(customProviders)
}

// SetCustomProviders 替换用户自定义提供商配置。
//
// 传入切片会被深拷贝后保存，调用方之后修改自己的切片不会影响目录。
func SetCustomProviders(providers []Provider) {
	catalogMu.Lock()
	defer catalogMu.Unlock()
	customProviders = cloneProviders(providers)
}

// mergeProviders 把自定义提供商合并进内置目录：ID 相同则整条替换（不是
// 字段级合并），ID 不存在则追加到末尾。
func mergeProviders(builtins, custom []Provider) []Provider {
	merged := cloneProviders(builtins)
	for _, provider := range custom {
		// 逐个比较 ID，命中后整条替换并停止查找。
		replaced := false
		for index := range merged {
			if merged[index].ID == provider.ID {
				merged[index] = cloneProvider(provider)
				replaced = true
				break
			}
		}
		if !replaced {
			merged = append(merged, cloneProvider(provider))
		}
	}
	return merged
}

// cloneProviders 深拷贝提供商列表；空输入返回空切片而不是 nil。
func cloneProviders(providers []Provider) []Provider {
	result := make([]Provider, 0, len(providers))
	for _, provider := range providers {
		result = append(result, cloneProvider(provider))
	}
	return result
}

// cloneProvider 拷贝单个提供商。Models 切片单独分配，其余字段都是值类型，
// 直接赋值即可。
func cloneProvider(provider Provider) Provider {
	cloned := provider
	cloned.Models = append([]ModelInfo(nil), provider.Models...)
	return cloned
}

// DefaultModel 返回目录中的默认模型。
//
// 优先选择标记了 Default 的模型；没有标记时退化为目录中第一个提供商的
// 第一个模型，保证只要目录非空就一定有返回值。
func DefaultModel() (Provider, ModelInfo) {
	for _, provider := range Providers() {
		for _, item := range provider.Models {
			if item.Default {
				return provider, item
			}
		}
	}
	// 没有显式默认模型时的兜底选择。
	providers := Providers()
	return providers[0], providers[0].Models[0]
}
