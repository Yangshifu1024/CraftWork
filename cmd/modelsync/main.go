// Command modelsync 从 models.dev 拉取模型目录，过滤并转换成 CraftWork 的目录格式。
//
// 这个命令只在开发或发版前运行，不参与应用启动。它把协议映射、端点覆盖、
// 排除规则全部在构建期应用完，运行时只读取生成好的 catalog.json.gz，因此
// Go 代码里不再出现任何提供商或模型清单。
package main

import (
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/yangshifu1024/craftwork/internal/model/catalogdata"
)

// sourceLicense 是 models.dev 的许可证标识。完整文本写在 THIRD_PARTY_NOTICES.md。
const sourceLicense = "MIT"

// defaultSource 是 models.dev 的稳定目录接口。
const defaultSource = "https://models.dev/api.json"

// errOutdated 表示 -check 模式下目录与上游不一致。
var errOutdated = errors.New("模型目录需要更新")

// rules 是同步时使用的规则文件结构。
//
// 它只描述协议与端点，不包含任何模型清单；提供商与模型本身完全来自
// models.dev。这样可以把“手工维护”压缩到最小范围。
type rules struct {
	Schema            int               `json:"schema"`
	KindByNpm         map[string]string `json:"kindByNpm"`
	FallbackKind      string            `json:"fallbackKind"`
	EndpointOverrides map[string]string `json:"endpointOverrides"`
	ExcludedProviders []string          `json:"excludedProviders"`
	ExcludedNpm       []string          `json:"excludedNpm"`
}

// rawProvider 是 models.dev api.json 中的提供商结构；只声明用到的字段。
type rawProvider struct {
	ID     string              `json:"id"`
	Name   string              `json:"name"`
	Npm    string              `json:"npm"`
	Env    []string            `json:"env"`
	Doc    string              `json:"doc"`
	API    string              `json:"api"`
	Models map[string]rawModel `json:"models"`
}

// logoBaseURL 是 models.dev 站点的图标资源路径；api.json 不含 logo 字段，
// 但站点按 logos/<id>.svg 为每个提供商提供图标（覆盖全部目录条目）。
const logoBaseURL = "https://models.dev/logos/"

// rawModel 是 models.dev api.json 中的模型结构；只声明用到的字段。
type rawModel struct {
	Name             string        `json:"name"`
	Type             string        `json:"type"`
	ToolCall         bool          `json:"tool_call"`
	Reasoning        bool          `json:"reasoning"`
	StructuredOutput *bool         `json:"structured_output"`
	Modalities       rawModalities `json:"modalities"`
	Limit            rawLimit      `json:"limit"`
	Cost             *rawCost      `json:"cost"`
	Status           string        `json:"status"`
	ReleaseDate      string        `json:"release_date"`
	CanonicalModelID string        `json:"canonical_model_id"`
}

// rawModalities 是模型的输入 / 输出模态。
type rawModalities struct {
	Input []string `json:"input"`
}

// rawLimit 是模型的上下文与输出上限。
type rawLimit struct {
	Context int `json:"context"`
	Output  int `json:"output"`
}

// rawCost 是模型价格，单位美元 / 百万 token。
type rawCost struct {
	Input      float64 `json:"input"`
	Output     float64 `json:"output"`
	CacheRead  float64 `json:"cache_read"`
	CacheWrite float64 `json:"cache_write"`
}

// manifest 是人类可读的生成信息，便于审查数据规模与来源。
type manifest struct {
	Schema        int    `json:"schema"`
	GeneratedAt   string `json:"generatedAt"`
	Version       string `json:"version"`
	Source        string `json:"source"`
	SourceEtag    string `json:"sourceEtag,omitempty"`
	License       string `json:"license"`
	ProviderCount int    `json:"providerCount"`
	ModelCount    int    `json:"modelCount"`
	SHA256        string `json:"sha256"`
}

func main() {
	if err := run(); err != nil {
		if errors.Is(err, errOutdated) {
			fmt.Fprintln(os.Stderr, "模型目录与上游不一致，请运行 wails3 task models:sync")
			os.Exit(1)
		}
		fmt.Fprintln(os.Stderr, "同步模型目录失败:", err)
		os.Exit(1)
	}
}

