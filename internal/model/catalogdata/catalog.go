// Package catalogdata 读取构建期生成的模型目录快照。
//
// 目录只来自 models.dev，经 cmd/modelsync 过滤、映射并压缩后嵌入二进制。
// 本包不访问网络，也不包含任何手工维护的提供商或模型清单；它只负责把
// catalog.json.gz 解析成只读的查询结构，供“从模型目录添加”与手工补全使用。
package catalogdata

import (
	"bytes"
	"compress/gzip"
	"embed"
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"sync"
)

// catalogFile 是同步命令写入的压缩目录文件。
//
//go:embed catalog.json.gz
var catalogFile embed.FS

// SchemaVersion 是目录文件的格式版本。同步命令与运行时必须保持一致。
const SchemaVersion = 1

// Cost 是模型价格，单位为美元 / 百万 token。
type Cost struct {
	InputPerMTok      float64 `json:"inputPerMTok"`
	OutputPerMTok     float64 `json:"outputPerMTok"`
	CacheReadPerMTok  float64 `json:"cacheReadPerMTok,omitempty"`
	CacheWritePerMTok float64 `json:"cacheWritePerMTok,omitempty"`
}

// Caps 是目录里记录的能力。流式能力对所有对话模型默认开启，因此不存字段。
type Caps struct {
	Tools      bool `json:"tools"`
	Vision     bool `json:"vision"`
	Reasoning  bool `json:"reasoning"`
	Structured bool `json:"structured"`
}

// Model 是目录中的一个对话模型。
type Model struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	ContextK    int    `json:"contextK"`
	MaxTokens   int    `json:"maxTokens,omitempty"`
	Caps        Caps   `json:"caps"`
	Cost        *Cost  `json:"cost,omitempty"`
	Status      string `json:"status,omitempty"`
	ReleaseDate string `json:"releaseDate,omitempty"`
	CanonicalID string `json:"canonicalId,omitempty"`
}

// Provider 是目录中的一个提供商。
type Provider struct {
	ID       string   `json:"id"`
	Name     string   `json:"name"`
	Kind     string   `json:"kind"`
	Endpoint string   `json:"endpoint"`
	Doc      string   `json:"doc,omitempty"`
	Env      []string `json:"env,omitempty"`
	Models   []Model  `json:"models"`
}

// Dataset 是完整的目录快照。
type Dataset struct {
	Schema      int        `json:"schema"`
	GeneratedAt string     `json:"generatedAt"`
	Version     string     `json:"version"`
	Source      string     `json:"source"`
	SourceEtag  string     `json:"sourceEtag,omitempty"`
	License     string     `json:"license"`
	Providers   []Provider `json:"providers"`

	providerByID map[string]int
	modelByRef   map[string]Model
}

// Meta 是给界面展示的目录元信息。
type Meta struct {
	Schema        int    `json:"schema"`
	GeneratedAt   string `json:"generatedAt"`
	Version       string `json:"version"`
	Source        string `json:"source"`
	License       string `json:"license"`
	ProviderCount int    `json:"providerCount"`
	ModelCount    int    `json:"modelCount"`
}

// 全局只加载一次：目录是只读快照，进程内不会变化。
var (
	loadOnce sync.Once
	dataset  *Dataset
	loadErr  error
)

// Load 返回嵌入的目录快照。首次调用会解压、解码并校验数据。
func Load() (*Dataset, error) {
	loadOnce.Do(func() {
		dataset, loadErr = loadEmbedded()
	})
	return dataset, loadErr
}

// MustLoad 返回目录快照，加载失败时 panic。
//
// 只用于测试与启动期已经确认目录可用的路径；服务层应使用 Load 并把错误
// 转换成可读提示。
func MustLoad() *Dataset {
	current, err := Load()
	if err != nil {
		panic(err)
	}
	return current
}

// loadEmbedded 解压并校验嵌入的目录文件。
func loadEmbedded() (*Dataset, error) {
	compressed, err := catalogFile.ReadFile("catalog.json.gz")
	if err != nil {
		return nil, fmt.Errorf("读取模型目录: %w", err)
	}
	reader, err := gzip.NewReader(bytes.NewReader(compressed))
	if err != nil {
		return nil, fmt.Errorf("解压模型目录: %w", err)
	}
	defer reader.Close()
	raw, err := io.ReadAll(io.LimitReader(reader, 32<<20))
	if err != nil {
		return nil, fmt.Errorf("读取模型目录内容: %w", err)
	}
	return Parse(raw)
}

