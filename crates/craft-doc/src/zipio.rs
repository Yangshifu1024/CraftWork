//! zip 读写：条目读取与「复制其余条目、替换目标条目」的重写（rewriteZip 直译）。
//! TS 版经 JSZip；Rust 版直接操作 zip crate（全部条目 DEFLATE）。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;

/// 进程内唯一的临时文件名序号（见 [`write_entries`] 注释）。
fn next_tmp_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// 读出 zip 全部条目（name → bytes）。
pub fn load_entries(raw: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let reader = std::io::Cursor::new(raw);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("zip 打开失败：{e}"))?;
    let mut entries = HashMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("zip 条目读取失败：{e}"))?;
        let name = file.name().map(|n| n.to_string()).unwrap_or_default();
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data).map_err(|e| format!("zip 条目读取失败：{e}"))?;
        entries.insert(name, data);
    }
    Ok(entries)
}

/// 重写 zip：entries 为替换后的完整条目表；先写同目录临时文件，再原子 rename 覆盖目标。
pub fn write_entries(path: &str, entries: &HashMap<String, Vec<u8>>) -> Result<(), String> {
    // 临时文件与目标同目录，保证 rename 在同一文件系统内原子生效。
    // 唯一性用「进程 id + 进程内自增序号」：时间戳纳秒位在 macOS 上实际只有微秒精度，
    // 同一进程内两次改写可能取到相同值，临时文件互相覆盖会导致内容错乱。
    let seq = next_tmp_seq();
    let tmp = Path::new(path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".craftwork-doc-{}-{seq}.tmp", std::process::id()));
    let file = std::fs::File::create(&tmp).map_err(|e| format!("临时文件创建失败：{e}"))?;
    let mut writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // 名称排序保证输出确定性
    let mut names: Vec<&String> = entries.keys().collect();
    names.sort();
    for name in names {
        writer
            .start_file(name.as_str(), options)
            .map_err(|e| format!("zip 条目写入失败：{e}"))?;
        writer
            .write_all(&entries[name])
            .map_err(|e| format!("zip 条目写入失败：{e}"))?;
    }
    writer.finish().map_err(|e| format!("zip 收尾失败：{e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("原子替换失败：{e}")
    })
}

/// 替换 zip 中单个条目并写回（其余条目原样复制）。
pub fn rewrite_single(raw: &[u8], path: &str, target: &str, updated: &str) -> Result<(), String> {
    let mut entries = load_entries(raw)?;
    entries.insert(target.to_string(), updated.as_bytes().to_vec());
    write_entries(path, &entries)
}
