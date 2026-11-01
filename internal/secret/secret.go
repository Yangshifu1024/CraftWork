// Package secret 管理本地密钥文件 ~/.craftwork/keys.json。
//
// 约束：
//   - 目录 0700、文件 0600；
//   - 权限过宽时自动收紧并返回警告；
//   - 密钥只存在于 Go 进程，前端只能拿到来源与遮罩。
//
// 本文件负责密钥文件与 MCP 配置的读取、写入、删除，环境变量回退，以及写入
// 时使用临时文件加原子替换，避免写到一半留下损坏的文件。
package secret

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

// Store 是密钥文件的读写封装。
type Store struct {
	// path 是密钥文件的绝对路径，通常为 <数据目录>/keys.json。
	path string
	// mu 保护 data 与 warnings 的并发读写。
	mu sync.RWMutex
	// data 是密钥文件的内存副本。
	data fileData
	// warnings 记录打开文件时发现并已自动修复的问题。
	warnings []string
}

// fileData 是密钥文件的磁盘结构。
type fileData struct {
	// Version 是文件格式版本，当前为 1。
	Version int `json:"version"`
	// Providers 按提供商 ID 保存密钥明文。
	Providers map[string]string `json:"providers,omitempty"`
	// MCP 按服务器名称保存模型上下文协议（MCP）的附加配置。
	MCP map[string]map[string]any `json:"mcp,omitempty"`
}

// Open 读取密钥文件。文件不存在时返回空存储，首次写入时创建。
//
// 目录不存在时以 0700 权限创建；文件权限过宽时不拒绝启动，而是自动收紧为
// 0600 并记录一条警告，让用户能继续使用并看到问题。
func Open(dir string) (*Store, error) {
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("创建密钥目录: %w", err)
	}
	store := &Store{path: filepath.Join(dir, "keys.json"), data: fileData{Version: 1, Providers: map[string]string{}}}

	info, err := os.Stat(store.path)
	if os.IsNotExist(err) {
		return store, nil
	}
	if err != nil {
		return nil, fmt.Errorf("读取密钥文件信息: %w", err)
	}
	if info.Mode().Perm()&0o077 != 0 {
		// 组用户或其他用户有任何权限即视为过宽；权限过宽时收紧，而不是直接
		// 拒绝启动，用户需要能继续使用。
		if err := os.Chmod(store.path, 0o600); err != nil {
			return nil, fmt.Errorf("修正密钥文件权限: %w", err)
		}
		store.warnings = append(store.warnings, "密钥文件权限过宽，已自动收紧为 0600")
	}

	raw, err := os.ReadFile(store.path)
	if err != nil {
		return nil, fmt.Errorf("读取密钥文件: %w", err)
	}
	if len(raw) == 0 {
		return store, nil
	}
	if err := json.Unmarshal(raw, &store.data); err != nil {
		return nil, fmt.Errorf("解析密钥文件: %w", err)
	}
	// 兼容旧文件或空文件：确保密钥表已初始化，后续写入不必判空。
	if store.data.Providers == nil {
		store.data.Providers = map[string]string{}
	}
	return store, nil
}

// NewEmpty 返回一个不读取磁盘的空存储，用于密钥文件损坏时的降级启动。
// 之后写入会覆盖原文件，因此调用方应先通过 Warnings 等方式告知用户。
func NewEmpty(dir string) *Store {
	return &Store{
		path: filepath.Join(dir, "keys.json"),
		data: fileData{Version: 1, Providers: map[string]string{}},
	}
}

// Warnings 返回打开密钥文件时产生的问题。返回副本，调用方修改不会影响存储。
func (s *Store) Warnings() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return append([]string(nil), s.warnings...)
}

// Get 返回提供商密钥。文件优先，其次环境变量。
//
// 环境变量回退让用户可以在不落盘的情况下临时使用密钥，方便开发与持续集成。
// 文件里的空字符串视为未配置，继续查找环境变量。
func (s *Store) Get(providerID string) string {
	return s.GetFor(providerID, nil)
}

// GetFor 按目录给出的环境变量名读取密钥。
//
// envNames 来自 models.dev 的 provider.env；未设置时退回按提供商 ID 推导的
// 环境变量名，兼容旧配置。
func (s *Store) GetFor(providerID string, envNames []string) string {
	s.mu.RLock()
	value := s.data.Providers[providerID]
	s.mu.RUnlock()
	if value != "" {
		return value
	}
	for _, name := range envNames {
		if candidate := os.Getenv(name); candidate != "" {
			return candidate
		}
	}
	return os.Getenv(envName(providerID))
}

