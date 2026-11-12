//! 附件加载：文本/图片两类，越界与敏感名拦截，尺寸与总量限制。
//! 被跳过的附件保留在结果里并带说明，让界面能解释为什么没有发送。

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use craft_model::types::Attachment;
use craft_policy::attachment::is_sensitive_attachment;
use craft_policy::resolve::resolve_existing;

pub const TEXT_LIMIT: i64 = 1024 * 1024; // 单文本 1MB
pub const IMAGE_LIMIT: i64 = 5 * 1024 * 1024; // 单图 5MB
pub const TOTAL_LIMIT: i64 = 12 * 1024 * 1024; // 总量 12MB
pub const MAX_ATTACHMENTS: usize = 10; // 单次最多附件数

/// 发给模型的图片长边上限。
///
/// 取 Anthropic 的服务端上限（1568px）：超过这个尺寸的图，服务端本来就会缩到这里，
/// 客户端多传的字节只会白占上行带宽与首字延迟。OpenAI 侧还有自己的 2048/768 二次缩放，
/// 1568 对它是无损的（仍在其处理能力内）。
pub const VISION_MAX_EDGE: u32 = 1568;

/// 缩放的返回值：新字节 + 是否真的缩过 + 重编码后的 mime（未缩时为 None）。
///
/// mime 必须跟着字节走，不能沿用扩展名：WebP/GIF 超限后会被重编码成 PNG，
/// 若仍按扩展名声明 `media_type: image/webp`，Anthropic 会以内容类型不符拒绝整条请求。
fn downscale_for_vision(bytes: Vec<u8>) -> (Vec<u8>, Option<&'static str>) {
    let Some(size) = craft_model::imageinfo::probe(&bytes) else {
        return (bytes, None);
    };
    let long_edge = size.width.max(size.height);
    if long_edge <= VISION_MAX_EDGE {
        return (bytes, None);
    }
    let Ok(img) = image::load_from_memory(&bytes) else {
        return (bytes, None);
    };
    let scaled = img.resize(VISION_MAX_EDGE, VISION_MAX_EDGE, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    // 只有 JPEG 保留 JPEG（照片类）；其余一律 PNG——webp/gif 转 PNG 是有损语义上的安全选择
    // （GIF 动图本就只会取首帧，PNG 至少保住首帧画质）。
    let (encoded, mime) = if source_is_jpeg(&bytes) {
        (scaled.write_to(&mut out, image::ImageFormat::Jpeg), "image/jpeg")
    } else {
        (scaled.write_to(&mut out, image::ImageFormat::Png), "image/png")
    };
    if encoded.is_err() {
        return (bytes, None);
    }
    let encoded = out.into_inner();
    // 缩小后反而更大时不采用（极小图片或高压缩率原图可能出现）
    if encoded.len() >= bytes.len() {
        return (bytes, None);
    }
    (encoded, Some(mime))
}

/// 从魔数判断是否为 JPEG（决定预缩后是否保留 JPEG 编码）。
fn source_is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8])
}

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

/// 附件加载结果：成功载入与被跳过的条目都在 attachments 里（保序），被跳过的以 skipped 字段说明原因。
#[derive(Debug, Clone, Default)]
pub struct LoadResult {
    pub attachments: Vec<Attachment>,
    /// 成功载入的累计字节数（被跳过的不计）
    pub total_bytes: i64,
}

/// 构造被跳过附件的占位条目（skipped 写明原因，界面据此解释为何未发送）。
fn skipped(name: &str, size: i64, reason: &str) -> Attachment {
    Attachment {
        name: name.into(),
        mime: String::new(),
        text: String::new(),
        data: Arc::from(""),
        size,
        size_px: None,
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
                    // 按 Anthropic 的 1568 长边上限预缩：服务端本来就会缩到 1568，客户端先缩
                    // 只省上行体积与首字延迟（不省 token）。缩小失败时按原图发送，不阻断加载。
                    let (bytes, resized_mime) = downscale_for_vision(bytes);
                    let size_px = craft_model::imageinfo::probe(&bytes).map(|s| (s.width, s.height));
                    // mime 跟着实际字节走：重编码过的图不能沿用扩展名推断出的类型
                    let mime = resized_mime.unwrap_or(mime);
                    if bytes.len() as i64 != size {
                        total = total - size + bytes.len() as i64;
                    }
                    attachments.push(Attachment {
                        name: name.clone(),
                        mime: mime.into(),
                        text: String::new(),
                        data: Arc::from(base64_encode(&bytes)),
                        size: bytes.len() as i64,
                        size_px,
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
                    data: Arc::from(""),
                    size,
                    size_px: None,
                    skipped: String::new(),
                });
            }
            Err(_) => attachments.push(skipped(&name, size, "读取失败")),
        }
    }
    LoadResult { attachments, total_bytes: total }
}

