//! 测试辅助：进程内唯一的临时目录命名（仅 `cfg(test)` 编译）。
//!
//! 为什么需要它：测试代码此前普遍用
//! `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos()` 给临时目录
//! 加唯一后缀。macOS 上该值的实际分辨率只有微秒级——实测连续取 10000 次有 9315 次
//! 重复。cargo test 默认并行跑测试，两个用例在同一时刻取到相同后缀就会共用同一个
//! 临时目录、同一个 SQLite 文件；迁移阶段并发写必然有人拿到 `database is locked`。
//! 这类失败在 CI 上表现为「同一提交时红时绿」，极难排查。
//!
//! 自增序号在进程内绝对唯一，与时间分辨率无关；配合进程 id 可跨进程隔离。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// 构造一个进程内唯一的临时目录路径（不创建目录，调用方自负创建与清理）。
///
/// `tag` 用于在临时目录里区分场景，便于失败时定位来源。
pub fn unique_temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("cw-{tag}-{}-{seq}", std::process::id()))
}
