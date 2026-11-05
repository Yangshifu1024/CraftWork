//! 策略 crate：风险分级（R0 读 / R1 写工作区 / R2 执行 / R3 网络或越界 / R4 破坏性）
//! 与审批档位（graded 分级 / strict 严格 / readonly 只读 / free 自由）判定、
//! 命令风险分类、路径安全解析、敏感附件名识别。
//! 零第三方依赖、不依赖 Tauri，可独立 `cargo test`；处置判定统一经 `Engine` 收口，
//! 调用方（craft-tools / craft-agent）只消费 `Action` 结果。

pub mod attachment;
pub mod command;
pub mod engine;
pub mod resolve;

// 常用入口再导出：调用方无需深入 engine 模块。
pub use engine::{parse_level, reason_for, Action, Engine, Level, PolicyRisk};
