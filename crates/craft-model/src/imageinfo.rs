//! 图片尺寸探测：只读容器头部，不解码像素，不依赖任何图像 crate。
//!
//! 为什么需要：图片的 token 成本由**像素尺寸**决定（OpenAI 按 512×512 tile 计费、
//! Anthropic 按 `w*h/750` 计费），而附件加载侧此前只有字节数。没有尺寸，压缩估算
//! 只能把一张 5MB 截图算成 0 token（见 context.rs 的 estimate_message_tokens）。
//!
//! 支持范围与附件白名单一致：png / jpeg / gif / webp。其余类型与畸形头部一律返回
//! None，调用方按「尺寸未知」降级——探测失败不该阻断附件加载。

/// 图片像素尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

impl ImageSize {
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// 按魔数判断格式并取尺寸；未识别或头部不完整返回 None。
pub fn probe(bytes: &[u8]) -> Option<ImageSize> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_size(bytes);
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg_size(bytes);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return gif_size(bytes);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return webp_size(bytes);
    }
    None
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn u24_le(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at.checked_add(3)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], 0]))
}

/// 尺寸为 0 视为无效：某些畸形文件头能通过魔数校验但宽高为 0，直接当作探测失败。
fn validated(width: u32, height: u32) -> Option<ImageSize> {
    if width == 0 || height == 0 {
        return None;
    }
    Some(ImageSize { width, height })
}

/// PNG：签名 8 字节 + IHDR 长度(4) + "IHDR"(4) + width(4) + height(4)。
fn png_size(bytes: &[u8]) -> Option<ImageSize> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    validated(u32_be(bytes, 16)?, u32_be(bytes, 20)?)
}

/// GIF：签名 6 字节后是小端 uint16 宽、高。
fn gif_size(bytes: &[u8]) -> Option<ImageSize> {
    validated(u32::from(u16_le(bytes, 6)?), u32::from(u16_le(bytes, 8)?))
}

/// JPEG：逐段扫描到 SOF（Start Of Frame）。
///
/// 关键细节：C4(DHT) / C8(JPG) / CC(DAC) 落在 C0~CF 区间但**不是** SOF，含它们的
/// 文件直接按 `0xC0..=0xCF` 匹配会读到错误的后续字节（这两段的数据区不遵循
/// 「长度 + 宽高」布局）。这是 JPEG 尺寸解析最常见的一个坑。
fn jpeg_size(bytes: &[u8]) -> Option<ImageSize> {
    let mut i = 2usize; // 跳过 SOI
    loop {
        // 段标记前允许有填充 0xFF
        while bytes.get(i)? == &0xFF {
            i += 1;
        }
        let marker = *bytes.get(i)?;
        i += 1;
        // 无长度字段的独立标记：TEM(01)、RST0~RST7(D0~D7)、SOI(D8)、EOI(D9)
        if marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            continue;
        }
        let len = usize::from(u16_be(bytes, i)?);
        if len < 2 {
            return None;
        }
        let is_sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            // 段内布局：精度(1) + 高(2) + 宽(2)
            let height = u32::from(u16_be(bytes, i + 3)?);
            let width = u32::from(u16_be(bytes, i + 5)?);
            return validated(width, height);
        }
        // 走到 SOS 说明该 JPEG 没有可用的 SOF（或已损坏），不再继续
        if marker == 0xDA {
            return None;
        }
        i = i.checked_add(len)?;
    }
}

