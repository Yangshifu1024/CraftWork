// 本文件负责模型请求的重试包装：对“请求尚未建立流”阶段的可重试错误做
// 指数退避重试，避免瞬时故障直接暴露给用户。

package model

import (
	"context"
	"time"
)

// 重试策略参数。
const (
	// defaultRetryAttempts 是默认总尝试次数，包含第一次调用。
	defaultRetryAttempts = 3
	// defaultRetryBaseDelay 是第一次重试前的等待时长，之后每次翻倍。
	defaultRetryBaseDelay = 700 * time.Millisecond
	// maxRetryDelay 是单次等待时长的上限，避免退避时间无限增长。
	maxRetryDelay = 5 * time.Second
)

// retryClient 对“请求尚未建立流”阶段的可重试错误做指数退避重试。
//
// 重试只发生在尚未产生任何文本或工具调用之前，避免重复内容。流建立后
// 的错误由 Runner 按失败处理，不会再次发起请求。
type retryClient struct {
	// inner 是被包装的真实客户端。
	inner Client
	// provider 是用于拼接错误文案的提供商名称。
	provider string
	// attempts 是总尝试次数，至少为 1。
	attempts int
	// baseDelay 是第一次重试前的等待时长。
	baseDelay time.Duration
}

// NewRetryClient 包装一个客户端。attempts 是总尝试次数，至少为 1；
// baseDelay 为负数时按 0 处理。
func NewRetryClient(inner Client, provider string, attempts int, baseDelay time.Duration) Client {
	// 参数兜底：尝试次数不能小于 1，等待时长不能为负。
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

// Stream 实现 Client 接口：在尚未建立流时按指数退避重试可重试错误。
//
// 退避时长为 baseDelay 左移（attempt-1）位，并受 maxRetryDelay 限制。
// 上下文取消、不可重试错误会立即返回；所有尝试都失败时返回最后一次错误。
func (c *retryClient) Stream(ctx context.Context, req Request) (<-chan Chunk, error) {
	var lastErr error
	for attempt := 0; attempt < c.attempts; attempt++ {
		if attempt > 0 {
			// 第 1 次重试等待 baseDelay，第 2 次翻倍，以此类推，但不超过上限。
			delay := c.baseDelay << (attempt - 1)
			if delay > maxRetryDelay {
				delay = maxRetryDelay
			}
			if delay > 0 {
				timer := time.NewTimer(delay)
				select {
				case <-timer.C:
				case <-ctx.Done():
					// 等待期间被取消：停止计时并立即返回取消错误。
					timer.Stop()
					return nil, TransportError(c.provider, ctx.Err())
				}
			}
		}

		channel, err := c.inner.Stream(ctx, req)
		if err == nil {
			// 流已建立，之后的中断由 Runner 处理，不再重试。
			return channel, nil
		}
		lastErr = err
		if ctx.Err() != nil {
			return nil, TransportError(c.provider, ctx.Err())
		}
		if !Retryable(err) {
			// 鉴权、参数错误等重试也不会成功，直接返回。
			return nil, err
		}
	}
	return nil, lastErr
}

// 编译期断言：retryClient 必须实现 Client 接口。
var _ Client = (*retryClient)(nil)
