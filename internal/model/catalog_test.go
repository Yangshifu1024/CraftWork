package model

import "testing"

// provider 构造一个测试用的提供商。
func provider(id, name, endpoint string, models ...ModelInfo) Provider {
	for index := range models {
		models[index].Provider = id
	}
	return Provider{
		ID:          id,
		Name:        name,
		Kind:        "openai-compatible",
		Endpoint:    endpoint,
		KeyRequired: true,
		Models:      models,
	}
}

func TestProvidersOnlyReturnUserConfiguration(t *testing.T) {
	t.Cleanup(func() { SetCustomProviders(nil) })

	// 没有用户配置时目录必须为空，应用不应再有任何内置提供商。
	SetCustomProviders(nil)
	if providers := Providers(); len(providers) != 0 {
		t.Fatalf("首次启动不应有内置提供商: %+v", providers)
	}

	SetCustomProviders([]Provider{provider("gateway", "网关", "https://gateway.example.com/v1",
		ModelInfo{ID: "gateway-model", Name: "Gateway Model", ContextK: 64,
			Caps: Capabilities{Stream: true, Tools: true}})})

	item, info, found := FindModel("gateway-model")
	if !found || item.Name != "网关" || info.ContextK != 64 {
		t.Fatalf("用户模型未生效: %+v %+v", item, info)
	}
	if providers := Providers(); len(providers) != 1 || providers[0].ID != "gateway" {
		t.Fatalf("目录应只包含用户配置: %+v", providers)
	}
}

func TestFindModelByRefAndLegacyID(t *testing.T) {
	t.Cleanup(func() { SetCustomProviders(nil) })
	SetCustomProviders([]Provider{
		provider("gateway-a", "网关 A", "https://a.example.com/v1",
			ModelInfo{ID: "gpt-5.5", Name: "A 的 GPT-5.5", ContextK: 400}),
		provider("gateway-b", "网关 B", "https://b.example.com/v1",
			ModelInfo{ID: "gpt-5.5", Name: "B 的 GPT-5.5", ContextK: 400},
			ModelInfo{ID: "deepseek/v4", Name: "DeepSeek V4", ContextK: 1000}),
	})

	// 引用可以区分不同提供商下的同名模型。
	first, firstInfo, ok := FindModelByRef("gateway-a/gpt-5.5")
	if !ok || first.ID != "gateway-a" || firstInfo.Name != "A 的 GPT-5.5" {
		t.Fatalf("引用解析错误: %+v %+v", first, firstInfo)
	}
	second, secondInfo, ok := FindModelByRef("gateway-b/gpt-5.5")
	if !ok || second.ID != "gateway-b" || secondInfo.Name != "B 的 GPT-5.5" {
		t.Fatalf("引用解析错误: %+v %+v", second, secondInfo)
	}

	// 模型 ID 自身可以包含斜杠，引用按第一个斜杠切分。
	_, nested, ok := FindModelByRef("gateway-b/deepseek/v4")
	if !ok || nested.ID != "deepseek/v4" {
		t.Fatalf("带斜杠的模型 ID 解析错误: %+v", nested)
	}

	// 旧数据只有原始模型 ID，Resolve 仍然兼容，但不能区分同名模型。
	legacyProvider, legacyInfo, ok := Resolve("gpt-5.5")
	if !ok || legacyProvider.ID != "gateway-a" || legacyInfo.ID != "gpt-5.5" {
		t.Fatalf("旧模型 ID 兼容失败: %+v %+v", legacyProvider, legacyInfo)
	}

	// 引用优先于全局匹配，保证新数据不会串到另一个提供商。
	resolvedProvider, resolvedInfo, ok := Resolve("gateway-b/gpt-5.5")
	if !ok || resolvedProvider.ID != "gateway-b" || resolvedInfo.Ref() != "gateway-b/gpt-5.5" {
		t.Fatalf("引用优先级错误: %+v %+v", resolvedProvider, resolvedInfo)
	}
}

func TestResolveMissingModel(t *testing.T) {
	t.Cleanup(func() { SetCustomProviders(nil) })
	SetCustomProviders(nil)

	if _, _, ok := Resolve(""); ok {
		t.Fatal("空模型不应解析成功")
	}
	if _, _, ok := Resolve("missing-model"); ok {
		t.Fatal("不存在的模型不应回退到任何默认模型")
	}
}

func TestModelInfoRef(t *testing.T) {
	info := ModelInfo{ID: "gpt-5.5", Provider: "gateway"}
	if ref := info.Ref(); ref != "gateway/gpt-5.5" {
		t.Fatalf("模型引用不符: %s", ref)
	}
}
