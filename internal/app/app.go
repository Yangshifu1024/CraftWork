// Package app 负责 CraftWork 的进程级装配：
// 数据目录、数据库、密钥、事件总线、策略引擎、工具注册表、Agent 运行器、
// Wails3 应用与窗口。
//
// 这里不包含业务逻辑，只负责把各层连接起来。
package app

import (
	"embed"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"sync"

	"craftwork/internal/agent"
	"craftwork/internal/events"
	"craftwork/internal/id"
	"craftwork/internal/model"
	"craftwork/internal/policy"
	"craftwork/internal/secret"
	"craftwork/internal/service"
	"craftwork/internal/store"
	"craftwork/internal/tools"
	"craftwork/internal/version"

	"github.com/wailsapp/wails/v3/pkg/application"
)

const (
	appName    = "CraftWork"
	appID      = "com.craftwork.app"
	windowName = "main"
)

// emitter 允许事件总线在 Wails 应用创建完成后再绑定发送函数。
type emitter struct {
	mu  sync.RWMutex
	app *application.App
}

func (e *emitter) emit(name string, data any) {
	e.mu.RLock()
	current := e.app
	e.mu.RUnlock()
	if current != nil {
		current.Event.Emit(name, data)
	}
}

func (e *emitter) bind(app *application.App) {
	e.mu.Lock()
	e.app = app
	e.mu.Unlock()
}

// Run 启动桌面应用，阻塞直到应用退出。
func Run(frontend embed.FS, build version.Info) error {
	dataDir, err := resolveDataDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return fmt.Errorf("创建数据目录 %s: %w", dataDir, err)
	}

	db, err := store.Open(filepath.Join(dataDir, "craftwork.db"))
	if err != nil {
		return err
	}
	defer db.Close()
	if err := db.InterruptStaleRuns(); err != nil {
		slog.Warn("清理上次未完成的运行失败", "err", err)
	}

	secrets, err := secret.Open(dataDir)
	if err != nil {
		slog.Warn("读取密钥文件失败，本次以空密钥启动", "err", err)
		secrets = secret.NewEmpty(dataDir)
	}
	for _, warning := range secrets.Warnings() {
		slog.Warn(warning)
	}

	emitter := &emitter{}
	bus := events.NewBus(emitter.emit)
	defer bus.Close()

	engine := policy.New(policy.LevelGraded, "")
	registry := tools.NewRegistry(tools.Options{
		BackupDir: filepath.Join(dataDir, "backups"),
		RecordBackup: func(path, backupPath, hash string, size int64) error {
			return db.AddBackup(&store.Backup{
				ID:         id.New("bk"),
				Path:       path,
				BackupPath: backupPath,
				Hash:       hash,
				Size:       size,
			})
		},
	})
	settingsService := service.NewSettingsService(db, bus, engine, registry)
	modelFactory := model.NewFactory(secrets)
	modelFactory.SetFallbackResolver(func(string) string {
		return settingsService.Get().FallbackModel
	})
	broker := agent.NewBroker(db, bus)
	runner := agent.NewRunner(db, bus, registry, engine, broker, modelFactory)

	appService := service.NewAppService(dataDir, build)
	threadService := service.NewThreadService(db, bus)
	chatService := service.NewChatService(db, bus, runner, broker)
	workspaceService := service.NewWorkspaceService(db, bus, engine, registry, filepath.Join(dataDir, "backups"))
	providerService := service.NewProviderService(secrets)
	auditService := service.NewAuditService(db)
	terminalService := service.NewTerminalService(bus, registry.Workspace)
	documentService := service.NewDocumentService(db)

	// 启动时恢复活动工作区与审批档位。
	settings := settingsService.Get()
	engine.SetLevel(policy.ParseLevel(settings.ApprovalLevel))
	if settings.ActiveWorkspace != "" {
		if workspace, err := db.GetWorkspace(settings.ActiveWorkspace); err == nil {
			engine.SetWorkspace(workspace.Path)
			engine.SetRestricted(workspace.Trust != "trusted")
			registry.SetWorkspace(workspace.Path)
		}
	}
	if os.Getenv("CRAFTWORK_DEMO") == "1" {
		seedDemo(db, settingsService, settings)
	}

	application.RegisterEvent[events.Batch](events.BatchName)

	wailsApp := application.New(application.Options{
		Name:        appName,
		Description: "本地优先、可审计的桌面 Agent",
		Services: []application.Service{
			application.NewService(appService),
			application.NewService(settingsService),
			application.NewService(threadService),
			application.NewService(chatService),
			application.NewService(workspaceService),
			application.NewService(providerService),
			application.NewService(auditService),
			application.NewService(terminalService),
			application.NewService(documentService),
		},
		Assets: application.AssetOptions{
			Handler:    application.AssetFileServerFS(frontend),
			Middleware: securityHeaders,
		},
		SingleInstance: &application.SingleInstanceOptions{
			UniqueID:               appID,
			OnSecondInstanceLaunch: focusMainWindow,
		},
		ErrorHandler: func(err error) {
			slog.Error("wails 运行时错误", "err", err)
		},
		OnShutdown: func() {
			bus.Close()
		},
		Mac: application.MacOptions{
			ApplicationShouldTerminateAfterLastWindowClosed: true,
		},
	})
	emitter.bind(wailsApp)

	wailsApp.Window.NewWithOptions(application.WebviewWindowOptions{
		Name:             windowName,
		Title:            appName,
		Width:            1440,
		Height:           900,
		MinWidth:         960,
		MinHeight:        640,
		BackgroundColour: application.NewRGB(11, 13, 17),
		URL:              "/",
		EnableFileDrop:   true,
		Mac: application.MacWindow{
			TitleBar:                application.MacTitleBarHiddenInset,
			InvisibleTitleBarHeight: 48,
		},
	})

	return wailsApp.Run()
}

