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
	// GetFor 优先使用目录给出的环境变量名读取密钥，便于导入提供商直接复用环境变量。
	GetFor(providerID string, envNames []string) string
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

// Client 返回模型对应的客户端。
//
// 网络型客户端会被包装为带指数退避的重试客户端；重试只针对请求建立阶段的
// 429、5xx、超时与连接失败。
func (f *Factory) Client(modelID string) (Client, error) {
	provider, _, ok := Resolve(modelID)
	if !ok {
		return nil, NewError(CodeProviderUnavailable, "模型服务",
			"模型不存在或已被删除，请在设置中重新选择模型")
	}
	// 按提供商的连接方式选择适配器；网络型客户端统一套上重试包装。
	switch provider.Kind {
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
	default:
		// 未知连接方式：给出可读错误而不是静默回退。
		return nil, NewError(CodeProviderUnavailable, provider.Name,
			fmt.Sprintf("%s 的协议不受支持", provider.Name))
	}
}

// FallbackModel 返回可用于失败转移的模型引用；没有配置或不可用时返回空字符串。
//
// 下列情况不视为可用目标：未设置解析器、解析结果为空、与当前模型相同、
// 模型不在用户目录中、目标需要密钥但实际没有配置。
func (f *Factory) FallbackModel(modelID string) string {
	if f.fallback == nil {
		return ""
	}
	candidate := strings.TrimSpace(f.fallback(modelID))
	if candidate == "" || candidate == modelID {
		return ""
	}
	provider, info, ok := Resolve(candidate)
	if !ok {
		return ""
	}
	ref := info.Ref()
	if ref == modelID {
		return ""
	}
	// 旧线程可能保存原始模型 ID；解析成引用后再比较，避免把同一个模型当成失败转移目标。
	if _, current, ok := Resolve(modelID); ok && current.Ref() == ref {
		return ""
	}
	// 需要密钥却取不到时，转移也只会失败，直接视为不可用。
	if provider.KeyRequired {
		if _, err := f.key(provider); err != nil {
			return ""
		}
	}
	return ref
}

// key 读取提供商密钥。需要密钥但取不到时返回 CodeAuth 错误，并在文案里
// 指出配置位置。
func (f *Factory) key(provider Provider) (string, error) {
	key := ""
	if f.keys != nil {
		key = strings.TrimSpace(f.keys.GetFor(provider.ID, provider.EnvVars))
	}
	// 只有声明必须密钥的提供商才强制校验；Ollama 等本地服务允许空密钥。
	if provider.KeyRequired && key == "" {
		return "", NewError(CodeAuth, provider.Name,
			fmt.Sprintf("%s 尚未配置密钥，请在设置 → 模型 中粘贴 API Key", provider.Name))
	}
	return key, nil
}
