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
	cwchat "github.com/yangshifu1024/craftwork/internal/ui/chat"
	"github.com/yangshifu1024/craftwork/internal/ui/overlays"
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
	// mu 保护 win 与 batchHandler 指针，允许绑定与派发并发进行。
	mu sync.RWMutex
	// win 是主窗口，未绑定时为 nil。
	win *mygo.Window
	// onBatch 是界面侧的批次处理入口（05 号票聊天列的 reducer 派发），
	// 装配聊天列后设置；为空时事件进主线程即止。
	onBatch func(batch events.Batch)
}

// emit 把一批事件转交给主线程；窗口尚未绑定时直接丢弃——与 Wails 时期
// 「应用未创建前事件丢弃」的语义一致。win.Update 保证回调在主线程执行，
// 界面在下一帧统一消费。
func (e *emitter) emit(batch events.Batch) {
	e.mu.RLock()
	current := e.win
	handler := e.onBatch
	e.mu.RUnlock()
	if current == nil {
		return
	}
	current.Update(func() {
		if handler != nil {
			handler(batch)
		}
	})
}

// setBatchHandler 在装配聊天列后登记批次处理入口（主线程状态写入）。
func (e *emitter) setBatchHandler(fn func(batch events.Batch)) {
	e.mu.Lock()
	e.onBatch = fn
	e.mu.Unlock()
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

	// native UI 的 view 直调服务方法：浮层（设置中心、数据面板、命令面板）
	// 的后端门面在这里装配；thread / audit（04 侧栏）、chat / app（05 聊天
	// 列，下文的聊天接线消费）、workspace / document（08 面板）由下文的
	// 侧栏、聊天与面板接线消费。
	overlays.Bind(&overlays.Backend{
		Settings:  settingsService,
		Providers: providerService,
		Audit:     auditService,
		MCP:       mcpService,
		Workspace: workspaceService,
		Threads:   threadService,
		App:       appService,
	})

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
	// 外观设置（主题 / 字号）同样在首帧前恢复，显式主题覆盖系统偏好。
	cwui.SetAppearance(settings.Theme, settings.FontSize)

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
	// 由后续 UI 票接线写入。面板（08 号票）在装配期拿到它直调的服务，
	// 09 号票的终端（plugins/terminal 自管 PTY）经设置服务读字号。
	shell := cwui.NewShell()
	shell.SetPanelServices(workspaceService, documentService)
	shell.SetSettingsService(settingsService)

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

	// 聊天列（05 号票）：状态壳 + 服务接线。事件总线经 emitter 桥进主线程
	// 后交给 reducer 派发；需要刷新会话列表时联动侧栏 Refresh。
	chatState := cwchat.NewChat()
	shell.ChatView = chatState.View
	// 面板 tab 按「工作区 + 线程」过滤：把当前会话 Id 给到面板（08 号票
	// 留的接缝）；path:line 引用经面板 OpenFileAt 打开（正式入口）。
	shell.ThreadID = chatState.ActiveThreadID
	cwchat.SetAppService(appService)
	chatClient := cwchat.NewClient(
		chatState, chatService, threadService, workspaceService,
		emitter.update,
		sidebarClient.Refresh,
		func(id string) { sidebarState.Select(id) },
		func(mode string) string { return sidebarState.ActiveInMode(cwsidebar.Mode(mode)) },
	)
	emitter.setBatchHandler(func(batch events.Batch) {
		if chatClient.ApplyBatch(batch) {
			// 其他会话有变更或一次运行收尾：侧栏列表的排序与状态跟上数据库。
			sidebarClient.Refresh()
		}
	})

	// 侧栏动作接缝（04 号票留下的接线位）：选中会话 → 聊天列加载快照；
	// 模式切换 → 聊天列同步兜底模式并加载该模式的选中 / 第一条会话。
	acts := sidebarClient.Actions()
	acts.SelectThread = chatClient.SelectThread
	// path:line 引用（markdown.OnLink → craftwork://）→ 面板打开文件定位行。
	chatActions := chatClient.Actions()
	chatActions.OpenFile = shell.OpenFileAt
	chatState.SetActions(chatActions)
	switchMode := acts.SwitchMode
	acts.SwitchMode = func(m cwsidebar.Mode) {
		chatState.SetFallbackMode(cwchat.ModeFromSidebar(string(m)))
		switchMode(m)
		chatClient.SelectThreadOrFirst(sidebarState.ActiveInMode(m), string(m))
	}
	sidebarState.SetActions(acts)

	// 浮层触达其他视图状态的回调在这里接线：面板展开 / 收起是外壳状态；
	// 会话 / 模式 / 项目动作委托侧栏编排；文件打开走面板的 OpenFileAt。
	// 传 05 号票包装过的 acts，浮层切模式与侧栏走同一条聊天联动路径。
	wireOverlayHooks(shell, sidebarClient, acts)

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
			// 三栏外壳 + 浮层体系：顶部拖拽区、侧栏 / 对话 / 面板，浮层压在
			// 外壳之上。主题在 view 内每帧套用并跟随系统。窗口没有页面可
			// 加载，应用内不出现任何 WebView。
			Content: ui.View(cwui.RootView(shell)),
		})
		// 绑定之后，事件总线产生的事件才会真正进主线程；面板的后台
		// IO（文件树、读写、保存）结果也经这里回主线程。
		emitter.bind(win)
		// 浮层后台结果（连接测试、MCP 连接）经窗口投回主线程。
		overlays.BindWindow(win)
		// 面板的后台 IO（文件树、读写、保存）结果经窗口回主线程。
		shell.BindWindow(win)
		// 窗口关闭 / 应用退出回收面板的终端进程（插件挂断 PTY，不留
		// 孤儿 shell）。
		win.OnClosed(func() { shell.Shutdown() })
		// 侧栏启动首拉：恢复模式、工作区清单、当前模式列表与计数。
		sidebarClient.Boot()
		// 聊天列启动首拉：按恢复的模式选中会话并加载最近一页快照。
		chatClient.Boot(settingsService.Get().Mode)
		// 审批档位的中文短标签（Composer 只读展示；浮层归 07 号票）。
		chatState.SetApprovalLevel(levelLabel(settingsService.Get().ApprovalLevel))
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

