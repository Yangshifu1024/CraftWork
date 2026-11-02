// Package app 负责 CraftWork 的进程级装配：
// 数据目录、数据库、密钥、事件总线、策略引擎、工具注册表、Agent 运行器、
// Wails3 应用与窗口。
//
// 这里不包含业务逻辑，只负责把各层连接起来。本文件同时承担启动顺序的控制：
// 先恢复数据与配置，再创建服务与窗口，最后进入事件循环；退出时统一关闭资源。
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
	"craftwork/internal/notify"
	"craftwork/internal/policy"
	"craftwork/internal/record"
	"craftwork/internal/secret"
	"craftwork/internal/service"
	"craftwork/internal/store"
	"craftwork/internal/tools"
	"craftwork/internal/version"

	"github.com/wailsapp/wails/v3/pkg/application"
)

const (
	// appName 是窗口标题与系统通知里展示的产品名。
	appName = "CraftWork"
	// appID 是单实例与系统层面使用的应用标识。
	appID = "com.craftwork.app"
	// windowName 是主窗口的标识，用于按名称查找并唤起窗口。
	windowName = "main"
)

// emitter 允许事件总线在 Wails 应用创建完成后再绑定发送函数。
//
// 事件总线在装配早期就需要一个发送函数，而 Wails 应用要到后面才创建，
// 因此这里用一层可延迟绑定的包装，读路径加读锁保证并发安全。
type emitter struct {
	// mu 保护 app 指针，允许绑定与发送并发进行。
	mu sync.RWMutex
	// app 是 Wails 应用实例，未绑定时为 nil。
	app *application.App
}

// emit 把事件转发给 Wails 的前端事件通道；尚未绑定时直接丢弃。
func (e *emitter) emit(name string, data any) {
	e.mu.RLock()
	current := e.app
	e.mu.RUnlock()
	if current != nil {
		current.Event.Emit(name, data)
	}
}

// bind 在 Wails 应用创建完成后记录实例，后续事件即可正常发送。
func (e *emitter) bind(app *application.App) {
	e.mu.Lock()
	e.app = app
	e.mu.Unlock()
}

