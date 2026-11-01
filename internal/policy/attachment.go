// 本文件判断哪些文件名不应作为附件发送给模型。
//
// 文件选择与粘贴附件共用同一套规则：前者在读取时按路径最后一段检查，
// 后者在落盘前按用户提供的原始文件名检查，避免借粘贴绕过密钥文件拦截。
// 这是一层启发式防护，不能替代路径边界校验与内容类型限制。

package policy

import "strings"

// IsSensitiveAttachment 判断文件名是否属于不应发送给模型的敏感文件。
//
// 覆盖环境变量文件（.env 及其变体）、SSH 私钥，以及常见证书与私钥扩展名。
// 文件名比较统一转小写，避免大小写绕过；调用方传入文件名而不是完整路径。
func IsSensitiveAttachment(name string) bool {
	lower := strings.ToLower(name)
	if lower == ".env" || strings.HasPrefix(lower, ".env.") ||
		lower == "id_rsa" || lower == "id_ed25519" {
		return true
	}
	for _, suffix := range []string{".pem", ".key", ".p12", ".pfx", ".jks"} {
		if strings.HasSuffix(lower, suffix) {
			return true
		}
	}
	return false
}
