//! 敏感附件文件名启发式：识别密钥/证书后缀（.pem/.key/.p12/.pfx/.jks）、
//! 环境变量文件（.env 与 .env.*）与 SSH 私钥（id_rsa/id_ed25519），大小写不敏感。
//! 这是审批提示用的启发式防护，不能替代路径边界与内容类型限制；
//! 调用方只传文件名（不含路径），路径边界由 crate::resolve 负责。

/// 证书与密钥类后缀（小写比较）。
const SENSITIVE_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks"];

/// 文件名是否疑似敏感（密钥/证书/环境变量/SSH 私钥）；大小写不敏感，只看文件名本身。
pub fn is_sensitive_attachment(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower == ".env" || lower.starts_with(".env.") {
        return true;
    }
    if lower == "id_rsa" || lower == "id_ed25519" {
        return true;
    }
    SENSITIVE_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_attachment_heuristics() {
        assert!(is_sensitive_attachment(".env"));
        assert!(is_sensitive_attachment(".env.local"));
        assert!(is_sensitive_attachment("server.pem"));
        assert!(is_sensitive_attachment("ID_RSA"));
        assert!(is_sensitive_attachment("cert.PFX"));
        assert!(!is_sensitive_attachment("readme.md"));
        assert!(!is_sensitive_attachment("enviroment.txt"));
        assert!(!is_sensitive_attachment("keys.md"));
    }
}
