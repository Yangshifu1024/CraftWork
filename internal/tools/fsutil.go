package tools

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"craftwork/internal/policy"
)

// 这些目录不进入文件树与搜索，避免把依赖与构建产物塞进上下文。
var ignoredDirs = map[string]bool{
	".git": true, "node_modules": true, "dist": true, "bin": true,
	".task": true, "vendor": true, "__pycache__": true, ".venv": true,
	"build": true, ".next": true, ".cache": true,
}

func resolvePath(workspace, rel string) (string, error) {
	return policy.Resolve(workspace, rel)
}

func displayPath(workspace, abs string) string {
	return policy.Relative(workspace, abs)
}

func backupName(absPath string) string {
	base := filepath.Base(absPath)
	return fmt.Sprintf("%s.%s", base, time.Now().Format("20060102-150405.000"))
}

func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

func isBinary(data []byte) bool {
	limit := len(data)
	if limit > 8000 {
		limit = 8000
	}
	for _, value := range data[:limit] {
		if value == 0 {
			return true
		}
	}
	return false
}

func isIgnoredDir(name string) bool {
	return ignoredDirs[strings.ToLower(name)]
}

func writeAtomic(path string, data []byte, mode os.FileMode) error {
	temp, err := os.CreateTemp(filepath.Dir(path), ".craftwork-*.tmp")
	if err != nil {
		return fmt.Errorf("创建临时文件: %w", err)
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)

	if err := temp.Chmod(mode); err != nil {
		temp.Close()
		return fmt.Errorf("设置临时文件权限: %w", err)
	}
	if _, err := temp.Write(data); err != nil {
		temp.Close()
		return fmt.Errorf("写入临时文件: %w", err)
	}
	if err := temp.Close(); err != nil {
		return fmt.Errorf("关闭临时文件: %w", err)
	}
	if err := os.Rename(tempPath, path); err != nil {
		return fmt.Errorf("替换文件: %w", err)
	}
	return nil
}

func humanSize(size int64) string {
	const unit = 1024
	if size < unit {
		return fmt.Sprintf("%d B", size)
	}
	value := float64(size)
	units := []string{"KB", "MB", "GB"}
	for _, suffix := range units {
		value /= unit
		if value < unit {
			return fmt.Sprintf("%.1f %s", value, suffix)
		}
	}
	return fmt.Sprintf("%.1f TB", value/unit)
}
