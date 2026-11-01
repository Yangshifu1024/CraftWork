package service

import (
	"path/filepath"
	"strings"
	"testing"

	"craftwork/internal/model"
	"craftwork/internal/secret"
	"craftwork/internal/store"
)

func newProviderService(t *testing.T) (*ProviderService, *secret.Store) {
	t.Helper()
	db, err := store.Open(filepath.Join(t.TempDir(), "test.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	secrets := secret.NewEmpty(t.TempDir())
	service := NewProviderService(db, secrets)
	t.Cleanup(func() { model.SetCustomProviders(nil) })
	return service, secrets
}

func TestSaveCustomProviderAndDelete(t *testing.T) {
	service, secrets := newProviderService(t)
	view, err := service.SaveProvider(ProviderInput{
		Name:        "Proxy",
		Kind:        "openai-compatible",
		Endpoint:    "https://proxy.example.com/v1",
		KeyRequired: true,
		Models: []ModelInput{{
			ID:        "proxy-model",
			Name:      "Proxy Model",
			ContextK:  64,
			MaxTokens: 2048,
			Caps:      model.Capabilities{Tools: true, Vision: true},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if !view.Custom || view.Overridden || !strings.HasPrefix(view.ID, "custom_") {
		t.Fatalf("自定义提供商标记不符: %+v", view)
	}
	if len(view.Models) != 1 || view.Models[0].ID != "proxy-model" || !view.Models[0].Caps.Stream {
		t.Fatalf("模型配置不符: %+v", view.Models)
	}
	if view.Models[0].MaxTokens != 2048 {
		t.Fatalf("最大输出参数不符: %+v", view.Models[0])
	}
	if _, _, found := model.FindModel("proxy-model"); !found {
		t.Fatal("自定义模型应立即可用")
	}

	if err := secrets.Set(view.ID, "test-key"); err != nil {
		t.Fatal(err)
	}
	reloaded := NewProviderService(service.db, secrets)
	if _, _, found := model.FindModel("proxy-model"); !found {
		t.Fatal("重启后自定义模型应仍然可用")
	}
	if len(reloaded.List()) != len(service.List()) {
		t.Fatal("重启后提供商数量应一致")
	}

	if err := service.DeleteProvider(view.ID); err != nil {
		t.Fatal(err)
	}
	if _, _, found := model.FindModel("proxy-model"); found {
		t.Fatal("删除后自定义模型不应存在")
	}
	if secrets.Get(view.ID) != "" {
		t.Fatal("删除自定义提供商时应同时删除密钥")
	}
}

func TestOverrideBuiltinProviderAndRestore(t *testing.T) {
	service, secrets := newProviderService(t)
	if err := secrets.Set("openai", "openai-key"); err != nil {
		t.Fatal(err)
	}
	view, err := service.SaveProvider(ProviderInput{
		ID:          "openai",
		Name:        "OpenAI Proxy",
		Kind:        "openai-compatible",
		Endpoint:    "https://proxy.example.com/v1",
		KeyRequired: true,
		Models: []ModelInput{{
			ID: "proxy-gpt", Name: "Proxy GPT", ContextK: 128,
			Caps: model.Capabilities{Tools: true},
		}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if !view.Custom || !view.Overridden {
		t.Fatalf("覆盖标记不符: %+v", view)
	}
	if _, _, found := model.FindModel("gpt-4o"); found {
		t.Fatal("覆盖后内置模型应被替换")
	}

	if err := service.DeleteProvider("openai"); err != nil {
		t.Fatal(err)
	}
	if _, _, found := model.FindModel("gpt-4o"); !found {
		t.Fatal("恢复后应重新出现内置模型")
	}
	if secrets.Get("openai") != "openai-key" {
		t.Fatal("恢复内置配置不应删除密钥")
	}
}

func TestSaveProviderValidation(t *testing.T) {
	service, _ := newProviderService(t)
	cases := []ProviderInput{
		{Name: "", Kind: "openai-compatible", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "Bad", Kind: "mock", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "Http", Kind: "openai-compatible", Endpoint: "http://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "NoModels", Kind: "openai-compatible", Endpoint: "https://example.com"},
		{Name: "DupBuiltin", Kind: "openai-compatible", Endpoint: "https://example.com", Models: []ModelInput{{ID: "gpt-4o"}}},
		{Name: "Dup", Kind: "openai-compatible", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}, {ID: "a"}}},
	}
	for _, input := range cases {
		if _, err := service.SaveProvider(input); err == nil {
			t.Fatalf("无效配置应被拒绝: %+v", input)
		}
	}
	if _, err := service.SaveProvider(ProviderInput{ID: "missing", Name: "X", Kind: "openai", Endpoint: "https://example.com", Models: []ModelInput{{ID: "x"}}}); err == nil {
		t.Fatal("未知提供商标识应被拒绝")
	}
}