// run 解析参数并执行同步或检查。
func run() error {
	source := flag.String("source", defaultSource, "models.dev 目录地址")
	out := flag.String("out", filepath.Join("internal", "model", "catalogdata", "catalog.json.gz"), "输出的 gzip 目录文件")
	manifestPath := flag.String("manifest", filepath.Join("internal", "model", "catalogdata", "catalog.manifest.json"), "输出的清单文件")
	rulesPath := flag.String("rules", filepath.Join("internal", "model", "catalogdata", "rules.json"), "同步规则文件")
	check := flag.Bool("check", false, "只检查目录是否与上游一致，不写文件")
	timeout := flag.Duration("timeout", 45*time.Second, "HTTP 请求超时")
	flag.Parse()

	loadedRules, err := loadRules(*rulesPath)
	if err != nil {
		return err
	}

	body, etag, err := fetch(*source, *timeout)
	if err != nil {
		return err
	}

	dataset, err := buildDataset(body, loadedRules, *source, etag)
	if err != nil {
		return err
	}

	if *check {
		existing, err := readExistingVersion(*out)
		if err != nil {
			return err
		}
		if existing == dataset.Version {
			fmt.Printf("模型目录已是最新：%s（%d 个提供商，%d 个模型）\n",
				dataset.Version, len(dataset.Providers), countModels(dataset))
			return nil
		}
		fmt.Printf("模型目录存在更新：本地 %s，上游 %s（%d 个提供商，%d 个模型）\n",
			existing, dataset.Version, len(dataset.Providers), countModels(dataset))
		return errOutdated
	}

	compressed, err := encodeDataset(dataset)
	if err != nil {
		return err
	}
	if err := writeFileAtomic(*out, compressed); err != nil {
		return err
	}
	generatedManifest := manifest{
		Schema:        dataset.Schema,
		GeneratedAt:   dataset.GeneratedAt,
		Version:       dataset.Version,
		Source:        dataset.Source,
		SourceEtag:    dataset.SourceEtag,
		License:       dataset.License,
		ProviderCount: len(dataset.Providers),
		ModelCount:    countModels(dataset),
		SHA256:        sha256Hex(compressed),
	}
	if err := writeManifest(*manifestPath, generatedManifest); err != nil {
		return err
	}
	fmt.Printf("已生成模型目录：%d 个提供商，%d 个模型，版本 %s\n",
		len(dataset.Providers), generatedManifest.ModelCount, dataset.Version)
	return nil
}

// loadRules 读取并校验同步规则。
func loadRules(path string) (rules, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return rules{}, fmt.Errorf("读取同步规则: %w", err)
	}
	var loaded rules
	if err := json.Unmarshal(raw, &loaded); err != nil {
		return rules{}, fmt.Errorf("解析同步规则: %w", err)
	}
	if loaded.Schema != 1 {
		return rules{}, fmt.Errorf("同步规则版本不受支持: %d", loaded.Schema)
	}
	if loaded.FallbackKind == "" {
		loaded.FallbackKind = "openai-compatible"
	}
	for npm, kind := range loaded.KindByNpm {
		if !supportedKind(kind) {
			return rules{}, fmt.Errorf("同步规则中 %s 映射到了不支持的协议: %s", npm, kind)
		}
	}
	if !supportedKind(loaded.FallbackKind) {
		return rules{}, fmt.Errorf("同步规则的默认协议不受支持: %s", loaded.FallbackKind)
	}
	return loaded, nil
}

// fetch 下载 models.dev 目录，返回响应体与 ETag。
func fetch(source string, timeout time.Duration) ([]byte, string, error) {
	request, err := http.NewRequest(http.MethodGet, source, nil)
	if err != nil {
		return nil, "", fmt.Errorf("构造目录请求: %w", err)
	}
	request.Header.Set("Accept", "application/json")
	request.Header.Set("User-Agent", "CraftWork-model-sync/1.0")
	client := &http.Client{Timeout: timeout}
	response, err := client.Do(request)
	if err != nil {
		return nil, "", fmt.Errorf("下载模型目录: %w", err)
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, "", fmt.Errorf("下载模型目录失败: HTTP %d", response.StatusCode)
	}
	// 目录当前约 6 MB；限制在 32 MB 以内，避免异常响应撑爆内存。
	body, err := io.ReadAll(io.LimitReader(response.Body, 32<<20))
	if err != nil {
		return nil, "", fmt.Errorf("读取模型目录响应: %w", err)
	}
	return body, response.Header.Get("ETag"), nil
}

