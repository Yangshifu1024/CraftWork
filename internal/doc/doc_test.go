package doc

import (
	"archive/zip"
	"bytes"
	"compress/zlib"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeZip(t *testing.T, path string, entries map[string]string) {
	t.Helper()
	file, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	writer := zip.NewWriter(file)
	for name, content := range entries {
		entry, err := writer.Create(name)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := entry.Write([]byte(content)); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}

func docxFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "note.docx")
	writeZip(t, path, map[string]string{
		"word/document.xml": `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
<w:p><w:r><w:t>你好</w:t></w:r><w:r><w:t>世界</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B1</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>`,
	})
	return path
}

func TestReadDOCX(t *testing.T) {
	document, err := ReadFile(docxFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	if document.Kind != "docx" {
		t.Fatalf("类型不符: %s", document.Kind)
	}
	if !strings.Contains(document.Text, "你好世界") || !strings.Contains(document.Text, "A1\tB1") {
		t.Fatalf("文本不符: %q", document.Text)
	}
}

func TestReplaceDOCXText(t *testing.T) {
	path := docxFixture(t)
	if err := Replace(path, Edit{Kind: "text", Old: "你好", New: "您好"}); err != nil {
		t.Fatal(err)
	}
	document, err := ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(document.Text, "您好世界") {
		t.Fatalf("替换结果不符: %q", document.Text)
	}
	if err := Replace(path, Edit{Kind: "text", Old: "不存在", New: "x"}); err == nil {
		t.Fatal("不存在的文本应报错")
	}
}

func xlsxFixture(t *testing.T) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "table.xlsx")
	writeZip(t, path, map[string]string{
		"xl/workbook.xml": `<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>`,
		"xl/_rels/workbook.xml.rels": `<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>`,
		"xl/sharedStrings.xml": `<?xml version="1.0" encoding="UTF-8"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>名称</t></si><si><t>苹果</t></si></sst>`,
		"xl/worksheets/sheet1.xml": `<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="s"><v>0</v></c></row>
<row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2" s="1"><v>3</v></c></row>
</sheetData></worksheet>`,
	})
	return path
}

func TestReadXLSX(t *testing.T) {
	document, err := ReadFile(xlsxFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(document.Text, "工作表：Sheet1") ||
		!strings.Contains(document.Text, "A1: 名称") ||
		!strings.Contains(document.Text, "A2: 苹果") ||
		!strings.Contains(document.Text, "B2: 3") {
		t.Fatalf("表格文本不符: %q", document.Text)
	}
}

func TestSetXLSXCell(t *testing.T) {
	path := xlsxFixture(t)
	if err := Replace(path, Edit{Kind: "cell", Sheet: "Sheet1", Cell: "B2", Value: "4"}); err != nil {
		t.Fatal(err)
	}
	if err := Replace(path, Edit{Kind: "cell", Sheet: "", Cell: "C3", Value: "汇总"}); err != nil {
		t.Fatal(err)
	}
	if err := Replace(path, Edit{Kind: "cell", Sheet: "Sheet1", Cell: "D3", Value: "=SUM(B2:B3)"}); err != nil {
		t.Fatal(err)
	}
	document, err := ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(document.Text, "B2: 4") {
		t.Fatalf("数值单元格未更新: %q", document.Text)
	}
	if !strings.Contains(document.Text, "C3: 汇总") {
		t.Fatalf("新增文本单元格未写入: %q", document.Text)
	}
	if !strings.Contains(document.Text, "D3: =SUM(B2:B3)") {
		t.Fatalf("公式未写入: %q", document.Text)
	}
}

func pdfFixture(t *testing.T) string {
	t.Helper()
	var compressed bytes.Buffer
	writer := zlib.NewWriter(&compressed)
	if _, err := writer.Write([]byte("BT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET")); err != nil {
		t.Fatal(err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	var file bytes.Buffer
	file.WriteString("%PDF-1.4\n1 0 obj << /Length ")
	file.WriteString("128 /Filter /FlateDecode >>\nstream\n")
	file.Write(compressed.Bytes())
	file.WriteString("\nendstream\nendobj\ntrailer <<>>\n%%EOF\n")
	path := filepath.Join(t.TempDir(), "note.pdf")
	if err := os.WriteFile(path, file.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestReadPDFText(t *testing.T) {
	document, err := ReadFile(pdfFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(document.Text, "Hello PDF") {
		t.Fatalf("PDF 文本不符: %q", document.Text)
	}
}

func TestReadEncryptedPDF(t *testing.T) {
	path := filepath.Join(t.TempDir(), "encrypted.pdf")
	if err := os.WriteFile(path, []byte("%PDF-1.4\n/Encrypt\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := ReadFile(path); err == nil {
		t.Fatal("加密 PDF 应返回错误")
	}
}
