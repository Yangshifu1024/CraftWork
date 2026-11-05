//! SQLite 存储：事实源。表结构与迁移链以 schemaVersion 演进，历史迁移不修改；
//! 同一连接上的访问经 Mutex 串行化（seq 分配与插入必须同事务，避免并发错序）。
//!
//! 调用约定：领域层与壳层在阻塞线程里同步调用，async 侧由壳层负责 spawn_blocking。

pub mod db;
pub mod events;
pub mod recorder;
pub mod records;
pub mod runs;
pub mod threads;
pub mod view;
pub mod workspaces;

pub use db::{Db, StoreError};
