// Package platform 提供文件、进程与环境相关的平台原语。
//
// tools、mcp 与 service 都从这里取用同一份实现，避免原子写、哈希、
// 二进制判定与环境清洗各自维护副本。
//
// 本文件负责文件写入、内容哈希、二进制判定与备份命名。
package platform

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

// WriteFileAtomic 先写临时文件再原子替换，保留调用方指定的权限。
//
// 临时文件建在目标文件所在目录，保证替换发生在同一个文件系统内，
// rename 才是原子操作；写入或关闭失败时由 defer 删除临时文件，
// 不会留下半成品。
func WriteFileAtomic(path string, data []byte, mode os.FileMode) error {
	temp, err := os.CreateTemp(filepath.Dir(path), ".craftwork-*.tmp")
	if err != nil {
		return fmt.Errorf("创建临时文件: %w", err)
	}
	tempPath := temp.Name()
	defer os.Remove(tempPath)

	// 在写入前设置权限，避免临时文件以默认权限短暂暴露内容。
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

// Sha256Hex 返回内容的十六进制 SHA-256。
//
// 用于记录文件版本，在外部修改时配合 expectedHash 阻止覆盖。
func Sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

// IsBinary 按前 8000 字节是否包含 NUL 判断二进制内容。
//
// 这是启发式判断：文本文件几乎不会出现 NUL 字节，而常见二进制格式
// （图片、压缩包、可执行文件）在前部通常就会包含。只看前 8000 字节
// 是为了避免大文件被完整扫描。
func IsBinary(data []byte) bool {
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

// BackupName 生成带时间戳的备份文件名。
//
// 只返回文件名而不是完整路径，调用方负责与备份目录拼接；
// 时间戳精确到毫秒，减少同一秒内多次备份重名的概率。
func BackupName(absPath string) string {
	return fmt.Sprintf("%s.%s", filepath.Base(absPath), time.Now().Format("20060102-150405.000"))
}
