package model

import "testing"

func TestCustomProvidersOverrideAndRestore(t *testing.T) {
	t.Cleanup(func() { SetCustomProviders(nil) })

	custom := Provider{
		ID:          "openai",
		Name:        "OpenAI Proxy",
		Kind:        "openai-compatible",
		Endpoint:    "https://proxy.example.com/v1",
		KeyRequired: true,
		Models: []ModelInfo{{
			ID:       "proxy-model",
			Name:     "Proxy Model",
			Provider: "openai",
			ContextK: 64,
			Caps:     Capabilities{Stream: true, Tools: true},
		}},
	}
	SetCustomProviders([]Provider{custom})

	provider, info, found := FindModel("proxy-model")
	if !found || provider.Name != "OpenAI Proxy" || info.ContextK != 64 {
		t.Fatalf("自定义模型未生效: %+v %+v", provider, info)
	}
	if _, _, found := FindModel("gpt-4o"); found {
		t.Fatal("覆盖内置提供商后旧模型不应继续存在")
	}
	if _, defaultInfo := DefaultModel(); defaultInfo.ID != "local-demo" {
		t.Fatalf("默认模型不应被自定义配置改变: %s", defaultInfo.ID)
	}

	SetCustomProviders(nil)
	if _, _, found := FindModel("gpt-4o"); !found {
		t.Fatal("清除自定义配置后应恢复内置模型")
	}
}

func TestCustomProviderAppearsAfterBuiltins(t *testing.T) {
	t.Cleanup(func() { SetCustomProviders(nil) })
	SetCustomProviders([]Provider{{
		ID:       "custom_1",
		Name:     "Custom",
		Kind:     "openai-compatible",
		Endpoint: "https://example.com/v1",
		Models: []ModelInfo{{
			ID: "custom-model", Name: "Custom Model", Provider: "custom_1", ContextK: 32,
			Caps: Capabilities{Stream: true},
		}},
	}})
	providers := Providers()
	if len(providers) != len(BuiltinProviders())+1 {
		t.Fatalf("自定义提供商数量不符: %d", len(providers))
	}
	if providers[len(providers)-1].ID != "custom_1" {
		t.Fatalf("自定义提供商应追加在末尾: %+v", providers[len(providers)-1])
	}
}