/// 保存工具产生的媒体（当前只有 MCP 工具）到附件目录，返回可引用的文件名列表。
///
/// 为什么写附件目录而不是临时目录：历史重建（`history.rs`）与界面缩略图
/// （`readAttachment` 通道）都以附件目录为根；放在同一处，图片在「当轮 → 下一轮 →
/// 界面回看」三个消费方之间是同一份文件，不需要各写一套读取逻辑。
///
/// 文件名形如 `<stem>-<att_id>.<ext>`（规格第 8 节）：`stem` 是采集时的名字去掉扩展名
/// （`mcp-image-1`），`att_id` 是附件 ID，两者拼起来既保留了「第几张、什么格式」的
/// 来源信息，也避免两个工具调用在同毫秒产出同名文件互相覆盖。
pub fn save_tool_media(attachment_dir: &str, media: &[craft_tools::ToolMedia]) -> Vec<String> {
    use base64::Engine;
    if std::fs::create_dir_all(attachment_dir).is_err() {
        return Vec::new();
    }
    let mut names = Vec::with_capacity(media.len());
    for item in media {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&item.base64) else {
            continue;
        };
        // 按第一个 `.` 切开：`mcp-image-1.png` → (`mcp-image-1`, `png`)。
        // 不能用 rsplit：那样取到的是扩展名，文件名会变成 `png-att_xxx.png`。
        let (stem, extension) = match item.name.split_once('.') {
            Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem, ext),
            _ => ("mcp-image", "png"),
        };
        let file_name = format!("{stem}-{}.{extension}", craft_core::ids::new_id(craft_core::ids::id_prefix::ATTACHMENT));
        let path = std::path::Path::new(attachment_dir).join(&file_name);
        if std::fs::write(&path, bytes).is_ok() {
            names.push(file_name);
        }
    }
    names
}

/// 工具结果带图时的文本说明（规格第 8 节）。
///
/// 为什么必须写进文本：模型只读 tool 消息的文字部分，图片是另挂的结构化附件。
/// 没有这句话，模型会以为工具「什么也没返回图片」；界面侧也无法解释工具行下方
/// 多出来的缩略图。落盘失败时按差额逐条占位，而不是谎报张数。
///
/// 返回空串表示「没有图片，不追加任何内容」。
pub fn tool_media_notice(saved: usize, expected: usize) -> String {
    if expected == 0 {
        return String::new();
    }
    let mut text = if saved == 0 {
        format!("[本次结果含 {expected} 张图片，但全部保存失败，已省略]")
    } else {
        format!("[本次结果含 {saved} 张图片]")
    };
    if saved < expected {
        let failed = expected - saved;
        text.push_str(&format!("\n[MCP 图片保存失败 {failed} 张，已省略]"));
    }
    text
}

