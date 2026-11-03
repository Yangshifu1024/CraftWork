// Package app 负责 CraftWork 的进程级装配：
// 数据目录、数据库、密钥、事件总线、策略引擎、工具注册表、Agent 运行器、
// mygo 应用与窗口。
//
// 这里不包含业务逻辑，只负责把各层连接起来。本文件同时承担启动顺序的控制：
// 先恢复数据与配置，再创建服务与窗口，最后进入事件循环；退出时统一关闭资源。
package app

import (
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sync"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"

	"github.com/yangshifu1024/craftwork/internal/agent"
	"github.com/yangshifu1024/craftwork/internal/events"
	"github.com/yangshifu1024/craftwork/internal/id"
	"github.com/yangshifu1024/craftwork/internal/model"
	"github.com/yangshifu1024/craftwork/internal/notify"
	"github.com/yangshifu1024/craftwork/internal/policy"
	"github.com/yangshifu1024/craftwork/internal/record"
	"github.com/yangshifu1024/craftwork/internal/secret"
	"github.com/yangshifu1024/craftwork/internal/service"
	"github.com/yangshifu1024/craftwork/internal/store"
	"github.com/yangshifu1024/craftwork/internal/tools"
	cwui "github.com/yangshifu1024/craftwork/internal/ui"
	cwsidebar "github.com/yangshifu1024/craftwork/internal/ui/sidebar"
	"github.com/yangshifu1024/craftwork/internal/version"
)

const (
	// appName 是窗口标题、单实例锁与系统通知里展示的产品名。
	appName = "CraftWork"
	// windowStateKey 是主窗口的状态键，mygo 用它记忆窗口的位置与尺寸。
	windowStateKey = "main"
)

// emitter 允许事件总线在主窗口创建后再绑定派发目标。
//
// 事件总线在装配早期就需要一个派发函数，而窗口要到 WhenReady 里才创建，
// 因此这里用一层可延迟绑定的包装，读路径加读锁保证并发安全。
type emitter struct {
	// mu 保护 win 指针，允许绑定与派发并发进行。
	mu sync.RWMutex
	// win 是主窗口，未绑定时为 nil。
	win *mygo.Window
}

// emit 把一批事件转交给主线程；窗口尚未绑定时直接丢弃——与 Wails 时期
// 「应用未创建前事件丢弃」的语义一致。win.Update 保证回调在主线程执行，
// 界面在下一帧统一消费；界面状态装配完成前（后续 UI 票接线）回调为空，
// 事件进主线程即止。
func (e *emitter) emit(batch events.Batch) {
	e.mu.RLock()
	current := e.win
	e.mu.RUnlock()
	if current == nil {
		return
	}
	current.Update(func() {
		// 界面状态装配（后续 UI 票）会在这里把 batch 写进界面状态。
	})
}

// bind 在主窗口创建完成后记录实例，后续事件即可正常进主线程。
func (e *emitter) bind(win *mygo.Window) {
	e.mu.Lock()
	e.win = win
	e.mu.Unlock()
}

// update 把一个函数调度进主线程（win.Update 的空安全包装）；窗口尚未
// 绑定时丢弃。侧栏等服务接线的异步回调用它把结果写进界面状态。
func (e *emitter) update(fn func()) {
	e.mu.RLock()
	current := e.win
	e.mu.RUnlock()
	if current == nil {
		return
	}
	current.Update(fn)
}

