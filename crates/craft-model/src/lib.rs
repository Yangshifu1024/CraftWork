//! 统一模型客户端（craft-model）：请求与流式类型、错误归一化、模型目录（构建期快照 + 用户提供商）、
//! SSE 公共骨架、OpenAI/Anthropic 协议适配器、重试装饰器与客户端工厂。
//!
//! 决策 D2：完全自研，不依赖第三方 Agent 框架。协议仅三种：openai / anthropic / openai-compatible。
//! 内置模型目录是 models.dev 的构建期快照（assets/catalog/catalog.json.gz 经 include_bytes
//! 编译期嵌入，与 modelsync 产物同源），应用不内置任何可用提供商——目录只提供「可添加候选」，
//! 可调用的提供商必须由用户添加。

pub mod anthropic;
pub mod catalog;
pub mod errors;
pub mod factory;
pub mod imageinfo;
pub mod integrity;
pub mod net;
pub mod openai;
pub mod retry;
pub mod sse;
pub mod types;

pub use factory::{KeyProvider, ModelFactory};
pub use types::{Chunk, Client, Request, ToolCall};
