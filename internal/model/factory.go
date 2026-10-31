package model

import (
	"fmt"
	"strings"
)

// KeyProvider 提供提供商密钥，由 internal/secret 实现。
type KeyProvider interface {
	Get(providerID string) string
}

// Factory 根据模型标识创建客户端。
type Factory struct {
	keys KeyProvider
}

// NewFactory 创建模型客户端工厂。
func NewFactory(keys KeyProvider) *Factory {
	return &Factory{keys: keys}
}

// Resolve 查找模型；找不到时回退到默认模型。
func Resolve(modelID string) (Provider, ModelInfo) {
	if provider, info, ok := FindModel(modelID); ok {
		return provider, info
	}
	return DefaultModel()
}

// Client 返回模型对应的客户端。
func (f *Factory) Client(modelID string) (Client, error) {
	provider, info := Resolve(modelID)
	switch provider.Kind {
	case "mock":
		return NewMock(info.ID), nil
	case "openai", "openai-compatible":
		key := ""
		if f.keys != nil {
			key = strings.TrimSpace(f.keys.Get(provider.ID))
		}
		if provider.KeyRequired && key == "" {
			return nil, fmt.Errorf("%s 尚未配置密钥", provider.Name)
		}
		return NewOpenAICompat(provider, provider.Endpoint, key), nil
	default:
		return nil, fmt.Errorf("%s 的原生协议适配器尚未接入", provider.Name)
	}
}