// Run 启动桌面应用，阻塞直到应用退出。
//
// 启动顺序有依赖关系：单实例锁 → 数据目录 → 数据库（含中断清理）→ 密钥 →
// 事件总线 → 策略与工具 → 服务 → 恢复设置 → 创建窗口。任何一步失败都会
// 直接返回错误，让入口打印后退出，避免带着半初始化的状态继续运行。
func Run(build version.Info) error {
	// 单实例锁在任何数据被打开之前请求：第二个实例连数据库都不该碰。
	// 锁按应用名标识，而 go run / go build 的默认名是可执行文件名，
	// 因此先把名字固定下来。
	mygo.App.SetName(appName)
	if !mygo.App.RequestSingleInstanceLock() {
		// 命令行已移交给运行中的实例，由它负责唤起窗口；这里安静退出。
		slog.Info("CraftWork 已在运行，本次启动移交给已有实例")
		return nil
	}

	dataDir, err := resolveDataDir()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		return fmt.Errorf("创建数据目录 %s: %w", dataDir, err)
	}
	// mygo 自己的用户数据目录（窗口状态等壳层数据）重定向到 CraftWork
	// 数据目录之下，让 CRAFTWORK_HOME 的隔离同时覆盖窗口状态；mygo 对
	// 覆盖路径不做自动创建，这里先建好，否则状态文件写不进去。
	shellDir := filepath.Join(dataDir, "shell")
	if err := os.MkdirAll(shellDir, 0o700); err != nil {
		return fmt.Errorf("创建壳层数据目录 %s: %w", shellDir, err)
	}
	mygo.App.SetPath(mygo.PathUserData, shellDir)
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

	// 事件总线通过 emitter 发送，此时窗口还没创建，事件会暂时丢弃。
	emitter := &emitter{}
	bus := events.NewBus(emitter.emit)
	defer bus.Close()

	// 审批档位先用默认值，稍后从设置里恢复用户的选择。
	engine := policy.New(policy.LevelGraded)
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
	settingsService := service.NewSettingsService(db, bus, engine)
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
	workspaceService := service.NewWorkspaceService(db, bus, filepath.Join(dataDir, "backups"))
	providerService := service.NewProviderService(db, secrets)
	auditService := service.NewAuditService(db)
	documentService := service.NewDocumentService(db)
	mcpService := service.NewMCPService(db, registry)

	// native UI 的 view 直调服务方法：chat / provider / document / mcp
	// 仍由后续 UI 票按槽位接入；侧栏（04 号票）已经接线。
	_ = []any{
		appService, chatService,
		providerService, documentService, mcpService,
	}

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

	// 启动时恢复审批档位。活动工作区只是 UI 偏好，不进任何策略状态：
	// 每个会话在运行开始时从自己绑定的工作区解析执行环境。
	settings := settingsService.Get()
	engine.SetLevel(policy.ParseLevel(settings.ApprovalLevel))

	// 第二个实例启动时唤起已有窗口。单实例机制只负责通知，
	// 不会自动把窗口带到前台。
	mygo.App.OnSecondInstance(func([]string, string) {
		focusMainWindow()
	})
	// 退出前关闭事件总线，让后台还在派发事件的 goroutine 尽快结束。
	mygo.App.OnWillQuit(func(*mygo.QuitEvent) {
		bus.Close()
	})

	// 窗口底色跟随主题：显式浅色的用户在首帧前不再看到深色窗口。
	// dark 与跟随系统仍用深底；首帧之后由外壳 view 按系统实时接管。
	windowBackground := cwui.WindowBackgroundHex(settings.Theme)

	// 三栏外壳的界面状态只在主线程访问；后台事件经 emitter 进主线程后
	// 由后续 UI 票接线写入。
	shell := cwui.NewShell()

	// 侧栏（04 号票）：状态壳 + 服务接线。异步动作在 goroutine 里调服务，
	// 结果经 emitter.update（win.Update）回主线程写状态。
	sidebarState := cwsidebar.NewSidebar()
	shell.SidebarView = sidebarState.View
	sidebarClient := cwsidebar.NewClient(
		sidebarState, threadService, workspaceService, settingsService, auditService,
		emitter.update,
		func(message string) { emitter.update(func() { sidebarState.QueueToast(message) }) },
		confirmDeleteDialog(),
	)
	sidebarState.SetActions(sidebarClient.Actions())

	mygo.App.WhenReady(func() {
		win := mygo.NewWindow(mygo.WindowOptions{
			Title:           appName,
			Width:           1440,
			Height:          900,
			MinWidth:        960,
			MinHeight:       640,
			BackgroundColor: windowBackground,
			// 窗口位置与尺寸按状态键记忆，下次启动回到原位。
			StateKey: windowStateKey,
			// 隐藏标题栏：界面顶部自画拖拽区并避让红绿灯。
			TitleBarStyle: mygo.TitleBarHiddenInset,
			// 三栏外壳：顶部拖拽区 + 侧栏 / 对话 / 面板；主题在 view 内
			// 每帧套用并跟随系统。窗口没有页面可加载，应用内不出现任何
			// WebView。
			Content: ui.View(shell.View),
		})
		// 绑定之后，事件总线产生的事件才会真正进主线程。
		emitter.bind(win)
		// 侧栏启动首拉：恢复模式、工作区清单、当前模式列表与计数。
		sidebarClient.Boot()
	})

	// 窗口聚焦时不打扰用户；系统通知失败不会影响运行。
	runner.SetNotifier(func(title, message string) {
		if !settingsService.Get().Notifications {
			return
		}
		if mygo.FocusedWindow() != nil {
			return
		}
		notify.Send(title, message)
	})

	// 恢复 MCP 连接可能较慢，放到后台执行，不阻塞窗口显示。
	go mcpService.Restore()

	return mygo.App.Run()
}

// confirmDeleteDialog 返回侧栏删除确认的原生实现（04 号票的简易路径）：
// Dialog.Message 会阻塞调用 goroutine 直到用户作答，因此整体放后台，
// 结果经 onDone（内部再经 win.Update）回主线程。
// TODO(07): 换成统一浮层体系的确认对话框（危险按钮样式、应用内呈现）。
func confirmDeleteDialog() cwsidebar.ConfirmFn {
	return func(title, message, detail string, onDone func(ok bool)) {
		go func() {
			result, err := mygo.Dialog.Message(mygo.MessageOptions{
				Type:               mygo.MessageWarning,
				Title:              title,
				Message:            message,
				Detail:             detail,
				Buttons:            []string{"取消", "删除"},
				DefaultButton:      0,
				CancelButton:       0,
				DestructiveButtons: []int{1},
			})
			if err != nil {
				slog.Warn("弹出删除确认失败", "err", err)
				return
			}
			onDone(result.Button == 1)
		}()
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
//
// 单实例机制只通知不唤起，这里显式执行还原、显示与聚焦；
// 窗口不存在（全部关闭）时无事可做。
func focusMainWindow() {
	window, err := service.MainWindow()
	if err != nil {
		return
	}
	window.Restore()
	window.Show()
	window.Focus()
}
