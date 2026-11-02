// 本文件负责文档类工具：读取 Word/Excel/PDF 的文本，替换 Word 文本，
// 以及修改 Excel 单元格。实际解析与写回交给 internal/doc，这里只做
// 参数校验、写入前备份与结果整理。

package tools

import (
	"context"
	"fmt"
	"os"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/doc"
	"github.com/yangshifu1024/craftwork/internal/policy"
)

// ---------- doc_read ----------

// docReadTool 读取文档的文本内容，只读。
type docReadTool struct{}

// Spec 声明 doc_read 的元数据：path 必填，只读操作。
func (t *docReadTool) Spec() Spec {
	return Spec{
		Name:        "doc_read",
		Description: "读取 .docx / .xlsx / .pdf 的文本内容。PDF 只读取文本层，扫描件不支持 OCR。",
		Risk:        policy.R0,
		PathArg:     "path",
		Work:        true,
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path"},
			"properties": map[string]any{
				"path": map[string]any{"type": "string", "description": "相对主要目录或附加目录的文档路径"},
			},
		},
	}
}

// Execute 读取文档并按需要附上格式说明，供模型判断内容是否完整。
func (t *docReadTool) Execute(_ context.Context, call Call) Result {
	document, err := doc.ReadFile(call.Target)
	if err != nil {
		return errorResult("读取文档失败: %v", err)
	}
	text := document.Text
	if document.Note != "" {
		text = "说明：" + document.Note + "\n\n" + text
	}
	if strings.TrimSpace(text) == "" {
		return errorResult("文档没有可提取的文本")
	}
	return Result{
		Output: text,
		Meta: map[string]any{
			"path": call.Display, "kind": document.Kind, "truncated": document.Truncated,
		},
	}
}

// ---------- doc_edit ----------

// docEditTool 替换 Word 文档中的一段文本，属于写操作。
type docEditTool struct{}

// Spec 声明 doc_edit 的元数据：old_text 与 new_text 必填，风险等级 R1。
func (t *docEditTool) Spec() Spec {
	return Spec{
		Name:        "doc_edit",
		Description: "替换 Word 文档中的一段文本。文本必须完整位于同一个格式片段内；写入前会备份原文件。",
		Risk:        policy.R1,
		PathArg:     "path",
		Work:        true,
		Title:       "修改文档",
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path", "old_text", "new_text"},
			"properties": map[string]any{
				"path":     map[string]any{"type": "string", "description": "相对主要目录或附加目录的 .docx 路径"},
				"old_text": map[string]any{"type": "string", "description": "要替换的原文，必须唯一"},
				"new_text": map[string]any{"type": "string", "description": "替换后的文本"},
			},
		},
	}
}

// Execute 在写入前备份原文档，再交给 doc.Replace 做文本替换。
func (t *docEditTool) Execute(_ context.Context, call Call) Result {
	args := call.Args
	oldText := stringArg(args, "old_text")
	newText := stringArg(args, "new_text")
	if strings.TrimSpace(oldText) == "" {
		return errorResult("old_text 不能为空")
	}
	abs, display := call.Target, call.Display
	original, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文档失败: %v", err)
	}
	if err := call.Backup(original); err != nil {
		return errorResult("写入前备份失败: %v", err)
	}
	if err := doc.Replace(abs, doc.Edit{Kind: "text", Old: oldText, New: newText}); err != nil {
		return errorResult("%v", err)
	}
	return Result{
		Output: fmt.Sprintf("已替换 %s 中的 1 处文本。", display),
		Meta:   map[string]any{"path": display},
	}
}

// ---------- xlsx_set_cell ----------

// xlsxSetCellTool 修改 Excel 单元格的值或公式，属于写操作。
type xlsxSetCellTool struct{}

// Spec 声明 xlsx_set_cell 的元数据：path、cell、value 必填，风险等级 R1。
func (t *xlsxSetCellTool) Spec() Spec {
	return Spec{
		Name:        "xlsx_set_cell",
		Description: "修改 Excel 单元格的值或公式，保留工作表其余内容与样式；写入前会备份原文件。",
		Risk:        policy.R1,
		PathArg:     "path",
		Work:        true,
		Title:       "修改表格",
		SummaryArg:  "path",
		Schema: map[string]any{
			"type":     "object",
			"required": []string{"path", "cell", "value"},
			"properties": map[string]any{
				"path":  map[string]any{"type": "string", "description": "相对主要目录或附加目录的 .xlsx 路径"},
				"sheet": map[string]any{"type": "string", "description": "工作表名称或序号（从 1 开始），默认第一个"},
				"cell":  map[string]any{"type": "string", "description": "单元格引用，例如 B8"},
				"value": map[string]any{"type": "string", "description": "新值；以 = 开头时写入公式"},
			},
		},
	}
}

// Execute 备份原表格后写入指定单元格，保留其余内容与样式。
func (t *xlsxSetCellTool) Execute(_ context.Context, call Call) Result {
	args := call.Args
	abs, display := call.Target, call.Display
	cell := stringArg(args, "cell")
	value := stringArg(args, "value")
	if strings.TrimSpace(cell) == "" {
		return errorResult("cell 不能为空")
	}
	original, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文档失败: %v", err)
	}
	if err := call.Backup(original); err != nil {
		return errorResult("写入前备份失败: %v", err)
	}
	if err := doc.Replace(abs, doc.Edit{
		Kind: "cell", Sheet: stringArg(args, "sheet"), Cell: cell, Value: value,
	}); err != nil {
		return errorResult("%v", err)
	}
	return Result{
		Output: fmt.Sprintf("已更新 %s 的单元格 %s。", display, strings.ToUpper(strings.TrimSpace(cell))),
		Meta:   map[string]any{"path": display, "cell": strings.ToUpper(strings.TrimSpace(cell))},
	}
}
