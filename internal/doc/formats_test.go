package doc

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type fakeFormat struct{}

func (fakeFormat) Exts() []string { return []string{".cwtest"} }

func (fakeFormat) Read(string) (Document, error) {
	return Document{Kind: "cwtest", Text: "ok"}, nil
}

func (fakeFormat) Replace(string, Edit) error { return nil }

func TestSupportedFormats(t *testing.T) {
	if !Supported("report.docx") || !Supported("sheet.XLSX") || !Supported("paper.pdf") {
		t.Fatal("内置格式应被识别")
	}
	if Supported("notes.txt") {
		t.Fatal("普通文本不应被当作文档格式")
	}
}

func TestReplaceRejectsReadOnlyFormat(t *testing.T) {
	err := Replace("paper.pdf", Edit{Kind: "text", Old: "a", New: "b"})
	if !errors.Is(err, ErrReadOnly) {
		t.Fatalf("PDF 写入应返回只读错误: %v", err)
	}
}

func TestLegacyOfficeFormatReturnsNote(t *testing.T) {
	path := filepath.Join(t.TempDir(), "old.doc")
	if err := os.WriteFile(path, []byte("legacy"), 0o644); err != nil {
		t.Fatal(err)
	}
	document, err := ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if document.Kind != "doc" || !strings.Contains(document.Note, "转换") {
		t.Fatalf("旧格式说明不符: %+v", document)
	}
}

func TestRegisterAddsFormat(t *testing.T) {
	Register(fakeFormat{})
	if !Supported("demo.cwtest") {
		t.Fatal("注册后的格式应被识别")
	}
	path := filepath.Join(t.TempDir(), "demo.cwtest")
	if err := os.WriteFile(path, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	document, err := ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if document.Kind != "cwtest" {
		t.Fatalf("注册格式的读取结果不符: %+v", document)
	}
}
