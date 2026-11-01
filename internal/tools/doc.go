package tools

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"craftwork/internal/doc"
)

// ---------- doc_read ----------

type docReadTool struct{ registry *Registry }

func (t *docReadTool) Name() string { return "doc_read" }

func (t *docReadTool) Description() string {
	return "读取 .docx / .xlsx / .pdf 的文本内容。PDF 只读取文本层，扫描件不支持 OCR。"
}

func (t *docReadTool) Schema() map[string]any {
	return map[string]any{
		"type":     "object",
		"required": []string{"path"},
		"properties": map[string]any{
			"path": map[string]any{"type": "string", "description": "相对工作区的文档路径"},
		},
	}
}

func (t *docReadTool) Execute(_ context.Context, args map[string]any) Result {
	abs, display, err := t.registry.resolve(stringArg(args, "path"))
	if err != nil {
		return errorResult("%v", err)
	}
	document, err := doc.ReadFile(abs)
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
		Output: truncate(text, maxOutputBytes),
		Meta: map[string]any{
			"path": display, "kind": document.Kind, "truncated": document.Truncated,
		},
	}
}

// ---------- doc_edit ----------

type docEditTool struct{ registry *Registry }

func (t *docEditTool) Name() string { return "doc_edit" }

func (t *docEditTool) Description() string {
	return "替换 Word 文档中的一段文本。文本必须完整位于同一个格式片段内；写入前会备份原文件。"
}

func (t *docEditTool) Schema() map[string]any {
	return map[string]any{
		"type":     "object",
		"required": []string{"path", "old_text", "new_text"},
		"properties": map[string]any{
			"path":     map[string]any{"type": "string", "description": "相对工作区的 .docx 路径"},
			"old_text": map[string]any{"type": "string", "description": "要替换的原文，必须唯一"},
			"new_text": map[string]any{"type": "string", "description": "替换后的文本"},
		},
	}
}

func (t *docEditTool) Execute(_ context.Context, args map[string]any) Result {
	oldText := stringArg(args, "old_text")
	newText := stringArg(args, "new_text")
	if strings.TrimSpace(oldText) == "" {
		return errorResult("old_text 不能为空")
	}
	abs, display, err := t.registry.resolve(stringArg(args, "path"))
	if err != nil {
		return errorResult("%v", err)
	}
	if strings.ToLower(filepath.Ext(abs)) != ".docx" {
		return errorResult("doc_edit 只支持 .docx；Excel 请使用 xlsx_set_cell，PDF 不支持编辑")
	}
	original, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文档失败: %v", err)
	}
	if err := t.registry.backup(abs, original); err != nil {
		return errorResult("写入前备份失败: %v", err)
	}
	if err := doc.ReplaceDOCXText(abs, oldText, newText); err != nil {
		return errorResult("%v", err)
	}
	return Result{
		Output: fmt.Sprintf("已替换 %s 中的 1 处文本。", display),
		Meta:   map[string]any{"path": display},
	}
}

// ---------- xlsx_set_cell ----------

type xlsxSetCellTool struct{ registry *Registry }

func (t *xlsxSetCellTool) Name() string { return "xlsx_set_cell" }

func (t *xlsxSetCellTool) Description() string {
	return "修改 Excel 单元格的值或公式，保留工作表其余内容与样式；写入前会备份原文件。"
}

func (t *xlsxSetCellTool) Schema() map[string]any {
	return map[string]any{
		"type":     "object",
		"required": []string{"path", "cell", "value"},
		"properties": map[string]any{
			"path":  map[string]any{"type": "string", "description": "相对工作区的 .xlsx 路径"},
			"sheet": map[string]any{"type": "string", "description": "工作表名称或序号（从 1 开始），默认第一个"},
			"cell":  map[string]any{"type": "string", "description": "单元格引用，例如 B8"},
			"value": map[string]any{"type": "string", "description": "新值；以 = 开头时写入公式"},
		},
	}
}

func (t *xlsxSetCellTool) Execute(_ context.Context, args map[string]any) Result {
	abs, display, err := t.registry.resolve(stringArg(args, "path"))
	if err != nil {
		return errorResult("%v", err)
	}
	if strings.ToLower(filepath.Ext(abs)) != ".xlsx" {
		return errorResult("xlsx_set_cell 只支持 .xlsx 文件")
	}
	cell := stringArg(args, "cell")
	value := stringArg(args, "value")
	if strings.TrimSpace(cell) == "" {
		return errorResult("cell 不能为空")
	}
	original, err := os.ReadFile(abs)
	if err != nil {
		return errorResult("读取文档失败: %v", err)
	}
	if err := t.registry.backup(abs, original); err != nil {
		return errorResult("写入前备份失败: %v", err)
	}
	if err := doc.SetXLSXCell(abs, stringArg(args, "sheet"), cell, value); err != nil {
		return errorResult("%v", err)
	}
	return Result{
		Output: fmt.Sprintf("已更新 %s 的单元格 %s。", display, strings.ToUpper(strings.TrimSpace(cell))),
		Meta:   map[string]any{"path": display, "cell": strings.ToUpper(strings.TrimSpace(cell))},
	}
}
