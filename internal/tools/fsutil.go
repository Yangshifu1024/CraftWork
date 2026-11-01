package tools

import (
	"fmt"
	"strings"

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

func isIgnoredDir(name string) bool {
	return ignoredDirs[strings.ToLower(name)]
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
