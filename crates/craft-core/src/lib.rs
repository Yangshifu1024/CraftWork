//! 基础设施：事件总线、共享契约类型、ID 生成、平台能力（原子写/哈希/环境清洗/子进程）、
//! 密钥、行级 diff、出站网络配置解析。
//! 本 crate 不依赖 Tauri，可独立 cargo test。

pub mod bus;
pub mod contract;
pub mod editor;
pub mod ids;
pub mod network;
pub mod platform;
pub mod secret;
pub mod shellenv;
// 测试辅助模块：仅 `cfg(test)` 时在本 crate 内可用 + 供其他 crate 的测试使用，
// 因此不能整体 cfg(test)——跨 crate 引用时外部看不到。它不进入发布产物的关键
// 在于没有任何非测试代码引用它（`pub` 只为让别的 crate 的 test target 用得上）。
#[doc(hidden)]
pub mod testutil;
