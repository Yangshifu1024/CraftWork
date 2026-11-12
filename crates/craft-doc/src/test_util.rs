//! 测试辅助：进程内唯一的临时文件命名（仅 `cfg(test)` 编译）。
//!
//! 为什么需要它：测试代码此前用
//! `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos()` 给临时文件
//! 加唯一后缀。macOS 上该值的实际分辨率只有微秒级——实测连续取 10000 次有 9315 次
//! 重复，并行测试会撞名并互相覆盖同一个文件。
//!
//! 自增序号在进程内绝对唯一，与时间分辨率无关。

use std::sync::atomic::{AtomicU64, Ordering};

/// 构造一个进程内唯一的临时文件路径（不创建文件，调用方自负创建与清理）。
pub fn unique_temp_file(tag: &str, ext: &str) -> std::path::PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("cw-{tag}-{}-{seq}.{ext}", std::process::id()))
}
