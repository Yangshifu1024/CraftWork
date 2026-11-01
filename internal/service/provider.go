package service

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"craftwork/internal/id"
	"craftwork/internal/model"
	"craftwork/internal/secret"
	"craftwork/internal/store"
)

const customProvidersKey = "model.custom_providers"

// ModelView 是模型目录的展示结构。
type ModelView struct {
	ID           string             `json:"id"`
	Name         string             `json:"name"`
	Provider     string             `json:"provider"`
	ProviderName string             `json:"providerName"`
	ContextK     int                `json:"contextK"`
	MaxTokens    int                `json:"maxTokens,omitempty"`
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
	Custom      bool        `json:"custom"`
	Overridden  bool        `json:"overridden"`
	Models      []ModelView `json:"models"`
}

// ProviderInput 是新增或编辑提供商的输入。
type ProviderInput struct {
	ID          string       `json:"id"`
	Name        string       `json:"name"`
	Kind        string       `json:"kind"`
	Endpoint    string       `json:"endpoint"`
	KeyRequired bool         `json:"keyRequired"`
	Models      []ModelInput `json:"models"`
}

// ModelInput 是模型配置输入。
type ModelInput struct {
	ID        string             `json:"id"`
	Name      string             `json:"name"`
	ContextK  int                `json:"contextK"`
	MaxTokens int                `json:"maxTokens,omitempty"`
	Caps      model.Capabilities `json:"caps"`
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
	db      *store.DB
	secrets *secret.Store

	mu     sync.RWMutex
	custom []model.Provider
}

// NewProviderService 创建提供商服务，并加载已保存的自定义提供商配置。
func NewProviderService(db *store.DB, secrets *secret.Store) *ProviderService {
	service := &ProviderService{db: db, secrets: secrets}
	var custom []model.Provider
	if db != nil {
		if err := db.GetSetting(customProvidersKey, &custom); err != nil {
			custom = nil
		}
	}
	service.custom = cloneProviders(custom)
	model.SetCustomProviders(service.custom)
	return service
}

// List 返回所有提供商与模型。密钥只返回来源与遮罩。
func (s *ProviderService) List() []ProviderView {
	s.mu.RLock()
	custom := cloneProviders(s.custom)
	s.mu.RUnlock()

	customSet := map[string]bool{}
	for _, provider := range custom {
		customSet[provider.ID] = true
	}
	providers := model.Providers()
	result := make([]ProviderView, 0, len(providers))
	for _, provider := range providers {
		result = append(result, s.viewFor(provider, customSet))
	}
	return result
}

func (s *ProviderService) viewFor(provider model.Provider, customSet map[string]bool) ProviderView {
	keySource := s.secrets.Source(provider.ID)
	available := provider.Kind != "unsupported" && (!provider.KeyRequired || keySource != "")
	custom := customSet[provider.ID]
	overridden := custom && isBuiltinProvider(provider.ID)
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
		Custom:      custom,
		Overridden:  overridden,
	}
	for _, item := range provider.Models {
		view.Models = append(view.Models, ModelView{
			ID:           item.ID,
			Name:         item.Name,
			Provider:     item.Provider,
			ProviderName: provider.Name,
			ContextK:     item.ContextK,
			MaxTokens:    item.MaxTokens,
			Caps:         item.Caps,
			Default:      item.Default,
			Available:    available,
		})
	}
	return view
}

// SaveProvider 新增或覆盖一个提供商配置，并持久化到本地设置。
func (s *ProviderService) SaveProvider(input ProviderInput) (ProviderView, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	provider, err := s.buildProvider(input, s.custom)
	if err != nil {
		return ProviderView{}, err
	}

	next := cloneProviders(s.custom)
	replaced := false
	for index := range next {
		if next[index].ID == provider.ID {
			next[index] = provider
			replaced = true
			break
		}
	}
	if !replaced {
		next = append(next, provider)
	}
	if err := s.db.SetSetting(customProvidersKey, next); err != nil {
		return ProviderView{}, fmt.Errorf("保存提供商配置: %w", err)
	}
	s.custom = next
	model.SetCustomProviders(next)

	customSet := map[string]bool{}
	for _, item := range next {
		customSet[item.ID] = true
	}
	return s.viewFor(provider, customSet), nil
}

// DeleteProvider 删除自定义配置；内置提供商删除自定义配置后恢复默认。
func (s *ProviderService) DeleteProvider(providerID string) error {
	s.mu.Lock()
	current := cloneProviders(s.custom)
	index := -1
	for i, provider := range current {
		if provider.ID == providerID {
			index = i
			break
		}
	}
	if index < 0 {
		s.mu.Unlock()
		if isBuiltinProvider(providerID) {
			return errors.New("内置提供商没有自定义配置")
		}
		return errors.New("未知提供商")
	}
	builtin := isBuiltinProvider(providerID)
	next := append(current[:index], current[index+1:]...)
	if err := s.db.SetSetting(customProvidersKey, next); err != nil {
		s.mu.Unlock()
		return fmt.Errorf("删除提供商配置: %w", err)
	}
	s.custom = next
	model.SetCustomProviders(next)
	s.mu.Unlock()

	if !builtin {
		_ = s.secrets.Delete(providerID)
	}
	return nil
}

