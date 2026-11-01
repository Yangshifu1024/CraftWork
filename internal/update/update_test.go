package update

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

func signedManifest(t *testing.T, mutate func(*Manifest)) ([]byte, ed25519.PublicKey) {
	t.Helper()
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	manifest := Manifest{
		Version:     "0.2.0",
		Channel:     "stable",
		Notes:       "修复若干问题",
		PublishedAt: 1767225600000,
		Artifact: Artifact{
			URL:    "https://example.com/craftwork-0.2.0.zip",
			SHA256: strings.Repeat("a", 64),
			Size:   1024,
		},
	}
	if mutate != nil {
		mutate(&manifest)
	}
	payload, err := Payload(manifest)
	if err != nil {
		t.Fatal(err)
	}
	manifest.Signature = base64.StdEncoding.EncodeToString(ed25519.Sign(privateKey, payload))
	raw, err := json.Marshal(manifest)
	if err != nil {
		t.Fatal(err)
	}
	return raw, publicKey
}

func TestVerifyValidManifest(t *testing.T) {
	raw, publicKey := signedManifest(t, nil)
	manifest, err := Verify(raw, publicKey)
	if err != nil {
		t.Fatal(err)
	}
	if manifest.Version != "0.2.0" || manifest.Channel != "stable" {
		t.Fatalf("清单字段不符: %+v", manifest)
	}
}

func TestVerifyRejectsTamperedManifest(t *testing.T) {
	raw, publicKey := signedManifest(t, nil)
	tampered := strings.Replace(string(raw), "修复若干问题", "被篡改的内容", 1)
	if _, err := Verify([]byte(tampered), publicKey); err == nil {
		t.Fatal("篡改后的清单应校验失败")
	}
}

func TestVerifyRejectsInsecureURL(t *testing.T) {
	raw, publicKey := signedManifest(t, func(manifest *Manifest) {
		manifest.Artifact.URL = "http://example.com/app.zip"
	})
	if _, err := Verify(raw, publicKey); err == nil {
		t.Fatal("HTTP 地址应被拒绝")
	}
}

func TestVerifyRejectsWrongKey(t *testing.T) {
	raw, _ := signedManifest(t, nil)
	otherPublic, _, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Verify(raw, otherPublic); err == nil {
		t.Fatal("错误公钥应校验失败")
	}
}
