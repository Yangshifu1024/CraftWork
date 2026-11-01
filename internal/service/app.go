// Package service 是 Go 与前端之间的绑定层。
//
// 每个导出类型都会被 Wails3 注册为服务，导出方法会生成对应的 TypeScript
// 调用入口。服务层只做参数校验、编排与错误转换，业务逻辑放在领域包中。
//
// 本文件提供应用级能力：构建信息读取、窗口控制、文件选择、剪贴板，
// 以及唯一允许打开外部链接的出口。
package service

import (
	"errors"
	"fmt"
	"net/url"
	"runtime"
	"strings"

	"craftwork/internal/version"

	"github.com/wailsapp/wails/v3/pkg/application"
)

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
	// build 是编译期注入的版本信息。
	build version.Info
}

// NewAppService 创建应用服务。
func NewAppService(dataDir string, build version.Info) *AppService {
	return &AppService{dataDir: dataDir, build: build}
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
