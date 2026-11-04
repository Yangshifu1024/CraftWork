//! SQLite 存储：打开/迁移、threads、workspaces、events（快照重建）、runs/approvals、审计/用量/设置/备份。
//!
//! 对应 Electron 主进程 `src/main/store/`，自 P2（docs/tauri-migration/issues/03）起实现。
//! 核心约束：事件写入在事务内完成（seq 分配与插入同事务），单写线程 + WAL。