// levelLabel 返回审批档位的中文短标签（Composer.tsx levelLabel 同表）。
func levelLabel(level string) string {
	switch level {
	case "strict":
		return "严格"
	case "readonly":
		return "只读"
	case "free":
		return "自由"
	default:
		return "分级"
	}
}

// wireOverlayHooks 把浮层体系的跨视图钩子接到侧栏编排与外壳 / 面板状态：
// 面板展开 / 收起、关闭当前 tab、打开文件归外壳与面板；新建会话、临时
// 会话、切模式、创建项目后续动作委托侧栏编排（acts 已含 05 号票的聊天
// 联动包装，切模式会同步聊天列兜底模式并加载选中会话）。
func wireOverlayHooks(shell *cwui.Shell, sidebarClient *cwsidebar.Client, acts *cwsidebar.Actions) {
	hooks := overlays.Hooks{
		TogglePanel: func() { shell.TogglePanel() },
		PanelOpen:   func() bool { return shell.PanelOpen() },
	}
	hooks.ClosePanelTab = func() { shell.ClosePanelActiveTab() }
	hooks.OpenFile = func(path string, line int) { shell.OpenFileAt(path, line) }
	hooks.NewThread = sidebarClient.CreateThreadInActive
	hooks.NewTempThread = acts.CreateTempThread
	hooks.SetMode = func(mode string) { acts.SwitchMode(cwsidebar.NormalizeMode(mode)) }
	hooks.ProjectCreated = func(workspaceID string) { sidebarClient.CreateThreadIn(workspaceID) }
	overlays.BindHooks(hooks)
}

// confirmDeleteDialog 返回侧栏删除确认（07 号票浮层体系）：应用内危险
// 样式确认框替代 04 号票的原生 Dialog.Message；回调已在主线程执行，
// 与侧栏 deliver 的线程约定一致。
func confirmDeleteDialog() cwsidebar.ConfirmFn {
	return func(title, message, detail string, onDone func(ok bool)) {
		overlays.Confirm(nil, overlays.ConfirmOpts{
			Title:       title,
			Description: message,
			Detail:      detail,
			ConfirmText: "删除",
			Danger:      true,
		}, onDone)
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
