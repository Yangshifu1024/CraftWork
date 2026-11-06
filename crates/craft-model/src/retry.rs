//! 重试装饰器：指数退避，仅对可重试错误码（rate_limit / timeout / provider_unavailable）生效。
//! 只重试「流尚未产出片段」阶段的失败：一旦向下游产出过片段就不再重试——
//! 重放会重复内容，污染会话上下文。跨模型的失败转移由调用方基于工厂的
//! fallback_model 决策，不在本层。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;

use crate::errors::{retryable_code, ModelError};
use crate::types::{Chunk, Client, LogHook, Request};

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
        };
        use futures_util::StreamExt;
        let chunks: Vec<_> = client.stream(&req).collect().await;
        assert!(chunks.iter().all(|r| r.is_ok()));
        let got = lines.lock().unwrap();
        assert_eq!(got.len(), 1, "一次重试应记一行: {got:?}");
        assert!(got[0].contains("retry provider=prov model=m attempt=1/3"), "{got:?}");
    }
}
