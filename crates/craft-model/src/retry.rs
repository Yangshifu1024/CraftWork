//! 重试装饰器：指数退避，仅对可重试错误码（rate_limit / timeout / provider_unavailable）生效。
//! 只重试「流尚未产出片段」阶段的失败：一旦向下游产出过片段就不再重试——
//! 重放会重复内容，污染会话上下文。跨模型的失败转移由调用方基于工厂的
//! fallback_model 决策，不在本层。
//!
//! 另含 max_tokens 超限自愈装饰器：目录带入的输出上限（models.dev 快照，官方 API 值）
//! 可能超过销售网关的配额（如编码计划限制 [1,131072]），网关以 invalid_request 400
//! 拒绝且限制值只出现在错误消息里。自愈层从消息解析出上限、clamp 后立即重发一次。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;

use crate::errors::{retryable_code, ModelError};
use crate::types::{Chunk, Client, LogHook, Request};

/// 从错误消息解析 max_tokens 上限：消息需含 "max_tokens"（大小写不敏感），
/// 收集其中出现的十进制数、剔除与当前请求值相同的数后取最大——「请求值 307200
/// 超过上限 8192」剔除请求值才能读到 8192，「限制数值范围[1,131072]」取到 131072。
/// 解析不出返回 None（调用方保持原错误透传）。
pub fn parse_max_tokens_cap(message: &str, current: i64) -> Option<i64> {
    if !message.to_ascii_lowercase().contains("max_tokens") {
        return None;
    }
    let mut best: Option<i64> = None;
    let mut cur: Option<i64> = None;
    for ch in message.chars() {
        if let Some(d) = ch.to_digit(10) {
            cur = Some(cur.unwrap_or(0).saturating_mul(10).saturating_add(i64::from(d)));
        } else if let Some(v) = cur.take()
            && v != current
            && best.is_none_or(|b| v > b)
        {
            best = Some(v);
        }
    }
    if let Some(v) = cur.take()
        && v != current
        && best.is_none_or(|b| v > b)
    {
        best = Some(v);
    }
    best.filter(|v| *v > 0)
}

/// max_tokens 超限自愈装饰器：建流阶段收到 invalid_request 且消息可解析出比当前
/// 更小的上限时，clamp 后立即重发一次（仅一次；产出片段后或解析失败都原样透传）。
pub struct ClampMaxTokensClient {
    inner: Box<dyn Client>,
    /// 提供商显示名（自愈日志的归因上下文）
    provider: String,
    /// 观测日志回调（自愈事件；None=不记录）
    log: Option<LogHook>,
}

impl ClampMaxTokensClient {
    pub fn new(inner: Box<dyn Client>, provider: &str, log: Option<LogHook>) -> Self {
        Self { inner, provider: provider.to_string(), log }
    }

    fn log_line(&self, message: &str) {
        if let Some(log) = &self.log {
            log(message);
        }
    }
}

impl Client for ClampMaxTokensClient {
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(ClampMaxTokensClient {
            inner: self.inner.clone_box(),
            provider: self.provider.clone(),
            log: self.log.clone(),
        })
    }

    /// 自愈语义见模块文档：仅建流阶段、仅一次、仅 invalid_request + max_tokens 超限。
    fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
        Box::pin(async_stream::stream! {
            let mut produced = false;
            let mut stream = self.inner.stream(req);
            while let Some(item) = stream.next().await {
                let Err(err) = item else {
                    produced = true;
                    yield item;
                    continue;
                };
                // 产出过片段 / 非 invalid_request / 无 max_tokens 参数：原样透传
                if produced || err.code != crate::errors::code::INVALID_REQUEST || req.max_tokens <= 0 {
                    yield Err(err);
                    return;
                }
                let Some(cap) = parse_max_tokens_cap(&err.message, req.max_tokens)
                    .filter(|cap| *cap < req.max_tokens)
                else {
                    yield Err(err);
                    return;
                };
                self.log_line(&format!(
                    "retry_max_tokens_clamp provider={} model={} prev={} cap={}",
                    self.provider, req.model, req.max_tokens, cap,
                ));
                let mut healed = req.clone();
                healed.max_tokens = cap;
                let mut retry = self.inner.stream(&healed);
                while let Some(next) = retry.next().await {
                    yield next;
                }
                return;
            }
        })
    }
}

