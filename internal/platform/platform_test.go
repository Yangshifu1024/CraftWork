package platform

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSanitizedEnvRemovesSecrets(t *testing.T) {
	t.Setenv("CRAFTWORK_TEST_API_KEY", "secret")
	t.Setenv("CRAFTWORK_TEST_VISIBLE", "ok")

	joined := strings.Join(SanitizedEnv(), "\n")
	if strings.Contains(joined, "CRAFTWORK_TEST_API_KEY") {
		t.Fatal("API Key 环境变量不应传给子进程")
	}
	if !strings.Contains(joined, "CRAFTWORK_TEST_VISIBLE") {
		t.Fatal("普通环境变量应保留")
	}
	if !strings.Contains(joined, "CRAFTWORK=1") {
		t.Fatal("应写入 CRAFTWORK 标记")
	}
}

func TestLimitedBufferTruncates(t *testing.T) {
	buffer := NewLimitedBuffer(5)
	if _, err := buffer.Write([]byte("abcdef")); err != nil {
		t.Fatal(err)
	}
	text := buffer.String()
	if !strings.HasPrefix(text, "abcde") {
		t.Fatalf("应保留前 5 字节: %q", text)
	}
	if !strings.Contains(text, "输出已截断") {
		t.Fatalf("应注明截断: %q", text)
	}
}

func TestWriteFileAtomicAndHelpers(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "note.txt")
	if err := WriteFileAtomic(path, []byte("hello"), 0o600); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "hello" {
		t.Fatalf("写入内容不符: %q", data)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("权限不符: %v", info.Mode().Perm())
	}
	if Sha256Hex([]byte("hello")) == "" {
		t.Fatal("SHA-256 不应为空")
	}
	if !IsBinary([]byte{0x00, 0x01}) {
		t.Fatal("含 NUL 的内容应判为二进制")
	}
	if IsBinary([]byte("plain text")) {
		t.Fatal("文本不应判为二进制")
	}
	if !strings.HasPrefix(BackupName("/tmp/a.txt"), "a.txt.") {
		t.Fatalf("备份名不符: %s", BackupName("/tmp/a.txt"))
	}
}
