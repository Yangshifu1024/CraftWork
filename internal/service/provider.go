package service

import (
	"errors"
	"fmt"
	"io"
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
	Code      string `json:"code,omitempty"`
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

// Warnings 返回密钥文件权限等问题，供界面在启动时提示用户。
func (s *ProviderService) Warnings() []string {
	return s.secrets.Warnings()
}

// ClearKey 删除提供商密钥。
func (s *ProviderService) ClearKey(providerID string) error {
	return s.secrets.Delete(providerID)
}

// Test 测试提供商连接。不同协议使用各自的列表接口与鉴权头。
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
		return ConnectionResult{}, fmt.Errorf("%s 尚未配置密钥，请先在设置 → 模型 中保存 API Key", provider.Name)
	}

	request, err := connectionRequest(provider, key)
	if err != nil {
		return ConnectionResult{}, err
	}

	started := time.Now()
	client := &http.Client{Timeout: 10 * time.Second}
	response, err := client.Do(request)
	latency := time.Since(started).Milliseconds()
	if err != nil {
		coded := model.TransportError(provider.Name, err)
		return ConnectionResult{
			Code:      string(coded.Code),
			Message:   connectionMessage(coded, provider),
			LatencyMs: latency,
		}, nil
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(response.Body, 8192))
		coded := model.HTTPError(provider.Name, response.StatusCode, body)
		return ConnectionResult{
			Code:      string(coded.Code),
			Message:   connectionMessage(coded, provider),
			LatencyMs: latency,
		}, nil
	}
	return ConnectionResult{OK: true, Message: "连接正常", LatencyMs: latency}, nil
}

// connectionRequest 按协议构造连接测试请求。密钥只放在请求头中，避免进入 URL 与日志。
func connectionRequest(provider model.Provider, key string) (*http.Request, error) {
	var target string
	switch provider.Kind {
	case "anthropic":
		target = joinServiceURL(provider.Endpoint, "/v1/models") + "?limit=1"
	case "gemini":
		target = joinServiceURL(provider.Endpoint, "/v1beta/models") + "?pageSize=1"
	default:
		target = strings.TrimRight(provider.Endpoint, "/") + "/models"
	}

	request, err := http.NewRequest(http.MethodGet, target, nil)
	if err != nil {
		return nil, err
	}
	switch provider.Kind {
	case "anthropic":
		request.Header.Set("x-api-key", key)
		request.Header.Set("anthropic-version", "2023-06-01")
	case "gemini":
		request.Header.Set("x-goog-api-key", key)
	default:
		if key != "" {
			request.Header.Set("Authorization", "Bearer "+key)
		}
	}
	return request, nil
}

// connectionMessage 把统一错误码转换成连接测试场景下可操作的提示。
func connectionMessage(err *model.Error, provider model.Provider) string {
	switch err.Code {
	case model.CodeAuth:
		return fmt.Sprintf("密钥无效或权限不足：请确认 API Key 属于 %s，并且允许调用模型。", provider.Name)
	case model.CodeRateLimit:
		return "请求被限流或额度不足：请稍后重试，或检查账户余额与配额。"
	case model.CodeTimeout:
		return "连接超时：请检查网络、代理或端点地址。"
	case model.CodeProviderUnavailable:
		return "服务暂时不可用：请稍后重试。"
	case model.CodeInvalidRequest:
		return fmt.Sprintf("端点或模型不可用：%s", err.Message)
	default:
		return err.Message
	}
}

// joinServiceURL 拼接端点与接口路径，避免重复携带 /v1 或 /v1beta。
func joinServiceURL(base, path string) string {
	base = strings.TrimRight(base, "/")
	path = strings.TrimPrefix(path, "/")
	segments := strings.SplitN(path, "/", 2)
	if len(segments) == 2 && strings.HasSuffix(base, "/"+segments[0]) {
		return base + "/" + segments[1]
	}
	return base + "/" + path
}

func findProvider(providerID string) (model.Provider, bool) {
	for _, provider := range model.Providers() {
		if provider.ID == providerID {
			return provider, true
		}
	}
	return model.Provider{}, false
}
