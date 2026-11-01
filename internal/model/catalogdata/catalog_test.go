package catalogdata

import (
	"strings"
	"testing"
)

func TestEmbeddedCatalogIsUsable(t *testing.T) {
	dataset, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	meta := dataset.Meta()
	if meta.Schema != SchemaVersion {
		t.Fatalf("目录版本不符: %d", meta.Schema)
	}
	if meta.Version == "" || meta.GeneratedAt == "" {
		t.Fatalf("目录缺少版本信息: %+v", meta)
	}
	if meta.ProviderCount < 100 || meta.ModelCount < 1000 {
		t.Fatalf("目录规模异常: %+v", meta)
	}
	if meta.License != "MIT" {
		t.Fatalf("许可证不符: %s", meta.License)
	}

	// 只允许三种协议。
	for _, provider := range dataset.AllProviders() {
		switch provider.Kind {
		case "openai", "anthropic", "openai-compatible":
		default:
			t.Fatalf("目录包含不支持的协议: %s %s", provider.ID, provider.Kind)
		}
		if strings.TrimSpace(provider.Endpoint) == "" {
			t.Fatalf("目录提供商缺少端点: %s", provider.ID)
		}
		for _, item := range provider.Models {
			if item.ContextK <= 0 {
				t.Fatalf("目录模型缺少上下文: %s/%s", provider.ID, item.ID)
			}
		}
	}

	// Google 原生协议不再支持，但 OpenRouter 等兼容网关里的 Gemini 模型保留。
	if _, found := dataset.Provider("google"); found {
		t.Fatal("目录不应包含 Google 原生协议提供商")
	}
	if _, found := dataset.Provider("google-vertex"); found {
		t.Fatal("目录不应包含 Google Vertex 提供商")
	}
	if _, found := dataset.Provider("openrouter"); !found {
		t.Fatal("目录应包含 OpenRouter")
	}
	if _, found := dataset.Provider("openai"); !found {
		t.Fatal("目录应包含 OpenAI")
	}
	if _, found := dataset.Provider("anthropic"); !found {
		t.Fatal("目录应包含 Anthropic")
	}
}

func TestCatalogLookupAndSearch(t *testing.T) {
	dataset, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	provider, found := dataset.Provider("openai")
	if !found {
		t.Fatal("未找到 OpenAI")
	}
	if len(provider.Models) == 0 {
		t.Fatal("OpenAI 没有模型")
	}
	first := provider.Models[0]
	item, found := dataset.Model("openai", first.ID)
	if !found || item.Name != first.Name {
		t.Fatalf("模型查找失败: %+v", item)
	}
	if _, found := dataset.Model("openai", "definitely-missing"); found {
		t.Fatal("不存在的模型不应找到")
	}

	matches := dataset.SearchProviders("OPENROUTER")
	if len(matches) != 1 || matches[0].ID != "openrouter" {
		t.Fatalf("搜索提供商失败: %+v", matches)
	}
	if len(dataset.SearchProviders("")) != len(dataset.AllProviders()) {
		t.Fatal("空关键字应返回全部提供商")
	}
}

func TestParseRejectsUnsupportedSchema(t *testing.T) {
	if _, err := Parse([]byte(`{"schema":99,"providers":[]}`)); err == nil {
		t.Fatal("不支持的目录版本应被拒绝")
	}
}
