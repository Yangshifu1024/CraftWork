// 本文件维护用户添加的提供商目录。
//
// 应用没有任何内置提供商：Providers 只返回用户保存的配置，首次启动时为空。
// 目录数据来自 models.dev，仅用于“从模型目录添加”和手工补全，不参与启动。

package model

import "sync"

// FindModel 在用户提供商目录中按原始模型 ID 查找模型及其提供商。
//
// 返回值第三个表示是否找到。如果同一模型 ID 出现在多个提供商下，返回第一个
// 匹配项；新数据应使用 FindModelByRef 或 Resolve，避免同名模型产生歧义。
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

// 目录的全局状态。读写提供商时都先拷贝，避免调用方持有内部切片后并发修改。
var (
	// catalogMu 保护 customProviders 的读写锁。
	catalogMu sync.RWMutex
	// customProviders 是用户保存的提供商配置；没有内置条目可以覆盖。
	customProviders []Provider
)

// Providers 返回用户保存的提供商目录。
func Providers() []Provider {
	catalogMu.RLock()
	defer catalogMu.RUnlock()
	return cloneProviders(customProviders)
}

// CustomProviders 返回用户提供商配置的副本。
func CustomProviders() []Provider {
	catalogMu.RLock()
	defer catalogMu.RUnlock()
	return cloneProviders(customProviders)
}

// SetCustomProviders 替换用户提供商配置。
//
// 传入切片会被深拷贝后保存，调用方之后修改自己的切片不会影响目录。
func SetCustomProviders(providers []Provider) {
	catalogMu.Lock()
	defer catalogMu.Unlock()
	customProviders = cloneProviders(providers)
}

// FindProvider 按标识查找用户提供商。
func FindProvider(providerID string) (Provider, bool) {
	for _, provider := range Providers() {
		if provider.ID == providerID {
			return provider, true
		}
	}
	return Provider{}, false
}

// cloneProviders 深拷贝提供商列表；空输入返回空切片而不是 nil。
func cloneProviders(providers []Provider) []Provider {
	result := make([]Provider, 0, len(providers))
	for _, provider := range providers {
		result = append(result, cloneProvider(provider))
	}
	return result
}

// cloneProvider 拷贝单个提供商。Models 与 EnvVars 切片单独分配。
func cloneProvider(provider Provider) Provider {
	cloned := provider
	cloned.EnvVars = append([]string(nil), provider.EnvVars...)
	cloned.Models = append([]ModelInfo(nil), provider.Models...)
	return cloned
}
