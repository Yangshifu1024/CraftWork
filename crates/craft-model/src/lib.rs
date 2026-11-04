//! 统一模型客户端：类型、错误归一化、目录（构建期快照 + 用户提供商）、SSE、
//! OpenAI/Anthropic 适配器、重试、工厂。
//!
//! 对应 Electron 主进程 `src/main/model/`，自 P3（docs/tauri-migration/issues/04）起实现。
//! 决策 D2：完全自研，不依赖第三方 Agent 框架；协议分层参考 codex-rs（仅设计）。
