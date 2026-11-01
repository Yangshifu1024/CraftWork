package platform

import (
	"bytes"
	"sync"
)

// LimitedBuffer 是并发安全的有界输出缓冲。
//
// 超过上限后继续接收写入但不再保存，并在 String 中注明截断。
type LimitedBuffer struct {
	mu       sync.Mutex
	buffer   bytes.Buffer
	limit    int
	overflow bool
}

// NewLimitedBuffer 创建有界缓冲。
func NewLimitedBuffer(limit int) *LimitedBuffer {
	return &LimitedBuffer{limit: limit}
}

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

func (b *LimitedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	text := b.buffer.String()
	if b.overflow {
		text += "\n…（输出已截断）"
	}
	return text
}
