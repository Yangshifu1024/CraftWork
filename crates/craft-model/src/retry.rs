//! 重试装饰器：指数退避，只对可重试错误生效。
//! 只对「请求尚未建立流」阶段的可重试错误重试：一旦产出片段就不再重试——
//! 重复内容会污染会话上下文。

use futures_core::stream::BoxStream;
use futures_util::StreamExt;

use crate::errors::{retryable_code, ModelError};
use crate::types::{Chunk, Client, Request};

pub struct RetryClient {
    inner: Box<dyn Client>,
    attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
}

impl RetryClient {
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
