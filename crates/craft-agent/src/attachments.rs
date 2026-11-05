//! 附件加载：文本/图片两类，越界与敏感名拦截，尺寸与总量限制。
//! 被跳过的附件保留在结果里并带说明，让界面能解释为什么没有发送。

use std::collections::HashSet;
use std::path::Path;

use craft_model::types::Attachment;
use craft_policy::attachment::is_sensitive_attachment;
use craft_policy::resolve::resolve_existing;

pub const TEXT_LIMIT: i64 = 1024 * 1024; // 单文本 1MB
pub const IMAGE_LIMIT: i64 = 5 * 1024 * 1024; // 单图 5MB
pub const TOTAL_LIMIT: i64 = 12 * 1024 * 1024; // 总量 12MB
pub const MAX_ATTACHMENTS: usize = 10;

fn text_exts() -> &'static HashSet<&'static str> {
    static EXTS: std::sync::OnceLock<HashSet<&'static str>> = std::sync::OnceLock::new();
    EXTS.get_or_init(|| {
        [
            "txt", "md", "markdown", "go", "js", "jsx", "ts", "tsx", "json", "yaml", "yml", "toml", "csv", "tsv", "log",
            "py", "rb", "rs", "java", "kt", "c", "h", "cc", "cpp", "hpp", "cs", "php", "swift", "sh", "bash", "zsh",
            "sql", "html", "htm", "css", "scss", "xml", "ini", "cfg", "conf",
        ]
        .into_iter()
        .collect()
    })
}

fn image_mime(ext: &str) -> Option<&'static str> {
    match ext {
        ".png" => Some("image/png"),
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".gif" => Some("image/gif"),
        ".webp" => Some("image/webp"),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadResult {
    pub attachments: Vec<Attachment>,
    pub total_bytes: i64,
}

fn skipped(name: &str, size: i64, reason: &str) -> Attachment {
    Attachment {
        name: name.into(),
        mime: String::new(),
        text: String::new(),
        data: String::new(),
        size,
        skipped: reason.into(),
    }
}

/// 解析并读取附件列表：
/// - 数量截断（超出忽略）
/// - 敏感文件名拦截（密钥/环境文件不发送给模型）
/// - 路径解析：先按附件目录根，回落工作区根（两处都做越界校验；附件目录由应用管理，不受工作区限制）
pub fn load_attachments(workspace: &str, attachment_dir: &str, paths: &[String]) -> LoadResult {
    let list: Vec<&String> = paths.iter().take(MAX_ATTACHMENTS).collect();
    let mut attachments: Vec<Attachment> = Vec::new();
    let mut total: i64 = 0;

    for raw in list {
        let name = raw.rsplit('/').next().unwrap_or(raw).to_string();
        if is_sensitive_attachment(&name) {
            attachments.push(skipped(&name, 0, "密钥或环境文件不会发送给模型"));
            continue;
        }
        let mut roots: Vec<String> = Vec::new();
        if !attachment_dir.is_empty() {
            roots.push(attachment_dir.to_string());
        }
        if !workspace.is_empty() {
            roots.push(workspace.to_string());
        }
        let abs = match resolve_existing(&roots, raw) {
            Ok(resolved) => resolved.abs,
            Err(_) => {
                attachments.push(skipped(&name, 0, "路径无法解析或超出可读范围"));
                continue;
            }
        };
        let meta = match std::fs::metadata(&abs) {
            Ok(meta) => meta,
            Err(_) => {
                attachments.push(skipped(&name, 0, "文件不存在"));
                continue;
            }
        };
        if meta.is_dir() {
            attachments.push(skipped(&name, 0, "文件夹不能作为附件"));
            continue;
        }
        let size = meta.len() as i64;
        let ext = Path::new(&abs)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        if let Some(mime) = image_mime(&ext) {
            if size > IMAGE_LIMIT || total + size > TOTAL_LIMIT {
                attachments.push(skipped(&name, size, "图片超过大小限制（单图 5MB / 总量 12MB）"));
                continue;
            }
            match std::fs::read(&abs) {
                Ok(bytes) => {
                    total += size;
                    attachments.push(Attachment {
                        name: name.clone(),
                        mime: mime.into(),
                        text: String::new(),
                        data: base64_encode(&bytes),
                        size,
                        skipped: String::new(),
                    });
                }
                Err(_) => attachments.push(skipped(&name, size, "读取失败")),
            }
            continue;
        }
        let ext_name = ext.trim_start_matches('.');
        if !text_exts().contains(ext_name) {
            attachments.push(skipped(&name, size, "暂不支持该附件类型（支持文本与 png/jpg/gif/webp 图片）"));
            continue;
        }
        if size > TEXT_LIMIT || total + size > TOTAL_LIMIT {
            attachments.push(skipped(&name, size, "文件超过大小限制（单文件 1MB / 总量 12MB）"));
            continue;
        }
        match std::fs::read(&abs) {
            Ok(bytes) => {
                if bytes.contains(&0) {
                    attachments.push(skipped(&name, size, "看起来是二进制文件，无法作为文本读取"));
                    continue;
                }
                total += size;
                attachments.push(Attachment {
                    name: name.clone(),
                    mime: "text/plain".into(),
                    text: String::from_utf8_lossy(&bytes).to_string(),
                    data: String::new(),
                    size,
                    skipped: String::new(),
                });
            }
            Err(_) => attachments.push(skipped(&name, size, "读取失败")),
        }
    }
    LoadResult { attachments, total_bytes: total }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (String, String) {
        let dir = std::env::temp_dir().join(format!(
            "cw-attach-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()
        ));
        let dir_str = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(format!("{dir_str}/attach")).unwrap();
        std::fs::create_dir_all(format!("{dir_str}/ws")).unwrap();
        std::fs::write(format!("{dir_str}/ws/note.md"), "# 说明".as_bytes()).unwrap();
        std::fs::write(format!("{dir_str}/attach/secret.pem"), b"PRIVATE").unwrap();
        (dir.join("attach").to_string_lossy().to_string(), dir.join("ws").to_string_lossy().to_string())
    }

    #[test]
    fn attachment_loading_text_hit_sensitive_names_and_missing() {
        let (attach_dir, ws) = setup();
        let result = load_attachments(&ws, &attach_dir, &["note.md".into(), "secret.pem".into(), "missing.txt".into()]);
        assert_eq!(result.attachments.len(), 3);
        assert_eq!(result.attachments[0].text, "# 说明");
        assert!(result.attachments[0].skipped.is_empty());
        assert!(result.attachments[1].skipped.contains("密钥"));
        assert!(result.attachments[2].skipped.contains("不存在"));
    }

    #[test]
    fn attachment_loading_unsupported_type() {
        let (attach_dir, ws) = setup();
        std::fs::write(format!("{ws}/x.exe"), b"MZ").unwrap();
        let result = load_attachments(&ws, &attach_dir, &["x.exe".into()]);
        assert!(result.attachments[0].skipped.contains("不支持"));
    }
}