/// WebP：RIFF 容器内的首个 VP8 / VP8L / VP8X 块，三种子格式各有自己的尺寸位。
fn webp_size(bytes: &[u8]) -> Option<ImageSize> {
    let chunk = bytes.get(12..16)?;
    match chunk {
        // 有损 VP8：帧头里 3 字节起始码 + 小端 uint16 宽、高（各取低 14 位）
        b"VP8 " => {
            if bytes.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let width = u32::from(u16_le(bytes, 26)? & 0x3FFF);
            let height = u32::from(u16_le(bytes, 28)? & 0x3FFF);
            validated(width, height)
        }
        // 无损 VP8L：1 字节签名 0x2F 后，14 位宽 / 14 位高按位打包
        b"VP8L" => {
            if *bytes.get(20)? != 0x2F {
                return None;
            }
            let b0 = u32::from(*bytes.get(21)?);
            let b1 = u32::from(*bytes.get(22)?);
            let b2 = u32::from(*bytes.get(23)?);
            let b3 = u32::from(*bytes.get(24)?);
            let bits = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);
            validated((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1)
        }
        // 扩展 VP8X：canvas 宽高为 24 位小端，存的是「实际值 - 1」
        b"VP8X" => {
            let width = u24_le(bytes, 24)? + 1;
            let height = u24_le(bytes, 27)? + 1;
            validated(width, height)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小的 PNG 头部：签名 + IHDR 长度/类型 + 宽高。
    fn png_prefix(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out
    }

    #[test]
    fn png_ihdr_dimensions_probed() {
        let size = probe(&png_prefix(2892, 1898)).unwrap();
        assert_eq!(size, ImageSize { width: 2892, height: 1898 });
        assert_eq!(size.pixels(), 5489016);
    }

    #[test]
    fn gif_logical_screen_dimensions_probed() {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&800u16.to_le_bytes());
        bytes.extend_from_slice(&600u16.to_le_bytes());
        assert_eq!(probe(&bytes).unwrap(), ImageSize { width: 800, height: 600 });
        // GIF87a 同布局
        let mut old = b"GIF87a".to_vec();
        old.extend_from_slice(&32u16.to_le_bytes());
        old.extend_from_slice(&16u16.to_le_bytes());
        assert_eq!(probe(&old).unwrap(), ImageSize { width: 32, height: 16 });
    }

    /// SOF 段必须能穿过前置的 APP0(DHT 之外的段)找到；同时验证 C4/C8/CC 不被误认。
    #[test]
    fn jpeg_sof_dimensions_probed_skipping_non_sof_markers() {
        let mut bytes = vec![0xFF, 0xD8];
        // APP0 段（长度 4：2 字节长度 + 2 字节数据）
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        // DHT(C4)：数据区故意不是「精度+宽高」布局，若被误判为 SOF 会读出垃圾
        bytes.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x04, 0xAA, 0xBB]);
        // DAC(CC)：同上
        bytes.extend_from_slice(&[0xFF, 0xCC, 0x00, 0x04, 0xCC, 0xDD]);
        // SOF0(C0)：长度 11 + 精度 8 + 高 1080 + 宽 1920
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        bytes.extend_from_slice(&1080u16.to_be_bytes());
        bytes.extend_from_slice(&1920u16.to_be_bytes());
        bytes.extend_from_slice(&[0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
        assert_eq!(probe(&bytes).unwrap(), ImageSize { width: 1920, height: 1080 });
    }

    #[test]
    fn webp_vp8x_canvas_dimensions_probed_minus_one() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8X");
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // flags + reserved
        bytes.extend_from_slice(&[0xAF, 0x0B, 0x00]); // width-1 = 2991
        bytes.extend_from_slice(&[0x6D, 0x07, 0x00]); // height-1 = 1901
        assert_eq!(probe(&bytes).unwrap(), ImageSize { width: 2992, height: 1902 });
    }

    #[test]
    fn webp_vp8l_packed_dimensions_probed() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8L");
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.push(0x2F);
        // 宽-1 = 999（0x3E7）、高-1 = 499（0x1F3），按 14 位打包
        let bits: u32 = 999 | (499u32 << 14);
        bytes.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(probe(&bytes).unwrap(), ImageSize { width: 1000, height: 500 });
    }

    #[test]
    fn unsupported_and_truncated_headers_return_none() {
        assert!(probe(b"").is_none());
        assert!(probe(b"not an image at all").is_none());
        // 只有 PNG 签名，没有 IHDR
        assert!(probe(b"\x89PNG\r\n\x1a\n").is_none());
        // 尺寸为 0 的畸形头按探测失败处理
        assert!(probe(&png_prefix(0, 100)).is_none());
        // JPEG 截断在 SOF 之前
        assert!(probe(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]).is_none());
    }

    /// 真实 PNG（附件目录里的粘贴图）必须能被解析出非零尺寸。
    #[test]
    fn real_png_from_fixture_probed() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/images/paste-sample.png");
        let Ok(bytes) = std::fs::read(&path) else {
            return; // 夹具缺失时跳过（不在无夹具环境里制造假失败）
        };
        let size = probe(&bytes).expect("真实 PNG 应可解析");
        assert!(size.width > 0 && size.height > 0);
    }
}
