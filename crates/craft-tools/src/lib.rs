//! 工具框架（Registry）与内置工具：fs_* / apply_patch / shell_exec / git_* /
//! todo_write / doc_*。
//!
//! 架构位置：本 crate 是 Agent 的工具层，不依赖 Tauri；壳层（src-tauri）负责把
//! 内置工具注册进 [`registry::Registry`]。工具实现 [`registry::Tool`] trait
//! （spec + execute，同步执行，调用方负责放入阻塞线程）；路径解析、输出截断与
//! 写前备份（ctx.env.backup）由 Registry 统一处理，单个工具不自行重复这些横切逻辑。

pub mod doc;
pub mod fs;
pub mod plan;
pub mod registry;
pub mod shell;
pub mod skills;
pub mod todo;

pub use registry::{human_size, is_ignored_dir, more_hint, truncate_output, Assessment, Registry, Tool, ToolCtx, ToolEnv, ToolResult, ToolSpec, MAX_OUTPUT_BYTES, MAX_READ_BYTES, MAX_READ_LINES};
