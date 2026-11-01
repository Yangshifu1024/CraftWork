// Package update 实现更新清单的签名校验。
//
// 清单包含版本、渠道、产物地址、SHA-256 与 Ed25519 签名。客户端内置公钥，
// 只有签名、地址与摘要都通过校验的清单才会被接受。下载、替换与回滚需要
// 签名密钥与更新服务器，当前仓库不包含这些外部资源。
//
// 本文件负责更新清单的结构定义、参与签名的规范化载荷，以及校验流程：
// 解析 JSON、检查字段合法性、校验 HTTPS 地址与 SHA-256 格式，最后用内置
// 公钥验证 Ed25519 签名。
package update

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"
)

// Artifact 是更新产物。
type Artifact struct {
	// URL 是产物的下载地址，必须使用 HTTPS。
	URL string `json:"url"`
	// SHA256 是产物内容的 SHA-256 摘要，十六进制小写或大写均可。
	SHA256 string `json:"sha256"`
	// Size 是产物字节数，必须大于 0。
	Size int64 `json:"size"`
}

// Manifest 是签名更新清单。
type Manifest struct {
	// Version 是目标版本号，例如 0.4.0。
	Version string `json:"version"`
	// Channel 是发布渠道，只接受 stable 或 beta。
	Channel string `json:"channel"`
	// Notes 是更新说明，可为空。
	Notes string `json:"notes,omitempty"`
	// PublishedAt 是发布时间，毫秒时间戳。
	PublishedAt int64 `json:"publishedAt"`
	// Artifact 是更新产物信息。
	Artifact Artifact `json:"artifact"`
	// Signature 是清单内容的 Ed25519 签名，Base64 编码。
	Signature string `json:"signature"`
}

// Payload 返回参与签名的规范化 JSON（不包含 signature 字段）。
//
// 签名与校验必须使用同一份字节表示。这里把签名字段清空后按结构体字段顺序
// 序列化，不依赖原始清单的空白与字段顺序，因此重新排版不会影响校验。
func Payload(manifest Manifest) ([]byte, error) {
	manifest.Signature = ""
	return json.Marshal(manifest)
}

// Verify 校验清单签名与基本字段。
//
// 校验顺序是先做便宜的字段检查（版本、渠道、地址、摘要格式、大小），全部
// 通过后才解析并验证签名；任何一项失败都返回可读的中文错误。
func Verify(raw []byte, publicKey ed25519.PublicKey) (Manifest, error) {
	var manifest Manifest
	if err := json.Unmarshal(raw, &manifest); err != nil {
		return Manifest{}, fmt.Errorf("解析更新清单: %w", err)
	}
	// 公钥长度不对说明内置公钥配置有误，直接拒绝。
	if len(publicKey) != ed25519.PublicKeySize {
		return Manifest{}, fmt.Errorf("更新公钥长度不正确")
	}
	if manifest.Version == "" {
		return Manifest{}, fmt.Errorf("更新清单缺少版本号")
	}
	if manifest.Channel != "stable" && manifest.Channel != "beta" {
		return Manifest{}, fmt.Errorf("未知更新渠道：%s", manifest.Channel)
	}
	// 只接受 HTTPS，防止更新包被中间人替换。
	parsed, err := url.Parse(manifest.Artifact.URL)
	if err != nil || parsed.Scheme != "https" || parsed.Host == "" {
		return Manifest{}, fmt.Errorf("更新地址必须是 HTTPS")
	}
	if !isSHA256(manifest.Artifact.SHA256) {
		return Manifest{}, fmt.Errorf("更新产物 SHA-256 格式不正确")
	}
	if manifest.Artifact.Size <= 0 {
		return Manifest{}, fmt.Errorf("更新产物大小无效")
	}

	// 签名必须是合法的 Base64，且长度与 Ed25519 签名一致。
	signature, err := base64.StdEncoding.DecodeString(manifest.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize {
		return Manifest{}, fmt.Errorf("更新签名格式不正确")
	}
	payload, err := Payload(manifest)
	if err != nil {
		return Manifest{}, err
	}
	if !ed25519.Verify(publicKey, payload, signature) {
		return Manifest{}, fmt.Errorf("更新清单签名校验失败")
	}
	return manifest, nil
}

// isSHA256 判断字符串是否是 64 位十六进制 SHA-256 摘要。比较前统一转成
// 小写并去掉首尾空白。
func isSHA256(value string) bool {
	value = strings.ToLower(strings.TrimSpace(value))
	if len(value) != 64 {
		return false
	}
	for _, char := range value {
		if (char < '0' || char > '9') && (char < 'a' || char > 'f') {
			return false
		}
	}
	return true
}
