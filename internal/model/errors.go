package model

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"strings"
)

// ErrorCode 是跨提供商统一的错误码。前端按错误码给出可操作建议。
type ErrorCode string

const (
	// CodeAuth 表示密钥缺失、无效或权限不足。
	CodeAuth ErrorCode = "auth"
	// CodeRateLimit 表示请求被限流或额度不足。
	CodeRateLimit ErrorCode = "rate_limit"
	// CodeTimeout 表示连接或响应超时。
	CodeTimeout ErrorCode = "timeout"
	// CodeContextOverflow 表示输入超过模型上下文窗口。
	CodeContextOverflow ErrorCode = "context_overflow"
	// CodeProviderUnavailable 表示服务端 5xx、过载或连接失败。
	CodeProviderUnavailable ErrorCode = "provider_unavailable"
	// CodeInvalidRequest 表示请求参数、模型名称或端点不被接受。
	CodeInvalidRequest ErrorCode = "invalid_request"
	// CodeCanceled 表示用户取消或调用方取消了上下文。
	CodeCanceled ErrorCode = "canceled"
	// CodeUnknown 表示无法归类的错误。
	CodeUnknown ErrorCode = "unknown"
)

// Error 是模型调用的结构化错误。
type Error struct {
	Code      ErrorCode
	Provider  string
	Message   string
	Status    int
	Retryable bool
	Cause     error
}

// Error 实现 error 接口。Message 是可以直接展示给用户的文案。
func (e *Error) Error() string {
	if e == nil {
		return ""
	}
	return e.Message
}

// Unwrap 保留底层错误，便于 errors.Is / errors.As 判断。
func (e *Error) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Cause
}

// NewError 创建一个结构化错误。
func NewError(code ErrorCode, provider, message string) *Error {
	return &Error{
		Code:      code,
		Provider:  provider,
		Message:   message,
		Retryable: codeRetryable(code),
	}
}

// WrapError 创建一个带底层原因的结构化错误。
func WrapError(code ErrorCode, provider, message string, cause error) *Error {
	err := NewError(code, provider, message)
	err.Cause = cause
	return err
}

// CodeOf 返回错误的统一错误码。普通错误返回 CodeUnknown。
func CodeOf(err error) ErrorCode {
	if err == nil {
		return ""
	}
	var coded *Error
	if errors.As(err, &coded) && coded.Code != "" {
		return coded.Code
	}
	switch {
	case errors.Is(err, context.Canceled):
		return CodeCanceled
	case errors.Is(err, context.DeadlineExceeded):
		return CodeTimeout
	}
	return CodeUnknown
}

// Retryable 判断错误是否适合自动重试或失败转移。
func Retryable(err error) bool {
	var coded *Error
	if errors.As(err, &coded) {
		return coded.Retryable
	}
	return false
}

func codeRetryable(code ErrorCode) bool {
	switch code {
	case CodeRateLimit, CodeTimeout, CodeProviderUnavailable:
		return true
	default:
		return false
	}
}

// StatusError 把 HTTP 状态码与提供商返回的文案转换成统一错误。
func StatusError(provider string, status int, rawMessage string) *Error {
	message := strings.TrimSpace(rawMessage)
	if message == "" {
		message = http.StatusText(status)
	}
	lower := strings.ToLower(message)
	code := codeForStatus(status)
	if code == CodeInvalidRequest && looksLikeContextOverflow(lower) {
		code = CodeContextOverflow
	}
	if code == CodeInvalidRequest && looksLikeRateLimit(lower) {
		code = CodeRateLimit
	}
	if code == CodeAuth && looksLikeRateLimit(lower) {
		code = CodeRateLimit
	}
	display := fmt.Sprintf("%s 返回 HTTP %d：%s", provider, status, message)
	err := NewError(code, provider, display)
	err.Status = status
	return err
}

// HTTPError 从响应体提取可读信息后生成统一错误。
func HTTPError(provider string, status int, body []byte) *Error {
	message, _ := ExtractErrorMessage(body)
	return StatusError(provider, status, message)
}

// StreamError 把流式响应中返回的错误对象转换成统一错误。
func StreamError(provider, message, kind string) *Error {
	message = strings.TrimSpace(message)
	if message == "" {
		message = "模型服务返回未知错误"
	}
	lower := strings.ToLower(message + " " + kind)
	code := CodeUnknown
	switch {
	case looksLikeContextOverflow(lower):
		code = CodeContextOverflow
	case looksLikeRateLimit(lower):
		code = CodeRateLimit
	case looksLikeAuth(lower):
		code = CodeAuth
	case looksLikeTimeout(lower):
		code = CodeTimeout
	case looksLikeUnavailable(lower):
		code = CodeProviderUnavailable
	}
	return NewError(code, provider, message)
}

