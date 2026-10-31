package model

import (
	"context"
	"time"
)

const (
	defaultRetryAttempts  = 3
	defaultRetryBaseDelay = 700 * time.Millisecond
	maxRetryDelay         = 5 * time.Second
)

// retryClient 对“请求尚未建立流”阶段的可重试错误做指数退避重试。
//
// 重试只发生在尚未产生任何文本或工具调用之前，避免重复内容。流建立后
// 的错误由 Runner 按失败处理，不会再次发起请求。
type retryClient struct {
	inner     Client
	provider  string
	attempts  int
	baseDelay time.Duration
}

// NewRetryClient 包装一个客户端。attempts 是总尝试次数，至少为 1。
func NewRetryClient(inner Client, provider string, attempts int, baseDelay time.Duration) Client {
	if attempts < 1 {
		attempts = 1
	}
	if baseDelay < 0 {
		baseDelay = 0
	}
	return &retryClient{
		inner:     inner,
		provider:  provider,
		attempts:  attempts,
		baseDelay: baseDelay,
	}
}

func (c *retryClient) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	var lastErr error
	for attempt := 0; attempt < c.attempts; attempt++ {
		if attempt > 0 {
			delay := c.baseDelay << (attempt - 1)
			if delay > maxRetryDelay {
				delay = maxRetryDelay
			}
			if delay > 0 {
				timer := time.NewTimer(delay)
				select {
				case <-timer.C:
				case <-ctx.Done():
					timer.Stop()
					return nil, TransportError(c.provider, ctx.Err())
				}
			}
		}

		channel, err := c.inner.Stream(ctx, req)
		if err == nil {
			return channel, nil
		}
		lastErr = err
		if ctx.Err() != nil {
			return nil, TransportError(c.provider, ctx.Err())
		}
		if !Retryable(err) {
			return nil, err
		}
	}
	return nil, lastErr
}

var _ Client = (*retryClient)(nil)