// seedDemo 在演示模式下准备一个工作区与两条示例线程。
func seedDemo(db *store.DB, settingsService *service.SettingsService, settings service.Settings) {
	workspacePath := ""
	if settings.ActiveWorkspace != "" {
		if workspace, err := db.GetWorkspace(settings.ActiveWorkspace); err == nil {
			workspacePath = workspace.Path
		}
	}
	if workspacePath == "" {
		current, err := os.Getwd()
		if err != nil {
			return
		}
		workspace := &store.Workspace{
			ID:           id.New("ws"),
			Name:         filepath.Base(current),
			Path:         current,
			Trust:        "trusted",
			CreatedAt:    store.NowMillis(),
			LastOpenedAt: store.NowMillis(),
		}
		if err := db.UpsertWorkspace(workspace); err != nil {
			slog.Warn("创建演示工作区失败", "err", err)
			return
		}
		settings.ActiveWorkspace = workspace.ID
		if _, err := settingsService.Save(settings); err != nil {
			slog.Warn("保存演示设置失败", "err", err)
			return
		}
		workspacePath = workspace.Path
	}
	if err := db.SeedDemo(workspacePath); err != nil {
		slog.Warn("写入演示数据失败", "err", err)
	}
}

// resolveDataDir 返回本地数据目录，默认 ~/.craftwork。
// CRAFTWORK_HOME 仅用于测试与开发隔离。
func resolveDataDir() (string, error) {
	if dir := os.Getenv("CRAFTWORK_HOME"); dir != "" {
		return filepath.Abs(dir)
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("获取用户主目录: %w", err)
	}
	return filepath.Join(home, ".craftwork"), nil
}

// focusMainWindow 在第二个实例启动时唤起已有窗口。
func focusMainWindow(application.SecondInstanceData) {
	current := application.Get()
	if current == nil {
		return
	}
	if window, ok := current.Window.GetByName(windowName); ok {
		window.Restore()
		window.Show()
		window.Focus()
	}
}

// securityHeaders 为所有本地资源响应追加安全头。
// Wails3 默认不设置 CSP，也不拦截窗口导航，因此这层是必需的。
func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		header := w.Header()
		header.Set("Content-Security-Policy", contentSecurityPolicy())
		header.Set("X-Content-Type-Options", "nosniff")
		header.Set("Referrer-Policy", "no-referrer")
		next.ServeHTTP(w, r)
	})
}
