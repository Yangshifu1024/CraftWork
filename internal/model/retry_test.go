package model

import (
	"context"
	"testing"
	"time"
)

type flakyClient struct {
	calls    int
	failures int
	err      error
}

func (c *flakyClient) Stream(context.Context, Request) (<-chan Chunk, error) {
	c.calls++
	if c.calls <= c.failures {
		return nil, c.err
	}
	channel := make(chan Chunk, 1)
	channel <- Chunk{Done: true}
	close(channel)
	return channel, nil
}

func TestRetryClientRetriesRetryableErrors(t *testing.T) {
	inner := &flakyClient{failures: 2, err: NewError(CodeRateLimit, "测试", "请求过于频繁")}
	client := NewRetryClient(inner, "测试", 3, time.Millisecond)

	stream, err := client.Stream(context.Background(), Request{})
	if err != nil {
		t.Fatalf("重试后应成功: %v", err)
	}
	for range stream {
	}
	if inner.calls != 3 {
		t.Fatalf("尝试次数不符: %d", inner.calls)
	}
}

func TestRetryClientStopsAfterAttempts(t *testing.T) {
	inner := &flakyClient{failures: 10, err: NewError(CodeProviderUnavailable, "测试", "服务不可用")}
	client := NewRetryClient(inner, "测试", 3, time.Millisecond)

	if _, err := client.Stream(context.Background(), Request{}); err == nil {
		t.Fatal("应返回错误")
	}
	if inner.calls != 3 {
		t.Fatalf("尝试次数不符: %d", inner.calls)
	}
}

func TestRetryClientDoesNotRetryAuthErrors(t *testing.T) {
	inner := &flakyClient{failures: 10, err: NewError(CodeAuth, "测试", "密钥无效")}
	client := NewRetryClient(inner, "测试", 3, time.Millisecond)

	if _, err := client.Stream(context.Background(), Request{}); err == nil {
		t.Fatal("应返回错误")
	}
	if inner.calls != 1 {
		t.Fatalf("认证错误不应重试: %d", inner.calls)
	}
}
