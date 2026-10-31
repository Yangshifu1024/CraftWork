// Package id 生成短小、可读、无外部依赖的标识符。
//
// 形如 th_a1b2c3d4e5f6g7h8，前缀用于日志与调试时区分实体类型。
package id

import (
	"crypto/rand"
	"encoding/hex"
)

// New 返回 "前缀_16 位十六进制" 形式的标识符。
func New(prefix string) string {
	buf := make([]byte, 8)
	if _, err := rand.Read(buf); err != nil {
		// crypto/rand 在正常系统上不会失败；真失败时退化为固定串，
		// 由数据库唯一约束在上层暴露问题，而不是在这里静默 panic。
		return prefix + "_0000000000000000"
	}
	return prefix + "_" + hex.EncodeToString(buf)
}
