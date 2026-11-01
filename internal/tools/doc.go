package tools

import (
	"context"
	"fmt"
	"os"
	"strings"

	"craftwork/internal/doc"
	"craftwork/internal/policy"
)

// ---------- doc_read ----------

type docReadTool struct{}

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
				"path": map[string]any{"type": "string", "description": "相对工作区的文档路径"},
			},
		},
	}
}

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

type docEditTool struct{}

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
				"path":     map[string]any{"type": "string", "description": "相对工作区的 .docx 路径"},
				"old_text": map[string]any{"type": "string", "description": "要替换的原文，必须唯一"},
				"new_text": map[string]any{"type": "string", "description": "替换后的文本"},
			},
		},
	}
}

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

type xlsxSetCellTool struct{}

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
				"path":  map[string]any{"type": "string", "description": "相对工作区的 .xlsx 路径"},
				"sheet": map[string]any{"type": "string", "description": "工作表名称或序号（从 1 开始），默认第一个"},
				"cell":  map[string]any{"type": "string", "description": "单元格引用，例如 B8"},
				"value": map[string]any{"type": "string", "description": "新值；以 = 开头时写入公式"},
			},
		},
	}
}

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