/// 客户端装饰器：包裹内层 Client，对建流阶段的可重试错误做指数退避重试。
pub struct RetryClient {
    inner: Box<dyn Client>,
    /// 提供商显示名（重试日志的归因上下文）
    provider: String,
    attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
    /// 观测日志回调（重试与耗尽事件；None=不记录）
    log: Option<LogHook>,
}

impl RetryClient {
    /// 参数依次为：最大尝试次数、基础退避毫秒、退避上限毫秒（provider 用于重试日志归因，
    /// 错误归因由内层客户端携带）。
    pub fn new(
        inner: Box<dyn Client>,
        provider: &str,
        attempts: u32,
        base_delay_ms: u64,
        max_delay_ms: u64,
        log: Option<LogHook>,
    ) -> Self {
        Self {
            inner,
            provider: provider.to_string(),
            attempts,
            base_delay_ms,
            max_delay_ms,
            log,
        }
    }

    fn log_line(&self, message: &str) {
        if let Some(log) = &self.log {
            log(message);
        }
    }
}

impl Client for RetryClient {
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(RetryClient {
            inner: self.inner.clone_box(),
            provider: self.provider.clone(),
            attempts: self.attempts,
            base_delay_ms: self.base_delay_ms,
            max_delay_ms: self.max_delay_ms,
            log: self.log.clone(),
        })
    }

    /// 重试语义见模块文档：错误在产出片段前且可重试才退避重来，否则原样透传给下游。
    fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
        let attempts = self.attempts;
        let base = self.base_delay_ms;
        let max = self.max_delay_ms;
        Box::pin(async_stream::stream! {
            let mut last_err: Option<ModelError> = None;
            for attempt in 1..=attempts {
                let mut produced = false;
                let mut stream = self.inner.stream(req);
                loop {
                    match stream.next().await {
                        Some(Ok(chunk)) => {
                            produced = true;
                            yield Ok(chunk);
                        }
                        Some(Err(err)) => {
                            // 一旦产出过片段，任何错误都直接终止（不重试）
                            if produced || attempt >= attempts || !retryable_code(&err.code) {
                                yield Err(err);
                                break;
                            }
                            self.log_line(&format!(
                                "retry provider={} model={} attempt={}/{} code={} delay_ms={}",
                                self.provider,
                                req.model,
                                attempt,
                                attempts,
                                err.code,
                                (base.saturating_mul(1u64 << (attempt - 1).min(16))).min(max),
                            ));
                            last_err = Some(err);
                            break;
                        }
                        None => {
                            // 正常结束
                            return;
                        }
                    }
                }
                // 到这里说明本次尝试可重试失败：退避后重试
                let Some(err) = last_err.clone() else { return };
                // 指数退避：base * 2^(attempt-1)，封顶 max（移位上限 16 防溢出）
                let delay = (base.saturating_mul(1u64 << (attempt - 1).min(16))).min(max);
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                let _ = err;
            }
            if let Some(err) = last_err {
                self.log_line(&format!(
                    "retry_exhausted provider={} model={} code={} attempts={}",
                    self.provider, req.model, err.code, attempts
                ));
                yield Err(err);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DoneVia;

    #[test]
    fn parses_cap_from_gateway_range_message() {
        // zhipuai 编码计划网关的实际消息形态（含 HTTP 状态码与错误码噪声）
        let msg = "Zhipu AI Coding Plan 返回 HTTP 400：max_tokens参数非法：限制数值范围[1,131072] (1210)";
        assert_eq!(parse_max_tokens_cap(msg, 307_200), Some(131_072));
    }

    #[test]
    fn parses_cap_from_english_message_excluding_requested_value() {
        let msg = "max_tokens must be between 1 and 4096, got 307200";
        assert_eq!(parse_max_tokens_cap(msg, 307_200), Some(4_096));
        // 「请求值 X 超过上限 Y」形态：剔除请求值后读到上限
        let msg = "max_tokens参数非法：请求值 307200 超过上限 8192";
        assert_eq!(parse_max_tokens_cap(msg, 307_200), Some(8_192));
    }

    #[test]
    fn rejects_message_without_max_tokens_or_numbers() {
        assert_eq!(parse_max_tokens_cap("invalid model name", 4096), None);
        assert_eq!(parse_max_tokens_cap("max_tokens 参数非法", 4096), None);
    }

    #[test]
    fn rejects_cap_not_smaller_than_current() {
        // 消息里只有当前请求值本身：剔除后无候选，返回 None（调用方透传原错误）
        assert_eq!(parse_max_tokens_cap("max_tokens 307200 超限", 307_200), None);
        // 候选不比当前小：解析层不做大小比较（Some 透传），由调用方 filter 拦截不重试
        assert_eq!(parse_max_tokens_cap("max_tokens limit is 307200", 131_072), Some(307_200));
        assert_eq!(parse_max_tokens_cap("max_tokens 0 无效", 131_072), None);
    }

    /// stub 客户端：记录每次收到的 max_tokens；第一次回 max_tokens 超限错误，
    /// 之后按记录到的值成功收尾。
    struct ClampThenOk {
        seen: std::sync::Arc<std::sync::Mutex<Vec<i64>>>,
    }

    impl Client for ClampThenOk {
        fn stream<'a>(&'a self, req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
            self.seen.lock().unwrap().push(req.max_tokens);
            let first = self.seen.lock().unwrap().len() == 1;
            Box::pin(futures_util::stream::once(async move {
                if first {
                    Err(ModelError::new(
                        crate::errors::code::INVALID_REQUEST,
                        "p",
                        "max_tokens参数非法：限制数值范围[1,131072] (1210)",
                        400,
                    ))
                } else {
                    Ok(Chunk { done: true, end: Some(crate::types::StreamEnd { done_via: DoneVia::DoneLine, ..Default::default() }), ..Default::default() })
                }
            }))
        }

        fn clone_box(&self) -> Box<dyn Client> {
            Box::new(ClampThenOk { seen: std::sync::Arc::clone(&self.seen) })
        }
    }

    fn base_request(max_tokens: i64) -> Request {
        Request {
            model: "m".into(),
            mode: String::new(),
            system: vec![],
            messages: vec![],
            tools: vec![],
            max_tokens,
            reasoning_kind: String::new(),
            reasoning_level: String::new(),
            session_id: String::new(),
        }
    }

    #[tokio::test]
    async fn clamps_max_tokens_and_retries_once_with_log() {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&lines);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let client = ClampMaxTokensClient::new(
            Box::new(ClampThenOk { seen: std::sync::Arc::clone(&seen) }),
            "Zhipu AI Coding Plan",
            Some(std::sync::Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string()))),
        );
        let req = base_request(307_200);
        use futures_util::StreamExt;
        let chunks: Vec<_> = client.stream(&req).collect().await;
        assert!(chunks.iter().all(|r| r.is_ok()), "clamp 后应成功: {chunks:?}");
        assert_eq!(seen.lock().unwrap().as_slice(), &[307_200, 131_072], "第二次请求应携带 clamp 后的值");
        let got = lines.lock().unwrap();
        assert_eq!(got.len(), 1, "自愈应记一行: {got:?}");
        assert!(got[0].contains("retry_max_tokens_clamp provider=Zhipu AI Coding Plan model=m prev=307200 cap=131072"), "{got:?}");
    }

    /// stub 客户端：恒回 invalid_request 超限错误（自愈后仍失败）。
    struct AlwaysClampFail;

    impl Client for AlwaysClampFail {
        fn stream<'a>(&'a self, _req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
            Box::pin(futures_util::stream::once(async {
                Err(ModelError::new(crate::errors::code::INVALID_REQUEST, "p", "max_tokens 超限: [1,131072]", 400))
            }))
        }

        fn clone_box(&self) -> Box<dyn Client> {
            Box::new(AlwaysClampFail)
        }
    }

    #[tokio::test]
    async fn clamps_only_once_then_passes_error_through() {
        let client = ClampMaxTokensClient::new(Box::new(AlwaysClampFail), "prov", None);
        let req = base_request(307_200);
        use futures_util::StreamExt;
        let chunks: Vec<_> = client.stream(&req).collect().await;
        assert_eq!(chunks.len(), 1, "自愈仅一次，二次失败原样透传");
        assert!(chunks[0].is_err());
    }

    /// stub 客户端：先产出一个片段，然后回 invalid_request。
    struct ProduceThenFail;

    impl Client for ProduceThenFail {
        fn stream<'a>(&'a self, _req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
            Box::pin(futures_util::stream::iter(vec![
                Ok(Chunk { text: Some("hi".into()), ..Default::default() }),
                Err(ModelError::new(crate::errors::code::INVALID_REQUEST, "p", "max_tokens 超限: [1,131072]", 400)),
            ]))
        }

        fn clone_box(&self) -> Box<dyn Client> {
            Box::new(ProduceThenFail)
        }
    }

    #[tokio::test]
    async fn no_clamp_after_chunks_produced() {
        let client = ClampMaxTokensClient::new(Box::new(ProduceThenFail), "prov", None);
        let req = base_request(307_200);
        use futures_util::StreamExt;
        let chunks: Vec<_> = client.stream(&req).collect().await;
        assert_eq!(chunks.len(), 2, "产出片段后不自愈，错误直接透传");
        assert!(chunks[0].is_ok());
        assert!(chunks[1].is_err());
    }

    /// stub 客户端：固定产出 N 次可重试错误后成功。
    struct FailThenOk {
        failures: std::sync::atomic::AtomicU32,
    }

    impl Client for FailThenOk {
        fn stream<'a>(&'a self, _req: &'a Request) -> BoxStream<'a, Result<Chunk, ModelError>> {
            let n = self.failures.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(futures_util::stream::once(async move {
                if n > 1 {
                    Err(ModelError::new("rate_limit", "p", "慢点", 1))
                } else {
                    Ok(Chunk { done: true, end: Some(crate::types::StreamEnd { done_via: DoneVia::DoneLine, ..Default::default() }), ..Default::default() })
                }
            }))
        }

        fn clone_box(&self) -> Box<dyn Client> {
            Box::new(FailThenOk { failures: std::sync::atomic::AtomicU32::new(self.failures.load(std::sync::atomic::Ordering::SeqCst)) })
        }
    }

    #[tokio::test]
    async fn retry_attempts_logged_via_hook() {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&lines);
        let client = RetryClient::new(
            Box::new(FailThenOk { failures: std::sync::atomic::AtomicU32::new(2) }),
            "prov",
            3,
            1,
            4,
            Some(std::sync::Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string()))),
        );
        let req = Request {
            model: "m".into(),
            mode: String::new(),
            system: vec![],
            messages: vec![],
            tools: vec![],
            max_tokens: 0,
            reasoning_kind: String::new(),
            reasoning_level: String::new(),
            session_id: String::new(),
        };
        use futures_util::StreamExt;
        let chunks: Vec<_> = client.stream(&req).collect().await;
        assert!(chunks.iter().all(|r| r.is_ok()));
        let got = lines.lock().unwrap();
        assert_eq!(got.len(), 1, "一次重试应记一行: {got:?}");
        assert!(got[0].contains("retry provider=prov model=m attempt=1/3"), "{got:?}");
    }
}
