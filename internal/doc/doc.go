// Package doc 提供 Word / Excel / PDF 的文本级读取与有限写入。
//
// 设计原则：
//   - 只解析文本层，不执行宏、脚本或外部链接；
//   - 写入保留原始 ZIP 部件与样式，只替换目标文本或单元格；
//   - 读取有大小上限，超出时截断并给出说明。
package doc

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const (
	// MaxFileBytes 是单个文档允许读取的最大字节数。
	MaxFileBytes = 50 << 20
	// MaxTextBytes 是返回文本的截断上限。
	MaxTextBytes = 2 << 20
)

// Document 是文档的文本级读取结果。
type Document struct {
	Kind      string `json:"kind"`
	Text      string `json:"text"`
	Note      string `json:"note,omitempty"`
	Truncated bool   `json:"truncated"`
}

// ReadFile 按扩展名读取文档文本。
func ReadFile(path string) (Document, error) {
	info, err := os.Stat(path)
	if err != nil {
		return Document{}, err
	}
	if info.IsDir() {
		return Document{}, fmt.Errorf("这是一个目录")
	}
	if info.Size() > MaxFileBytes {
		return Document{}, fmt.Errorf("文件过大（%d MB），暂不支持解析", info.Size()/1024/1024)
	}
	switch strings.ToLower(filepath.Ext(path)) {
	case ".docx":
		return readDOCX(path)
	case ".xlsx":
		return readXLSX(path)
	case ".pdf":
		return readPDF(path)
	case ".doc", ".xls":
		return Document{
			Kind: strings.TrimPrefix(strings.ToLower(filepath.Ext(path)), "."),
			Note: "旧版 Office 格式请先转换为 .docx / .xlsx 后再读取。",
		}, nil
	default:
		return Document{}, fmt.Errorf("不支持的文档类型：%s", filepath.Ext(path))
	}
}

func truncateText(text string) (string, bool) {
	if len(text) <= MaxTextBytes {
		return text, false
	}
	return text[:MaxTextBytes] + "\n…（文本已截断）", true
}
