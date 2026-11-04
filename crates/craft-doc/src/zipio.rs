//! zip 读写：条目读取与「复制其余条目、替换目标条目」的重写（rewriteZip 直译）。
//! TS 版经 JSZip；Rust 版直接操作 zip crate（全部条目 DEFLATE）。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;

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

/// 重写 zip：entries 为替换后的完整条目表；原子写回目标路径。
pub fn write_entries(path: &str, entries: &HashMap<String, Vec<u8>>) -> Result<(), String> {
    let tmp = Path::new(path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!(".craftwork-doc-{}-{}.tmp", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
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
