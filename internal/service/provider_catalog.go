// 本文件是模型目录的浏览与导入接口。
//
// 目录来自嵌入的 models.dev 快照，只作为“可添加的候选列表”；应用不会自动
// 创建任何提供商。用户选中提供商与模型后，这里把目录条目转换成普通的用户
// 配置并写入同一个设置表。

package service

import (
	"errors"
	"fmt"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/model/catalogdata"
)

// CatalogMeta 是目录版本与来源信息。
type CatalogMeta struct {
	// Schema 是目录文件格式版本。
	Schema int `json:"schema"`
	// GeneratedAt 是目录生成时间。
	GeneratedAt string `json:"generatedAt"`
	// Version 是目录内容版本。
	Version string `json:"version"`
	// Source 是数据来源地址。
	Source string `json:"source"`
	// License 是数据许可证。
	License string `json:"license"`
	// ProviderCount 是提供商数量。
	ProviderCount int `json:"providerCount"`
	// ModelCount 是模型数量。
	ModelCount int `json:"modelCount"`
}

// CatalogProviderView 是目录中可供选择的提供商。
type CatalogProviderView struct {
	// ID 是 models.dev 提供商标识，导入后沿用为本地提供商 ID。
	ID string `json:"id"`
	// Name 是展示名称。
	Name string `json:"name"`
	// Kind 是映射后的协议。
	Kind string `json:"kind"`
	// Endpoint 是目录给出的端点。
	Endpoint string `json:"endpoint"`
	// Doc 是提供商文档地址。
	Doc string `json:"doc,omitempty"`
	// Env 是密钥可使用的环境变量名。
	Env []string `json:"env,omitempty"`
	// ModelCount 是该提供商下的对话模型数量。
	ModelCount int `json:"modelCount"`
	// Added 表示用户已经添加了同 ID 的提供商。
	Added bool `json:"added"`
}

// CatalogModelView 是目录中的一个模型。
type CatalogModelView struct {
	// ID 是原始模型 ID。
	ID string `json:"id"`
	// Ref 是“提供商/模型”的稳定引用。
	Ref string `json:"ref"`
	// Name 是展示名称。
	Name string `json:"name"`
	// Provider 是提供商标识。
	Provider string `json:"provider"`
	// ProviderName 是提供商展示名称。
	ProviderName string `json:"providerName"`
	// ContextK 是上下文窗口，单位千 token。
	ContextK int `json:"contextK"`
	// MaxTokens 是最大输出 token 数。
	MaxTokens int `json:"maxTokens,omitempty"`
	// Caps 是模型能力。
	Caps model.Capabilities `json:"caps"`
	// Cost 是价格信息。
	Cost *model.Cost `json:"cost,omitempty"`
	// Status 是模型状态。
	Status string `json:"status,omitempty"`
	// CanonicalID 是跨托管方的原始模型标识。
	CanonicalID string `json:"canonicalId,omitempty"`
	// ReleaseDate 是发布日期。
	ReleaseDate string `json:"releaseDate,omitempty"`
}

// CatalogImportInput 是从目录导入提供商的输入。
type CatalogImportInput struct {
	// ProviderID 是目录中的提供商标识。
	ProviderID string `json:"providerId"`
	// ModelIDs 是要导入的模型；All 为 true 时忽略。
	ModelIDs []string `json:"modelIds"`
	// All 表示导入该提供商下的全部对话模型。
	All bool `json:"all"`
	// Name 可选，覆盖提供商展示名称。
	Name string `json:"name,omitempty"`
	// Endpoint 可选，覆盖端点。
	Endpoint string `json:"endpoint,omitempty"`
	// Kind 可选，覆盖协议。
	Kind string `json:"kind,omitempty"`
	// KeyRequired 可选，覆盖是否需要密钥；空值按需要密钥处理。
	KeyRequired *bool `json:"keyRequired,omitempty"`
}

// maxImportedModels 限制单次导入的模型数量，避免误操作写入过大的配置。
const maxImportedModels = 500

// CatalogMeta 返回嵌入目录的版本、来源与规模。
func (s *ProviderService) CatalogMeta() (CatalogMeta, error) {
	dataset, err := catalogdata.Load()
	if err != nil {
		return CatalogMeta{}, err
	}
	meta := dataset.Meta()
	return CatalogMeta{
		Schema:        meta.Schema,
		GeneratedAt:   meta.GeneratedAt,
		Version:       meta.Version,
		Source:        meta.Source,
		License:       meta.License,
		ProviderCount: meta.ProviderCount,
		ModelCount:    meta.ModelCount,
	}, nil
}

