//! MCP 图片内容归一：把 `image` 内容块变成可落盘的媒体，超限与不支持的类型降级为文本。
//!
//! 预算取值与用户附件一致（单图 5MB / 单次 4 张），理由：模型侧走的是同一条
//! `Attachment` 通道，超预算的图片无论来源都会被 provider 拒绝或拖慢请求。
//!
//! 另一条可选路线是在内存里做 base64 内联预算 + 超限压缩；CraftWork 直接把图片
//! 写进 `attachments/` 目录再以文件名引用，模型侧由
//! `Attachment.data` 携带 base64，界面侧用 `readAttachment` 懒加载——因此这里只做
//! 「大小/类型/张数」的把关，不做压缩（不引入图像处理依赖）。

/// 单张图片的原始字节上限（与 craft-agent::attachments::IMAGE_LIMIT 一致）。
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// 单次工具结果最多携带的图片张数。
pub const MAX_IMAGES_PER_RESULT: usize = 4;

/// 允许的图片 MIME 类型（与附件白名单一致：png/jpeg/gif/webp）。
const ALLOWED_MIME: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// 一张待落盘的图片。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolMedia {
    pub name: String,
    pub mime: String,
    /// base64 编码后的内容（MCP 协议本身就是 base64，无需再编解码一遍）
    pub base64: String,
    /// 原始字节数
    pub size: usize,
}

impl ToolMedia {
    /// 按 mime 推断落盘扩展名；未知类型回落 `.bin`。
    pub fn extension(&self) -> &'static str {
        match self.mime.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "bin",
        }
    }
}

/// 一次结果里收集图片时的状态（张数与累计字节）。
#[derive(Debug, Default)]
pub struct MediaCollector {
    pub media: Vec<ToolMedia>,
    /// 被降级为文本占位的说明（按出现顺序，追加在文本结果里）
    pub notes: Vec<String>,
}

impl MediaCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// 尝试收下一个 MCP `image` 块；不满足条件时记一条占位说明并丢弃。
    ///
    /// `index` 仅用于占位文案里的「第 N 张」，便于用户对照服务器返回。
    pub fn push_image(&mut self, index: usize, data: &str, mime: &str) {
        let mime = mime.trim().to_ascii_lowercase();
        let ordinal = index + 1;
        if !ALLOWED_MIME.contains(&mime.as_str()) {
            self.notes.push(format!("[MCP 第 {ordinal} 张图片已省略：不支持的格式 {mime}]"));
            return;
        }
        if data.trim().is_empty() {
            self.notes.push(format!("[MCP 第 {ordinal} 张图片已省略：内容为空]"));
            return;
        }
        let size = base64_decoded_len(data);
        if size == 0 {
            self.notes.push(format!("[MCP 第 {ordinal} 张图片已省略：内容不是合法的 base64]"));
            return;
        }
        if size > MAX_IMAGE_BYTES {
            self.notes.push(format!(
                "[MCP 第 {ordinal} 张图片已省略：{} 超过单张上限 {}]",
                crate::human_size(size),
                crate::human_size(MAX_IMAGE_BYTES)
            ));
            return;
        }
        if self.media.len() >= MAX_IMAGES_PER_RESULT {
            self.notes.push(format!(
                "[MCP 第 {ordinal} 张图片已省略：单次结果最多 {} 张]",
                MAX_IMAGES_PER_RESULT
            ));
            return;
        }
        self.media.push(ToolMedia {
            name: format!("mcp-image-{ordinal}.{}", extension_for(&mime)),
            mime,
            base64: data.to_string(),
            size,
        });
    }
}

fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    }
}

/// 由 base64 字符串推算原始字节数（忽略换行、容忍缺失填充）；明显非法返回 0。
pub fn base64_decoded_len(data: &str) -> usize {
    let mut chars = 0usize;
    let mut padding = 0usize;
    for ch in data.bytes() {
        match ch {
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            b'=' => padding += 1,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => chars += 1,
            // base64url 变体也接受
            b'-' | b'_' => chars += 1,
            _ => return 0,
        }
    }
    if chars == 0 {
        return 0;
    }
    // 每组 4 个字符对应 3 字节；padding 只出现在末尾
    let total = chars + padding.min(2);
    total / 4 * 3 - padding.min(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1x1 PNG 的最小合法 base64（用于构造「格式合法、体积很小」的样本）
    const TINY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

    #[test]
    fn accepts_png_and_names_by_ordinal() {
        let mut collector = MediaCollector::new();
        collector.push_image(0, TINY_PNG, "image/png");
        assert_eq!(collector.media.len(), 1);
        assert_eq!(collector.media[0].mime, "image/png");
        assert_eq!(collector.media[0].name, "mcp-image-1.png");
        assert!(collector.notes.is_empty());
        assert_eq!(collector.media[0].size, 70);
    }

    #[test]
    fn rejects_unsupported_mime_with_note() {
        let mut collector = MediaCollector::new();
        collector.push_image(0, TINY_PNG, "image/bmp");
        assert!(collector.media.is_empty());
        assert_eq!(collector.notes.len(), 1);
        assert!(collector.notes[0].contains("不支持的格式 image/bmp"));
    }

    #[test]
    fn rejects_illegal_base64() {
        let mut collector = MediaCollector::new();
        collector.push_image(0, "not base64 !!!", "image/png");
        assert!(collector.media.is_empty());
        assert!(collector.notes[0].contains("不是合法的 base64"));
    }

    #[test]
    fn rejects_oversized_image() {
        // 构造一个解码后超过上限的 base64：直接堆字符即可（不校验真实图片结构）
        let big = "A".repeat((MAX_IMAGE_BYTES / 3 + 8) * 4);
        let mut collector = MediaCollector::new();
        collector.push_image(0, &big, "image/png");
        assert!(collector.media.is_empty(), "超限图片不应进入媒体列表");
        assert!(collector.notes[0].contains("超过单张上限"));
    }

    #[test]
    fn caps_image_count_per_result() {
        let mut collector = MediaCollector::new();
        for index in 0..(MAX_IMAGES_PER_RESULT + 2) {
            collector.push_image(index, TINY_PNG, "image/png");
        }
        assert_eq!(collector.media.len(), MAX_IMAGES_PER_RESULT);
        assert_eq!(collector.notes.len(), 2);
        assert!(collector.notes[0].contains("单次结果最多"));
    }

    #[test]
    fn base64_len_ignores_whitespace_and_counts_padding() {
        assert_eq!(base64_decoded_len("TWFu"), 3);
        assert_eq!(base64_decoded_len("TWE="), 2);
        assert_eq!(base64_decoded_len("TQ=="), 1);
        assert_eq!(base64_decoded_len("TW Fu\n"), 3);
        assert_eq!(base64_decoded_len(""), 0);
        assert_eq!(base64_decoded_len("!!!!"), 0);
    }
}
