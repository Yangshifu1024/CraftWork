// 本文件提供工具包内部共用的文件系统辅助函数：忽略目录判断、
// 路径解析与展示、以及人类可读的尺寸格式化。

package tools

import (
	"fmt"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/policy"
)

// 这些目录不进入文件树与搜索，避免把依赖与构建产物塞进上下文。
var ignoredDirs = map[string]bool{
	".git": true, "node_modules": true, "dist": true, "bin": true,
	".task": true, "vendor": true, "__pycache__": true, ".venv": true,
	"build": true, ".next": true, ".cache": true,
}

// resolvePath 把相对路径解析为工作区内的绝对路径，越界时返回错误。
// 具体的安全校验（词法检查与符号链接解析）由 policy.Resolve 负责。
func resolvePath(workspace, rel string) (string, error) {
	return policy.Resolve(workspace, rel)
}

// displayPath 把绝对路径转换成相对工作区的展示路径。
func displayPath(workspace, abs string) string {
	return policy.Relative(workspace, abs)
}

// isIgnoredDir 判断目录名是否在忽略列表中，比较时忽略大小写。
func isIgnoredDir(name string) bool {
	return ignoredDirs[strings.ToLower(name)]
}

// humanSize 把字节数格式化成 B、KB、MB、GB、TB 的可读文本。
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
