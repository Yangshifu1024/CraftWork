//! ID 生成：「前缀_」+ 16 位十六进制（8 字节随机），与 shared/ids.ts 逐字对齐。
//! 随机源失败时返回全 0 后缀，靠数据库唯一约束暴露问题而不是静默重试。

const HEX: &[u8; 16] = b"0123456789abcdef";

pub fn new_id(prefix: &str) -> String {
    let mut bytes = [0u8; 8];
    let ok = getrandom::fill(&mut bytes).is_ok();
    if !ok {
        return format!("{prefix}_0000000000000000");
    }
    let mut hex = String::with_capacity(16);
    for byte in bytes {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    format!("{prefix}_{hex}")
}

/// ID 前缀约定（shared/ids.ts 的 IdPrefix）。
pub mod id_prefix {
    pub const THREAD: &str = "th";
    pub const WORKSPACE: &str = "ws";
    pub const WORKSPACE_FOLDER: &str = "wfo";
    pub const RUN: &str = "run";
    pub const MESSAGE: &str = "msg";
    pub const PART: &str = "part";
    pub const APPROVAL: &str = "ap";
    pub const TOOL_CALL: &str = "call";
    pub const ATTACHMENT: &str = "att";
    pub const BACKUP: &str = "bk";
    pub const MCP_SERVER: &str = "mcp";
    pub const CUSTOM_PROVIDER: &str = "custom";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_generated_id_has_prefix_and_16_hex() {
        let id = new_id(id_prefix::THREAD);
        assert!(id.starts_with("th_"));
        assert_eq!(id.len(), "th_".len() + 16);
        let suffix = &id["th_".len()..];
        assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn same_prefix_generations_are_distinct() {
        assert_ne!(new_id(id_prefix::PART), new_id(id_prefix::PART));
    }
}
