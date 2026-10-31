// Package secret 管理本地密钥文件 ~/.craftwork/keys.json。
//
// 约束：
//   - 目录 0700、文件 0600；
//   - 权限过宽时自动收紧并返回警告；
//   - 密钥只存在于 Go 进程，前端只能拿到来源与遮罩。
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
	path     string
	mu       sync.RWMutex
	data     fileData
	warnings []string
}

type fileData struct {
	Version   int                       `json:"version"`
	Providers map[string]string         `json:"providers,omitempty"`
	MCP       map[string]map[string]any `json:"mcp,omitempty"`
}

// Open 读取密钥文件。文件不存在时返回空存储，首次写入时创建。
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
		// 权限过宽时收紧，而不是直接拒绝启动；用户需要能修好它。
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
	if store.data.Providers == nil {
		store.data.Providers = map[string]string{}
	}
	return store, nil
}

// NewEmpty 返回一个不读取磁盘的空存储，用于密钥文件损坏时的降级启动。
func NewEmpty(dir string) *Store {
	return &Store{
		path: filepath.Join(dir, "keys.json"),
		data: fileData{Version: 1, Providers: map[string]string{}},
	}
}

// Warnings 返回打开密钥文件时产生的问题。
func (s *Store) Warnings() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return append([]string(nil), s.warnings...)
}

// Get 返回提供商密钥。文件优先，其次环境变量。
func (s *Store) Get(providerID string) string {
	s.mu.RLock()
	value := s.data.Providers[providerID]
	s.mu.RUnlock()
	if value != "" {
		return value
	}
	return os.Getenv(envName(providerID))
}

// Source 返回密钥来源：file / env / 空。
func (s *Store) Source(providerID string) string {
	s.mu.RLock()
	value := s.data.Providers[providerID]
	s.mu.RUnlock()
	if value != "" {
		return "file"
	}
	if os.Getenv(envName(providerID)) != "" {
		return "env"
	}
	return ""
}

// Masked 返回用于界面展示的遮罩，例如 sk-ab…9f2c。
func (s *Store) Masked(providerID string) string {
	value := s.Get(providerID)
	if value == "" {
		return ""
	}
	if len(value) <= 10 {
		return "••••••"
	}
	return value[:5] + "…" + value[len(value)-4:]
}

// Set 写入提供商密钥并落盘。
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

func (s *Store) persistLocked() error {
	if s.data.Version == 0 {
		s.data.Version = 1
	}
	raw, err := json.MarshalIndent(s.data, "", "  ")
	if err != nil {
		return fmt.Errorf("编码密钥文件: %w", err)
	}
	temp, err := os.CreateTemp(filepath.Dir(s.path), ".keys-*.json")
	if err != nil {
		return fmt.Errorf("创建密钥临时文件: %w", err)
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)

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
	case "gemini":
		return "GEMINI_API_KEY"
	default:
		return strings.ToUpper(providerID) + "_API_KEY"
	}
}
