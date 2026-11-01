package service

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"craftwork/internal/doc"
	"craftwork/internal/platform"
	"craftwork/internal/policy"
	"craftwork/internal/store"
)

// DocumentPreview 是文档预览的返回结构。
type DocumentPreview struct {
	Kind      string `json:"kind"` // docx / xlsx / pdf / text
	Path      string `json:"path"`
	Name      string `json:"name"`
	Size      int64  `json:"size"`
	Modified  int64  `json:"modified"`
	Supported bool   `json:"supported"`
	Note      string `json:"note,omitempty"`
	Text      string `json:"text,omitempty"`
}

// DocumentService 提供文档预览。
//
// docx / xlsx / pdf 走 internal/doc 的文本级解析；其他文本文件直接预览。
// 写入由 Agent 的 doc_edit / xlsx_set_cell 工具完成，并经过审批与备份。
type DocumentService struct {
	db *store.DB
}

// NewDocumentService 创建文档服务。
func NewDocumentService(db *store.DB) *DocumentService {
	return &DocumentService{db: db}
}

// Preview 返回文档预览数据。
func (s *DocumentService) Preview(workspaceID, rel string) (DocumentPreview, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return DocumentPreview{}, err
	}
	abs, err := policy.Resolve(workspace.Path, rel)
	if err != nil {
		return DocumentPreview{}, err
	}
	info, err := os.Stat(abs)
	if err != nil {
		return DocumentPreview{}, fmt.Errorf("读取文件: %w", err)
	}

	preview := DocumentPreview{
		Kind:     "text",
		Path:     policy.Relative(workspace.Path, abs),
		Name:     filepath.Base(abs),
		Size:     info.Size(),
		Modified: info.ModTime().UnixMilli(),
	}

	switch strings.ToLower(filepath.Ext(abs)) {
	case ".docx", ".xlsx", ".pdf":
		document, readErr := doc.ReadFile(abs)
		if readErr != nil {
			preview.Note = readErr.Error()
			return preview, nil
		}
		preview.Kind = document.Kind
		preview.Text = document.Text
		preview.Note = document.Note
		preview.Supported = document.Text != ""
		if document.Truncated {
			preview.Note = strings.TrimSpace(preview.Note + " 文本已截断。")
		}
	default:
		preview.Supported = true
		data, readErr := os.ReadFile(abs)
		if readErr != nil {
			return preview, fmt.Errorf("读取文件: %w", readErr)
		}
		if platform.IsBinary(data) {
			preview.Supported = false
			preview.Note = "二进制文件，无法预览。"
			return preview, nil
		}
		text := string(data)
		if len(text) > 8000 {
			text = text[:8000] + "\n…（预览已截断）"
		}
		preview.Text = text
	}
	return preview, nil
}
