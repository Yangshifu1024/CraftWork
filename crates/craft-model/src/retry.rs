//! 重试装饰器：指数退避，仅对可重试错误码（rate_limit / timeout / provider_unavailable）生效。
//! 只重试「流尚未产出片段」阶段的失败：一旦向下游产出过片段就不再重试——
//! 重放会重复内容，污染会话上下文。跨模型的失败转移由调用方基于工厂的
//! fallback_model 决策，不在本层。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;

use crate::errors::{retryable_code, ModelError};
use crate::types::{Chunk, Client, Request};

/// 客户端装饰器：包裹内层 Client，对建流阶段的可重试错误做指数退避重试。
pub struct RetryClient {
    inner: Box<dyn Client>,
    attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
}

impl RetryClient {
    /// 参数依次为：最大尝试次数、基础退避毫秒、退避上限毫秒（_provider 预留未用，
    /// 错误归因由内层客户端携带）。
    pub fn new(inner: Box<dyn Client>, _provider: &str, attempts: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            inner,
            attempts,
            base_delay_ms,
            max_delay_ms,
        }
    }
}

impl Client for RetryClient {
    fn clone_box(&self) -> Box<dyn Client> {
        Box::new(RetryClient {
            inner: self.inner.clone_box(),
            attempts: self.attempts,
            base_delay_ms: self.base_delay_ms,
            max_delay_ms: self.max_delay_ms,
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
                yield Err(err);
            }
        })
    }
}
