// Package doc 提供 Word / Excel / PDF 的文本级读取与有限写入。
//
// 设计原则：
//   - 只解析文本层，不执行宏、脚本或外部链接；
//   - 写入保留原始 ZIP 部件与样式，只替换目标文本或单元格；
//   - 读取有大小上限，超出时截断并给出说明。
package doc

import (
	"errors"
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

// ErrReadOnly 表示该格式只支持读取，不支持文本级写入。
var ErrReadOnly = errors.New("该格式不支持编辑")

// Edit 描述一次文本级写入。
type Edit struct {
	Kind  string // text 或 cell
	Old   string
	New   string
	Sheet string
	Cell  string
	Value string
}

// Format 是文档格式适配器。
//
// 读、写与扩展名都由格式自己声明；新增格式只需实现这个接口并注册。
type Format interface {
	Exts() []string
	Read(path string) (Document, error)
	Replace(path string, edit Edit) error
}

var formats = map[string]Format{}

// legacyNotes 是只读目录里可以直接解释的旧格式。
var legacyNotes = map[string]string{
	".doc": "旧版 Office 格式请先转换为 .docx / .xlsx 后再读取。",
	".xls": "旧版 Office 格式请先转换为 .docx / .xlsx 后再读取。",
}

// Register 注册一个文档格式。
func Register(format Format) {
	for _, ext := range format.Exts() {
		formats[strings.ToLower(ext)] = format
	}
}

// Supported 判断路径是否为已注册的文档格式。
func Supported(path string) bool {
	_, ok := formats[strings.ToLower(filepath.Ext(path))]
	return ok
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
	ext := strings.ToLower(filepath.Ext(path))
	if format, ok := formats[ext]; ok {
		return format.Read(path)
	}
	if note, ok := legacyNotes[ext]; ok {
		return Document{Kind: strings.TrimPrefix(ext, "."), Note: note}, nil
	}
	return Document{}, fmt.Errorf("不支持的文档类型：%s", filepath.Ext(path))
}

// Replace 对文档执行一次文本级写入，格式差异由适配器负责。
func Replace(path string, edit Edit) error {
	format, ok := formats[strings.ToLower(filepath.Ext(path))]
	if !ok {
		return fmt.Errorf("不支持的文档类型：%s", filepath.Ext(path))
	}
	return format.Replace(path, edit)
}

func truncateText(text string) (string, bool) {
	if len(text) <= MaxTextBytes {
		return text, false
	}
	return text[:MaxTextBytes] + "\n…（文本已截断）", true
}
