// 本文件负责把内置格式适配器注册到统一注册表，让扩展名能映射到
// 具体的读取与写入实现。

package doc

import "fmt"

// 内置格式适配器。新增 Markdown / PPTX 只需要实现 Format 并在这里注册。
//
// docxFormat 是 Word（.docx）适配器：读取全文，写入只支持同一 XML
// 文本节点内的替换。
type docxFormat struct{}

// Exts 返回 docx 支持的扩展名。
func (docxFormat) Exts() []string { return []string{".docx"} }

// Read 读取 Word 文档文本。
func (docxFormat) Read(path string) (Document, error) { return readDOCX(path) }

// Replace 执行文本替换；docx 不支持按单元格写入。
func (docxFormat) Replace(path string, edit Edit) error {
	if edit.Kind != "text" {
		return fmt.Errorf("docx 只支持文本替换")
	}
	return replaceDOCXText(path, edit.Old, edit.New)
}

// xlsxFormat 是 Excel（.xlsx）适配器：读取所有工作表，写入单个单元格。
type xlsxFormat struct{}

// Exts 返回 xlsx 支持的扩展名。
func (xlsxFormat) Exts() []string { return []string{".xlsx"} }

// Read 读取 Excel 文档文本。
func (xlsxFormat) Read(path string) (Document, error) { return readXLSX(path) }

// Replace 执行单元格写入；xlsx 不支持整段文本替换。
func (xlsxFormat) Replace(path string, edit Edit) error {
	if edit.Kind != "cell" {
		return fmt.Errorf("xlsx 只支持单元格写入")
	}
	return setXLSXCell(path, edit.Sheet, edit.Cell, edit.Value)
}

// pdfFormat 是 PDF 适配器：只读取文本层，不支持写入。
type pdfFormat struct{}

// Exts 返回 pdf 支持的扩展名。
func (pdfFormat) Exts() []string { return []string{".pdf"} }

// Read 读取 PDF 文本层。
func (pdfFormat) Read(path string) (Document, error) { return readPDF(path) }

// Replace 对 PDF 始终返回 ErrReadOnly。
func (pdfFormat) Replace(string, Edit) error { return ErrReadOnly }

// init 在包初始化时注册全部内置格式。
func init() {
	Register(docxFormat{})
	Register(xlsxFormat{})
	Register(pdfFormat{})
}
