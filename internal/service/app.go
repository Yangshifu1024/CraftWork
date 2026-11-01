// Package service 是 Go 与前端之间的绑定层。
//
// 每个导出类型都会被 Wails3 注册为服务，导出方法会生成对应的 TypeScript
// 调用入口。服务层只做参数校验、编排与错误转换，业务逻辑放在领域包中。
//
// 本文件提供应用级能力：构建信息读取、窗口控制、文件选择、剪贴板，
// 以及唯一允许打开外部链接的出口。
package service

import (
	"encoding/base64"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"unicode"
	"unicode/utf8"

	"craftwork/internal/id"
	"craftwork/internal/policy"
	"craftwork/internal/version"

	"github.com/wailsapp/wails/v3/pkg/application"
)

// attachmentDirName 是数据目录下保存粘贴附件的子目录名。
const attachmentDirName = "attachments"

// maxPastedAttachmentBytes 是单个粘贴附件的最大字节数。
// 与 Agent 侧一条消息的附件总上限一致，超过后直接拒绝，避免无意义落盘。
const maxPastedAttachmentBytes = 12 << 20 // 12MB

// AttachmentDir 返回保存粘贴附件的绝对目录。
//
// 附件目录与工作区分离，避免把用户复制的内容写进项目；Agent 读取附件时
// 会同时接受这个目录。装配 Runner 与 AppService 时必须使用同一个路径。
func AttachmentDir(dataDir string) string {
	return filepath.Join(dataDir, attachmentDirName)
}

// AppInfo 是前端启动时读取的构建与运行环境信息。
type AppInfo struct {
	// Name 是应用名称，固定为 CraftWork。
	Name string `json:"name"`
	// Version 是构建版本号。
	Version string `json:"version"`
	// Commit 是构建对应的提交号。
	Commit string `json:"commit"`
	// Build 是构建时间。
	Build string `json:"buildTime"`
	// Platform 是运行平台，例如 darwin、windows、linux。
	Platform string `json:"platform"`
	// Arch 是运行架构，例如 arm64、amd64。
	Arch string `json:"arch"`
	// DataDir 是数据目录路径，数据库、密钥与备份都在其中。
	DataDir string `json:"dataDir"`
}

// AppService 提供应用级能力：信息读取、窗口控制、剪贴板与外部链接。
type AppService struct {
	// dataDir 是数据目录路径。
	dataDir string
	// attachmentDir 是粘贴附件的落盘目录。
	attachmentDir string
	// build 是编译期注入的版本信息。
	build version.Info
}

// NewAppService 创建应用服务。
func NewAppService(dataDir string, build version.Info) *AppService {
	return &AppService{dataDir: dataDir, attachmentDir: AttachmentDir(dataDir), build: build}
}

// Info 返回构建与运行环境信息。
func (s *AppService) Info() AppInfo {
	return AppInfo{
		Name:     "CraftWork",
		Version:  s.build.Version,
		Commit:   s.build.Commit,
		Build:    s.build.BuildTime,
		Platform: runtime.GOOS,
		Arch:     runtime.GOARCH,
		DataDir:  s.dataDir,
	}
}

// OpenExternal 用系统浏览器打开一个 http/https 链接。
//
// 这是应用内唯一允许打开外部页面的入口。javascript:、file:、data: 等
// 协议一律拒绝，避免把 WebView 变成任意代码或本地文件的执行入口。
func (s *AppService) OpenExternal(rawURL string) error {
	parsed, err := url.Parse(strings.TrimSpace(rawURL))
	if err != nil {
		return fmt.Errorf("无效的链接: %w", err)
	}
	switch strings.ToLower(parsed.Scheme) {
	case "http", "https":
	default:
		return errors.New("只允许打开 http 或 https 链接")
	}
	if parsed.Host == "" {
		return errors.New("链接缺少主机名")
	}
	current := application.Get()
	if current == nil {
		return errors.New("应用尚未初始化")
	}
	return current.Browser.OpenURL(parsed.String())
}

