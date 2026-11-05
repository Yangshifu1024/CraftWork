//! 模型错误归一化：错误码字符串直接传前端，文案中文可展示。
//! 状态码映射 + 关键词二次修正。

use std::fmt;

/// 错误码常量。
pub mod code {
    pub const AUTH: &str = "auth";
    pub const RATE_LIMIT: &str = "rate_limit";
    pub const TIMEOUT: &str = "timeout";
    pub const CONTEXT_OVERFLOW: &str = "context_overflow";
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    pub const INVALID_REQUEST: &str = "invalid_request";
    pub const CANCELED: &str = "canceled";
    pub const UNKNOWN: &str = "unknown";
}

/// 可重试错误码：仅这三个。
pub fn retryable_code(code: &str) -> bool {
    matches!(code, code::RATE_LIMIT | code::TIMEOUT | code::PROVIDER_UNAVAILABLE)
}

#[derive(Debug, Clone)]
pub struct ModelError {
    pub code: String,
    pub provider: String,
    pub message: String,
    pub status: u16,
}

impl ModelError {
    pub fn new(code: &str, provider: &str, message: impl Into<String>, status: u16) -> Self {
        Self {
            code: code.into(),
            provider: provider.into(),
            message: message.into(),
            status,
        }
    }

    pub fn retryable(&self) -> bool {
        retryable_code(&self.code)
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}:{}] {}", self.provider, self.code, self.message)
    }
}

impl std::error::Error for ModelError {}

fn code_for_status(status: u16) -> &'static str {
    match status {
        401 | 403 => code::AUTH,
        408 | 504 => code::TIMEOUT,
        429 => code::RATE_LIMIT,
        500 | 502 | 503 | 529 => code::PROVIDER_UNAVAILABLE,
        400 | 404 | 422 => code::INVALID_REQUEST,
        s if s >= 500 => code::PROVIDER_UNAVAILABLE,
        _ => code::INVALID_REQUEST,
    }
}

const OVERFLOW_KEYWORDS: &[&str] = &[
    "context length",
    "context_length_exceeded",
    "maximum context",
    "max context",
    "context window",
    "too many tokens",
    "prompt is too long",
    "input is too long",
    "token limit",
    "reduce the length",
    "exceeds the maximum",
    "上下文",
];

const RATE_LIMIT_KEYWORDS: &[&str] = &[
    "rate limit",
    "rate_limit",
    "too many requests",
    "quota",
    "insufficient_quota",
    "resource_exhausted",
];

const AUTH_KEYWORDS: &[&str] = &[
    "authentication",
    "unauthorized",
    "invalid api key",
    "invalid_api_key",
    "permission",
    "api key",
    "unauthenticated",
    "credentials",
];

const TIMEOUT_KEYWORDS: &[&str] = &["timeout", "timed out", "deadline", "deadline_exceeded"];

const UNAVAILABLE_KEYWORDS: &[&str] = &[
    "overloaded",
    "unavailable",
    "server error",
    "internal error",
    "connection",
    "bad gateway",
    "service_unavailable",
];

fn contains_any(text: &str, keywords: &[&str]) -> bool {
    let lower = text.to_lowercase();
    keywords.iter().any(|k| lower.contains(k))
}

/// HTTP 状态错误：先按状态码定基础码，再做文案二次修正
/// （很多提供商把上下文溢出/限流也用 400/401 返回）。
pub fn status_error(provider: &str, status: u16, body: &str) -> ModelError {
    let mut code = code_for_status(status);
    let message = {
        let extracted = extract_error_message(body);
        if extracted.is_empty() {
            body.chars().take(300).collect::<String>()
        } else {
            extracted
        }
    };
    if code == code::INVALID_REQUEST && contains_any(&message, OVERFLOW_KEYWORDS) {
        code = code::CONTEXT_OVERFLOW;
    } else if (code == code::INVALID_REQUEST || code == code::AUTH) && contains_any(&message, RATE_LIMIT_KEYWORDS) {
        code = code::RATE_LIMIT;
    }
    ModelError::new(code, provider, format!("{provider} 返回 HTTP {status}：{message}"), status)
}

/// 流内错误（HTTP 已 200）：按 message + kind 做关键词匹配。
pub fn stream_error(provider: &str, message: &str, kind: &str) -> ModelError {
    let combined = format!("{message} {kind}");
    let code = if contains_any(&combined, OVERFLOW_KEYWORDS) {
        code::CONTEXT_OVERFLOW
    } else if contains_any(&combined, RATE_LIMIT_KEYWORDS) {
        code::RATE_LIMIT
    } else if contains_any(&combined, AUTH_KEYWORDS) {
        code::AUTH
    } else if contains_any(&combined, TIMEOUT_KEYWORDS) {
        code::TIMEOUT
    } else if contains_any(&combined, UNAVAILABLE_KEYWORDS) {
        code::PROVIDER_UNAVAILABLE
    } else {
        code::UNKNOWN
    };
    ModelError::new(
        code,
        provider,
        if message.is_empty() {
            if kind.is_empty() { "模型返回错误" } else { kind }
        } else {
            message
        },
        0,
    )
}

