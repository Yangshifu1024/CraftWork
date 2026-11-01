// Package doc 提供 Word / Excel / PDF 的文本级读取与有限写入。
//
// 设计原则：
//   - 只解析文本层，不执行宏、脚本或外部链接；
//   - 写入保留原始 ZIP 部件与样式，只替换目标文本或单元格；
//   - 读取有大小上限，超出时截断并给出说明。
//
// 本文件负责格式注册表、统一读取入口、大小限制与文本截断；
// 具体格式的解析与写入分别在同包的其他文件中实现。
package doc

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// 文档读取与返回文本的大小上限。
const (
	// MaxFileBytes 是单个文档允许读取的最大字节数。
	MaxFileBytes = 50 << 20
	// MaxTextBytes 是返回文本的截断上限。
	MaxTextBytes = 2 << 20
)

// Document 是文档的文本级读取结果。
type Document struct {
	Kind      string `json:"kind"`           // 文档类型，例如 docx、xlsx、pdf
	Text      string `json:"text"`           // 提取出的纯文本
	Note      string `json:"note,omitempty"` // 无法给出文本时的说明，例如扫描件或旧格式
	Truncated bool   `json:"truncated"`      // 文本是否因超过上限而被截断
}

// ErrReadOnly 表示该格式只支持读取，不支持文本级写入。
var ErrReadOnly = errors.New("该格式不支持编辑")

// Edit 描述一次文本级写入。
//
// Kind 决定使用哪些字段：text 使用 Old / New 做文本替换，
// cell 使用 Sheet / Cell / Value 写入单元格。
type Edit struct {
	Kind  string // 编辑类型：text 表示文本替换，cell 表示单元格写入
	Old   string // 文本替换时要查找的原文
	New   string // 文本替换后的新内容
	Sheet string // 工作表名称或从 1 开始的序号；为空表示第一个工作表
	Cell  string // 单元格引用，例如 A1
	Value string // 写入单元格的值；以等号开头时按公式处理
}

// Format 是文档格式适配器。
//
// 读、写与扩展名都由格式自己声明；新增格式只需实现这个接口并注册。
type Format interface {
	// Exts 返回该格式支持的扩展名列表，必须包含前导点，例如 ".docx"。
	Exts() []string
	// Read 解析文件并返回文本级结果。
	Read(path string) (Document, error)
	// Replace 执行一次文本级写入；不支持写入的格式返回 ErrReadOnly。
	Replace(path string, edit Edit) error
}

// formats 是扩展名到格式适配器的注册表，键统一为小写并带前导点。
var formats = map[string]Format{}

// legacyNotes 是只读目录里可以直接解释的旧格式。
var legacyNotes = map[string]string{
	".doc": "旧版 Office 格式请先转换为 .docx / .xlsx 后再读取。",
	".xls": "旧版 Office 格式请先转换为 .docx / .xlsx 后再读取。",
}

// Register 注册一个文档格式。
//
// 同一格式声明的多个扩展名都会指向同一个适配器；扩展名统一转成
// 小写，保证查询时不受文件名大小写影响。
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
//
// 读取前先检查文件是否存在、是否为目录以及是否超过大小上限；
// 旧版 .doc / .xls 不解析，只返回转换提示，避免误判为损坏文件。
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
		// 旧格式只给出说明，Text 保持为空，前端据此显示提示而不是空白内容。
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

// truncateText 按 MaxTextBytes 截断文本，并返回是否发生截断。
func truncateText(text string) (string, bool) {
	if len(text) <= MaxTextBytes {
		return text, false
	}
	return text[:MaxTextBytes] + "\n…（文本已截断）", true
}