func (s *ProviderService) buildProvider(input ProviderInput, custom []model.Provider) (model.Provider, error) {
	name := strings.TrimSpace(input.Name)
	if name == "" {
		return model.Provider{}, errors.New("提供商名称不能为空")
	}
	kind := strings.TrimSpace(input.Kind)
	if kind == "" {
		kind = "openai-compatible"
	}
	switch kind {
	case "openai", "openai-compatible", "anthropic", "gemini":
	default:
		return model.Provider{}, errors.New("不支持的提供商协议")
	}

	providerID := strings.TrimSpace(input.ID)
	if providerID == "" {
		providerID = id.New("custom")
	} else {
		if providerID == "local" {
			return model.Provider{}, errors.New("本地演示提供商不能修改")
		}
		if !isBuiltinProvider(providerID) && !containsProvider(custom, providerID) {
			return model.Provider{}, errors.New("未知提供商")
		}
	}

	endpoint := strings.TrimSpace(input.Endpoint)
	if endpoint == "" {
		for _, builtin := range model.BuiltinProviders() {
			if builtin.ID == providerID {
				endpoint = builtin.Endpoint
				break
			}
		}
	}
	if endpoint == "" {
		return model.Provider{}, errors.New("端点不能为空")
	}
	if err := validateEndpoint(endpoint); err != nil {
		return model.Provider{}, err
	}

	if len(input.Models) == 0 {
		return model.Provider{}, errors.New("至少需要配置一个模型")
	}
	reserved := map[string]bool{}
	for _, provider := range append(model.BuiltinProviders(), custom...) {
		if provider.ID == providerID {
			continue
		}
		for _, item := range provider.Models {
			reserved[item.ID] = true
		}
	}
	models := make([]model.ModelInfo, 0, len(input.Models))
	seen := map[string]bool{}
	for _, item := range input.Models {
		modelID := strings.TrimSpace(item.ID)
		if modelID == "" {
			return model.Provider{}, errors.New("模型 ID 不能为空")
		}
		if strings.ContainsAny(modelID, " \t\n") {
			return model.Provider{}, fmt.Errorf("模型 ID 不能包含空白：%s", modelID)
		}
		if seen[modelID] || reserved[modelID] {
			return model.Provider{}, fmt.Errorf("模型 ID 重复：%s", modelID)
		}
		seen[modelID] = true
		modelName := strings.TrimSpace(item.Name)
		if modelName == "" {
			modelName = modelID
		}
		contextK := item.ContextK
		if contextK <= 0 {
			contextK = 32
		}
		if contextK > 10000 {
			return model.Provider{}, fmt.Errorf("模型 %s 的上下文窗口过大", modelName)
		}
		maxTokens := item.MaxTokens
		if maxTokens < 0 || maxTokens > 200000 {
			return model.Provider{}, fmt.Errorf("模型 %s 的最大输出 token 不合法", modelName)
		}
		caps := item.Caps
		caps.Stream = true
		models = append(models, model.ModelInfo{
			ID:        modelID,
			Name:      modelName,
			Provider:  providerID,
			ContextK:  contextK,
			MaxTokens: maxTokens,
			Caps:      caps,
		})
	}

	note := "自定义提供商"
	if isBuiltinProvider(providerID) {
		note = "已覆盖内置提供商配置"
	}
	return model.Provider{
		ID:          providerID,
		Name:        name,
		Kind:        kind,
		Endpoint:    endpoint,
		KeyRequired: input.KeyRequired,
		Note:        note,
		Models:      models,
	}, nil
}

func validateEndpoint(endpoint string) error {
	parsed, err := url.Parse(endpoint)
	if err != nil || parsed.Host == "" || (parsed.Scheme != "https" && parsed.Scheme != "http") {
		return errors.New("端点必须是合法的 HTTP(S) 地址")
	}
	if parsed.Scheme == "http" {
		host := strings.Trim(parsed.Hostname(), "[]")
		switch host {
		case "localhost", "127.0.0.1", "::1":
		default:
			return errors.New("非本地端点必须使用 HTTPS")
		}
	}
	return nil
}

func isBuiltinProvider(providerID string) bool {
	for _, provider := range model.BuiltinProviders() {
		if provider.ID == providerID {
			return true
		}
	}
	return false
}

func containsProvider(providers []model.Provider, providerID string) bool {
	for _, provider := range providers {
		if provider.ID == providerID {
			return true
		}
	}
	return false
}

func cloneProviders(providers []model.Provider) []model.Provider {
	result := make([]model.Provider, 0, len(providers))
	for _, provider := range providers {
		cloned := provider
		cloned.Models = append([]model.ModelInfo(nil), provider.Models...)
		result = append(result, cloned)
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
