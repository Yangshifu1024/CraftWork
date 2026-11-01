// 本文件是模型提供商相关的前端绑定：列出提供商与模型、维护自定义
// 配置、保存或清除密钥、测试连接。密钥只保存在 internal/secret，
// 对前端只返回来源与遮罩后的文本，绝不返回明文。

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

// customProvidersKey 是自定义提供商配置在设置表中的键名。
const customProvidersKey = "model.custom_providers"

// ModelView 是模型目录的展示结构。
type ModelView struct {
	// ID 是调用提供商接口时使用的原始模型 ID。
	ID string `json:"id"`
	// Ref 是“提供商/模型”的稳定引用，会话与设置使用它区分同名模型。
	Ref string `json:"ref"`
	// Name 是模型展示名称。
	Name string `json:"name"`
	// Provider 是所属提供商标识。
	Provider string `json:"provider"`
	// ProviderName 是所属提供商展示名称。
	ProviderName string `json:"providerName"`
	// ContextK 是上下文窗口大小，单位为千 token。
	ContextK int `json:"contextK"`
	// MaxTokens 是单次最大输出 token 数，0 表示不限制。
	MaxTokens int `json:"maxTokens,omitempty"`
	// Caps 声明模型支持的能力，例如流式输出与工具调用。
	Caps model.Capabilities `json:"caps"`
	// Cost 是模型价格；nil 表示未知。
	Cost *model.Cost `json:"cost,omitempty"`
	// Status 是模型状态：alpha、beta 或 deprecated。
	Status string `json:"status,omitempty"`
	// CanonicalID 是跨托管方的原始模型标识。
	CanonicalID string `json:"canonicalId,omitempty"`
	// ReleaseDate 是发布日期。
	ReleaseDate string `json:"releaseDate,omitempty"`
	// Available 表示模型当前可用（提供商可用且密钥已配置）。
	Available bool `json:"available"`
}

// ProviderView 是提供商的展示结构，绝不包含明文密钥。
type ProviderView struct {
	// ID 是提供商标识。
	ID string `json:"id"`
	// Name 是提供商展示名称。
	Name string `json:"name"`
	// Kind 是协议类型：openai、anthropic 或 openai-compatible。
	Kind string `json:"kind"`
	// Endpoint 是接口端点。
	Endpoint string `json:"endpoint"`
	// KeyRequired 表示调用该提供商必须配置密钥。
	KeyRequired bool `json:"keyRequired"`
	// KeySource 是密钥来源，例如环境变量或本地文件。
	KeySource string `json:"keySource"`
	// KeyMasked 是遮罩后的密钥，仅用于界面展示。
	KeyMasked string `json:"keyMasked"`
	// Available 表示提供商当前是否可用。
	Available bool `json:"available"`
	// Models 是该提供商下的模型列表。
	Models []ModelView `json:"models"`
}

// ProviderInput 是新增或编辑提供商的输入。
type ProviderInput struct {
	// ID 为空时自动生成；非空时表示更新已有提供商。
	ID string `json:"id"`
	// Name 是提供商名称，不能为空。
	Name string `json:"name"`
	// Kind 是协议类型，为空时按 openai-compatible 处理。
	Kind string `json:"kind"`
	// Endpoint 是接口地址，不能为空。
	Endpoint string `json:"endpoint"`
	// KeyRequired 表示是否必须配置密钥。
	KeyRequired bool `json:"keyRequired"`
	// Models 是至少一个模型配置。
	Models []ModelInput `json:"models"`
}

// ModelInput 是模型配置输入。
type ModelInput struct {
	// ID 是模型标识，不能包含空白。
	ID string `json:"id"`
	// Name 是展示名称，为空时使用 ID。
	Name string `json:"name"`
	// ContextK 是上下文窗口，单位千 token，默认 32。
	ContextK int `json:"contextK"`
	// MaxTokens 是最大输出 token 数，0 表示不限制。
	MaxTokens int `json:"maxTokens,omitempty"`
	// Caps 是模型能力声明。
	Caps model.Capabilities `json:"caps"`
	// Cost 是价格信息，可为空。
	Cost *model.Cost `json:"cost,omitempty"`
	// Status 是模型状态。
	Status string `json:"status,omitempty"`
	// CanonicalID 是跨托管方的原始模型标识。
	CanonicalID string `json:"canonicalId,omitempty"`
	// ReleaseDate 是发布日期。
	ReleaseDate string `json:"releaseDate,omitempty"`
}

// ConnectionResult 是连接测试结果。
type ConnectionResult struct {
	// OK 表示连接是否成功。
	OK bool `json:"ok"`
	// Message 是给用户看的说明。
	Message string `json:"message"`
	// LatencyMs 是请求耗时（毫秒）。
	LatencyMs int64 `json:"latencyMs"`
	// Code 是失败时的统一错误码。
	Code string `json:"code,omitempty"`
}