// buildDataset 把 models.dev 原始数据转换成 CraftWork 的目录快照。
func buildDataset(body []byte, loadedRules rules, source, etag string) (*catalogdata.Dataset, error) {
	var rawProviders map[string]rawProvider
	if err := json.Unmarshal(body, &rawProviders); err != nil {
		return nil, fmt.Errorf("解析 models.dev 目录: %w", err)
	}
	excludedProviders := toSet(loadedRules.ExcludedProviders)
	excludedNpm := toSet(loadedRules.ExcludedNpm)

	providers := make([]catalogdata.Provider, 0, len(rawProviders))
	for key, raw := range rawProviders {
		providerID := strings.TrimSpace(raw.ID)
		if providerID == "" {
			providerID = key
		}
		if excludedProviders[providerID] || excludedNpm[raw.Npm] {
			continue
		}
		kind := loadedRules.KindByNpm[raw.Npm]
		if kind == "" {
			kind = loadedRules.FallbackKind
		}
		// 端点覆盖优先于目录给出的 api：少数提供商给出的基础地址缺少版本段，
		// 需要用本项目的规则纠正。没有覆盖时再使用目录端点。
		endpoint := strings.TrimSpace(loadedRules.EndpointOverrides[providerID])
		if endpoint == "" {
			endpoint = strings.TrimSpace(raw.API)
		}
		if endpoint == "" || !validEndpoint(endpoint) {
			// 没有可用端点或端点包含模板变量时跳过，避免导入后必然失败。
			continue
		}
		models := make([]catalogdata.Model, 0, len(raw.Models))
		for modelKey, item := range raw.Models {
			if item.Type != "" && item.Type != "chat" {
				continue
			}
			modelID := strings.TrimSpace(modelKey)
			if modelID == "" || strings.ContainsAny(modelID, " \t\n") {
				continue
			}
			if item.Limit.Context <= 0 {
				continue
			}
			name := strings.TrimSpace(item.Name)
			if name == "" {
				name = modelID
			}
			contextK := (item.Limit.Context + 999) / 1000
			if contextK <= 0 {
				continue
			}
			maxTokens := item.Limit.Output
			if maxTokens < 0 {
				maxTokens = 0
			}
			converted := catalogdata.Model{
				ID:          modelID,
				Name:        name,
				ContextK:    contextK,
				MaxTokens:   maxTokens,
				Caps:        convertCaps(item),
				Status:      strings.TrimSpace(item.Status),
				ReleaseDate: strings.TrimSpace(item.ReleaseDate),
				CanonicalID: strings.TrimSpace(item.CanonicalModelID),
			}
			if item.Cost != nil {
				converted.Cost = &catalogdata.Cost{
					InputPerMTok:      item.Cost.Input,
					OutputPerMTok:     item.Cost.Output,
					CacheReadPerMTok:  item.Cost.CacheRead,
					CacheWritePerMTok: item.Cost.CacheWrite,
				}
			}
			models = append(models, converted)
		}
		if len(models) == 0 {
			continue
		}
		sortModels(models)
		providers = append(providers, catalogdata.Provider{
			ID:       providerID,
			Name:     strings.TrimSpace(raw.Name),
			Kind:     kind,
			Endpoint: endpoint,
			Doc:      strings.TrimSpace(raw.Doc),
			Logo:     validLogoURL(logoBaseURL + providerID + ".svg"),
			Env:      append([]string(nil), raw.Env...),
			Models:   models,
		})
	}
	if len(providers) == 0 {
		return nil, errors.New("过滤后没有可用的提供商，请检查同步规则")
	}
	sortProviders(providers)

	// 版本只由提供商与模型内容决定，不受 generatedAt 影响；这样 -check 可以
	// 用版本判断“数据是否变化”，而不是每次都因为时间戳不同而报错。
	canonical, err := json.Marshal(providers)
	if err != nil {
		return nil, fmt.Errorf("计算目录版本: %w", err)
	}
	version := sha256Hex(canonical)[:16]

	return &catalogdata.Dataset{
		Schema:      catalogdata.SchemaVersion,
		GeneratedAt: time.Now().UTC().Format(time.RFC3339),
		Version:     version,
		Source:      source,
		SourceEtag:  etag,
		License:     sourceLicense,
		Providers:   providers,
	}, nil
}

// convertCaps 把 models.dev 的模型布尔字段转换成目录能力。
func convertCaps(item rawModel) catalogdata.Caps {
	vision := false
	for _, modality := range item.Modalities.Input {
		if strings.HasPrefix(strings.ToLower(modality), "image") {
			vision = true
			break
		}
	}
	structured := false
	if item.StructuredOutput != nil {
		structured = *item.StructuredOutput
	}
	return catalogdata.Caps{
		Tools:      item.ToolCall,
		Vision:     vision,
		Reasoning:  item.Reasoning,
		Structured: structured,
	}
}

// validEndpoint 检查端点是否是可以直接使用的 HTTP(S) 地址。
func validEndpoint(endpoint string) bool {
	if strings.Contains(endpoint, "${") {
		return false
	}
	parsed, err := url.Parse(endpoint)
	if err != nil || parsed.Host == "" {
		return false
	}
	return parsed.Scheme == "https" || parsed.Scheme == "http"
}

// validLogoURL 清洗 models.dev 给出的 logo 地址：只保留合法的 HTTPS 地址，
// 其余（含模板变量与空值）一律丢弃，宁可回退文字头像也不引入奇怪的资源。
func validLogoURL(raw string) string {
	logo := strings.TrimSpace(raw)
	if logo == "" || len(logo) > 512 || strings.ContainsAny(logo, " \t\n\"'<>") {
		return ""
	}
	parsed, err := url.Parse(logo)
	if err != nil || parsed.Host == "" || parsed.Scheme != "https" {
		return ""
	}
	return logo
}

