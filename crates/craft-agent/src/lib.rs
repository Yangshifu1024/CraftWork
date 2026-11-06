//! Agent：Run 循环、审批状态机、授权缓存、历史重建、上下文压缩、自动命名、
//! 系统提示词组装、附件加载、环境上下文。
//!
//! 循环结构参考 openai/codex（仅参考设计）。
//! 取消语义用 CancellationToken（对应 TS AbortController）。

pub mod approvals;
pub mod attachments;
pub mod context;
pub mod envcontext;
pub mod grants;
pub mod history;
pub mod prompt;
pub mod runner;
pub mod session_log;
pub mod title;

pub use approvals::{ApprovalOutcome, ApprovalRequest, ApprovalScope, Approvals};
pub use grants::{command_prefix, Grants};
pub use runner::{estimate_cost_micros, RunEnv, Runner, RunnerDeps, SteerItem};
pub use session_log::SessionLog;
