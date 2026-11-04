//! 工具框架（Registry）与内置工具：fs_*、apply_patch、shell_exec、git_*、doc_*、xlsx_set_cell、todo_write。
//!
//! 对应 Electron 主进程 `src/main/tools/`，自 P4（docs/tauri-migration/issues/05）起实现；
//! shell_exec 完整执行体在 P6（issues/07）落地。路径解析、输出截断与写前备份由 Registry 统一处理。