/// 传输错误：reqwest 错误分类——超时 → timeout；连接类 → provider_unavailable；请求构建 → invalid_request。
pub fn transport_error(provider: &str, err: &reqwest::Error) -> ModelError {
    if err.is_timeout() {
        return ModelError::new(code::TIMEOUT, provider, format!("连接模型服务超时：{err}"), 0);
    }
    if err.is_connect() {
        return ModelError::new(code::PROVIDER_UNAVAILABLE, provider, format!("无法连接模型服务：{err}"), 0);
    }
    if err.is_builder() {
        return ModelError::new(code::INVALID_REQUEST, provider, format!("请求构建失败：{err}"), 0);
    }
    ModelError::new(code::PROVIDER_UNAVAILABLE, provider, format!("无法连接模型服务：{err}"), 0)
}

/// 取消错误（壳层中断流）。
pub fn canceled_error(provider: &str) -> ModelError {
    ModelError::new(code::CANCELED, provider, "已取消", 0)
}

/// 提供商缺少服务地址（如从目录添加但目录未收录 Endpoint）：配置问题，不可重试。
pub fn missing_endpoint_error(provider: &str) -> ModelError {
    ModelError::new(
        code::INVALID_REQUEST,
        provider,
        format!("{provider} 未配置服务地址，请在设置中编辑该提供商并补全 Endpoint"),
        0,
    )
}

/// 从错误响应体提取可读消息：兼容 error 对象 / error 字符串 / 顶层 message。
pub fn extract_error_message(body: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return String::new();
    };
    let err_field = parsed.get("error");
    match err_field {
        Some(serde_json::Value::String(s)) => return s.clone(),
        Some(obj @ serde_json::Value::Object(_)) => {
            if let Some(msg) = obj.get("message").and_then(|m| m.as_str()) {
                let extra = obj
                    .get("type")
                    .or_else(|| obj.get("code"))
                    .or_else(|| obj.get("status"))
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    });
                return match extra {
                    Some(extra) => format!("{msg} ({extra})"),
                    None => msg.to_string(),
                };
            }
        }
        _ => {}
    }
    parsed
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_mapping() {
        assert_eq!(code_for_status(401), code::AUTH);
        assert_eq!(code_for_status(403), code::AUTH);
        assert_eq!(code_for_status(429), code::RATE_LIMIT);
        assert_eq!(code_for_status(408), code::TIMEOUT);
        assert_eq!(code_for_status(504), code::TIMEOUT);
        assert_eq!(code_for_status(529), code::PROVIDER_UNAVAILABLE);
        assert_eq!(code_for_status(400), code::INVALID_REQUEST);
        assert_eq!(code_for_status(418), code::INVALID_REQUEST);
    }

    #[test]
    fn status_error_message_correction_to_context_overflow() {
        let err = status_error("openai", 400, r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#);
        assert_eq!(err.code, code::CONTEXT_OVERFLOW);
        assert!(!err.retryable());
    }

    #[test]
    fn status_error_401_rate_limit_keyword_corrected_to_rate_limit() {
        let err = status_error("x", 401, r#"{"error":{"message":"you have hit your rate limit quota"}}"#);
        assert_eq!(err.code, code::RATE_LIMIT);
        assert!(err.retryable());
    }

    #[test]
    fn in_stream_error_keyword_classification() {
        assert_eq!(stream_error("p", "prompt is too long", "").code, code::CONTEXT_OVERFLOW);
        assert_eq!(stream_error("p", "", "insufficient_quota").code, code::RATE_LIMIT);
        assert_eq!(stream_error("p", "invalid api key", "").code, code::AUTH);
        assert_eq!(stream_error("p", "request timed out", "").code, code::TIMEOUT);
        assert_eq!(stream_error("p", "service_unavailable", "").code, code::PROVIDER_UNAVAILABLE);
        assert_eq!(stream_error("p", "别的什么", "").code, code::UNKNOWN);
    }

    #[test]
    fn error_message_extraction_three_shapes() {
        assert_eq!(extract_error_message(r#"{"error":"plain string"}"#), "plain string");
        assert_eq!(
            extract_error_message(r#"{"error":{"message":"bad key","type":"invalid_request_error"}}"#),
            "bad key (invalid_request_error)"
        );
        assert_eq!(extract_error_message(r#"{"message":"top level"}"#), "top level");
        assert_eq!(extract_error_message("not json"), "");
    }

    #[test]
    fn retryable_error_decision() {
        assert!(ModelError::new(code::RATE_LIMIT, "p", "m", 0).retryable());
        assert!(ModelError::new(code::TIMEOUT, "p", "m", 0).retryable());
        assert!(ModelError::new(code::PROVIDER_UNAVAILABLE, "p", "m", 0).retryable());
        assert!(!ModelError::new(code::AUTH, "p", "m", 0).retryable());
        assert!(!ModelError::new(code::CONTEXT_OVERFLOW, "p", "m", 0).retryable());
    }
}