// TransportError 把网络层错误转换成统一错误。
func TransportError(provider string, err error) *Error {
	if err == nil {
		return NewError(CodeUnknown, provider, "未知网络错误")
	}
	switch {
	case errors.Is(err, context.Canceled):
		return WrapError(CodeCanceled, provider, "请求已取消", err)
	case errors.Is(err, context.DeadlineExceeded):
		return WrapError(CodeTimeout, provider, "请求超时：模型在限定时间内没有返回", err)
	}
	var netErr net.Error
	if errors.As(err, &netErr) && netErr.Timeout() {
		return WrapError(CodeTimeout, provider, "连接模型服务超时，请检查网络或代理设置", err)
	}
	return WrapError(CodeProviderUnavailable, provider, "无法连接模型服务："+err.Error(), err)
}

// ExtractErrorMessage 从各提供商的错误响应体中提取 message 字段。
func ExtractErrorMessage(body []byte) (string, string) {
	trimmed := strings.TrimSpace(string(body))
	if trimmed == "" {
		return "", ""
	}
	var payload struct {
		Error   json.RawMessage `json:"error"`
		Message string          `json:"message"`
		Type    string          `json:"type"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		return trimmed, ""
	}
	if len(payload.Error) > 0 {
		var object struct {
			Message string `json:"message"`
			Type    string `json:"type"`
			Code    string `json:"code"`
			Status  string `json:"status"`
		}
		if err := json.Unmarshal(payload.Error, &object); err == nil {
			kind := firstNonEmpty(object.Type, object.Code, object.Status)
			if object.Message != "" {
				return object.Message, kind
			}
		}
		var text string
		if err := json.Unmarshal(payload.Error, &text); err == nil && text != "" {
			return text, ""
		}
	}
	if payload.Message != "" {
		return payload.Message, payload.Type
	}
	return trimmed, payload.Type
}

func codeForStatus(status int) ErrorCode {
	switch status {
	case http.StatusUnauthorized, http.StatusForbidden:
		return CodeAuth
	case http.StatusRequestTimeout:
		return CodeTimeout
	case http.StatusTooManyRequests:
		return CodeRateLimit
	case http.StatusGatewayTimeout:
		return CodeTimeout
	case http.StatusInternalServerError, http.StatusBadGateway,
		http.StatusServiceUnavailable, 529: // 529 是 Anthropic 的过载状态码
		return CodeProviderUnavailable
	case http.StatusBadRequest, http.StatusNotFound, http.StatusUnprocessableEntity:
		return CodeInvalidRequest
	default:
		if status >= 500 {
			return CodeProviderUnavailable
		}
		return CodeInvalidRequest
	}
}

func looksLikeContextOverflow(text string) bool {
	for _, keyword := range []string{
		"context length", "context_length_exceeded", "maximum context", "max context",
		"context window", "too many tokens", "prompt is too long", "input is too long",
		"token limit", "reduce the length", "exceeds the maximum", "上下文",
	} {
		if strings.Contains(text, keyword) {
			return true
		}
	}
	return false
}

func looksLikeRateLimit(text string) bool {
	for _, keyword := range []string{
		"rate limit", "rate_limit", "too many requests", "quota", "insufficient_quota",
		"resource_exhausted",
	} {
		if strings.Contains(text, keyword) {
			return true
		}
	}
	return false
}

func looksLikeAuth(text string) bool {
	for _, keyword := range []string{
		"authentication", "unauthorized", "invalid api key", "invalid_api_key",
		"permission", "api key", "unauthenticated", "credentials",
	} {
		if strings.Contains(text, keyword) {
			return true
		}
	}
	return false
}

func looksLikeTimeout(text string) bool {
	for _, keyword := range []string{"timeout", "timed out", "deadline", "deadline_exceeded"} {
		if strings.Contains(text, keyword) {
			return true
		}
	}
	return false
}

func looksLikeUnavailable(text string) bool {
	for _, keyword := range []string{
		"overloaded", "unavailable", "server error", "internal error", "connection",
		"bad gateway", "service_unavailable",
	} {
		if strings.Contains(text, keyword) {
			return true
		}
	}
	return false
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}