// Run 启动桌面应用，阻塞直到应用退出。
//
// 启动顺序有依赖关系：数据目录 → 数据库（含中断清理）→ 密钥 → 事件总线 →
// 策略与工具 → 服务 → 恢复设置 → 创建窗口。任何一步失败都会直接返回错误，
// 让入口打印后退出，避免带着半初始化的状态继续运行。
func Run(frontend embed.FS, build version.Info) error {
	dataDir, err := resolveDataDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return fmt.Errorf("创建数据目录 %s: %w", dataDir, err)
	}
	// 粘贴附件统一放在数据目录，避免写进用户工作区；Agent 读取附件时也会
	// 接受该目录，因此这里先建好，保证路径解析时目录真实存在。
	attachmentDir := service.AttachmentDir(dataDir)
	if err := os.MkdirAll(attachmentDir, 0o700); err != nil {
		return fmt.Errorf("创建附件目录 %s: %w", attachmentDir, err)
	}

	db, err := store.Open(filepath.Join(dataDir, "craftwork.db"))
	if err != nil {
		return err
	}
	defer db.Close()
	// 上次进程退出时可能还有 running 状态的运行与挂起的审批，这里统一收尾。
	if interrupted, err := db.InterruptStaleRuns(); err != nil {
		slog.Warn("清理上次未完成的运行失败", "err", err)
	} else if interrupted > 0 {
		slog.Warn("已标记上次中断的运行", "count", interrupted)
	}

	// 密钥文件损坏时不阻止启动，而是以空密钥继续，让用户在设置页重新填写。
	secrets, err := secret.Open(dataDir)
	if err != nil {
		slog.Warn("读取密钥文件失败，本次以空密钥启动", "err", err)
		secrets = secret.NewEmpty(dataDir)
	}
	for _, warning := range secrets.Warnings() {
		slog.Warn(warning)
	}

	// 事件总线通过 emitter 发送，此时 Wails 应用还没创建，事件会暂时丢弃。
	emitter := &emitter{}
	bus := events.NewBus(emitter.emit)
	defer bus.Close()

	// 审批档位先用默认值，稍后从设置里恢复用户的选择。
	engine := policy.New(policy.LevelGraded, "")
	registry := tools.NewRegistry(tools.Options{
		BackupDir: filepath.Join(dataDir, "backups"),
		// 每次写文件前的备份都要登记到数据库，便于审计与回滚。
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
	// 失败转移模型从设置里实时读取，用户改设置后无需重启。
	modelFactory.SetFallbackResolver(func(string) string {
		return settingsService.Get().FallbackModel
	})
	recorder := record.New(db, bus)
	approvals := agent.NewApprovals(db, recorder)
	runner := agent.NewRunner(db, recorder, registry, engine, approvals, modelFactory, attachmentDir, dataDir)
	// 单次运行最大模型往返数从设置里实时读取，用户改设置后无需重启。
	runner.SetMaxIterationsResolver(func() int {
		return settingsService.Get().MaxIterations
	})

	appService := service.NewAppService(dataDir, build)
	threadService := service.NewThreadService(db, recorder)
	chatService := service.NewChatService(db, recorder, runner, approvals)
	workspaceService := service.NewWorkspaceService(db, bus, engine, registry, filepath.Join(dataDir, "backups"))
	providerService := service.NewProviderService(db, secrets)
	auditService := service.NewAuditService(db)
	terminalService := service.NewTerminalService(bus, registry.Workspace)
	documentService := service.NewDocumentService(db)
	mcpService := service.NewMCPService(db, registry)

	// 确保内置临时会话目录存在：临时会话绑定它运行，首次启动无需先添加
	// 工作区即可开聊；目录被用户删除后也会在这里自动重建。
	// 失败只降级不阻断启动：受影响的只是临时会话能力。
	conversation, err := service.EnsureConversationWorkspace(db, dataDir)
	if err != nil {
		slog.Warn("初始化临时会话目录失败", "err", err)
	} else if err := db.BackfillThreadWorkspaces(conversation.ID); err != nil {
		// 历史版本在无工作区时创建的会话没有绑定目录（运行时所有路径工具
		// 都会失败），统一归入临时目录，让它们可以继续使用。
		slog.Warn("回填会话工作区失败", "err", err)
	}

	// 启动时恢复活动工作区与审批档位。
	settings := settingsService.Get()
	engine.SetLevel(policy.ParseLevel(settings.ApprovalLevel))
	if settings.ActiveWorkspace != "" {
		if workspace, err := db.GetWorkspace(settings.ActiveWorkspace); err == nil {
			// 信任级别决定是否允许永久授权：非 trusted 一律按受限工作区处理。
			engine.SetWorkspace(workspace.Path)
			engine.SetRestricted(workspace.Trust != "trusted")
			registry.SetWorkspace(workspace.Path)
		}
	}
	// 批量事件通道需要在创建应用前注册，前端才能收到 cw:events。
	application.RegisterEvent[events.Batch](events.BatchName)

	wailsApp := application.New(application.Options{
		Name:        appName,
		Description: "本地优先、可审计的桌面 Agent",
		// 每个服务都会生成对应的前端绑定方法，新增服务必须在这里注册。
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
			application.NewService(mcpService),
		},
		Assets: application.AssetOptions{
			Handler: application.AssetFileServerFS(frontend),
			// Wails3 默认不设置安全头，必须由中间件补上。
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
			// 退出前关闭事件总线，让后台批量发送的 goroutine 尽快结束。
			bus.Close()
		},
		Mac: application.MacOptions{
			ApplicationShouldTerminateAfterLastWindowClosed: true,
		},
	})
	// 绑定之后，事件总线产生的事件才会真正发到前端。
	emitter.bind(wailsApp)

	wailsApp.Window.NewWithOptions(application.WebviewWindowOptions{
		Name:             windowName,
		Title:            appName,
		Width:            1440,
		Height:           900,
		MinWidth:         960,
		MinHeight:        640,
		BackgroundColour: application.NewRGB(11, 13, 17),
		// 窗口只加载本地根路径，绝对 URL 禁止传进来。
		URL:            "/",
		EnableFileDrop: true,
		Mac: application.MacWindow{
			TitleBar:                application.MacTitleBarHiddenInset,
			InvisibleTitleBarHeight: 48,
		},
	})

	// 窗口聚焦时不打扰用户；系统通知失败不会影响运行。
	runner.SetNotifier(func(title, message string) {
		if !settingsService.Get().Notifications {
			return
		}
		if current := wailsApp.Window.Current(); current != nil && current.IsFocused() {
			return
		}
		notify.Send(title, message)
	})

	// 恢复 MCP 连接可能较慢，放到后台执行，不阻塞窗口显示。
	go mcpService.Restore()

	return wailsApp.Run()
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
//
// Wails3 的单实例机制只负责通知，不会自动把窗口带到前台，
// 所以这里显式执行还原、显示与聚焦。
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
//
// 具体策略由构建标签决定：开发模式需要放宽以支持热更新，
// 生产模式只允许同源脚本与本地资源。
func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		header := w.Header()
		header.Set("Content-Security-Policy", contentSecurityPolicy())
		header.Set("X-Content-Type-Options", "nosniff")
		header.Set("Referrer-Policy", "no-referrer")
		next.ServeHTTP(w, r)
	})
}
