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
	if !strings.HasPrefix(view.ID, "custom_") {
		t.Fatalf("新增提供商应自动生成 ID: %+v", view)
	}
	if len(view.Models) != 1 || view.Models[0].ID != "proxy-model" || !view.Models[0].Caps.Stream {
		t.Fatalf("模型配置不符: %+v", view.Models)
	}
	if view.Models[0].Ref != view.ID+"/proxy-model" {
		t.Fatalf("模型引用不符: %+v", view.Models[0])
	}
	if view.Models[0].MaxTokens != 2048 {
		t.Fatalf("最大输出参数不符: %+v", view.Models[0])
	}
	if _, _, found := model.FindModelByRef(view.Models[0].Ref); !found {
		t.Fatal("自定义模型应立即可用")
	}

	if err := secrets.Set(view.ID, "test-key"); err != nil {
		t.Fatal(err)
	}
	reloaded := NewProviderService(service.db, secrets)
	if _, _, found := model.FindModelByRef(view.Models[0].Ref); !found {
		t.Fatal("重启后自定义模型应仍然可用")
	}
	if len(reloaded.List()) != len(service.List()) {
		t.Fatal("重启后提供商数量应一致")
	}

	if err := service.DeleteProvider(view.ID); err != nil {
		t.Fatal(err)
	}
	if _, _, found := model.FindModelByRef(view.Models[0].Ref); found {
		t.Fatal("删除后自定义模型不应存在")
	}
	if secrets.Get(view.ID) != "" {
		t.Fatal("删除提供商时应同时删除密钥")
	}
}

func TestUpdateProviderKeepsIdentity(t *testing.T) {
	service, _ := newProviderService(t)
	created, err := service.SaveProvider(ProviderInput{
		Name: "网关", Kind: "openai-compatible", Endpoint: "https://gateway.example.com/v1",
		Models: []ModelInput{{ID: "model-a", ContextK: 32}},
	})
	if err != nil {
		t.Fatal(err)
	}
	updated, err := service.SaveProvider(ProviderInput{
		ID: created.ID, Name: "网关（新）", Kind: "openai-compatible",
		Endpoint: "https://gateway.example.com/v1",
		Models:   []ModelInput{{ID: "model-a", ContextK: 64}, {ID: "model-b", ContextK: 128}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if updated.ID != created.ID || updated.Name != "网关（新）" || len(updated.Models) != 2 {
		t.Fatalf("更新结果不符: %+v", updated)
	}
	if providers := service.List(); len(providers) != 1 {
		t.Fatalf("更新不应新增提供商: %+v", providers)
	}
}

func TestDuplicateModelIDsAcrossProviders(t *testing.T) {
	service, _ := newProviderService(t)
	first, err := service.SaveProvider(ProviderInput{
		ID: "gateway-a", Name: "网关 A", Kind: "openai-compatible",
		Endpoint: "https://a.example.com/v1",
		Models:   []ModelInput{{ID: "shared-model", ContextK: 32}},
	})
	if err != nil {
		t.Fatal(err)
	}
	second, err := service.SaveProvider(ProviderInput{
		ID: "gateway-b", Name: "网关 B", Kind: "openai-compatible",
		Endpoint: "https://b.example.com/v1",
		Models:   []ModelInput{{ID: "shared-model", ContextK: 32}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, info, ok := model.FindModelByRef(first.Models[0].Ref); !ok || info.Provider != "gateway-a" {
		t.Fatalf("第一个同名模型解析错误: %+v", info)
	}
	if _, info, ok := model.FindModelByRef(second.Models[0].Ref); !ok || info.Provider != "gateway-b" {
		t.Fatalf("第二个同名模型解析错误: %+v", info)
	}
}

func TestSaveProviderValidation(t *testing.T) {
	service, _ := newProviderService(t)
	cases := []ProviderInput{
		{Name: "", Kind: "openai-compatible", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "Bad", Kind: "mock", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "Bad", Kind: "gemini", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "Http", Kind: "openai-compatible", Endpoint: "http://example.com", Models: []ModelInput{{ID: "a"}}},
		{Name: "NoModels", Kind: "openai-compatible", Endpoint: "https://example.com"},
		{Name: "Dup", Kind: "openai-compatible", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}, {ID: "a"}}},
		{ID: "Bad ID", Name: "Bad", Kind: "openai-compatible", Endpoint: "https://example.com", Models: []ModelInput{{ID: "a"}}},
	}
	for _, input := range cases {
		if _, err := service.SaveProvider(input); err == nil {
			t.Fatalf("无效配置应被拒绝: %+v", input)
		}
	}
}
