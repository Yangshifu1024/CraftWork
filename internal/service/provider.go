package service

import (
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"

	"craftwork/internal/model"
	"craftwork/internal/secret"
)

// ModelView 是模型目录的展示结构。
type ModelView struct {
	ID           string             `json:"id"`
	Name         string             `json:"name"`
	Provider     string             `json:"provider"`
	ProviderName string             `json:"providerName"`
	ContextK     int                `json:"contextK"`
	Caps         model.Capabilities `json:"caps"`
	Default      bool               `json:"default"`
	Available    bool               `json:"available"`
}

// ProviderView 是提供商的展示结构，绝不包含明文密钥。
type ProviderView struct {
	ID          string      `json:"id"`
	Name        string      `json:"name"`
	Kind        string      `json:"kind"`
	Endpoint    string      `json:"endpoint"`
	Note        string      `json:"note,omitempty"`
	KeyRequired bool        `json:"keyRequired"`
	KeySource   string      `json:"keySource"`
	KeyMasked   string      `json:"keyMasked"`
	Available   bool        `json:"available"`
	Models      []ModelView `json:"models"`
}

// ConnectionResult 是连接测试结果。
type ConnectionResult struct {
	OK        bool   `json:"ok"`
	Message   string `json:"message"`
	LatencyMs int64  `json:"latencyMs"`
}

// ProviderService 管理提供商目录、密钥状态与连接测试。
type ProviderService struct {
	secrets *secret.Store
}

// NewProviderService 创建提供商服务。
func NewProviderService(secrets *secret.Store) *ProviderService {
	return &ProviderService{secrets: secrets}
}

// List 返回所有提供商与模型。密钥只返回来源与遮罩。
func (s *ProviderService) List() []ProviderView {
	providers := model.Providers()
	result := make([]ProviderView, 0, len(providers))
	for _, provider := range providers {
		keySource := s.secrets.Source(provider.ID)
		available := provider.Kind != "unsupported" && (!provider.KeyRequired || keySource != "")
		view := ProviderView{
			ID:          provider.ID,
			Name:        provider.Name,
			Kind:        provider.Kind,
			Endpoint:    provider.Endpoint,
			Note:        provider.Note,
			KeyRequired: provider.KeyRequired,
			KeySource:   keySource,
			KeyMasked:   s.secrets.Masked(provider.ID),
			Available:   available,
		}
		for _, item := range provider.Models {
			view.Models = append(view.Models, ModelView{
				ID:           item.ID,
				Name:         item.Name,
				Provider:     item.Provider,
				ProviderName: provider.Name,
				ContextK:     item.ContextK,
				Caps:         item.Caps,
				Default:      item.Default,
				Available:    available,
			})
		}
		result = append(result, view)
	}
	return result
}

// SetKey 保存提供商密钥。
func (s *ProviderService) SetKey(providerID, key string) error {
	if _, found := findProvider(providerID); !found {
		return errors.New("未知提供商")
	}
	if strings.TrimSpace(key) == "" {
		return errors.New("密钥不能为空")
	}
	return s.secrets.Set(providerID, key)
}

// ClearKey 删除提供商密钥。
func (s *ProviderService) ClearKey(providerID string) error {
	return s.secrets.Delete(providerID)
}

// Test 测试提供商连接。
func (s *ProviderService) Test(providerID string) (ConnectionResult, error) {
	provider, found := findProvider(providerID)
	if !found {
		return ConnectionResult{}, errors.New("未知提供商")
	}
	switch provider.Kind {
	case "mock":
		return ConnectionResult{OK: true, Message: "本地演示模型可用"}, nil
	case "unsupported":
		return ConnectionResult{}, errors.New("该提供商的适配器尚未接入")
	}

	key := strings.TrimSpace(s.secrets.Get(provider.ID))
	if provider.KeyRequired && key == "" {
		return ConnectionResult{}, errors.New("尚未配置密钥")
	}

	started := time.Now()
	request, err := http.NewRequest(http.MethodGet, strings.TrimRight(provider.Endpoint, "/")+"/models", nil)
	if err != nil {
		return ConnectionResult{}, err
	}
	if key != "" {
		request.Header.Set("Authorization", "Bearer "+key)
	}
	client := &http.Client{Timeout: 10 * time.Second}
	response, err := client.Do(request)
	latency := time.Since(started).Milliseconds()
	if err != nil {
		return ConnectionResult{LatencyMs: latency, Message: err.Error()}, nil
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return ConnectionResult{
			LatencyMs: latency,
			Message:   fmt.Sprintf("服务返回 HTTP %d", response.StatusCode),
		}, nil
	}
	return ConnectionResult{OK: true, Message: "连接正常", LatencyMs: latency}, nil
}

func findProvider(providerID string) (model.Provider, bool) {
	for _, provider := range model.Providers() {
		if provider.ID == providerID {
			return provider, true
		}
	}
	return model.Provider{}, false
}
