//! 工具框架（Registry）与内置工具。
//! 对应 Electron 主进程 `src/main/tools/`：fs_* / apply_patch / shell_exec / git_* / todo_write；
//! doc_* 在 P7（docs/tauri-migration/issues/08）接入 craft-doc 后启用。
//! 路径解析、输出截断与写前备份由 Registry 统一处理。

pub mod doc;
pub mod fs;
pub mod registry;
pub mod shell;
pub mod todo;

pub use registry::{human_size, is_ignored_dir, more_hint, truncate_output, Assessment, Registry, Tool, ToolCtx, ToolEnv, ToolResult, ToolSpec, MAX_OUTPUT_BYTES, MAX_READ_BYTES, MAX_READ_LINES};