/// 把媒体说明追加到工具结果文本末尾（空说明时不改文本）。
pub fn append_media_notice(output: &str, saved: usize, expected: usize) -> String {
    let notice = tool_media_notice(saved, expected);
    if notice.is_empty() {
        return output.to_string();
    }
    if output.trim().is_empty() {
        return notice;
    }
    format!("{output}\n\n{notice}")
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (String, String) {
        let dir = craft_core::testutil::unique_temp_dir("attach");
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

    /// 加载图片时必须解析出像素尺寸——上下文压缩靠它估算图片 token。
    #[test]
    fn image_attachment_records_pixel_size() {
        let (attach_dir, ws) = setup();
        // 8x6 PNG（由 python 生成的合法最小图）
        let png = make_png(8, 6);
        std::fs::write(format!("{ws}/shot.png"), &png).unwrap();
        let result = load_attachments(&ws, &attach_dir, &["shot.png".into()]);
        let att = &result.attachments[0];
        assert!(att.skipped.is_empty(), "不应被跳过：{}", att.skipped);
        assert_eq!(att.size_px, Some((8, 6)), "尺寸应被解析出来");
        assert!(!att.data.is_empty());
    }

    /// 超过长边上限的图应被预缩，且缩后长边不超过 VISION_MAX_EDGE。
    #[test]
    fn oversized_image_is_downscaled_to_vision_max_edge() {
        let (attach_dir, ws) = setup();
        // 2000x1000 纯色 PNG：长边超 1568，必然触发预缩
        let png = make_png(2000, 1000);
        std::fs::write(format!("{ws}/big.png"), &png).unwrap();
        let result = load_attachments(&ws, &attach_dir, &["big.png".into()]);
        let att = &result.attachments[0];
        assert!(att.skipped.is_empty());
        let (w, h) = att.size_px.expect("缩后仍应可解析尺寸");
        assert_eq!(w.max(h), VISION_MAX_EDGE, "长边应被缩到上限：{w}x{h}");
        // 等比：2000x1000 → 1568x784
        assert_eq!((w, h), (1568, 784));
    }

    /// 未超上限的图原样保留，不做无谓的重编码（避免白掉画质与 CPU）。
    #[test]
    fn image_within_limit_is_not_reencoded() {
        let (attach_dir, ws) = setup();
        let png = make_png(800, 600);
        std::fs::write(format!("{ws}/small.png"), &png).unwrap();
        let before = std::fs::read(format!("{ws}/small.png")).unwrap();
        let result = load_attachments(&ws, &attach_dir, &["small.png".into()]);
        let att = &result.attachments[0];
        assert_eq!(att.size_px, Some((800, 600)));
        // base64 解码后应与原文件逐字节一致
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD.decode(att.data.as_bytes()).unwrap();
        assert_eq!(decoded, before, "未超限的图不应被重编码");
    }

    /// 构造一张合法的纯色 PNG（真编码，供 image crate 解码验证）。
    fn make_png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([120, 160, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// WebP 超限被重编码成 PNG 后，mime 必须跟着字节改成 image/png。
    ///
    /// 回归点：早期实现按扩展名声明 mime，重编码后仍是 `image/webp`，
    /// Anthropic 会以「media_type 与实际内容不符」拒绝整条请求。
    #[test]
    fn reencoded_webp_reports_png_mime() {
        let (attach_dir, ws) = setup();
        let img = image::RgbImage::from_pixel(2000, 1000, image::Rgb([10, 20, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::WebP).unwrap();
        std::fs::write(format!("{ws}/big.webp"), out.into_inner()).unwrap();

        let result = load_attachments(&ws, &attach_dir, &["big.webp".into()]);
        let att = &result.attachments[0];
        assert!(att.skipped.is_empty(), "不应被跳过：{}", att.skipped);
        // 重编码后确实变小了才会被采用；采用就必须改 mime
        if att.size_px == Some((1568, 784)) {
            assert_eq!(att.mime, "image/png", "重编码为 PNG 后 mime 必须同步");
        }
    }

    /// 未超限时 mime 保持不变（不能被重编码路径误改）。
    #[test]
    fn untransformed_image_keeps_declared_mime() {
        let (attach_dir, ws) = setup();
        std::fs::write(format!("{ws}/small.webp"), {
            let img = image::RgbImage::from_pixel(400, 300, image::Rgb([1, 2, 3]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::WebP).unwrap();
            out.into_inner()
        })
        .unwrap();
        let result = load_attachments(&ws, &attach_dir, &["small.webp".into()]);
        assert_eq!(result.attachments[0].mime, "image/webp");
    }

    /// 落盘文件名严格 `<stem>-<att_id>.<ext>`（规格第 8 节）。
    /// 旧实现用 rsplit 取 stem，得到的是扩展名，文件名成了 `png-att_xxx.png`。
    #[test]
    fn tool_media_filename_keeps_stem_and_extension() {
        let dir = craft_core::testutil::unique_temp_dir("tool-media");
        let dir_str = dir.to_string_lossy().to_string();
        // 1x1 PNG 的最小合法 base64
        let media = vec![craft_tools::ToolMedia {
            name: "mcp-image-1.png".into(),
            mime: "image/png".into(),
            base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==".into(),
            size: 70,
        }];
        let names = save_tool_media(&dir_str, &media);
        assert_eq!(names.len(), 1);
        let name = &names[0];
        assert!(name.starts_with("mcp-image-1-att_"), "stem 必须保留：{name}");
        assert!(name.ends_with(".png"), "扩展名必须保留：{name}");
        // 文件真的写出来了（后面的历史重建按名读取，写失败会静默丢图）
        assert!(std::path::Path::new(&dir_str).join(name).is_file());
    }

    /// 采集名没有扩展名时回落通用名，不让 `format!` 拼出畸形文件名。
    #[test]
    fn tool_media_filename_falls_back_when_name_has_no_extension() {
        let dir = craft_core::testutil::unique_temp_dir("tool-media-named");
        let dir_str = dir.to_string_lossy().to_string();
        let media = vec![craft_tools::ToolMedia {
            name: "screenshot".into(),
            mime: "image/png".into(),
            base64: "TWFu".into(),
            size: 3,
        }];
        let names = save_tool_media(&dir_str, &media);
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("mcp-image-att_"), "无名时应回落 mcp-image：{}", names[0]);
        assert!(names[0].ends_with(".png"));
    }

    /// 无图片时不追加任何文本（避免每一条工具结果都被塞一句空话）。
    #[test]
    fn media_notice_is_empty_without_images() {
        assert_eq!(tool_media_notice(0, 0), "");
        assert_eq!(append_media_notice("原始输出", 0, 0), "原始输出");
    }

    /// 有图片时结果文本必须说明张数：模型只读文本部分，图片是另挂的结构。
    #[test]
    fn media_notice_reports_saved_count() {
        assert_eq!(tool_media_notice(2, 2), "[本次结果含 2 张图片]");
        assert_eq!(append_media_notice("截图如下", 2, 2), "截图如下\n\n[本次结果含 2 张图片]");
        // 输出本身为空时只留说明
        assert_eq!(append_media_notice("", 1, 1), "[本次结果含 1 张图片]");
    }

    /// 落盘失败按差额占位，不谎报张数。
    #[test]
    fn media_notice_reports_partial_failures() {
        let notice = tool_media_notice(1, 3);
        assert!(notice.contains("本次结果含 1 张图片"), "{notice}");
        assert!(notice.contains("保存失败 2 张"), "{notice}");
        // 全部失败也要说清楚
        let notice = tool_media_notice(0, 2);
        assert!(notice.contains("全部保存失败"), "{notice}");
    }
}
