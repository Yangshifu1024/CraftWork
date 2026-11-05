//! 敏感附件文件名启发式。
//! 这是启发式防护，不能替代路径边界与内容类型限制；调用方传文件名而非完整路径。

const SENSITIVE_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".jks"];

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
    fn 敏感附件启发式() {
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
