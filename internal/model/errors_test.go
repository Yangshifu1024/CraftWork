package model

import (
	"context"
	"errors"
	"testing"
)

func TestStatusErrorMapping(t *testing.T) {
	cases := []struct {
		name      string
		status    int
		message   string
		wantCode  ErrorCode
		retryable bool
	}{
		{name: "认证失败", status: 401, message: "invalid api key", wantCode: CodeAuth},
		{name: "权限不足", status: 403, message: "permission denied", wantCode: CodeAuth},
		{name: "限流", status: 429, message: "rate limit exceeded", wantCode: CodeRateLimit, retryable: true},
		{name: "服务不可用", status: 503, message: "service unavailable", wantCode: CodeProviderUnavailable, retryable: true},
		{name: "网关超时", status: 504, message: "gateway timeout", wantCode: CodeTimeout, retryable: true},
		{name: "Anthropic 过载", status: 529, message: "overloaded", wantCode: CodeProviderUnavailable, retryable: true},
		{name: "上下文溢出", status: 400, message: "prompt is too long: 300000 tokens", wantCode: CodeContextOverflow},
		{name: "参数错误", status: 400, message: "invalid parameter", wantCode: CodeInvalidRequest},
		{name: "模型不存在", status: 404, message: "model not found", wantCode: CodeInvalidRequest},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := StatusError("测试提供商", tc.status, tc.message)
			if err.Code != tc.wantCode {
				t.Fatalf("错误码不符: got %s want %s", err.Code, tc.wantCode)
			}
			if err.Retryable != tc.retryable {
				t.Fatalf("可重试标记不符: got %v want %v", err.Retryable, tc.retryable)
			}
			if err.Status != tc.status {
				t.Fatalf("状态码不符: %d", err.Status)
			}
		})
	}
}

func TestTransportErrorMapping(t *testing.T) {
	timeout := TransportError("测试提供商", context.DeadlineExceeded)
	if timeout.Code != CodeTimeout || !timeout.Retryable {
		t.Fatalf("超时分类不符: %+v", timeout)
	}
	refused := TransportError("测试提供商", errors.New("dial tcp 127.0.0.1:443: connect: connection refused"))
	if refused.Code != CodeProviderUnavailable || !refused.Retryable {
		t.Fatalf("连接失败分类不符: %+v", refused)
	}
	canceled := TransportError("测试提供商", context.Canceled)
	if canceled.Code != CodeCanceled || canceled.Retryable {
		t.Fatalf("取消分类不符: %+v", canceled)
	}
}

func TestStreamErrorMapping(t *testing.T) {
	cases := []struct {
		message string
		kind    string
		want    ErrorCode
	}{
		{message: "overloaded_error", want: CodeProviderUnavailable},
		{message: "rate limit reached", want: CodeRateLimit},
		{message: "invalid api key", want: CodeAuth},
		{message: "context length exceeded", want: CodeContextOverflow},
		{message: "request timed out", want: CodeTimeout},
		{message: "something odd", want: CodeUnknown},
	}
	for _, tc := range cases {
		err := StreamError("测试提供商", tc.message, tc.kind)
		if err.Code != tc.want {
			t.Fatalf("%q 分类不符: got %s want %s", tc.message, err.Code, tc.want)
		}
	}
}

func TestExtractErrorMessage(t *testing.T) {
	message, kind := ExtractErrorMessage([]byte(`{"error":{"message":"模型不存在","type":"invalid_request_error"}}`))
	if message != "模型不存在" || kind != "invalid_request_error" {
		t.Fatalf("提取失败: %q %q", message, kind)
	}
	message, _ = ExtractErrorMessage([]byte(`{"error":"纯文本错误"}`))
	if message != "纯文本错误" {
		t.Fatalf("字符串错误提取失败: %q", message)
	}
	message, _ = ExtractErrorMessage([]byte("plain body"))
	if message != "plain body" {
		t.Fatalf("非 JSON 响应提取失败: %q", message)
	}
}

func TestCodeOfFallback(t *testing.T) {
	if got := CodeOf(errors.New("普通错误")); got != CodeUnknown {
		t.Fatalf("普通错误码不符: %s", got)
	}
	if got := CodeOf(NewError(CodeAuth, "测试", "未配置密钥")); got != CodeAuth {
		t.Fatalf("结构化错误码不符: %s", got)
	}
}
