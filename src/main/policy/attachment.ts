// 敏感附件文件名启发式。
// 这是启发式防护，不能替代路径边界与内容类型限制；调用方传文件名而非完整路径。

const SENSITIVE_SUFFIXES = [".pem", ".key", ".p12", ".pfx", ".jks"];

export function isSensitiveAttachment(name: string): boolean {
  const lower = name.toLowerCase();
  if (lower === ".env" || lower.startsWith(".env.")) return true;
  if (lower === "id_rsa" || lower === "id_ed25519") return true;
  return SENSITIVE_SUFFIXES.some((s) => lower.endsWith(s));
}