// CatalogProviders 返回可浏览的提供商列表，支持按名称或标识搜索。
//
// 列表只描述“可以添加什么”，不会自动创建提供商；Added 用于界面提示。
func (s *ProviderService) CatalogProviders(query string) ([]CatalogProviderView, error) {
	dataset, err := catalogdata.Load()
	if err != nil {
		return nil, err
	}
	s.mu.RLock()
	added := make(map[string]bool, len(s.custom))
	for _, provider := range s.custom {
		added[provider.ID] = true
	}
	s.mu.RUnlock()

	providers := dataset.SearchProviders(query)
	result := make([]CatalogProviderView, 0, len(providers))
	for _, provider := range providers {
		result = append(result, CatalogProviderView{
			ID:         provider.ID,
			Name:       provider.Name,
			Kind:       provider.Kind,
			Endpoint:   provider.Endpoint,
			Doc:        provider.Doc,
			Env:        append([]string(nil), provider.Env...),
			ModelCount: len(provider.Models),
			Added:      added[provider.ID],
		})
	}
	return result, nil
}

// CatalogModels 返回指定提供商下的全部对话模型。
func (s *ProviderService) CatalogModels(providerID string) ([]CatalogModelView, error) {
	dataset, err := catalogdata.Load()
	if err != nil {
		return nil, err
	}
	provider, found := dataset.Provider(providerID)
	if !found {
		return nil, fmt.Errorf("模型目录中没有提供商 %s", providerID)
	}
	result := make([]CatalogModelView, 0, len(provider.Models))
	for _, item := range provider.Models {
		result = append(result, toCatalogModelView(provider, item))
	}
	return result, nil
}

// LookupCatalogModel 按提供商与模型标识查找目录条目，用于手工添加模型时补全。
//
// providerID 不匹配时会在整个目录中查找同名模型；只有唯一匹配才返回，
// 多个提供商都有该模型时要求调用方明确提供商，避免静默补错元数据。
func (s *ProviderService) LookupCatalogModel(providerID, modelID string) (*CatalogModelView, error) {
	dataset, err := catalogdata.Load()
	if err != nil {
		return nil, err
	}
	modelID = strings.TrimSpace(modelID)
	if modelID == "" {
		return nil, errors.New("模型 ID 不能为空")
	}
	if provider, found := dataset.Provider(providerID); found {
		if item, ok := dataset.Model(providerID, modelID); ok {
			view := toCatalogModelView(provider, item)
			return &view, nil
		}
	}
	// 手工添加的提供商 ID 通常不在目录里；在全部提供商中查找唯一匹配。
	var matchedProvider catalogdata.Provider
	var matchedModel catalogdata.Model
	matches := 0
	for _, candidate := range dataset.AllProviders() {
		if item, ok := dataset.Model(candidate.ID, modelID); ok {
			matchedProvider = candidate
			matchedModel = item
			matches++
		}
	}
	switch matches {
	case 0:
		return nil, fmt.Errorf("模型目录中没有找到模型 %s", modelID)
	case 1:
		view := toCatalogModelView(matchedProvider, matchedModel)
		return &view, nil
	default:
		return nil, fmt.Errorf("模型 %s 存在于多个提供商，请先填写提供商 ID", modelID)
	}
}

