package service

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/yangshifu1024/craftwork/internal/version"
)

// newTestAppService 创建使用临时数据目录的应用服务，返回服务与附件目录。
func newTestAppService(t *testing.T) (*AppService, string) {
	t.Helper()
	dataDir := t.TempDir()
	return NewAppService(dataDir, version.Info{}), AttachmentDir(dataDir)
}

func TestSavePastedAttachmentWritesInsideAttachmentDir(t *testing.T) {
	svc, attachmentDir := newTestAppService(t)
	content := base64.StdEncoding.EncodeToString([]byte("hello"))
	path, err := svc.SavePastedAttachment("note.txt", content)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Dir(path) != attachmentDir {
		t.Fatalf("附件应写入附件目录: %s", path)
	}
	if !strings.HasSuffix(filepath.Base(path), "_note.txt") {
		t.Fatalf("附件名应保留扩展名: %s", path)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "hello" {
		t.Fatalf("附件内容不符: %q", data)
	}
}

func TestSavePastedAttachmentSanitizesName(t *testing.T) {
	svc, attachmentDir := newTestAppService(t)
	content := base64.StdEncoding.EncodeToString([]byte("x"))
	path, err := svc.SavePastedAttachment("../../evil/报告 1?.txt", content)
	if err != nil {
		t.Fatal(err)
	}
	if filepath.Dir(path) != attachmentDir {
		t.Fatalf("附件不应逃逸目录: %s", path)
	}
	name := filepath.Base(path)
	if strings.Contains(name, "..") || strings.ContainsAny(name, `/\?`) {
		t.Fatalf("文件名未被清洗: %s", name)
	}
	if !strings.HasSuffix(name, "_报告 1_.txt") {
		t.Fatalf("应保留中文与扩展名: %s", name)
	}
}

func TestSavePastedAttachmentRejectsSensitiveName(t *testing.T) {
	svc, _ := newTestAppService(t)
	content := base64.StdEncoding.EncodeToString([]byte("SECRET=1"))
	if _, err := svc.SavePastedAttachment(".env", content); err == nil {
		t.Fatal("环境文件应被拒绝")
	}
}

func TestSavePastedAttachmentValidatesContent(t *testing.T) {
	svc, _ := newTestAppService(t)
	if _, err := svc.SavePastedAttachment("a.txt", "not base64!"); err == nil {
		t.Fatal("非法 base64 应被拒绝")
	}
	if _, err := svc.SavePastedAttachment("a.txt", ""); err == nil {
		t.Fatal("空内容应被拒绝")
	}
}
