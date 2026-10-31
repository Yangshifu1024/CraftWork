package model

import "strings"

// joinAPIPath 拼接端点与接口路径，并避免重复携带 /v1 或 /v1beta 前缀。
//
// 例如 base=https://api.anthropic.com/v1、path=/v1/messages 时，
// 结果为 https://api.anthropic.com/v1/messages。
func joinAPIPath(base, path string) string {
	base = strings.TrimRight(base, "/")
	path = strings.TrimPrefix(path, "/")
	segments := strings.SplitN(path, "/", 2)
	if len(segments) == 2 && strings.HasSuffix(base, "/"+segments[0]) {
		return base + "/" + segments[1]
	}
	return base + "/" + path
}
