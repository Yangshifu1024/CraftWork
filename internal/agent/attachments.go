package agent

import (
	"bytes"
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"

	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/store"
)

const (
	maxTextAttachmentBytes   = 1 << 20  // 1MB
	maxImageAttachmentBytes  = 5 << 20  // 5MB
	maxTotalAttachmentBytes  = 12 << 20 // 12MB
	maxAttachmentsPerMessage = 10
)

var textAttachmentExtensions = map[string]bool{
	".txt": true, ".md": true, ".markdown": true, ".go": true, ".js": true, ".jsx": true,
	".ts": true, ".tsx": true, ".json": true, ".yaml": true, ".yml": true, ".toml": true,
	".csv": true, ".tsv": true, ".log": true, ".py": true, ".rb": true, ".rs": true,
	".java": true, ".kt": true, ".c": true, ".h": true, ".cc": true, ".cpp": true,
	".hpp": true, ".cs": true, ".php": true, ".swift": true, ".sh": true, ".bash": true,
	".zsh": true, ".sql": true, ".html": true, ".htm": true, ".css": true, ".scss": true,
	".xml": true, ".ini": true, ".cfg": true, ".conf": true,
}

var imageAttachmentMIME = map[string]string{
	".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
	".gif": "image/gif", ".webp": "image/webp",
}

// loadAttachments 读取用户选中的附件。路径必须位于当前工作区内；密钥、
// 环境文件与不支持的二进制类型不会发送给模型，而是以说明文本代替。
func loadAttachments(workspace string, paths []string, caps model.Capabilities) []model.Attachment {
	if len(paths) == 0 {
		return nil
	}
	result := make([]model.Attachment, 0, len(paths))
	total := int64(0)
	for index, rawPath := range paths {
		if index >= maxAttachmentsPerMessage {
			break
		}
		name := filepath.Base(rawPath)
		attachment := model.Attachment{Name: name}
		if blockedAttachment(name) {
			attachment.Skipped = "密钥或环境文件不会发送给模型"
			result = append(result, attachment)
			continue
		}

		resolved, err := policy.Resolve(workspace, rawPath)
		if err != nil {
			attachment.Skipped = "附件不在当前工作区"
			result = append(result, attachment)
			continue
		}
		info, err := os.Stat(resolved)
		if err != nil {
			attachment.Skipped = "无法读取文件信息"
			result = append(result, attachment)
			continue
		}
		if info.IsDir() {
			attachment.Skipped = "文件夹不能作为附件"
			result = append(result, attachment)
			continue
		}
		attachment.Size = info.Size()
		extension := strings.ToLower(filepath.Ext(name))

		if mime, isImage := imageAttachmentMIME[extension]; isImage {
			switch {
			case !caps.Vision:
				attachment.Skipped = "当前模型不支持图片输入"
			case info.Size() > maxImageAttachmentBytes:
				attachment.Skipped = "图片超过 5MB"
			case total+info.Size() > maxTotalAttachmentBytes:
				attachment.Skipped = "附件总大小超过 12MB"
			default:
				data, err := os.ReadFile(resolved)
				if err != nil {
					attachment.Skipped = "读取文件失败"
				} else {
					attachment.MIME = mime
					attachment.Data = base64.StdEncoding.EncodeToString(data)
					total += info.Size()
				}
			}
			result = append(result, attachment)
			continue
		}

		if !textAttachmentExtensions[extension] {
			attachment.Skipped = "暂不支持该附件类型（支持文本与 PNG/JPEG/GIF/WebP）"
			result = append(result, attachment)
			continue
		}
		switch {
		case info.Size() > maxTextAttachmentBytes:
			attachment.Skipped = "文本附件超过 1MB"
		case total+info.Size() > maxTotalAttachmentBytes:
			attachment.Skipped = "附件总大小超过 12MB"
		default:
			data, err := os.ReadFile(resolved)
			switch {
			case err != nil:
				attachment.Skipped = "读取文件失败"
			case bytes.IndexByte(data, 0) >= 0:
				attachment.Skipped = "看起来是二进制文件"
			default:
				attachment.Text = string(data)
				total += info.Size()
			}
		}
		result = append(result, attachment)
	}
	return result
}

func blockedAttachment(name string) bool {
	lower := strings.ToLower(name)
	if lower == ".env" || strings.HasPrefix(lower, ".env.") ||
		lower == "id_rsa" || lower == "id_ed25519" {
		return true
	}
	for _, suffix := range []string{".pem", ".key", ".p12", ".pfx", ".jks"} {
		if strings.HasSuffix(lower, suffix) {
			return true
		}
	}
	return false
}

// userAttachmentPaths 从用户消息部件中收集附件路径。
func userAttachmentPaths(parts []store.Part) []string {
	var paths []string
	for _, part := range parts {
		if part.Kind != "text" {
			continue
		}
		data, _ := part.Data.(map[string]any)
		switch list := data["attachments"].(type) {
		case []any:
			for _, item := range list {
				if path, ok := item.(string); ok && path != "" {
					paths = append(paths, path)
				}
			}
		case []string:
			for _, path := range list {
				if path != "" {
					paths = append(paths, path)
				}
			}
		}
	}
	return paths
}