// ImportFromCatalog 把选中的目录提供商与模型保存为普通的用户配置。
func (s *ProviderService) ImportFromCatalog(input CatalogImportInput) (ProviderView, error) {
	dataset, err := catalogdata.Load()
	if err != nil {
		return ProviderView{}, err
	}
	provider, found := dataset.Provider(strings.TrimSpace(input.ProviderID))
	if !found {
		return ProviderView{}, fmt.Errorf("模型目录中没有提供商 %s", input.ProviderID)
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	if _, exists := findCustomProvider(s.custom, provider.ID); exists {
		return ProviderView{}, fmt.Errorf("%s 已经添加，请直接编辑现有配置", provider.Name)
	}

	selected := provider.Models
	if !input.All {
		selected = make([]catalogdata.Model, 0, len(input.ModelIDs))
		seen := map[string]bool{}
		for _, modelID := range input.ModelIDs {
			modelID = strings.TrimSpace(modelID)
			if modelID == "" || seen[modelID] {
				continue
			}
			item, ok := dataset.Model(provider.ID, modelID)
			if !ok {
				return ProviderView{}, fmt.Errorf("提供商 %s 下没有模型 %s", provider.Name, modelID)
			}
			seen[modelID] = true
			selected = append(selected, item)
		}
	}
	if len(selected) == 0 {
		return ProviderView{}, errors.New("至少选择一个模型")
	}
	if len(selected) > maxImportedModels {
		return ProviderView{}, fmt.Errorf("单次最多导入 %d 个模型", maxImportedModels)
	}

	name := strings.TrimSpace(input.Name)
	if name == "" {
		name = provider.Name
	}
	endpoint := strings.TrimSpace(input.Endpoint)
	if endpoint == "" {
		endpoint = provider.Endpoint
	}
	kind := strings.TrimSpace(input.Kind)
	if kind == "" {
		kind = provider.Kind
	}
	keyRequired := true
	if input.KeyRequired != nil {
		keyRequired = *input.KeyRequired
	}

	models := make([]ModelInput, 0, len(selected))
	for _, item := range selected {
		caps := model.Capabilities{
			Stream:     true,
			Tools:      item.Caps.Tools,
			Vision:     item.Caps.Vision,
			Reasoning:  item.Caps.Reasoning,
			Structured: item.Caps.Structured,
		}
		var cost *model.Cost
		if item.Cost != nil {
			cost = &model.Cost{
				InputPerMTok:      item.Cost.InputPerMTok,
				OutputPerMTok:     item.Cost.OutputPerMTok,
				CacheReadPerMTok:  item.Cost.CacheReadPerMTok,
				CacheWritePerMTok: item.Cost.CacheWritePerMTok,
			}
		}
		models = append(models, ModelInput{
			ID:          item.ID,
			Name:        item.Name,
			ContextK:    item.ContextK,
			MaxTokens:   item.MaxTokens,
			Caps:        caps,
			Cost:        cost,
			Status:      item.Status,
			CanonicalID: item.CanonicalID,
			ReleaseDate: item.ReleaseDate,
		})
	}

	// 复用普通提供商的校验逻辑，再把目录来源信息补回去。
	built, err := s.buildProvider(ProviderInput{
		ID:          provider.ID,
		Name:        name,
		Kind:        kind,
		Endpoint:    endpoint,
		KeyRequired: keyRequired,
		Models:      models,
	}, s.custom)
	if err != nil {
		return ProviderView{}, err
	}
	built.Source = "catalog"
	built.CatalogID = provider.ID
	built.CatalogVersion = dataset.Version
	built.EnvVars = append([]string(nil), provider.Env...)
	for index := range built.Models {
		built.Models[index].Source = "catalog"
	}

	next := append(cloneProviders(s.custom), built)
	if err := s.persist(next); err != nil {
		return ProviderView{}, err
	}
	return s.viewFor(built), nil
}

// toCatalogModelView 把目录模型转换成展示结构。
func toCatalogModelView(provider catalogdata.Provider, item catalogdata.Model) CatalogModelView {
	caps := model.Capabilities{
		Stream:     true,
		Tools:      item.Caps.Tools,
		Vision:     item.Caps.Vision,
		Reasoning:  item.Caps.Reasoning,
		Structured: item.Caps.Structured,
	}
	var cost *model.Cost
	if item.Cost != nil {
		cost = &model.Cost{
			InputPerMTok:      item.Cost.InputPerMTok,
			OutputPerMTok:     item.Cost.OutputPerMTok,
			CacheReadPerMTok:  item.Cost.CacheReadPerMTok,
			CacheWritePerMTok: item.Cost.CacheWritePerMTok,
		}
	}
	return CatalogModelView{
		ID:           item.ID,
		Ref:          model.Ref(provider.ID, item.ID),
		Name:         item.Name,
		Provider:     provider.ID,
		ProviderName: provider.Name,
		ContextK:     item.ContextK,
		MaxTokens:    item.MaxTokens,
		Caps:         caps,
		Cost:         cost,
		Status:       item.Status,
		CanonicalID:  item.CanonicalID,
		ReleaseDate:  item.ReleaseDate,
	}
}
