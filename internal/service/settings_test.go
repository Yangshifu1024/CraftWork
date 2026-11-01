package service

import (
	"testing"

	"craftwork/internal/model"
)

func TestNormalizeModelSettings(t *testing.T) {
	t.Cleanup(func() { model.SetCustomProviders(nil) })
	model.SetCustomProviders([]model.Provider{{
		ID: "test", Name: "Test", Kind: "openai-compatible",
		Endpoint: "https://example.com/v1",
		Models: []model.ModelInfo{
			{ID: "model-a", Name: "A", Provider: "test", ContextK: 32},
			{ID: "model-b", Name: "B", Provider: "test", ContextK: 32},
		},
	}})

	settings := Settings{DefaultModel: "model-a", FallbackModel: "model-b"}
	if changed := normalizeModelSettings(&settings); !changed {
		t.Fatal("旧模型 ID 应被规范化")
	}
	if settings.DefaultModel != "test/model-a" || settings.FallbackModel != "test/model-b" {
		t.Fatalf("规范化结果不符: %+v", settings)
	}

	// 引用与默认模型相同时，失败转移应被清空，避免切换到自己。
	settings = Settings{DefaultModel: "test/model-a", FallbackModel: "test/model-a"}
	normalizeModelSettings(&settings)
	if settings.FallbackModel != "" {
		t.Fatalf("失败转移不应与默认模型相同: %+v", settings)
	}

	// 指向已删除提供商的模型应清空，而不是回退到别的模型。
	settings = Settings{DefaultModel: "missing/provider", FallbackModel: "missing/provider"}
	normalizeModelSettings(&settings)
	if settings.DefaultModel != "" || settings.FallbackModel != "" {
		t.Fatalf("失效模型应被清空: %+v", settings)
	}
}