// ProviderService 管理提供商目录、密钥状态与连接测试。
type ProviderService struct {
	// db 是自定义提供商配置与设置的存储入口。
	db *store.DB
	// secrets 管理密钥文件，前端永远读不到明文。
	secrets *secret.Store

	// mu 保护 custom 列表的读写。
	mu sync.RWMutex
	// custom 是当前生效的自定义提供商配置。
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

// List 返回用户添加的提供商与模型。密钥只返回来源与遮罩。
func (s *ProviderService) List() []ProviderView {
	s.mu.RLock()
	providers := cloneProviders(s.custom)
	s.mu.RUnlock()

	result := make([]ProviderView, 0, len(providers))
	for _, provider := range providers {
		result = append(result, s.viewFor(provider))
	}
	return result
}

// viewFor 把内部提供商结构转换成展示视图，并计算可用性与密钥掩码。
func (s *ProviderService) viewFor(provider model.Provider) ProviderView {
	keySource := s.secrets.SourceFor(provider.ID, provider.EnvVars)
	available := !provider.KeyRequired || keySource != ""
	view := ProviderView{
		ID:          provider.ID,
		Name:        provider.Name,
		Kind:        provider.Kind,
		Endpoint:    provider.Endpoint,
		KeyRequired: provider.KeyRequired,
		KeySource:   keySource,
		KeyMasked:   s.secrets.MaskedFor(provider.ID, provider.EnvVars),
		Available:   available,
	}
	for _, item := range provider.Models {
		view.Models = append(view.Models, ModelView{
			ID:           item.ID,
			Ref:          item.Ref(),
			Name:         item.Name,
			Provider:     item.Provider,
			ProviderName: provider.Name,
			ContextK:     item.ContextK,
			MaxTokens:    item.MaxTokens,
			Caps:         item.Caps,
			Cost:         item.Cost,
			Status:       item.Status,
			CanonicalID:  item.CanonicalID,
			ReleaseDate:  item.ReleaseDate,
			Available:    available,
		})
	}
	return view
}

// SaveProvider 新增或更新一个提供商配置，并持久化到本地设置。
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
	if err := s.persist(next); err != nil {
		return ProviderView{}, err
	}
	return s.viewFor(provider), nil
}

// DeleteProvider 删除用户配置并同时删除密钥。
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
		return errors.New("未知提供商")
	}
	next := append(current[:index], current[index+1:]...)
	if err := s.persist(next); err != nil {
		s.mu.Unlock()
		return err
	}
	s.mu.Unlock()

	_ = s.secrets.Delete(providerID)
	s.clearModelSettings(providerID)
	return nil
}

// persist 保存提供商列表并同步到进程内目录。调用方必须持有 s.mu 写锁。
func (s *ProviderService) persist(providers []model.Provider) error {
	if err := s.db.SetSetting(customProvidersKey, providers); err != nil {
		return fmt.Errorf("保存提供商配置: %w", err)
	}
	s.custom = providers
	model.SetCustomProviders(providers)
	return nil
}

