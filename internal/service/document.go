// 本文件是文档预览的前端绑定。它根据扩展名选择 internal/doc 的
// 文本级解析器，返回供界面展示的文本、格式说明与截断状态；
// 文档写入不在这里进行，而是走 Agent 的 doc_edit / xlsx_set_cell。

package service

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/yangshifu1024/craftwork/internal/doc"
	"github.com/yangshifu1024/craftwork/internal/platform"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/store"
)

// DocumentPreview 是文档预览的返回结构。
type DocumentPreview struct {
	// Kind 是文件类型：docx、xlsx、pdf 或 text。
	Kind string `json:"kind"`
	// Path 是相对所属源文件夹的路径。
	Path string `json:"path"`
	// FolderID 是该文件所属源文件夹的标识；空值表示主要目录。
	FolderID string `json:"folderId,omitempty"`
	// Name 是文件名。
	Name string `json:"name"`
	// Size 是文件字节数。
	Size int64 `json:"size"`
	// Modified 是最后修改时间（毫秒时间戳）。
	Modified int64 `json:"modified"`
	// Supported 表示当前能否展示文本内容。
	Supported bool `json:"supported"`
	// Note 是格式限制或读取失败的说明。
	Note string `json:"note,omitempty"`
	// Text 是可展示的文本内容。
	Text string `json:"text,omitempty"`
}

// DocumentService 提供文档预览。
//
// docx / xlsx / pdf 走 internal/doc 的文本级解析；其他文本文件直接预览。
// 写入由 Agent 的 doc_edit / xlsx_set_cell 工具完成，并经过审批与备份。
type DocumentService struct {
	// db 用于按标识读取工作区根目录。
	db *store.DB
}

// NewDocumentService 创建文档服务。
func NewDocumentService(db *store.DB) *DocumentService {
	return &DocumentService{db: db}
}

// Preview 返回文档预览数据。
//
// docx、xlsx、pdf 由 internal/doc 提取文本层；其他文件按纯文本读取，
// 二进制文件只返回元数据。文本超过 8000 字节时截断并提示。
func (s *DocumentService) Preview(workspaceID, folderID, rel string) (DocumentPreview, error) {
	workspace, err := s.db.GetWorkspace(workspaceID)
	if err != nil {
		return DocumentPreview{}, err
	}
	// folderID 限定目标源文件夹（空值表示主要目录），与文件树的分段渲染一致。
	root := workspace.Path
	if folderID != "" {
		matched := false
		for _, folder := range workspace.Folders {
			if folder.ID == folderID {
				root = folder.Path
				matched = true
				break
			}
		}
		if !matched {
			return DocumentPreview{}, fmt.Errorf("源文件夹不存在或已被移除")
		}
	}
	abs, err := policy.Resolve(root, rel)
	if err != nil {
		return DocumentPreview{}, err
	}
	info, err := os.Stat(abs)
	if err != nil {
		return DocumentPreview{}, fmt.Errorf("读取文件: %w", err)
	}

	preview := DocumentPreview{
		Kind:     "text",
		Path:     policy.Relative(root, abs),
		FolderID: folderID,
		Name:     filepath.Base(abs),
		Size:     info.Size(),
		Modified: info.ModTime().UnixMilli(),
	}

	if doc.Supported(abs) {
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
	} else {
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