// Parse 解析一份未压缩的目录 JSON，并建立查询索引。
func Parse(raw []byte) (*Dataset, error) {
	var parsed Dataset
	if err := json.Unmarshal(raw, &parsed); err != nil {
		return nil, fmt.Errorf("解析模型目录: %w", err)
	}
	if err := parsed.validate(); err != nil {
		return nil, err
	}
	parsed.buildIndex()
	return &parsed, nil
}

// validate 检查目录的基本完整性，避免坏数据进入界面。
func (d *Dataset) validate() error {
	if d.Schema != SchemaVersion {
		return fmt.Errorf("模型目录版本不受支持: %d", d.Schema)
	}
	if len(d.Providers) == 0 {
		return fmt.Errorf("模型目录为空")
	}
	providerIDs := make(map[string]bool, len(d.Providers))
	for _, provider := range d.Providers {
		if strings.TrimSpace(provider.ID) == "" || strings.TrimSpace(provider.Name) == "" {
			return fmt.Errorf("模型目录中存在空的提供商")
		}
		if providerIDs[provider.ID] {
			return fmt.Errorf("模型目录中存在重复提供商: %s", provider.ID)
		}
		providerIDs[provider.ID] = true
		switch provider.Kind {
		case "openai", "anthropic", "openai-compatible":
		default:
			return fmt.Errorf("提供商 %s 的协议不受支持: %s", provider.ID, provider.Kind)
		}
		if provider.Endpoint == "" {
			return fmt.Errorf("提供商 %s 缺少端点", provider.ID)
		}
		if len(provider.Models) == 0 {
			return fmt.Errorf("提供商 %s 没有模型", provider.ID)
		}
		modelIDs := make(map[string]bool, len(provider.Models))
		for _, item := range provider.Models {
			if strings.TrimSpace(item.ID) == "" {
				return fmt.Errorf("提供商 %s 存在空模型 ID", provider.ID)
			}
			if modelIDs[item.ID] {
				return fmt.Errorf("提供商 %s 存在重复模型: %s", provider.ID, item.ID)
			}
			modelIDs[item.ID] = true
			if item.ContextK <= 0 {
				return fmt.Errorf("模型 %s/%s 的上下文窗口不合法", provider.ID, item.ID)
			}
		}
	}
	return nil
}

// buildIndex 建立提供商与模型的查询索引。
func (d *Dataset) buildIndex() {
	d.providerByID = make(map[string]int, len(d.Providers))
	d.modelByRef = make(map[string]Model)
	for index, provider := range d.Providers {
		d.providerByID[provider.ID] = index
		for _, item := range provider.Models {
			d.modelByRef[provider.ID+"/"+item.ID] = item
		}
	}
}

// Meta 返回目录的版本、来源与规模。
func (d *Dataset) Meta() Meta {
	modelCount := 0
	for _, provider := range d.Providers {
		modelCount += len(provider.Models)
	}
	return Meta{
		Schema:        d.Schema,
		GeneratedAt:   d.GeneratedAt,
		Version:       d.Version,
		Source:        d.Source,
		License:       d.License,
		ProviderCount: len(d.Providers),
		ModelCount:    modelCount,
	}
}

// AllProviders 返回提供商列表的副本，调用方可以安全遍历与修改切片。
func (d *Dataset) AllProviders() []Provider {
	result := make([]Provider, 0, len(d.Providers))
	for _, provider := range d.Providers {
		result = append(result, cloneProvider(provider))
	}
	return result
}

// Provider 按标识查找提供商；第二个返回值表示是否找到。
func (d *Dataset) Provider(providerID string) (Provider, bool) {
	index, ok := d.providerByID[providerID]
	if !ok {
		return Provider{}, false
	}
	return cloneProvider(d.Providers[index]), true
}

// Model 按提供商与模型标识查找模型。
func (d *Dataset) Model(providerID, modelID string) (Model, bool) {
	item, ok := d.modelByRef[providerID+"/"+modelID]
	return item, ok
}

// SearchProviders 按名称或标识做大小写不敏感的关键字过滤。
//
// 空关键字返回全部提供商；结果保持目录中的顺序。
func (d *Dataset) SearchProviders(query string) []Provider {
	query = strings.ToLower(strings.TrimSpace(query))
	if query == "" {
		return d.AllProviders()
	}
	result := make([]Provider, 0)
	for _, provider := range d.Providers {
		if strings.Contains(strings.ToLower(provider.ID), query) ||
			strings.Contains(strings.ToLower(provider.Name), query) {
			result = append(result, cloneProvider(provider))
		}
	}
	return result
}

// cloneProvider 复制提供商与模型切片，避免调用方改动内部状态。
func cloneProvider(provider Provider) Provider {
	cloned := provider
	cloned.Env = append([]string(nil), provider.Env...)
	cloned.Models = append([]Model(nil), provider.Models...)
	return cloned
}