// Source 返回密钥来源：file / env / 空。
//
// 前端只展示来源，不接触密钥本身。
func (s *Store) Source(providerID string) string {
	return s.SourceFor(providerID, nil)
}

// SourceFor 按目录环境变量名判断密钥来源。
func (s *Store) SourceFor(providerID string, envNames []string) string {
	s.mu.RLock()
	value := s.data.Providers[providerID]
	s.mu.RUnlock()
	if value != "" {
		return "file"
	}
	for _, name := range envNames {
		if os.Getenv(name) != "" {
			return "env"
		}
	}
	if os.Getenv(envName(providerID)) != "" {
		return "env"
	}
	return ""
}

// Masked 返回用于界面展示的遮罩，例如 sk-ab…9f2c。
//
// 只保留前 5 位与后 4 位；长度不超过 10 位时全部隐藏，避免短密钥被猜出。
func (s *Store) Masked(providerID string) string {
	return s.MaskedFor(providerID, nil)
}

// MaskedFor 按目录环境变量名读取密钥并生成遮罩。
func (s *Store) MaskedFor(providerID string, envNames []string) string {
	value := s.GetFor(providerID, envNames)
	if value == "" {
		return ""
	}
	if len(value) <= 10 {
		return "••••••"
	}
	return value[:5] + "…" + value[len(value)-4:]
}

// Set 写入提供商密钥并落盘。值会去掉首尾空白；空值等价于保留一个空条目，
// 读取时会被视为未配置。
func (s *Store) Set(providerID, value string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.data.Providers[providerID] = strings.TrimSpace(value)
	return s.persistLocked()
}

// Delete 删除提供商密钥并落盘。
func (s *Store) Delete(providerID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.data.Providers, providerID)
	return s.persistLocked()
}

// persistLocked 把内存数据写回磁盘。调用方必须已经持有写锁。
//
// 写入流程是“临时文件 -> 设置 0600 权限 -> 写入并关闭 -> 原子重命名”，
// 这样即使中途崩溃也不会留下半个密钥文件。
func (s *Store) persistLocked() error {
	if s.data.Version == 0 {
		s.data.Version = 1
	}
	raw, err := json.MarshalIndent(s.data, "", "  ")
	if err != nil {
		return fmt.Errorf("编码密钥文件: %w", err)
	}
	// 临时文件与目标文件同目录，保证重命名在同一文件系统内完成。
	temp, err := os.CreateTemp(filepath.Dir(s.path), ".keys-*.json")
	if err != nil {
		return fmt.Errorf("创建密钥临时文件: %w", err)
	}
	tempPath := temp.Name()
	// 无论成功失败都尝试清理临时文件；重命名成功后原路径已不存在，删除是空操作。
	defer os.Remove(tempPath)

	// CreateTemp 默认权限是 0600，这里显式设置，避免受 umask 影响。
	if err := temp.Chmod(0o600); err != nil {
		temp.Close()
		return fmt.Errorf("设置密钥临时文件权限: %w", err)
	}
	if _, err := temp.Write(raw); err != nil {
		temp.Close()
		return fmt.Errorf("写入密钥临时文件: %w", err)
	}
	if err := temp.Close(); err != nil {
		return fmt.Errorf("关闭密钥临时文件: %w", err)
	}
	if err := os.Rename(tempPath, s.path); err != nil {
		return fmt.Errorf("替换密钥文件: %w", err)
	}
	return nil
}

// envName 返回提供商对应的环境变量名。常见提供商使用各自的惯用命名，
// 其他提供商按“大写 ID + _API_KEY”推导。
func envName(providerID string) string {
	switch providerID {
	case "openai":
		return "OPENAI_API_KEY"
	case "deepseek":
		return "DEEPSEEK_API_KEY"
	case "glm":
		return "ZHIPU_API_KEY"
	case "qwen":
		return "DASHSCOPE_API_KEY"
	case "anthropic":
		return "ANTHROPIC_API_KEY"
	default:
		return strings.ToUpper(providerID) + "_API_KEY"
	}
}