// MinimiseWindow 最小化主窗口。
func (s *AppService) MinimiseWindow() error {
	window, err := mainWindow()
	if err != nil {
		return err
	}
	window.Minimise()
	return nil
}

// ToggleMaximiseWindow 在最大化与还原之间切换主窗口。
func (s *AppService) ToggleMaximiseWindow() error {
	window, err := mainWindow()
	if err != nil {
		return err
	}
	window.ToggleMaximise()
	return nil
}

// CloseWindow 关闭主窗口。
func (s *AppService) CloseWindow() error {
	window, err := mainWindow()
	if err != nil {
		return err
	}
	window.Close()
	return nil
}

// PickFiles 打开系统文件选择对话框，返回选中的文件路径。
func (s *AppService) PickFiles() ([]string, error) {
	current := application.Get()
	if current == nil {
		return nil, errors.New("应用尚未初始化")
	}
	return current.Dialog.OpenFile().
		SetTitle("选择附件").
		CanChooseFiles(true).
		CanChooseDirectories(false).
		PromptForMultipleSelection()
}

// SavePastedAttachment 把剪贴板里的文件内容写入附件目录，返回绝对路径。
//
// WebView 无法把剪贴板中的文件暴露成本地路径，只能把字节传给 Go，因此前端
// 先转成 base64 再调用这里。返回的路径随后作为 ChatService.Send 的附件参数。
//
// 校验顺序：base64 解码 -> 大小上限 -> 敏感文件名 -> 目录创建 -> 唯一路径
// 写入。文件名只取最后一段并清洗，最终路径固定在附件目录内，调用方无法借
// 文件名跳出目录。
func (s *AppService) SavePastedAttachment(name, content string) (string, error) {
	data, err := base64.StdEncoding.DecodeString(strings.TrimSpace(content))
	if err != nil {
		return "", errors.New("附件内容不是有效的 base64 编码")
	}
	if len(data) == 0 {
		return "", errors.New("附件内容为空")
	}
	if len(data) > maxPastedAttachmentBytes {
		return "", fmt.Errorf("附件超过 %dMB", maxPastedAttachmentBytes>>20)
	}
	// 敏感文件名要在清洗前判断：清洗会去掉开头的点，.env 之类的名字会失去
	// 原本的语义，必须在原始名字上拦截。
	baseName := filepath.Base(strings.ReplaceAll(strings.TrimSpace(name), "\\", "/"))
	if policy.IsSensitiveAttachment(baseName) {
		return "", errors.New("密钥或环境文件不会作为附件发送")
	}
	if err := os.MkdirAll(s.attachmentDir, 0o700); err != nil {
		return "", fmt.Errorf("创建附件目录: %w", err)
	}
	// 随机前缀既避免同名覆盖，也避免用用户提供的名字直接拼出路径。
	path := filepath.Join(s.attachmentDir, id.New("att")+"_"+sanitizeAttachmentName(name))
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return "", fmt.Errorf("写入附件: %w", err)
	}
	if _, err := file.Write(data); err != nil {
		file.Close()
		_ = os.Remove(path)
		return "", fmt.Errorf("写入附件: %w", err)
	}
	if err := file.Close(); err != nil {
		_ = os.Remove(path)
		return "", fmt.Errorf("写入附件: %w", err)
	}
	return path, nil
}

// attachmentMimes 是图片附件的扩展名到 MIME 类型的映射，也是缩略图读取的
// 唯一白名单：预览接口只读图片，非图片扩展名一律拒绝。
var attachmentMimes = map[string]string{
	".png":  "image/png",
	".jpg":  "image/jpeg",
	".jpeg": "image/jpeg",
	".gif":  "image/gif",
	".webp": "image/webp",
	".bmp":  "image/bmp",
}

// AttachmentContent 是附件文件的内容，Base64 编码后连同 MIME 类型一起返回。
type AttachmentContent struct {
	// Base64 是文件字节的 base64 编码，前端据此拼出 data URL 展示。
	Base64 string `json:"base64"`
	// Mime 是按扩展名推断的图片 MIME 类型。
	Mime string `json:"mime"`
}

