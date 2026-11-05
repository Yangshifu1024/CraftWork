//! 工具框架（Registry）与内置工具。
//! 内置工具：fs_* / apply_patch / shell_exec / git_* / todo_write / doc_*。
//! 路径解析、输出截断与写前备份由 Registry 统一处理。

pub mod doc;
pub mod fs;
pub mod registry;
pub mod shell;
pub mod todo;

pub use registry::{human_size, is_ignored_dir, more_hint, truncate_output, Assessment, Registry, Tool, ToolCtx, ToolEnv, ToolResult, ToolSpec, MAX_OUTPUT_BYTES, MAX_READ_BYTES, MAX_READ_LINES};
