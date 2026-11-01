// 本文件负责根据模型标识创建对应的客户端：查目录、取密钥、选择协议适配器、
// 套上重试包装，并判断某个模型能否作为失败转移目标。

package model

import (
	"fmt"
	"strings"
)

// KeyProvider 提供提供商密钥，由 internal/secret 实现。
type KeyProvider interface {
	// Get 返回指定提供商的密钥；没有配置时返回空字符串。
	Get(providerID string) string
}

// FallbackResolver 返回当前配置的失败转移模型。参数是原本要使用的模型标识，
// 返回值可以是空字符串表示没有配置转移目标。
type FallbackResolver func(modelID string) string

// Factory 根据模型标识创建客户端。
type Factory struct {
	// keys 提供各提供商的密钥，可为空（此时只支持不需要密钥的提供商）。
	keys KeyProvider
	// fallback 解析失败转移模型；为空表示不启用失败转移。
	fallback FallbackResolver
}

// NewFactory 创建模型客户端工厂。
func NewFactory(keys KeyProvider) *Factory {
	return &Factory{keys: keys}
}

// SetFallbackResolver 设置失败转移模型解析器。
func (f *Factory) SetFallbackResolver(resolver FallbackResolver) {
	f.fallback = resolver
}

// Resolve 查找模型；找不到时回退到默认模型，保证调用总能拿到可用的模型。
// 注意这里不校验提供商是否真的能创建客户端（例如缺少密钥），只负责选模型。
func Resolve(modelID string) (Provider, ModelInfo) {
	if provider, info, ok := FindModel(modelID); ok {
		return provider, info
	}
	return DefaultModel()
}

// Client 返回模型对应的客户端。
//
// 网络型客户端会被包装为带指数退避的重试客户端；重试只针对请求建立阶段的
// 429、5xx、超时与连接失败。
func (f *Factory) Client(modelID string) (Client, error) {
	provider, info := Resolve(modelID)
	// 按提供商的连接方式选择适配器；网络型客户端统一套上重试包装。
	switch provider.Kind {
	case "mock":
		// 演示客户端不访问网络，也不需要密钥。
		return NewMock(info.ID), nil
	case "openai", "openai-compatible":
		key, err := f.key(provider)
		if err != nil {
			return nil, err
		}
		client := NewOpenAICompat(provider, provider.Endpoint, key)
		return NewRetryClient(client, provider.Name, defaultRetryAttempts, defaultRetryBaseDelay), nil
	case "anthropic":
		key, err := f.key(provider)
		if err != nil {
			return nil, err
		}
		client := NewAnthropic(provider, provider.Endpoint, key)
		return NewRetryClient(client, provider.Name, defaultRetryAttempts, defaultRetryBaseDelay), nil
	case "gemini":
		key, err := f.key(provider)
		if err != nil {
			return nil, err
		}
		client := NewGemini(provider, provider.Endpoint, key)
		return NewRetryClient(client, provider.Name, defaultRetryAttempts, defaultRetryBaseDelay), nil
	default:
		// 未知连接方式或尚未接入的协议：给出可读错误而不是静默回退。
		return nil, NewError(CodeProviderUnavailable, provider.Name,
			fmt.Sprintf("%s 的原生协议适配器尚未接入", provider.Name))
	}
}

// FallbackModel 返回可用于失败转移的模型 ID；没有配置或不可用时返回空字符串。
// FallbackModel 返回可用于失败转移的模型 ID；没有配置或不可用时返回空字符串。
//
// 下列情况不视为可用目标：未设置解析器、解析结果为空、与当前模型相同、
// 模型不在目录中、目标仍是演示或未支持的类型、目标需要密钥但实际没有配置。
func (f *Factory) FallbackModel(modelID string) string {
	if f.fallback == nil {
		return ""
	}
	candidate := strings.TrimSpace(f.fallback(modelID))
	if candidate == "" || candidate == modelID {
		return ""
	}
	provider, info, ok := FindModel(candidate)
	if !ok {
		return ""
	}
	// 演示模型与尚未支持的类型不能作为真实故障的转移目标。
	switch provider.Kind {
	case "mock", "unsupported":
		return ""
	}
	// 需要密钥却取不到时，转移也只会失败，直接视为不可用。
	if provider.KeyRequired {
		if _, err := f.key(provider); err != nil {
			return ""
		}
	}
	return info.ID
}

// key 读取提供商密钥。需要密钥但取不到时返回 CodeAuth 错误，并在文案里
// 指出配置位置。
func (f *Factory) key(provider Provider) (string, error) {
	key := ""
	if f.keys != nil {
		key = strings.TrimSpace(f.keys.Get(provider.ID))
	}
	// 只有声明必须密钥的提供商才强制校验；Ollama 等本地服务允许空密钥。
	if provider.KeyRequired && key == "" {
		return "", NewError(CodeAuth, provider.Name,
			fmt.Sprintf("%s 尚未配置密钥，请在设置 → 模型 中粘贴 API Key", provider.Name))
	}
	return key, nil
}
