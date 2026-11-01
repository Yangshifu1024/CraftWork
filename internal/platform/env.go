package platform

import (
	"os"
	"sort"
	"strings"
)

// SanitizedEnv 只传递必要的环境变量，避免把模型密钥泄漏给子进程。
func SanitizedEnv() []string {
	keep := map[string]bool{
		"PATH": true, "HOME": true, "USER": true, "LOGNAME": true,
		"SHELL": true, "LANG": true, "TERM": true, "TMPDIR": true,
		"TZ": true, "PWD": true, "COLORTERM": true, "SSH_AUTH_SOCK": true,
	}
	var result []string
	for _, entry := range os.Environ() {
		key := entry
		if index := strings.Index(entry, "="); index >= 0 {
			key = entry[:index]
		}
		upper := strings.ToUpper(key)
		if keep[key] || strings.HasPrefix(key, "LC_") {
			result = append(result, entry)
			continue
		}
		if strings.Contains(upper, "API_KEY") || strings.Contains(upper, "TOKEN") ||
			strings.Contains(upper, "SECRET") || strings.Contains(upper, "PASSWORD") ||
			strings.Contains(upper, "CREDENTIAL") {
			continue
		}
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
