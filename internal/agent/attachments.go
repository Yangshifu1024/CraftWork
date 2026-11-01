// 本文件负责读取用户选中的附件并转换成发送给模型的附件结构：校验路径必须
// 位于工作区或应用附件目录内，拒绝密钥与环境文件，限制单个与总大小，按模型
// 能力决定是否发送图片，无法发送的附件以说明文字代替。
//
// 附件有两个来源：系统文件选择框返回的工作区路径，以及输入区粘贴后由
// AppService 写入数据目录 attachments/ 的文件。后者不在工作区内，读取时
// 只接受绝对路径并经过同样的越界与符号链接检查。

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

// 附件读取限制。超过限制的附件不会读取内容，只在结果里标记跳过原因。
const (
	// maxTextAttachmentBytes 是单个文本附件的最大字节数（1MB）。
	maxTextAttachmentBytes = 1 << 20 // 1MB
	// maxImageAttachmentBytes 是单张图片的最大字节数（5MB）。
	maxImageAttachmentBytes = 5 << 20 // 5MB
	// maxTotalAttachmentBytes 是一条消息内所有附件合计的最大字节数（12MB）。
	maxTotalAttachmentBytes = 12 << 20 // 12MB
	// maxAttachmentsPerMessage 是一条消息最多处理的附件数量。
	maxAttachmentsPerMessage = 10
)

// textAttachmentExtensions 是允许作为文本附件读取的扩展名集合。列表之外的
// 类型一律跳过，避免把未知二进制内容当作文本发给模型。
var textAttachmentExtensions = map[string]bool{
	".txt": true, ".md": true, ".markdown": true, ".go": true, ".js": true, ".jsx": true,
	".ts": true, ".tsx": true, ".json": true, ".yaml": true, ".yml": true, ".toml": true,
	".csv": true, ".tsv": true, ".log": true, ".py": true, ".rb": true, ".rs": true,
	".java": true, ".kt": true, ".c": true, ".h": true, ".cc": true, ".cpp": true,
	".hpp": true, ".cs": true, ".php": true, ".swift": true, ".sh": true, ".bash": true,
	".zsh": true, ".sql": true, ".html": true, ".htm": true, ".css": true, ".scss": true,
	".xml": true, ".ini": true, ".cfg": true, ".conf": true,
}

// imageAttachmentMIME 把图片扩展名映射到媒体类型（MIME）。只有这里列出的
// 图片格式会以 base64 数据发送给模型。
var imageAttachmentMIME = map[string]string{
	".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
	".gif": "image/gif", ".webp": "image/webp",
}

// loadAttachments 读取用户选中的附件。路径必须位于当前工作区或应用附件
// 目录内；密钥、环境文件与不支持的二进制类型不会发送给模型，而是以说明
// 文本代替。
//
// 处理顺序：数量截断 -> 敏感文件名拦截 -> 路径解析 -> 文件信息检查 ->
// 图片或文本分支。被跳过的附件仍保留在结果里，并带上 Skipped 说明，让界面
// 能告诉用户为什么没发送。只有真正读取的附件才计入总大小。
func loadAttachments(workspace, attachmentDir string, paths []string, caps model.Capabilities) []model.Attachment {
	if len(paths) == 0 {
		return nil
	}
	result := make([]model.Attachment, 0, len(paths))
	// total 是已读取附件的累计字节数，用于控制总大小。
	total := int64(0)
	for index, rawPath := range paths {
		if index >= maxAttachmentsPerMessage {
			// 超出数量上限的附件直接忽略，不进入结果。
			break
		}
		name := filepath.Base(rawPath)
		attachment := model.Attachment{Name: name}
		// 先按文件名拦截敏感文件，这类文件既不会读取内容，也不会被解析路径。
		if policy.IsSensitiveAttachment(name) {
			attachment.Skipped = "密钥或环境文件不会发送给模型"
			result = append(result, attachment)
			continue
		}

		// 路径解析包含工作区越界与符号链接逃逸检查。
		resolved, err := resolveAttachmentPath(workspace, attachmentDir, rawPath)
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

		// 图片分支：模型不支持图片、单张超限与总量超限时跳过，否则读取并
		// 转成 base64。
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

		// 文本分支：扩展名不在白名单内直接跳过。
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
				// 含 NUL 字节说明是二进制内容，即使扩展名在白名单里也不发送。
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

// resolveAttachmentPath 把附件路径解析成经过符号链接检查的真实路径。
//
// 允许两种来源：工作区内的文件，以及应用附件目录中的粘贴文件。附件目录
// 只接受绝对路径，避免把工作区的相对路径误判成附件目录里的同名文件。两个
// 分支都经过 policy.Resolve，因此同样会拒绝越界与符号链接逃逸。
func resolveAttachmentPath(workspace, attachmentDir, rawPath string) (string, error) {
	if attachmentDir != "" && filepath.IsAbs(rawPath) {
		if resolved, err := policy.Resolve(attachmentDir, rawPath); err == nil {
			return resolved, nil
		}
	}
	return policy.Resolve(workspace, rawPath)
}

// userAttachmentPaths 从用户消息部件中收集附件路径。
//
// 只处理文本部件：附件路径保存在该部件数据的 attachments 字段里。JSON 反
// 序列化后通常是 []any，直接构造时可能是 []string，两种都兼容；空路径与
// 其他类型会被忽略。
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
