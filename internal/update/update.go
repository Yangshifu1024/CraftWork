// Package update 实现更新清单的签名校验。
//
// 清单包含版本、渠道、产物地址、SHA-256 与 Ed25519 签名。客户端内置公钥，
// 只有签名、地址与摘要都通过校验的清单才会被接受。下载、替换与回滚需要
// 签名密钥与更新服务器，当前仓库不包含这些外部资源。
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
	URL    string `json:"url"`
	SHA256 string `json:"sha256"`
	Size   int64  `json:"size"`
}

// Manifest 是签名更新清单。
type Manifest struct {
	Version     string   `json:"version"`
	Channel     string   `json:"channel"`
	Notes       string   `json:"notes,omitempty"`
	PublishedAt int64    `json:"publishedAt"`
	Artifact    Artifact `json:"artifact"`
	Signature   string   `json:"signature"`
}

// Payload 返回参与签名的规范化 JSON（不包含 signature 字段）。
func Payload(manifest Manifest) ([]byte, error) {
	manifest.Signature = ""
	return json.Marshal(manifest)
}

// Verify 校验清单签名与基本字段。
func Verify(raw []byte, publicKey ed25519.PublicKey) (Manifest, error) {
	var manifest Manifest
	if err := json.Unmarshal(raw, &manifest); err != nil {
		return Manifest{}, fmt.Errorf("解析更新清单: %w", err)
	}
	if len(publicKey) != ed25519.PublicKeySize {
		return Manifest{}, fmt.Errorf("更新公钥长度不正确")
	}
	if manifest.Version == "" {
		return Manifest{}, fmt.Errorf("更新清单缺少版本号")
	}
	if manifest.Channel != "stable" && manifest.Channel != "beta" {
		return Manifest{}, fmt.Errorf("未知更新渠道：%s", manifest.Channel)
	}
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
