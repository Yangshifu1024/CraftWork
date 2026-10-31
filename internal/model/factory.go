package model

import (
	"fmt"
	"strings"
)

// KeyProvider 提供提供商密钥，由 internal/secret 实现。
type KeyProvider interface {
	Get(providerID string) string
}

// FallbackResolver 返回当前配置的失败转移模型。
type FallbackResolver func(modelID string) string

// Factory 根据模型标识创建客户端。
type Factory struct {
	keys     KeyProvider
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

// Resolve 查找模型；找不到时回退到默认模型。
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
	switch provider.Kind {
	case "mock":
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
		return nil, NewError(CodeProviderUnavailable, provider.Name,
			fmt.Sprintf("%s 的原生协议适配器尚未接入", provider.Name))
	}
}

// FallbackModel 返回可用于失败转移的模型 ID；没有配置或不可用时返回空字符串。
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
	switch provider.Kind {
	case "mock", "unsupported":
		return ""
	}
	if provider.KeyRequired {
		if _, err := f.key(provider); err != nil {
			return ""
		}
	}
	return info.ID
}

func (f *Factory) key(provider Provider) (string, error) {
	key := ""
	if f.keys != nil {
		key = strings.TrimSpace(f.keys.Get(provider.ID))
	}
	if provider.KeyRequired && key == "" {
		return "", NewError(CodeAuth, provider.Name,
			fmt.Sprintf("%s 尚未配置密钥，请在设置 → 模型 中粘贴 API Key", provider.Name))
	}
	return key, nil
}
