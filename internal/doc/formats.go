package doc

import "fmt"

// 内置格式适配器。新增 Markdown / PPTX 只需要实现 Format 并在这里注册。

type docxFormat struct{}

func (docxFormat) Exts() []string { return []string{".docx"} }

func (docxFormat) Read(path string) (Document, error) { return readDOCX(path) }

func (docxFormat) Replace(path string, edit Edit) error {
	if edit.Kind != "text" {
		return fmt.Errorf("docx 只支持文本替换")
	}
	return replaceDOCXText(path, edit.Old, edit.New)
}

type xlsxFormat struct{}

func (xlsxFormat) Exts() []string { return []string{".xlsx"} }

func (xlsxFormat) Read(path string) (Document, error) { return readXLSX(path) }

func (xlsxFormat) Replace(path string, edit Edit) error {
	if edit.Kind != "cell" {
		return fmt.Errorf("xlsx 只支持单元格写入")
	}
	return setXLSXCell(path, edit.Sheet, edit.Cell, edit.Value)
}

type pdfFormat struct{}

func (pdfFormat) Exts() []string { return []string{".pdf"} }

func (pdfFormat) Read(path string) (Document, error) { return readPDF(path) }

func (pdfFormat) Replace(string, Edit) error { return ErrReadOnly }

func init() {
	Register(docxFormat{})
	Register(xlsxFormat{})
	Register(pdfFormat{})
}
