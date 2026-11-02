package service

import (
	"strings"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/model"
)

func TestCatalogMetadataAndProviders(t *testing.T) {
	service, _ := newProviderService(t)
	meta, err := service.CatalogMeta()
	if err != nil {
		t.Fatal(err)
	}
	if meta.ProviderCount < 100 || meta.ModelCount < 1000 {
		t.Fatalf("目录规模异常: %+v", meta)
	}
	if meta.License != "MIT" {
		t.Fatalf("许可证不符: %+v", meta)
	}

	providers, err := service.CatalogProviders("openrouter")
	if err != nil {
		t.Fatal(err)
	}
	if len(providers) != 1 || providers[0].ID != "openrouter" || providers[0].Added {
		t.Fatalf("目录提供商不符: %+v", providers)
	}
	if providers[0].Kind != "openai-compatible" || providers[0].ModelCount == 0 {
		t.Fatalf("目录提供商映射不符: %+v", providers[0])
	}

	models, err := service.CatalogModels("openrouter")
	if err != nil {
		t.Fatal(err)
	}
	if len(models) != providers[0].ModelCount {
		t.Fatalf("模型数量不符: %d != %d", len(models), providers[0].ModelCount)
	}
	if models[0].Ref != model.Ref("openrouter", models[0].ID) {
		t.Fatalf("目录模型引用不符: %+v", models[0])
	}
}

func TestImportFromCatalog(t *testing.T) {
	service, _ := newProviderService(t)
	models, err := service.CatalogModels("openrouter")
	if err != nil {
		t.Fatal(err)
	}
	if len(models) < 2 {
		t.Fatal("测试需要至少两个模型")
	}
	selected := []string{models[0].ID, models[1].ID}

	keyRequired := true
	view, err := service.ImportFromCatalog(CatalogImportInput{
		ProviderID:  "openrouter",
		ModelIDs:    selected,
		KeyRequired: &keyRequired,
	})
	if err != nil {
		t.Fatal(err)
	}
	if view.ID != "openrouter" || len(view.Models) != 2 || !view.KeyRequired {
		t.Fatalf("导入结果不符: %+v", view)
	}
	for _, item := range view.Models {
		if item.Ref == "" || !strings.HasPrefix(item.Ref, "openrouter/") {
			t.Fatalf("导入模型引用不符: %+v", item)
		}
		if _, _, found := model.FindModelByRef(item.Ref); !found {
			t.Fatalf("导入后模型不可用: %s", item.Ref)
		}
	}
	if providers := service.List(); len(providers) != 1 || providers[0].ID != "openrouter" {
		t.Fatalf("用户目录应只有导入的提供商: %+v", providers)
	}

	// 同一提供商不能重复导入，避免覆盖用户后续编辑。
	if _, err := service.ImportFromCatalog(CatalogImportInput{
		ProviderID: "openrouter", ModelIDs: selected,
	}); err == nil {
		t.Fatal("重复导入应被拒绝")
	}

	// 目录列表应标记为已添加。
	providers, err := service.CatalogProviders("openrouter")
	if err != nil {
		t.Fatal(err)
	}
	if !providers[0].Added {
		t.Fatal("已导入的提供商应标记为已添加")
	}
}

func TestImportFromCatalogValidation(t *testing.T) {
	service, _ := newProviderService(t)
	models, err := service.CatalogModels("openrouter")
	if err != nil || len(models) == 0 {
		t.Fatalf("读取目录模型失败: %v", err)
	}

	cases := []CatalogImportInput{
		{ProviderID: "definitely-missing", ModelIDs: []string{models[0].ID}},
		{ProviderID: "openrouter", ModelIDs: nil},
		{ProviderID: "openrouter", ModelIDs: []string{"definitely-missing"}},
	}
	for _, input := range cases {
		if _, err := service.ImportFromCatalog(input); err == nil {
			t.Fatalf("无效导入应被拒绝: %+v", input)
		}
	}

	// 覆盖协议时仍然只允许三种。
	if _, err := service.ImportFromCatalog(CatalogImportInput{
		ProviderID: "openrouter", ModelIDs: []string{models[0].ID}, Kind: "gemini",
	}); err == nil {
		t.Fatal("不支持的协议应被拒绝")
	}
}

func TestLookupCatalogModel(t *testing.T) {
	service, _ := newProviderService(t)
	models, err := service.CatalogModels("openrouter")
	if err != nil || len(models) == 0 {
		t.Fatalf("读取目录模型失败: %v", err)
	}
	item, err := service.LookupCatalogModel("openrouter", models[0].ID)
	if err != nil {
		t.Fatal(err)
	}
	if item.ID != models[0].ID || item.Provider != "openrouter" {
		t.Fatalf("补全结果不符: %+v", item)
	}
	if _, err := service.LookupCatalogModel("openrouter", "definitely-missing"); err == nil {
		t.Fatal("不存在的模型不应补全成功")
	}
}
