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

// resolvePathAny 把路径解析到多个源文件夹之一，第一个是主要目录。
//
// 采用「存在优先」：目标真实存在于某个根时命中该根（按根顺序取最靠前者），
// 全部根都不存在时按词法命中主根——这样附加根里的同名既有文件不会被
// 主根的虚构路径抢占，而新建文件仍然落在主要目录。
func resolvePathAny(roots []string, rel string) (policy.Resolved, error) {
	return policy.ResolveExisting(roots, rel)
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
