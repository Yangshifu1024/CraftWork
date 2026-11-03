package service

import (
	"testing"

	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// newSettingsService 起一个落在临时目录的设置服务（审计用例的公共底座）。
func newSettingsService(t *testing.T) *SettingsService {
	t.Helper()
	db, err := store.Open(t.TempDir() + "/settings.db")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	bus := events.NewBus(func(events.Batch) {})
	t.Cleanup(bus.Close)
	return NewSettingsService(db, bus, policy.New(policy.LevelGraded))
}

func TestSaveApprovalLevelSwitchWritesAudit(t *testing.T) {
	svc := newSettingsService(t)

	// 默认档是分级；切到严格应留一条可追溯的审计记录。
	next, err := svc.Save(Settings{ApprovalLevel: "strict"})
	if err != nil {
		t.Fatal(err)
	}
	if next.ApprovalLevel != "strict" {
		t.Fatalf("保存后的档位不符: %s", next.ApprovalLevel)
	}
	entries, err := svc.db.ListAudit(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Fatalf("档位切换应写一条审计，实际 %d 条", len(entries))
	}
	entry := entries[0]
	if entry.Tool != "approval_level" {
		t.Errorf("审计 Tool 不符: %s", entry.Tool)
	}
	if entry.Detail == "" {
		t.Errorf("审计 Detail 应包含档位说明")
	}
	if entry.Scope != "user" {
		t.Errorf("审计 Scope 应为 user: %s", entry.Scope)
	}

	// 保存其他设置（档位未变）不应再写审计。
	if _, err := svc.Save(Settings{ApprovalLevel: "strict", FontSize: 14}); err != nil {
		t.Fatal(err)
	}
	entries, err = svc.db.ListAudit(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Fatalf("档位未变不应写审计，实际 %d 条", len(entries))
	}
}

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

func TestClampMaxIterations(t *testing.T) {
	// 合法值原样保留，0 表示不限制。
	for _, want := range []int{0, 1, 24, 300, 1000} {
		if got := clampMaxIterations(want); got != want {
			t.Fatalf("clampMaxIterations(%d) = %d", want, got)
		}
	}
	// 负数与超界值回退默认 300。
	for _, bad := range []int{-5, -1, 1001, 99999} {
		if got := clampMaxIterations(bad); got != 300 {
			t.Fatalf("clampMaxIterations(%d) 应回退 300，实际 %d", bad, got)
		}
	}
}
