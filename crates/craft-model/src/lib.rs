//! 统一模型客户端：类型、错误归一化、目录（构建期快照 + 用户提供商）、SSE、
//! OpenAI/Anthropic 适配器、重试、工厂。
//!
//! 决策 D2：完全自研，不依赖第三方 Agent 框架。
//! 内置目录快照经 include_bytes 编译期嵌入 assets/catalog/catalog.json.gz（与 modelsync 产物同源）。

pub mod anthropic;
pub mod catalog;
pub mod errors;
pub mod factory;
pub mod openai;
pub mod retry;
pub mod sse;
pub mod types;

pub use factory::{KeyProvider, ModelFactory};
pub use types::{Chunk, Client, Request, ToolCall};