// ReadAttachment 读取一张图片附件的内容，供前端渲染缩略图与放大预览。
//
// 附件路径有两类来源：粘贴落盘的附件目录，以及用户通过系统文件选择框主动
// 选取的任意路径。两者都是用户明确提供的文件，因此这里按「只读图片」收窄
// 风险面：扩展名必须在白名单内、大小不超过粘贴附件的上限，其余一律拒绝。
// 返回 base64 而不是文件路径，避免 WebView 直接加载 file:// 地址。
func (s *AppService) ReadAttachment(path string) (*AttachmentContent, error) {
	trimmed := strings.TrimSpace(path)
	if trimmed == "" {
		return nil, errors.New("附件路径为空")
	}
	ext := strings.ToLower(filepath.Ext(trimmed))
	mime, ok := attachmentMimes[ext]
	if !ok {
		return nil, errors.New("只支持预览图片附件")
	}
	info, err := os.Stat(trimmed)
	if err != nil {
		return nil, errors.New("附件文件不存在或无法访问")
	}
	if info.IsDir() {
		return nil, errors.New("附件路径指向的是目录")
	}
	if info.Size() > maxPastedAttachmentBytes {
		return nil, fmt.Errorf("图片超过 %dMB，不进行预览", maxPastedAttachmentBytes>>20)
	}
	data, err := os.ReadFile(trimmed)
	if err != nil {
		return nil, fmt.Errorf("读取附件: %w", err)
	}
	return &AttachmentContent{Base64: base64.StdEncoding.EncodeToString(data), Mime: mime}, nil
}

// sanitizeAttachmentName 把用户提供的文件名清洗成可安全落盘的片段。
//
// 只保留字母、数字、点、下划线、连字符与空格，其余字符（路径分隔符、控制
// 字符、引号等）替换为下划线；去掉开头的点避免生成隐藏文件；空名字回退为
// pasted-file；过长时按字符截断主体并整体保留扩展名。
func sanitizeAttachmentName(name string) string {
	// 先按两种路径分隔符取最后一段：粘贴过来的名字可能带 Windows 风格路径。
	name = filepath.Base(strings.ReplaceAll(strings.TrimSpace(name), "\\", "/"))
	var cleaned strings.Builder
	for _, r := range name {
		switch {
		case unicode.IsLetter(r) || unicode.IsDigit(r):
			cleaned.WriteRune(r)
		case r == '.' || r == '_' || r == '-' || r == ' ':
			cleaned.WriteRune(r)
		default:
			cleaned.WriteRune('_')
		}
	}
	result := strings.TrimLeft(strings.TrimSpace(cleaned.String()), ".")
	if result == "" {
		result = "pasted-file"
	}
	// 按字符截断，避免切断中文等多字节字符。
	const maxNameRunes = 96
	if utf8.RuneCountInString(result) > maxNameRunes {
		extension := filepath.Ext(result)
		if utf8.RuneCountInString(extension) > 16 {
			extension = ""
		}
		body := []rune(strings.TrimSuffix(result, extension))
		limit := maxNameRunes - utf8.RuneCountInString(extension)
		if limit < 1 {
			limit = 1
		}
		if len(body) > limit {
			body = body[:limit]
		}
		result = string(body) + extension
	}
	return result
}

// WriteClipboard 写入系统剪贴板，返回是否成功。
func (s *AppService) WriteClipboard(text string) bool {
	current := application.Get()
	if current == nil {
		return false
	}
	return current.Clipboard.SetText(text)
}

// ReadClipboard 读取系统剪贴板文本。
func (s *AppService) ReadClipboard() string {
	current := application.Get()
	if current == nil {
		return ""
	}
	text, _ := current.Clipboard.Text()
	return text
}

// mainWindow 取名为 main 的主窗口；应用未初始化或窗口不存在时返回错误。
func mainWindow() (application.Window, error) {
	current := application.Get()
	if current == nil {
		return nil, errors.New("应用尚未初始化")
	}
	window, ok := current.Window.GetByName("main")
	if !ok {
		return nil, errors.New("主窗口不存在")
	}
	return window, nil
}