// buildProvider 校验输入并构造提供商配置。
func (s *ProviderService) buildProvider(input ProviderInput, custom []model.Provider) (model.Provider, error) {
	name := strings.TrimSpace(input.Name)
	if name == "" {
		return model.Provider{}, errors.New("提供商名称不能为空")
	}
	kind := strings.TrimSpace(input.Kind)
	if kind == "" {
		kind = "openai-compatible"
	}
	if !supportedProviderKind(kind) {
		return model.Provider{}, errors.New("不支持的提供商协议")
	}

	providerID := strings.TrimSpace(input.ID)
	if providerID == "" {
		providerID = id.New("custom")
	} else if !validProviderID(providerID) {
		return model.Provider{}, errors.New("提供商 ID 只能包含小写字母、数字、点、下划线和短横线")
	}

	endpoint := strings.TrimSpace(input.Endpoint)
	if endpoint == "" {
		return model.Provider{}, errors.New("端点不能为空")
	}
	if err := validateEndpoint(endpoint); err != nil {
		return model.Provider{}, err
	}

	if len(input.Models) == 0 {
		return model.Provider{}, errors.New("至少需要配置一个模型")
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
		// 只限制同一提供商内部重复；不同提供商可以出现同名模型，
		// 由“提供商/模型”引用区分。
		if seen[modelID] {
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
		if contextK > maxContextK {
			return model.Provider{}, fmt.Errorf("模型 %s 的上下文窗口过大", modelName)
		}
		maxTokens := item.MaxTokens
		if maxTokens < 0 || maxTokens > maxOutputTokens {
			return model.Provider{}, fmt.Errorf("模型 %s 的最大输出 token 不合法", modelName)
		}
		caps := item.Caps
		caps.Stream = true
		models = append(models, model.ModelInfo{
			ID:          modelID,
			Name:        modelName,
			Provider:    providerID,
			ContextK:    contextK,
			MaxTokens:   maxTokens,
			Caps:        caps,
			Cost:        item.Cost,
			Status:      strings.TrimSpace(item.Status),
			CanonicalID: strings.TrimSpace(item.CanonicalID),
			ReleaseDate: strings.TrimSpace(item.ReleaseDate),
			Source:      "manual",
		})
	}

	// 更新已有提供商时继承目录来源信息，避免编辑一次就丢掉导入时的元数据。
	source := "manual"
	catalogID := ""
	catalogVersion := ""
	envVars := []string(nil)
	if existing, found := findCustomProvider(custom, providerID); found {
		source = existing.Source
		catalogID = existing.CatalogID
		catalogVersion = existing.CatalogVersion
		envVars = append([]string(nil), existing.EnvVars...)
	}

	return model.Provider{
		ID:             providerID,
		Name:           name,
		Kind:           kind,
		Endpoint:       endpoint,
		KeyRequired:    input.KeyRequired,
		Source:         source,
		CatalogID:      catalogID,
		CatalogVersion: catalogVersion,
		EnvVars:        envVars,
		Models:         models,
	}, nil
}

// clearModelSettings 在删除提供商后清理指向它的默认模型与失败转移模型。
func (s *ProviderService) clearModelSettings(providerID string) {
	var settings Settings
	if err := s.db.GetSetting(settingsKey, &settings); err != nil {
		return
	}
	changed := false
	if refersToProvider(settings.DefaultModel, providerID) {
		settings.DefaultModel = ""
		changed = true
	}
	if refersToProvider(settings.FallbackModel, providerID) {
		settings.FallbackModel = ""
		changed = true
	}
	if changed {
		_ = s.db.SetSetting(settingsKey, settings)
	}
}

// refersToProvider 判断模型引用是否属于指定提供商。
func refersToProvider(refOrID, providerID string) bool {
	if refOrID == "" {
		return false
	}
	if strings.HasPrefix(refOrID, providerID+"/") {
		return true
	}
	provider, _, ok := model.Resolve(refOrID)
	return ok && provider.ID == providerID
}

// validProviderID 校验提供商标识，保证它能安全地用作密钥键与环境变量名。
func validProviderID(providerID string) bool {
	if len(providerID) < 2 || len(providerID) > 64 {
		return false
	}
	for index, char := range providerID {
		switch {
		case char >= 'a' && char <= 'z':
		case char >= '0' && char <= '9':
		case char == '_' || char == '-' || char == '.':
			if index == 0 {
				return false
			}
		default:
			return false
		}
	}
	return true
}

// supportedProviderKind 判断协议是否在当前支持范围内。
func supportedProviderKind(kind string) bool {
	switch kind {
	case "openai", "anthropic", "openai-compatible":
		return true
	default:
		return false
	}
}

// findCustomProvider 在给定列表中查找提供商。
func findCustomProvider(providers []model.Provider, providerID string) (model.Provider, bool) {
	for _, provider := range providers {
		if provider.ID == providerID {
			return provider, true
		}
	}
	return model.Provider{}, false
}

// 上下文与输出上限需要覆盖 models.dev 中的大模型；超出范围视为配置错误。
const (
	maxContextK     = 100000
	maxOutputTokens = 4000000
)

// validateEndpoint 校验端点必须是 HTTP(S) 地址。
//
// 除 localhost、127.0.0.1、::1 之外，一律要求 HTTPS，
// 避免明文传输密钥与对话内容。
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

// cloneProviders 复制提供商列表与其模型切片，避免调用方修改共享配置。
func cloneProviders(providers []model.Provider) []model.Provider {
	result := make([]model.Provider, 0, len(providers))
	for _, provider := range providers {
		cloned := provider
		cloned.EnvVars = append([]string(nil), provider.EnvVars...)
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

	key := strings.TrimSpace(s.secrets.GetFor(provider.ID, provider.EnvVars))
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
		// 网络错误统一映射成模型错误码，前端只处理稳定的错误类型。
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

// findProvider 在当前生效的提供商目录中查找指定标识。
func findProvider(providerID string) (model.Provider, bool) {
	for _, provider := range model.Providers() {
		if provider.ID == providerID {
			return provider, true
		}
	}
	return model.Provider{}, false
}
