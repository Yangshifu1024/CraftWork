// 本文件负责并发安全的有界输出缓冲，供子进程的标准输出与标准错误共用，
// 在保留有限内容的同时记录“已经发生截断”这一事实。

package platform

import (
	"bytes"
	"sync"
)

// LimitedBuffer 是并发安全的有界输出缓冲。
//
// 超过上限后继续接收写入但不再保存，并在 String 中注明截断。
// 加锁是因为标准输出与标准错误可能由不同的拷贝协程同时写入。
type LimitedBuffer struct {
	mu       sync.Mutex   // 保护 buffer 与 overflow
	buffer   bytes.Buffer // 已保存的内容
	limit    int          // 最多保存的字节数
	overflow bool         // 是否发生过截断
}

// NewLimitedBuffer 创建有界缓冲。
func NewLimitedBuffer(limit int) *LimitedBuffer {
	return &LimitedBuffer{limit: limit}
}

// Write 写入数据，超过上限时只保存能容纳的部分。
//
// 无论是否截断都返回 len(data) 与 nil，满足 io.Writer 的约定：
// 返回短写会让调用方误以为写入失败而重试或报错。
func (b *LimitedBuffer) Write(data []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	remaining := b.limit - b.buffer.Len()
	if remaining <= 0 {
		b.overflow = true
		return len(data), nil
	}
	if len(data) > remaining {
		b.buffer.Write(data[:remaining])
		b.overflow = true
		return len(data), nil
	}
	b.buffer.Write(data)
	return len(data), nil
}

// String 返回已保存的内容；发生过截断时在末尾追加提示。
func (b *LimitedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	text := b.buffer.String()
	if b.overflow {
		text += "\n…（输出已截断）"
	}
	return text
}
