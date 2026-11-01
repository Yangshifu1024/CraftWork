// 本文件负责子进程环境变量的清洗：只保留运行所需的基础变量，
// 并剔除任何可能携带密钥的变量，防止模型密钥泄漏给工具进程。

package platform

import (
	"os"
	"sort"
	"strings"
)

// SanitizedEnv 只传递必要的环境变量，避免把模型密钥泄漏给子进程。
//
// 规则分三层：先按白名单保留基础变量与 LC_* 本地化变量；再按关键字
// 剔除 API_KEY、TOKEN、SECRET 一类敏感变量；最后剔除云服务与模型
// 提供商的专属前缀。结果排序后返回，并追加 CRAFTWORK=1 作为运行标记。
func SanitizedEnv() []string {
	// keep 是命中也必须保留的基础变量，子进程的正常运行依赖它们。
	keep := map[string]bool{
		"PATH": true, "HOME": true, "USER": true, "LOGNAME": true,
		"SHELL": true, "LANG": true, "TERM": true, "TMPDIR": true,
		"TZ": true, "PWD": true, "COLORTERM": true, "SSH_AUTH_SOCK": true,
	}
	var result []string
	for _, entry := range os.Environ() {
		// 环境变量形如 KEY=VALUE，这里只取等号前的键名。
		key := entry
		if index := strings.Index(entry, "="); index >= 0 {
			key = entry[:index]
		}
		upper := strings.ToUpper(key)
		if keep[key] || strings.HasPrefix(key, "LC_") {
			result = append(result, entry)
			continue
		}
		// 变量名统一转成大写再匹配，避免小写写法绕过检查。
		if strings.Contains(upper, "API_KEY") || strings.Contains(upper, "TOKEN") ||
			strings.Contains(upper, "SECRET") || strings.Contains(upper, "PASSWORD") ||
			strings.Contains(upper, "CREDENTIAL") {
			continue
		}
		// 云服务与模型提供商的专属前缀一律不传给子进程。
		switch {
		case strings.HasPrefix(upper, "AWS_"),
			strings.HasPrefix(upper, "GITHUB_"),
			strings.HasPrefix(upper, "GH_"),
			strings.HasPrefix(upper, "OPENAI_"),
			strings.HasPrefix(upper, "ANTHROPIC_"),
			strings.HasPrefix(upper, "DEEPSEEK_"),
			strings.HasPrefix(upper, "ZHIPU_"),
			strings.HasPrefix(upper, "DASHSCOPE_"):
			continue
		}
		result = append(result, entry)
	}
	result = append(result, "CRAFTWORK=1")
	sort.Strings(result)
	return result
}
