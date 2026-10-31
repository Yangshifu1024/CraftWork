package service

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

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
// 当前版本只返回文件信息与纯文本预览；docx / xlsx / pdf 的结构化解析
// 在 M3 接入。界面仍然按正式渲染路径展示，避免后续返工。
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
	case ".docx":
		preview.Kind = "docx"
		preview.Note = "Word 结构化解析将在 M3 接入；当前仅展示文件信息。"
	case ".xlsx":
		preview.Kind = "xlsx"
		preview.Note = "Excel 结构化解析将在 M3 接入；当前仅展示文件信息。"
	case ".pdf":
		preview.Kind = "pdf"
		preview.Note = "PDF 文本层提取将在 M3 接入；当前仅展示文件信息。"
	default:
		preview.Supported = true
		data, readErr := os.ReadFile(abs)
		if readErr != nil {
			return preview, fmt.Errorf("读取文件: %w", readErr)
		}
		if isBinaryData(data) {
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