// supportedKind 判断协议是否在当前支持范围内。
func supportedKind(kind string) bool {
	switch kind {
	case "openai", "anthropic", "openai-compatible":
		return true
	default:
		return false
	}
}

// sortProviders 按名称与标识排序，保证生成文件稳定。
func sortProviders(providers []catalogdata.Provider) {
	sort.SliceStable(providers, func(i, j int) bool {
		left := strings.ToLower(providers[i].Name)
		right := strings.ToLower(providers[j].Name)
		if left != right {
			return left < right
		}
		return providers[i].ID < providers[j].ID
	})
}

// sortModels 按发布日期从新到旧排序，日期相同再按名称与标识排序。
func sortModels(models []catalogdata.Model) {
	sort.SliceStable(models, func(i, j int) bool {
		if models[i].ReleaseDate != models[j].ReleaseDate {
			return models[i].ReleaseDate > models[j].ReleaseDate
		}
		left := strings.ToLower(models[i].Name)
		right := strings.ToLower(models[j].Name)
		if left != right {
			return left < right
		}
		return models[i].ID < models[j].ID
	})
}

// encodeDataset 把目录编码成确定性的 gzip 文件。
func encodeDataset(dataset *catalogdata.Dataset) ([]byte, error) {
	raw, err := json.Marshal(dataset)
	if err != nil {
		return nil, fmt.Errorf("编码模型目录: %w", err)
	}
	var buffer bytes.Buffer
	writer, err := gzip.NewWriterLevel(&buffer, gzip.BestCompression)
	if err != nil {
		return nil, fmt.Errorf("创建压缩目录: %w", err)
	}
	// 固定头部时间与操作系统字段，保证相同数据生成完全相同的字节。
	writer.ModTime = time.Time{}
	writer.OS = 255
	if _, err := writer.Write(raw); err != nil {
		return nil, fmt.Errorf("压缩模型目录: %w", err)
	}
	if err := writer.Close(); err != nil {
		return nil, fmt.Errorf("关闭压缩目录: %w", err)
	}
	return buffer.Bytes(), nil
}

// readExistingVersion 读取已生成目录的版本号。
func readExistingVersion(path string) (string, error) {
	compressed, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("读取现有模型目录: %w", err)
	}
	reader, err := gzip.NewReader(bytes.NewReader(compressed))
	if err != nil {
		return "", fmt.Errorf("解压现有模型目录: %w", err)
	}
	defer reader.Close()
	raw, err := io.ReadAll(reader)
	if err != nil {
		return "", fmt.Errorf("读取现有模型目录: %w", err)
	}
	var existing catalogdata.Dataset
	if err := json.Unmarshal(raw, &existing); err != nil {
		return "", fmt.Errorf("解析现有模型目录: %w", err)
	}
	return existing.Version, nil
}

// writeManifest 写入人类可读的生成信息。
func writeManifest(path string, info manifest) error {
	raw, err := json.MarshalIndent(info, "", "  ")
	if err != nil {
		return fmt.Errorf("编码模型目录清单: %w", err)
	}
	raw = append(raw, '\n')
	return writeFileAtomic(path, raw)
}

// writeFileAtomic 先写临时文件再重命名，避免中途失败留下半成品。
func writeFileAtomic(path string, data []byte) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("创建目录 %s: %w", dir, err)
	}
	temp, err := os.CreateTemp(dir, ".modelsync-*")
	if err != nil {
		return fmt.Errorf("创建临时文件: %w", err)
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)
	if _, err := temp.Write(data); err != nil {
		temp.Close()
		return fmt.Errorf("写入临时文件: %w", err)
	}
	if err := temp.Chmod(0o644); err != nil {
		temp.Close()
		return fmt.Errorf("设置文件权限: %w", err)
	}
	if err := temp.Close(); err != nil {
		return fmt.Errorf("关闭临时文件: %w", err)
	}
	if err := os.Rename(tempPath, path); err != nil {
		return fmt.Errorf("替换文件 %s: %w", path, err)
	}
	return nil
}

// countModels 统计目录中的模型总数。
func countModels(dataset *catalogdata.Dataset) int {
	total := 0
	for _, provider := range dataset.Providers {
		total += len(provider.Models)
	}
	return total
}

// toSet 把字符串切片转换成集合。
func toSet(values []string) map[string]bool {
	result := make(map[string]bool, len(values))
	for _, value := range values {
		result[value] = true
	}
	return result
}

// sha256Hex 返回字节内容的 SHA-256 十六进制摘要。
func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}
