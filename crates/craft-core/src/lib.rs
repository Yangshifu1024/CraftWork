//! 基础设施：事件总线、共享契约类型、ID 生成、平台能力（原子写/哈希/环境清洗/子进程）、密钥、行级 diff。
//!
//! 对应 Electron 主进程 `src/main/core/` 与 `src/shared/` 的跨进程契约；
//! 本 crate 不依赖 Tauri，可独立 cargo test。

pub mod bus;
pub mod contract;
pub mod editor;
pub mod ids;
pub mod platform;
pub mod secret;
