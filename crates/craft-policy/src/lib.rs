//! 策略：风险分级与审批档位、命令分类、路径安全解析、敏感附件名。
//! 本 crate 零依赖、可独立测试。

pub mod attachment;
pub mod command;
pub mod engine;
pub mod resolve;

pub use engine::{parse_level, reason_for, Action, Engine, Level, PolicyRisk};
