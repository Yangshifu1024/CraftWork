// 本文件提供“提供商限定”的模型引用。
//
// 模型 ID 只在单个提供商内唯一：两个 OpenAI 兼容网关可能都提供 gpt-5.5。
// 因此会话、设置与失败转移统一保存 providerID/modelID 形式的引用；适配器
// 请求仍然使用 ModelInfo.ID 这个原始模型 ID。

package model

import "strings"

// Ref 返回“提供商/模型”的稳定引用。
func Ref(providerID, modelID string) string {
	return providerID + "/" + modelID
}

// FindModelByRef 按完整引用查找模型。
//
// 引用在第一个斜杠处切分：提供商 ID 不含斜杠，模型 ID 可以包含斜杠，
// 例如 gateway/deepseek/deepseek-v4.1。
func FindModelByRef(ref string) (Provider, ModelInfo, bool) {
	providerID, modelID, ok := splitRef(ref)
	if !ok {
		return Provider{}, ModelInfo{}, false
	}
	provider, found := FindProvider(providerID)
	if !found {
		return Provider{}, ModelInfo{}, false
	}
	for _, item := range provider.Models {
		if item.ID == modelID {
			return provider, item, true
		}
	}
	return Provider{}, ModelInfo{}, false
}

// Resolve 解析引用或旧的原始模型 ID。
//
// 先按完整引用匹配，再按原始模型 ID 全局匹配，兼容旧版本自定义配置。
// 找不到时返回 false，调用方应提示用户重新选择模型，而不是静默回退。
func Resolve(refOrID string) (Provider, ModelInfo, bool) {
	value := strings.TrimSpace(refOrID)
	if value == "" {
		return Provider{}, ModelInfo{}, false
	}
	if provider, item, ok := FindModelByRef(value); ok {
		return provider, item, true
	}
	return FindModel(value)
}

// splitRef 把引用拆成提供商 ID 与模型 ID。
func splitRef(ref string) (string, string, bool) {
	index := strings.Index(ref, "/")
	if index <= 0 || index >= len(ref)-1 {
		return "", "", false
	}
	return ref[:index], ref[index+1:], true
}
